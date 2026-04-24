//! Async turn execution ([`super::AinlRuntime::run_turn_async`]): SQLite and graph reads/writes run
//! on Tokio’s blocking pool (`tokio::task::spawn_blocking`) while other runtime state stays on the
//! async caller. Graph memory is shared as `Arc<std::sync::Mutex<ainl_memory::GraphMemory>>`; we
//! intentionally do **not** use `tokio::sync::Mutex` for that inner lock so [`super::AinlRuntime::new`]
//! and short borrows such as [`super::AinlRuntime::sqlite_store`] are safe on any thread—including
//! Tokio worker threads used by `#[tokio::test]`—without the “block inside async context” failure
//! mode of `Mutex::blocking_lock` on an async mutex (see [Tokio mutex blocking_lock](https://docs.rs/tokio/latest/tokio/sync/struct.Mutex.html#method.blocking_lock)).

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use ainl_graph_extractor::GraphExtractorTask;
use ainl_memory::{
    AinlMemoryNode, AinlNodeType, GraphMemory, GraphStore, GraphValidationReport, RuntimeStateNode,
};
use uuid::Uuid;

use super::{
    compile_persona_from_nodes, emit_target_name, maybe_persist_trajectory_after_episode,
    normalize_tools_for_episode, persona_snapshot_if_evolved, procedural_label,
    record_turn_episode, try_export_graph_json_armaraos,
};
use crate::adapters::GraphPatchAdapter;
use crate::engine::{
    AinlRuntimeError, MemoryContext, PatchDispatchContext, PatchDispatchResult, PatchSkipReason,
    TurnInput, TurnOutcome, TurnPhase, TurnResult, TurnStatus, TurnWarning, EMIT_TO_EDGE,
};

async fn graph_spawn<T, F>(arc: Arc<Mutex<GraphMemory>>, f: F) -> Result<T, AinlRuntimeError>
where
    T: Send + 'static,
    F: FnOnce(&GraphMemory) -> Result<T, String> + Send + 'static,
{
    tokio::task::spawn_blocking(move || {
        let guard = arc.lock().expect("graph mutex poisoned");
        f(&guard)
    })
    .await
    .map_err(|e| AinlRuntimeError::AsyncJoinError(e.to_string()))?
    .map_err(AinlRuntimeError::from)
}

impl super::AinlRuntime {
    /// Async single-turn orchestration: graph SQLite I/O is offloaded with `spawn_blocking`.
    ///
    /// The graph remains `Arc<std::sync::Mutex<GraphMemory>>` (see crate `README.md`); this method
    /// does not switch that inner lock to `tokio::sync::Mutex`.
    ///
    /// Requires the `async` crate feature and a Tokio runtime (multi-thread recommended).
    pub async fn run_turn_async(
        &mut self,
        input: TurnInput,
    ) -> Result<TurnOutcome, AinlRuntimeError> {
        let depth = self.current_depth.fetch_add(1, Ordering::SeqCst);
        let cd = Arc::clone(&self.current_depth);
        let _depth_guard = scopeguard::guard((), move |()| {
            cd.fetch_sub(1, Ordering::SeqCst);
        });

        if depth >= self.config.max_delegation_depth {
            return Err(AinlRuntimeError::DelegationDepthExceeded {
                depth,
                max: self.config.max_delegation_depth,
            });
        }

        if let Some(ref hooks_async) = self.hooks_async {
            hooks_async.on_turn_start(&input).await;
        }

        if !self.config.enable_graph_memory {
            let memory_context = MemoryContext::default();
            let result = TurnResult {
                memory_context,
                status: TurnStatus::GraphMemoryDisabled,
                ..Default::default()
            };
            let outcome = TurnOutcome::Complete(result);
            self.hooks.on_turn_complete(&outcome);
            if let Some(ref hooks_async) = self.hooks_async {
                hooks_async.on_turn_complete(&outcome).await;
            }
            return Ok(outcome);
        }

        if self.config.agent_id.is_empty() {
            return Err(AinlRuntimeError::Message(
                "RuntimeConfig.agent_id must be set for run_turn".into(),
            ));
        }

        let span = tracing::info_span!(
            "ainl_runtime.run_turn_async",
            agent_id = %self.config.agent_id,
            turn = self.turn_count,
            depth = input.depth,
        );
        let _span_enter = span.enter();

        let arc = self.memory.shared_arc();
        let agent_id = self.config.agent_id.clone();

        let validation: GraphValidationReport = graph_spawn(Arc::clone(&arc), {
            let agent_id = agent_id.clone();
            move |m| m.sqlite_store().validate_graph(&agent_id)
        })
        .await?;

        if !validation.is_valid {
            let mut msg = String::from("graph validation failed before turn");
            for d in &validation.dangling_edge_details {
                msg.push_str(&format!(
                    "; {} -> {} [{}]",
                    d.source_id, d.target_id, d.edge_type
                ));
            }
            return Err(AinlRuntimeError::Message(msg));
        }

        self.hooks
            .on_artifact_loaded(&self.config.agent_id, validation.node_count);

        let mut turn_warnings: Vec<TurnWarning> = Vec::new();

        let t_persona = Instant::now();
        let persona_prompt_contribution = if let Some(cached) = &self.persona_cache {
            Some(cached.clone())
        } else {
            let nodes = graph_spawn(Arc::clone(&arc), {
                let agent_id = agent_id.clone();
                move |m| m.sqlite_store().query(&agent_id).persona_nodes()
            })
            .await?;
            let compiled = compile_persona_from_nodes(&nodes).map_err(AinlRuntimeError::from)?;
            self.persona_cache = compiled.clone();
            compiled
        };
        self.hooks
            .on_persona_compiled(persona_prompt_contribution.as_deref());
        tracing::debug!(
            target: "ainl_runtime",
            duration_ms = t_persona.elapsed().as_millis() as u64,
            has_contribution = persona_prompt_contribution.is_some(),
            "persona_phase_async"
        );

        let t_memory = Instant::now();
        let (recent_episodes, all_semantic, active_patches) = graph_spawn(Arc::clone(&arc), {
            let agent_id = agent_id.clone();
            move |m| {
                let store = m.sqlite_store();
                let q = store.query(&agent_id);
                let recent_episodes = q.recent_episodes(5)?;
                let all_semantic = q.semantic_nodes()?;
                let active_patches = q.active_patches()?;
                Ok((recent_episodes, all_semantic, active_patches))
            }
        })
        .await?;

        let relevant_semantic =
            self.relevant_semantic_nodes(input.user_message.as_str(), all_semantic, 10);
        let memory_context = MemoryContext {
            recent_episodes,
            relevant_semantic,
            active_patches,
            persona_snapshot: persona_snapshot_if_evolved(&self.extractor),
            compiled_at: chrono::Utc::now(),
        };

        self.hooks.on_memory_context_ready(&memory_context);
        tracing::debug!(
            target: "ainl_runtime",
            duration_ms = t_memory.elapsed().as_millis() as u64,
            episode_count = memory_context.recent_episodes.len(),
            semantic_count = memory_context.relevant_semantic.len(),
            patch_count = memory_context.active_patches.len(),
            "memory_context_async"
        );

        let t_patches = Instant::now();
        let patch_dispatch_results = if self.config.enable_graph_memory {
            self.dispatch_patches_collect_async(
                &memory_context.active_patches,
                &input.frame,
                &arc,
                &mut turn_warnings,
            )
            .await?
        } else {
            Vec::new()
        };
        for r in &patch_dispatch_results {
            tracing::debug!(
                target: "ainl_runtime",
                label = %r.label,
                dispatched = r.dispatched,
                fitness_before = r.fitness_before,
                fitness_after = r.fitness_after,
                "patch_dispatch_async"
            );
        }
        tracing::debug!(
            target: "ainl_runtime",
            duration_ms = t_patches.elapsed().as_millis() as u64,
            "patch_dispatch_phase_async"
        );

        let dispatched_count = patch_dispatch_results
            .iter()
            .filter(|r| r.dispatched)
            .count() as u32;
        if dispatched_count >= self.config.max_steps {
            let result = TurnResult {
                patch_dispatch_results,
                memory_context,
                persona_prompt_contribution,
                steps_executed: dispatched_count,
                status: TurnStatus::StepLimitExceeded {
                    steps_executed: dispatched_count,
                },
                ..Default::default()
            };
            let outcome = TurnOutcome::Complete(result);
            self.hooks.on_turn_complete(&outcome);
            if let Some(ref hooks_async) = self.hooks_async {
                hooks_async.on_turn_complete(&outcome).await;
            }
            return Ok(outcome);
        }

        let t_episode = Instant::now();
        let tools_canonical = normalize_tools_for_episode(&input.tools_invoked);
        let tools_for_episode = tools_canonical.clone();
        let input_clone = input.clone();
        let episode_id = match graph_spawn(Arc::clone(&arc), {
            let agent_id = agent_id.clone();
            move |m| record_turn_episode(m, &agent_id, &input_clone, &tools_for_episode)
        })
        .await
        {
            Ok(id) => id,
            Err(e) => {
                let e = e.message_str().unwrap_or("episode write").to_string();
                tracing::warn!(
                    phase = ?TurnPhase::EpisodeWrite,
                    error = %e,
                    "non-fatal turn write failed — continuing"
                );
                turn_warnings.push(TurnWarning {
                    phase: TurnPhase::EpisodeWrite,
                    error: e,
                });
                Uuid::nil()
            }
        };
        self.hooks.on_episode_recorded(episode_id);
        tracing::debug!(
            target: "ainl_runtime",
            duration_ms = t_episode.elapsed().as_millis() as u64,
            episode_id = %episode_id,
            "episode_record_async"
        );

        if !episode_id.is_nil() {
            for &tid in &input.emit_targets {
                let eid = episode_id;
                if let Err(e) = graph_spawn(Arc::clone(&arc), move |m| {
                    m.sqlite_store()
                        .insert_graph_edge_checked(eid, tid, EMIT_TO_EDGE)
                })
                .await
                {
                    let e = e.message_str().unwrap_or("edge").to_string();
                    tracing::warn!(
                        phase = ?TurnPhase::EpisodeWrite,
                        error = %e,
                        "non-fatal turn write failed — continuing"
                    );
                    turn_warnings.push(TurnWarning {
                        phase: TurnPhase::EpisodeWrite,
                        error: e,
                    });
                }
            }
        }

        let emit_payload = serde_json::json!({
            "episode_id": episode_id.to_string(),
            "user_message": input.user_message,
            "tools_invoked": tools_canonical,
            "persona_contribution": persona_prompt_contribution,
            "turn_count": self.turn_count.wrapping_add(1),
        });
        let emit_neighbors = graph_spawn(Arc::clone(&arc), {
            let agent_id = agent_id.clone();
            let eid = episode_id;
            move |m| {
                let store = m.sqlite_store();
                store.query(&agent_id).neighbors(eid, EMIT_TO_EDGE)
            }
        })
        .await;
        match emit_neighbors {
            Ok(neighbors) => {
                for n in neighbors {
                    let target = emit_target_name(&n);
                    self.hooks.on_emit(&target, &emit_payload);
                }
            }
            Err(e) => {
                let e = e.message_str().unwrap_or("emit").to_string();
                tracing::warn!(
                    phase = ?TurnPhase::EpisodeWrite,
                    error = %e,
                    "non-fatal turn write failed — continuing"
                );
                turn_warnings.push(TurnWarning {
                    phase: TurnPhase::EpisodeWrite,
                    error: format!("emit_routing: {e}"),
                });
            }
        }

        if !episode_id.is_nil() {
            let agent_id_traj = agent_id.clone();
            let input_traj = input.clone();
            let tools_traj = tools_canonical.clone();
            let patches_traj = patch_dispatch_results.clone();
            let eid = episode_id;
            match graph_spawn(Arc::clone(&arc), move |m| {
                maybe_persist_trajectory_after_episode(
                    m,
                    &agent_id_traj,
                    eid,
                    &tools_traj,
                    &patches_traj,
                    &input_traj,
                )
            })
            .await
            {
                Ok(()) => {}
                Err(e) => {
                    let e = e.to_string();
                    tracing::warn!(
                        phase = ?TurnPhase::EpisodeWrite,
                        error = %e,
                        "non-fatal trajectory persist failed — continuing"
                    );
                    turn_warnings.push(TurnWarning {
                        phase: TurnPhase::EpisodeWrite,
                        error: format!("trajectory_persist: {e}"),
                    });
                }
            }
        }

        self.turn_count = self.turn_count.wrapping_add(1);

        let should_extract = self.config.extraction_interval > 0
            && self.turn_count.saturating_sub(self.last_extraction_at_turn)
                >= self.config.extraction_interval as u64;

        let t_extract = Instant::now();
        let (extraction_report, _extraction_failed) = if should_extract {
            let force_fail = std::mem::take(&mut self.test_force_extraction_failure);

            let res = if force_fail {
                let e = "test_forced".to_string();
                tracing::warn!(
                    phase = ?TurnPhase::ExtractionPass,
                    error = %e,
                    "non-fatal turn write failed — continuing"
                );
                turn_warnings.push(TurnWarning {
                    phase: TurnPhase::ExtractionPass,
                    error: e,
                });
                tracing::debug!(
                    target: "ainl_runtime",
                    duration_ms = t_extract.elapsed().as_millis() as u64,
                    signals_ingested = 0u64,
                    skipped = false,
                    "extraction_pass_async"
                );
                (None, true)
            } else {
                let mem = Arc::clone(&arc);
                let placeholder = GraphExtractorTask::new(&agent_id);
                let mut task = std::mem::replace(&mut self.extractor, placeholder);
                let (task_back, report) = tokio::task::spawn_blocking(move || {
                    let g = mem.lock().expect("graph mutex poisoned");
                    let report = task.run_pass(g.sqlite_store());
                    (task, report)
                })
                .await
                .map_err(|e| AinlRuntimeError::AsyncJoinError(e.to_string()))?;
                self.extractor = task_back;

                if let Some(ref e) = report.extract_error {
                    tracing::warn!(
                        phase = ?TurnPhase::ExtractionPass,
                        error = %e,
                        "non-fatal turn write failed — continuing"
                    );
                    turn_warnings.push(TurnWarning {
                        phase: TurnPhase::ExtractionPass,
                        error: e.clone(),
                    });
                }
                if let Some(ref e) = report.pattern_error {
                    tracing::warn!(
                        phase = ?TurnPhase::PatternPersistence,
                        error = %e,
                        "non-fatal turn write failed — continuing"
                    );
                    turn_warnings.push(TurnWarning {
                        phase: TurnPhase::PatternPersistence,
                        error: e.clone(),
                    });
                }
                if let Some(ref e) = report.persona_error {
                    tracing::warn!(
                        phase = ?TurnPhase::PersonaEvolution,
                        error = %e,
                        "non-fatal turn write failed — continuing"
                    );
                    turn_warnings.push(TurnWarning {
                        phase: TurnPhase::PersonaEvolution,
                        error: e.clone(),
                    });
                }
                let extraction_failed = report.has_errors();
                if !extraction_failed {
                    tracing::info!(
                        agent_id = %report.agent_id,
                        signals_extracted = report.signals_extracted,
                        signals_applied = report.signals_applied,
                        semantic_nodes_updated = report.semantic_nodes_updated,
                        "ainl-graph-extractor pass completed (scheduled, async)"
                    );
                }
                self.hooks.on_extraction_complete(&report);
                self.persona_cache = None;
                tracing::debug!(
                    target: "ainl_runtime",
                    duration_ms = t_extract.elapsed().as_millis() as u64,
                    signals_ingested = report.signals_extracted as u64,
                    skipped = false,
                    "extraction_pass_async"
                );
                (Some(report), extraction_failed)
            };
            self.last_extraction_at_turn = self.turn_count;
            res
        } else {
            tracing::debug!(
                target: "ainl_runtime",
                duration_ms = t_extract.elapsed().as_millis() as u64,
                signals_ingested = 0u64,
                skipped = true,
                "extraction_pass_async"
            );
            (None, false)
        };

        if let Err(e) = graph_spawn(Arc::clone(&arc), {
            let agent_id = agent_id.clone();
            move |m| try_export_graph_json_armaraos(m.sqlite_store(), &agent_id)
        })
        .await
        {
            let e = e.message_str().unwrap_or("export").to_string();
            tracing::warn!(
                phase = ?TurnPhase::ExportRefresh,
                error = %e,
                "non-fatal turn write failed — continuing"
            );
            turn_warnings.push(TurnWarning {
                phase: TurnPhase::ExportRefresh,
                error: e,
            });
        }

        if !self.config.agent_id.is_empty() {
            let state = RuntimeStateNode {
                agent_id: self.config.agent_id.clone(),
                turn_count: self.turn_count,
                last_extraction_at_turn: self.last_extraction_at_turn,
                persona_snapshot_json: self
                    .persona_cache
                    .as_ref()
                    .and_then(|p| serde_json::to_string(p).ok()),
                updated_at: chrono::Utc::now().timestamp(),
            };
            let force_fail = std::mem::take(&mut self.test_force_runtime_state_write_failure);
            let write_res: Result<(), AinlRuntimeError> = if force_fail {
                Err(AinlRuntimeError::Message(
                    "injected runtime state write failure".into(),
                ))
            } else {
                graph_spawn(Arc::clone(&arc), move |m| m.write_runtime_state(&state)).await
            };
            if let Err(e) = write_res {
                let e = e.to_string();
                tracing::warn!(
                    phase = ?TurnPhase::RuntimeStatePersist,
                    error = %e,
                    "failed to persist runtime state — cadence will reset on next restart"
                );
                turn_warnings.push(TurnWarning {
                    phase: TurnPhase::RuntimeStatePersist,
                    error: e,
                });
            }
        }

        let result = TurnResult {
            episode_id,
            persona_prompt_contribution,
            memory_context,
            extraction_report,
            steps_executed: dispatched_count,
            patch_dispatch_results,
            status: TurnStatus::Ok,
            vitals_gate: input.vitals_gate.clone(),
            vitals_phase: input.vitals_phase.clone(),
            vitals_trust: input.vitals_trust,
        };

        let outcome = if turn_warnings.is_empty() {
            TurnOutcome::Complete(result)
        } else {
            TurnOutcome::PartialSuccess {
                result,
                warnings: turn_warnings,
            }
        };

        self.hooks.on_turn_complete(&outcome);
        if let Some(ref hooks_async) = self.hooks_async {
            hooks_async.on_turn_complete(&outcome).await;
        }
        Ok(outcome)
    }

    async fn dispatch_patches_collect_async(
        &mut self,
        patches: &[AinlMemoryNode],
        frame: &HashMap<String, serde_json::Value>,
        arc: &Arc<Mutex<GraphMemory>>,
        turn_warnings: &mut Vec<TurnWarning>,
    ) -> Result<Vec<PatchDispatchResult>, AinlRuntimeError> {
        let mut out = Vec::new();
        for node in patches {
            let res = self
                .dispatch_one_patch_async(node, frame, Arc::clone(arc))
                .await?;
            if let Some(PatchSkipReason::PersistFailed(ref e)) = res.skip_reason {
                tracing::warn!(
                    phase = ?TurnPhase::FitnessWriteBack,
                    error = %e,
                    "non-fatal turn write failed — continuing"
                );
                turn_warnings.push(TurnWarning {
                    phase: TurnPhase::FitnessWriteBack,
                    error: format!("{}: {}", res.label, e),
                });
            }
            out.push(res);
        }
        Ok(out)
    }

    async fn dispatch_one_patch_async(
        &mut self,
        node: &AinlMemoryNode,
        frame: &HashMap<String, serde_json::Value>,
        arc: Arc<Mutex<GraphMemory>>,
    ) -> Result<PatchDispatchResult, AinlRuntimeError> {
        let label_default = String::new();
        let (label_src, pv, retired, reads, fitness_opt) = match &node.node_type {
            AinlNodeType::Procedural { procedural } => (
                procedural_label(procedural),
                procedural.patch_version,
                procedural.retired,
                procedural.declared_reads.clone(),
                procedural.fitness,
            ),
            _ => {
                return Ok(PatchDispatchResult {
                    label: label_default,
                    patch_version: 0,
                    fitness_before: 0.0,
                    fitness_after: 0.0,
                    dispatched: false,
                    skip_reason: Some(PatchSkipReason::NotProcedural),
                    adapter_output: None,
                    adapter_name: None,
                    dispatch_duration_ms: 0,
                });
            }
        };

        if pv == 0 {
            return Ok(PatchDispatchResult {
                label: label_src,
                patch_version: pv,
                fitness_before: fitness_opt.unwrap_or(0.5),
                fitness_after: fitness_opt.unwrap_or(0.5),
                dispatched: false,
                skip_reason: Some(PatchSkipReason::ZeroVersion),
                adapter_output: None,
                adapter_name: None,
                dispatch_duration_ms: 0,
            });
        }
        if retired {
            return Ok(PatchDispatchResult {
                label: label_src.clone(),
                patch_version: pv,
                fitness_before: fitness_opt.unwrap_or(0.5),
                fitness_after: fitness_opt.unwrap_or(0.5),
                dispatched: false,
                skip_reason: Some(PatchSkipReason::Retired),
                adapter_output: None,
                adapter_name: None,
                dispatch_duration_ms: 0,
            });
        }
        for key in &reads {
            if !frame.contains_key(key) {
                return Ok(PatchDispatchResult {
                    label: label_src.clone(),
                    patch_version: pv,
                    fitness_before: fitness_opt.unwrap_or(0.5),
                    fitness_after: fitness_opt.unwrap_or(0.5),
                    dispatched: false,
                    skip_reason: Some(PatchSkipReason::MissingDeclaredRead(key.clone())),
                    adapter_output: None,
                    adapter_name: None,
                    dispatch_duration_ms: 0,
                });
            }
        }

        let patch_label = label_src.clone();
        let adapter_key = patch_label.as_str();
        let ctx = PatchDispatchContext {
            patch_label: adapter_key,
            node,
            frame,
        };
        let (adapter_output, adapter_name, dispatch_duration_ms) = if let Some(adapter) = self
            .adapter_registry
            .get(adapter_key)
            .or_else(|| self.adapter_registry.get(GraphPatchAdapter::NAME))
        {
            let aname = adapter.name().to_string();
            let t_exec = Instant::now();
            let (out, name) = match adapter.execute_patch(&ctx) {
                Ok(output) => {
                    tracing::debug!(
                        label = %patch_label,
                        adapter = %aname,
                        "adapter executed patch (async)"
                    );
                    (Some(output), Some(aname))
                }
                Err(e) => {
                    tracing::warn!(
                        label = %patch_label,
                        adapter = %aname,
                        error = %e,
                        "adapter execution failed — continuing as metadata dispatch"
                    );
                    (None, Some(aname))
                }
            };
            let ms = t_exec.elapsed().as_millis() as u64;
            (out, name, ms)
        } else {
            (None, None, 0u64)
        };

        let fitness_before = fitness_opt.unwrap_or(0.5);
        let fitness_after = 0.2_f32 * 1.0 + 0.8 * fitness_before;

        let nid = node.id;
        let updated = match graph_spawn(Arc::clone(&arc), move |m| {
            let store = m.sqlite_store();
            store.read_node(nid)
        })
        .await?
        {
            Some(mut n) => {
                if let AinlNodeType::Procedural { ref mut procedural } = n.node_type {
                    procedural.fitness = Some(fitness_after);
                }
                n
            }
            None => {
                return Ok(PatchDispatchResult {
                    label: label_src,
                    patch_version: pv,
                    fitness_before,
                    fitness_after: fitness_before,
                    dispatched: false,
                    skip_reason: Some(PatchSkipReason::MissingDeclaredRead("node_row".into())),
                    adapter_output,
                    adapter_name,
                    dispatch_duration_ms,
                });
            }
        };

        if self.test_force_fitness_write_failure {
            self.test_force_fitness_write_failure = false;
            let e = "injected fitness write failure".to_string();
            return Ok(PatchDispatchResult {
                label: label_src.clone(),
                patch_version: pv,
                fitness_before,
                fitness_after: fitness_before,
                dispatched: false,
                skip_reason: Some(PatchSkipReason::PersistFailed(e)),
                adapter_output,
                adapter_name,
                dispatch_duration_ms,
            });
        }

        let updated_clone = updated.clone();
        if let Err(e) = graph_spawn(arc, move |m| m.write_node(&updated_clone)).await {
            return Ok(PatchDispatchResult {
                label: label_src.clone(),
                patch_version: pv,
                fitness_before,
                fitness_after: fitness_before,
                dispatched: false,
                skip_reason: Some(PatchSkipReason::PersistFailed(
                    e.message_str().unwrap_or("write").to_string(),
                )),
                adapter_output,
                adapter_name,
                dispatch_duration_ms,
            });
        }

        self.hooks
            .on_patch_dispatched(label_src.as_str(), fitness_after);
        if let Some(ref hooks_async) = self.hooks_async {
            let hook_ctx = PatchDispatchContext {
                patch_label: adapter_key,
                node,
                frame,
            };
            let _ = hooks_async.on_patch_dispatched(&hook_ctx).await;
        }

        Ok(PatchDispatchResult {
            label: label_src,
            patch_version: pv,
            fitness_before,
            fitness_after,
            dispatched: true,
            skip_reason: None,
            adapter_output,
            adapter_name,
            dispatch_duration_ms,
        })
    }
}
