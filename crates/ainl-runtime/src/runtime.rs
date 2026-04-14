//! Unified-graph orchestration runtime (v0.2): load, compile context, patch dispatch, record, emit, extract.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Instant;

use ainl_graph_extractor::GraphExtractorTask;
use ainl_memory::{
    AinlMemoryNode, AinlNodeType, GraphStore, GraphValidationReport, PersonaNode, ProceduralNode,
    RuntimeStateNode, SqliteGraphStore,
};
use ainl_persona::axes::default_axis_map;
use ainl_persona::{
    EvolutionEngine, PersonaAxis, PersonaSnapshot, RawSignal, INGEST_SCORE_EPSILON,
};
use ainl_semantic_tagger::infer_topic_tags;
use ainl_semantic_tagger::tag_tool_names;
use ainl_semantic_tagger::TagNamespace;
use uuid::Uuid;

use crate::adapters::{AdapterRegistry, GraphPatchAdapter};
use crate::engine::{
    AinlGraphArtifact, AinlRuntimeError, MemoryContext, PatchDispatchContext, PatchDispatchResult,
    PatchSkipReason, TurnInput, TurnOutcome, TurnPhase, TurnResult, TurnStatus, TurnWarning,
    EMIT_TO_EDGE,
};
use crate::graph_cell::{GraphCell, SqliteStoreRef};
use crate::hooks::{NoOpHooks, TurnHooks};
#[cfg(feature = "async")]
use crate::hooks::TurnHooksAsync;
use crate::RuntimeConfig;

/// Orchestrates ainl-memory, persona snapshot state, and graph extraction for one agent.
///
/// ## Evolution writes vs ArmaraOS / openfang-runtime
///
/// In production ArmaraOS, **openfang-runtime**’s `GraphMemoryWriter::run_persona_evolution_pass`
/// is the active writer of the evolution persona row ([`crate::EVOLUTION_TRAIT_NAME`]) to each
/// agent’s `ainl_memory.db`. This struct holds its own [`GraphExtractorTask`] and
/// [`EvolutionEngine`]. Calling [`Self::persist_evolution_snapshot`] or
/// [`Self::evolve_persona_from_graph_signals`] **concurrently** with that pass on the **same**
/// SQLite store is undefined (competing last-writer wins on the same persona node).
///
/// Prefer [`Self::with_evolution_writes_enabled(false)`] when a host embeds `AinlRuntime` alongside
/// openfang while openfang remains the sole evolution writer. [`Self::evolution_engine_mut`] can
/// still mutate in-memory axis state; calling [`EvolutionEngine::write_persona_node`] yourself
/// bypasses this guard and must be avoided in that configuration.
pub struct AinlRuntime {
    config: RuntimeConfig,
    memory: GraphCell,
    extractor: GraphExtractorTask,
    turn_count: u64,
    last_extraction_at_turn: u64,
    /// Current delegation depth for the active `run_turn` call chain (incremented per nested entry).
    current_depth: Arc<AtomicU32>,
    hooks: Box<dyn TurnHooks>,
    /// When `false`, [`Self::persist_evolution_snapshot`] and [`Self::evolve_persona_from_graph_signals`]
    /// return [`Err`] immediately so this runtime does not compete with another evolution writer
    /// (e.g. openfang’s post-turn pass) on the same DB. Default: `true`.
    evolution_writes_enabled: bool,
    persona_cache: Option<String>,
    /// Test hook: when set, the next scheduled extraction pass is treated as failed (`PartialSuccess`).
    #[doc(hidden)]
    test_force_extraction_failure: bool,
    /// Test hook: next fitness write-back from procedural dispatch fails without touching SQLite.
    #[doc(hidden)]
    test_force_fitness_write_failure: bool,
    /// Test hook: next runtime-state SQLite persist fails (non-fatal warning).
    #[doc(hidden)]
    test_force_runtime_state_write_failure: bool,
    adapter_registry: AdapterRegistry,
    /// Optional async hooks for [`Self::run_turn_async`] (see `async` feature).
    #[cfg(feature = "async")]
    hooks_async: Option<std::sync::Arc<dyn TurnHooksAsync>>,
}

impl AinlRuntime {
    pub fn new(config: RuntimeConfig, store: SqliteGraphStore) -> Self {
        let agent_id = config.agent_id.clone();
        let memory = GraphCell::new(store);
        let (init_turn_count, init_persona_cache, init_last_extraction_at_turn) =
            if agent_id.is_empty() {
                (0, None, 0)
            } else {
                match memory.read_runtime_state(&agent_id) {
                    Ok(Some(state)) => {
                        tracing::info!(
                            agent_id = %agent_id,
                            turn_count = state.turn_count,
                            "restored runtime state"
                        );
                        let persona_cache = state
                            .persona_snapshot_json
                            .as_ref()
                            .and_then(|json| serde_json::from_str::<String>(json).ok());
                        (
                            state.turn_count,
                            persona_cache,
                            state.last_extraction_at_turn,
                        )
                    }
                    Ok(None) => (0, None, 0),
                    Err(e) => {
                        tracing::warn!(error = %e, "failed to load runtime state — starting fresh");
                        (0, None, 0)
                    }
                }
            };
        Self {
            extractor: GraphExtractorTask::new(&agent_id),
            memory,
            config,
            turn_count: init_turn_count,
            last_extraction_at_turn: init_last_extraction_at_turn,
            current_depth: Arc::new(AtomicU32::new(0)),
            hooks: Box::new(NoOpHooks),
            evolution_writes_enabled: true,
            persona_cache: init_persona_cache,
            test_force_extraction_failure: false,
            test_force_fitness_write_failure: false,
            test_force_runtime_state_write_failure: false,
            adapter_registry: AdapterRegistry::new(),
            #[cfg(feature = "async")]
            hooks_async: None,
        }
    }

    /// Register a [`crate::PatchAdapter`] keyed by [`PatchAdapter::name`] (e.g. procedural patch label).
    pub fn register_adapter(&mut self, adapter: impl crate::PatchAdapter + 'static) {
        self.adapter_registry.register(adapter);
    }

    /// Install the reference [`GraphPatchAdapter`] as fallback for procedural patches without a
    /// label-specific adapter (see [`PatchDispatchContext`]).
    pub fn register_default_patch_adapters(&mut self) {
        self.register_adapter(GraphPatchAdapter::new());
    }

    /// Names of currently registered patch adapters.
    pub fn registered_adapters(&self) -> Vec<&str> {
        self.adapter_registry.registered_names()
    }

    #[doc(hidden)]
    pub fn test_turn_count(&self) -> u64 {
        self.turn_count
    }

    #[doc(hidden)]
    pub fn test_persona_cache(&self) -> Option<&str> {
        self.persona_cache.as_deref()
    }

    #[doc(hidden)]
    pub fn test_delegation_depth(&self) -> u32 {
        self.current_depth.load(Ordering::SeqCst)
    }

    #[doc(hidden)]
    pub fn test_set_delegation_depth(&mut self, depth: u32) {
        self.current_depth.store(depth, Ordering::SeqCst);
    }

    #[doc(hidden)]
    pub fn test_set_force_extraction_failure(&mut self, fail: bool) {
        self.test_force_extraction_failure = fail;
    }

    #[doc(hidden)]
    pub fn test_set_force_fitness_write_failure(&mut self, fail: bool) {
        self.test_force_fitness_write_failure = fail;
    }

    /// Test hook: access the graph extractor task for per-phase error injection.
    #[doc(hidden)]
    pub fn test_extractor_mut(&mut self) -> &mut GraphExtractorTask {
        &mut self.extractor
    }

    #[doc(hidden)]
    pub fn test_set_force_runtime_state_write_failure(&mut self, fail: bool) {
        self.test_force_runtime_state_write_failure = fail;
    }

    pub fn with_hooks(mut self, hooks: impl TurnHooks + 'static) -> Self {
        self.hooks = Box::new(hooks);
        self
    }

    /// Install async turn hooks ([`TurnHooksAsync`]) for [`Self::run_turn_async`].
    #[cfg(feature = "async")]
    pub fn with_hooks_async(mut self, hooks: std::sync::Arc<dyn TurnHooksAsync>) -> Self {
        self.hooks_async = Some(hooks);
        self
    }

    /// Set whether [`Self::persist_evolution_snapshot`] and [`Self::evolve_persona_from_graph_signals`]
    /// may write the evolution persona row. When `false`, those methods return [`Err`]. Chaining
    /// after [`Self::new`] is the supported way to disable writes for hosts that delegate evolution
    /// persistence elsewhere (see struct-level docs).
    pub fn with_evolution_writes_enabled(mut self, enabled: bool) -> Self {
        self.evolution_writes_enabled = enabled;
        self
    }

    fn require_evolution_writes_enabled(&self) -> Result<(), String> {
        if self.evolution_writes_enabled {
            Ok(())
        } else {
            Err(
                "ainl_runtime: evolution_writes_enabled is false — persist_evolution_snapshot and \
                 evolve_persona_from_graph_signals are disabled so this runtime does not compete \
                 with openfang-runtime GraphMemoryWriter::run_persona_evolution_pass on the same \
                 ainl_memory.db"
                    .to_string(),
            )
        }
    }

    /// Borrow the backing SQLite store (same connection as graph memory).
    ///
    /// When built with the `async` feature, this locks the in-runtime graph mutex for the lifetime
    /// of the returned guard (see [`SqliteStoreRef`]). That mutex is [`std::sync::Mutex`] (shared
    /// via [`std::sync::Arc`]), not `tokio::sync::Mutex`, so this helper remains usable from Tokio
    /// worker threads for quick reads without forcing an async lock API.
    pub fn sqlite_store(&self) -> SqliteStoreRef<'_> {
        self.memory.sqlite_ref()
    }

    /// Borrow the persona [`EvolutionEngine`] for this runtime’s agent.
    ///
    /// This is the **same** `EvolutionEngine` instance held by [`GraphExtractorTask::evolution_engine`].
    /// Scheduled [`GraphExtractorTask::run_pass`] continues to feed graph + pattern signals into it;
    /// hosts may also call [`EvolutionEngine::ingest_signals`], [`EvolutionEngine::correction_tick`],
    /// [`EvolutionEngine::extract_signals`], or [`EvolutionEngine::evolve`] directly, then
    /// [`Self::persist_evolution_snapshot`] to write the [`PersonaSnapshot`] row ([`crate::EVOLUTION_TRAIT_NAME`]).
    pub fn evolution_engine(&self) -> &EvolutionEngine {
        &self.extractor.evolution_engine
    }

    /// Mutable access to the persona [`EvolutionEngine`] (see [`Self::evolution_engine`]).
    ///
    /// Direct calls to [`EvolutionEngine::write_persona_node`] bypass [`Self::evolution_writes_enabled`].
    pub fn evolution_engine_mut(&mut self) -> &mut EvolutionEngine {
        &mut self.extractor.evolution_engine
    }

    /// Ingest explicit [`RawSignal`]s without reading the graph (wrapper for [`EvolutionEngine::ingest_signals`]).
    pub fn apply_evolution_signals(&mut self, signals: Vec<RawSignal>) -> usize {
        self.extractor.evolution_engine.ingest_signals(signals)
    }

    /// Apply a host correction nudge on one axis ([`EvolutionEngine::correction_tick`]).
    pub fn evolution_correction_tick(&mut self, axis: PersonaAxis, correction: f32) {
        self.extractor
            .evolution_engine
            .correction_tick(axis, correction);
    }

    /// Snapshot current axis EMA state and persist the evolution persona bundle to the store.
    ///
    /// Returns [`Err`] when [`Self::evolution_writes_enabled`] is `false` (see [`Self::with_evolution_writes_enabled`]).
    pub fn persist_evolution_snapshot(&mut self) -> Result<PersonaSnapshot, String> {
        self.require_evolution_writes_enabled()?;
        let snap = self.extractor.evolution_engine.snapshot();
        self.memory.with(|m| {
            self.extractor
                .evolution_engine
                .write_persona_node(m.sqlite_store(), &snap)
        })?;
        Ok(snap)
    }

    /// Graph-backed evolution only: extract signals from the store, ingest, write ([`EvolutionEngine::evolve`]).
    ///
    /// This does **not** run semantic `recurrence_count` bumps or the extractor’s `extract_pass`
    /// heuristics — use [`GraphExtractorTask::run_pass`] for the full scheduled pipeline.
    ///
    /// Returns [`Err`] when [`Self::evolution_writes_enabled`] is `false` (see [`Self::with_evolution_writes_enabled`]).
    pub fn evolve_persona_from_graph_signals(&mut self) -> Result<PersonaSnapshot, String> {
        self.require_evolution_writes_enabled()?;
        self.memory
            .with(|m| self.extractor.evolution_engine.evolve(m.sqlite_store()))
    }

    /// Boot: export + validate the agent subgraph.
    pub fn load_artifact(&self) -> Result<AinlGraphArtifact, String> {
        self.memory
            .with(|m| AinlGraphArtifact::load(m.sqlite_store(), &self.config.agent_id))
    }

    /// Same as [`Self::compile_memory_context_for`] with `user_message: None` (treated as empty for
    /// semantic ranking; see [`Self::compile_memory_context_for`]).
    pub fn compile_memory_context(&self) -> Result<MemoryContext, String> {
        self.compile_memory_context_for(None)
    }

    /// Build [`MemoryContext`] from the live store plus current extractor axis state.
    ///
    /// `relevant_semantic` is ranked from this `user_message` only (`ainl-semantic-tagger` topic tags
    /// + recurrence); `None` is treated as empty (high-recurrence fallback), not the latest episode text.
    pub fn compile_memory_context_for(
        &self,
        user_message: Option<&str>,
    ) -> Result<MemoryContext, String> {
        if self.config.agent_id.is_empty() {
            return Err("RuntimeConfig.agent_id must be set".to_string());
        }
        self.memory.with(|m| {
        let store = m.sqlite_store();
        let q = store.query(&self.config.agent_id);
        let recent_episodes = q.recent_episodes(5)?;
        let all_semantic = q.semantic_nodes()?;
        let relevant_semantic = self.relevant_semantic_nodes(
            user_message.unwrap_or(""),
            all_semantic,
            10,
        );
        let active_patches = q.active_patches()?;
        let persona_snapshot = persona_snapshot_if_evolved(&self.extractor);
        Ok(MemoryContext {
            recent_episodes,
            relevant_semantic,
            active_patches,
            persona_snapshot,
            compiled_at: chrono::Utc::now(),
        })
        })
    }

    /// Route `EMIT_TO` edges from an episode to hook targets (host implements [`TurnHooks::on_emit`]).
    pub fn route_emit_edges(
        &self,
        episode_id: Uuid,
        turn_output_payload: &serde_json::Value,
    ) -> Result<(), String> {
        self.memory.with(|m| {
            let store = m.sqlite_store();
            let neighbors = store
                .query(&self.config.agent_id)
                .neighbors(episode_id, EMIT_TO_EDGE)?;
            for n in neighbors {
                let target = emit_target_name(&n);
                self.hooks.on_emit(&target, turn_output_payload);
            }
            Ok(())
        })
    }

    /// Full single-turn orchestration (no LLM / no IR parse).
    pub fn run_turn(&mut self, input: TurnInput) -> Result<TurnOutcome, AinlRuntimeError> {
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

        if !self.config.enable_graph_memory {
            let memory_context = MemoryContext::default();
            let result = TurnResult {
                memory_context,
                status: TurnStatus::GraphMemoryDisabled,
                ..Default::default()
            };
            let outcome = TurnOutcome::Complete(result);
            self.hooks.on_turn_complete(&outcome);
            return Ok(outcome);
        }

        if self.config.agent_id.is_empty() {
            return Err(AinlRuntimeError::Message(
                "RuntimeConfig.agent_id must be set for run_turn".into(),
            ));
        }

        let span = tracing::info_span!(
            "ainl_runtime.run_turn",
            agent_id = %self.config.agent_id,
            turn = self.turn_count,
            depth = input.depth,
        );
        let _span_enter = span.enter();

        let validation: GraphValidationReport = self
            .memory
            .with(|m| m.sqlite_store().validate_graph(&self.config.agent_id))
            .map_err(AinlRuntimeError::from)?;
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
            let nodes = self
                .memory
                .with(|m| {
                    m.sqlite_store()
                        .query(&self.config.agent_id)
                        .persona_nodes()
                })
                .map_err(AinlRuntimeError::from)?;
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
            "persona_phase"
        );

        let t_memory = Instant::now();
        let memory_context = self
            .compile_memory_context_for(Some(&input.user_message))
            .map_err(AinlRuntimeError::from)?;
        self.hooks.on_memory_context_ready(&memory_context);
        tracing::debug!(
            target: "ainl_runtime",
            duration_ms = t_memory.elapsed().as_millis() as u64,
            episode_count = memory_context.recent_episodes.len(),
            semantic_count = memory_context.relevant_semantic.len(),
            patch_count = memory_context.active_patches.len(),
            "memory_context"
        );

        let t_patches = Instant::now();
        let patch_dispatch_results = if self.config.enable_graph_memory {
            self.dispatch_patches_collect(
                &memory_context.active_patches,
                &input.frame,
                &mut turn_warnings,
            )
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
                "patch_dispatch"
            );
        }
        tracing::debug!(
            target: "ainl_runtime",
            duration_ms = t_patches.elapsed().as_millis() as u64,
            "patch_dispatch_phase"
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
            return Ok(outcome);
        }

        let t_episode = Instant::now();
        let tools_canonical = normalize_tools_for_episode(&input.tools_invoked);
        let episode_id = match self.memory.with(|m| {
            record_turn_episode(m, &self.config.agent_id, &input, &tools_canonical)
        }) {
            Ok(id) => id,
            Err(e) => {
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
            "episode_record"
        );

        if !episode_id.is_nil() {
            for &tid in &input.emit_targets {
                if let Err(e) = self.memory.with(|m| {
                    m.sqlite_store()
                        .insert_graph_edge_checked(episode_id, tid, EMIT_TO_EDGE)
                }) {
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
        if let Err(e) = self.route_emit_edges(episode_id, &emit_payload) {
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
                    "extraction_pass"
                );
                (None, true)
            } else {
                let report = self
                    .memory
                    .with(|m| self.extractor.run_pass(m.sqlite_store()));
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
                        "ainl-graph-extractor pass completed (scheduled)"
                    );
                }
                self.hooks.on_extraction_complete(&report);
                self.persona_cache = None;
                tracing::debug!(
                    target: "ainl_runtime",
                    duration_ms = t_extract.elapsed().as_millis() as u64,
                    signals_ingested = report.signals_extracted as u64,
                    skipped = false,
                    "extraction_pass"
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
                "extraction_pass"
            );
            (None, false)
        };

        if let Err(e) = self.memory.with(|m| {
            try_export_graph_json_armaraos(m.sqlite_store(), &self.config.agent_id)
        }) {
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
            let write_res = if std::mem::take(&mut self.test_force_runtime_state_write_failure) {
                Err("injected runtime state write failure".to_string())
            } else {
                self.memory.with(|m| m.write_runtime_state(&state))
            };
            if let Err(e) = write_res {
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
        Ok(outcome)
    }

    /// Score and rank semantic nodes from `user_message` via `infer_topic_tags` (topic overlap +
    /// recurrence tiebreaker), or high-recurrence fallback when the message is empty or yields no topic tags.
    fn relevant_semantic_nodes(
        &self,
        user_message: &str,
        all_semantic: Vec<AinlMemoryNode>,
        limit: usize,
    ) -> Vec<AinlMemoryNode> {
        let user_tags = infer_topic_tags(user_message);
        let user_topics: HashSet<String> = user_tags
            .iter()
            .filter(|t| t.namespace == TagNamespace::Topic)
            .map(|t| t.value.to_lowercase())
            .collect();

        if user_message.trim().is_empty() || user_topics.is_empty() {
            return fallback_high_recurrence_semantic(all_semantic, limit);
        }

        let mut scored: Vec<(f32, u32, AinlMemoryNode)> = Vec::new();
        for n in all_semantic {
            let (score, rec) = match &n.node_type {
                AinlNodeType::Semantic { semantic } => {
                    let mut s = 0f32;
                    if let Some(cluster) = &semantic.topic_cluster {
                        for slug in cluster
                            .split([',', ';'])
                            .map(|s| s.trim().to_lowercase())
                            .filter(|s| !s.is_empty())
                        {
                            if user_topics.contains(&slug) {
                                s += 1.0;
                            }
                        }
                    }
                    if s == 0.0 {
                        for tag in &semantic.tags {
                            let tl = tag.to_lowercase();
                            if let Some(rest) = tl.strip_prefix("topic:") {
                                let slug = rest.trim().to_lowercase();
                                if user_topics.contains(&slug) {
                                    s = 0.5;
                                    break;
                                }
                            }
                        }
                    }
                    (s, semantic.recurrence_count)
                }
                _ => (0.0, 0),
            };
            scored.push((score, rec, n));
        }

        scored.sort_by(|a, b| {
            b.0.partial_cmp(&a.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.1.cmp(&a.1))
        });
        scored.into_iter().take(limit).map(|t| t.2).collect()
    }

    pub fn dispatch_patches(
        &mut self,
        patches: &[AinlMemoryNode],
        frame: &HashMap<String, serde_json::Value>,
    ) -> Vec<PatchDispatchResult> {
        let mut w = Vec::new();
        self.dispatch_patches_collect(patches, frame, &mut w)
    }

    fn dispatch_patches_collect(
        &mut self,
        patches: &[AinlMemoryNode],
        frame: &HashMap<String, serde_json::Value>,
        turn_warnings: &mut Vec<TurnWarning>,
    ) -> Vec<PatchDispatchResult> {
        let mut out = Vec::new();
        for node in patches {
            let res = self.dispatch_one_patch(node, frame);
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
        out
    }

    fn dispatch_one_patch(
        &mut self,
        node: &AinlMemoryNode,
        frame: &HashMap<String, serde_json::Value>,
    ) -> PatchDispatchResult {
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
                return PatchDispatchResult {
                    label: label_default,
                    patch_version: 0,
                    fitness_before: 0.0,
                    fitness_after: 0.0,
                    dispatched: false,
                    skip_reason: Some(PatchSkipReason::NotProcedural),
                    adapter_output: None,
                    adapter_name: None,
                };
            }
        };

        if pv == 0 {
            return PatchDispatchResult {
                label: label_src,
                patch_version: pv,
                fitness_before: fitness_opt.unwrap_or(0.5),
                fitness_after: fitness_opt.unwrap_or(0.5),
                dispatched: false,
                skip_reason: Some(PatchSkipReason::ZeroVersion),
                adapter_output: None,
                adapter_name: None,
            };
        }
        if retired {
            return PatchDispatchResult {
                label: label_src.clone(),
                patch_version: pv,
                fitness_before: fitness_opt.unwrap_or(0.5),
                fitness_after: fitness_opt.unwrap_or(0.5),
                dispatched: false,
                skip_reason: Some(PatchSkipReason::Retired),
                adapter_output: None,
                adapter_name: None,
            };
        }
        for key in &reads {
            if !frame.contains_key(key) {
                return PatchDispatchResult {
                    label: label_src.clone(),
                    patch_version: pv,
                    fitness_before: fitness_opt.unwrap_or(0.5),
                    fitness_after: fitness_opt.unwrap_or(0.5),
                    dispatched: false,
                    skip_reason: Some(PatchSkipReason::MissingDeclaredRead(key.clone())),
                    adapter_output: None,
                    adapter_name: None,
                };
            }
        }

        let patch_label = label_src.clone();
        let adapter_key = patch_label.as_str();
        let ctx = PatchDispatchContext {
            patch_label: adapter_key,
            node,
            frame,
        };
        let (adapter_output, adapter_name) = if let Some(adapter) = self
            .adapter_registry
            .get(adapter_key)
            .or_else(|| self.adapter_registry.get(GraphPatchAdapter::NAME))
        {
            let aname = adapter.name().to_string();
            match adapter.execute_patch(&ctx) {
                Ok(output) => {
                    tracing::debug!(
                        label = %patch_label,
                        adapter = %aname,
                        "adapter executed patch"
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
            }
        } else {
            (None, None)
        };

        let fitness_before = fitness_opt.unwrap_or(0.5);
        let fitness_after = 0.2_f32 * 1.0 + 0.8 * fitness_before;

        let updated = match self.memory.with(|m| {
            let store = m.sqlite_store();
            store.read_node(node.id)
        }) {
            Ok(Some(mut n)) => {
                if let AinlNodeType::Procedural { ref mut procedural } = n.node_type {
                    procedural.fitness = Some(fitness_after);
                }
                n
            }
            Ok(None) => {
                return PatchDispatchResult {
                    label: label_src,
                    patch_version: pv,
                    fitness_before,
                    fitness_after: fitness_before,
                    dispatched: false,
                    skip_reason: Some(PatchSkipReason::MissingDeclaredRead("node_row".into())),
                    adapter_output,
                    adapter_name,
                };
            }
            Err(e) => {
                return PatchDispatchResult {
                    label: label_src,
                    patch_version: pv,
                    fitness_before,
                    fitness_after: fitness_before,
                    dispatched: false,
                    skip_reason: Some(PatchSkipReason::PersistFailed(e)),
                    adapter_output,
                    adapter_name,
                };
            }
        };

        if self.test_force_fitness_write_failure {
            self.test_force_fitness_write_failure = false;
            let e = "injected fitness write failure".to_string();
            return PatchDispatchResult {
                label: label_src.clone(),
                patch_version: pv,
                fitness_before,
                fitness_after: fitness_before,
                dispatched: false,
                skip_reason: Some(PatchSkipReason::PersistFailed(e)),
                adapter_output,
                adapter_name,
            };
        }

        if let Err(e) = self.memory.with(|m| m.write_node(&updated)) {
            return PatchDispatchResult {
                label: label_src.clone(),
                patch_version: pv,
                fitness_before,
                fitness_after: fitness_before,
                dispatched: false,
                skip_reason: Some(PatchSkipReason::PersistFailed(e)),
                adapter_output,
                adapter_name,
            };
        }

        self.hooks
            .on_patch_dispatched(label_src.as_str(), fitness_after);

        PatchDispatchResult {
            label: label_src,
            patch_version: pv,
            fitness_before,
            fitness_after,
            dispatched: true,
            skip_reason: None,
            adapter_output,
            adapter_name,
        }
    }
}

pub(crate) fn emit_target_name(n: &AinlMemoryNode) -> String {
    match &n.node_type {
        AinlNodeType::Persona { persona } => persona.trait_name.clone(),
        AinlNodeType::Procedural { procedural } => procedural_label(procedural),
        AinlNodeType::Semantic { semantic } => semantic.fact.chars().take(64).collect(),
        AinlNodeType::Episode { episodic } => episodic.turn_id.to_string(),
        AinlNodeType::RuntimeState { runtime_state } => {
            format!("runtime_state:{}", runtime_state.agent_id)
        }
    }
}

pub(crate) fn procedural_label(p: &ProceduralNode) -> String {
    if !p.label.is_empty() {
        p.label.clone()
    } else {
        p.pattern_name.clone()
    }
}

pub(crate) fn fallback_high_recurrence_semantic(
    all: Vec<AinlMemoryNode>,
    limit: usize,
) -> Vec<AinlMemoryNode> {
    let mut v: Vec<_> = all
        .into_iter()
        .filter(|n| {
            matches!(&n.node_type, AinlNodeType::Semantic { semantic } if semantic.recurrence_count >= 2)
        })
        .collect();
    v.sort_by(|a, b| {
        let ra = match &a.node_type {
            AinlNodeType::Semantic { semantic } => semantic.recurrence_count,
            _ => 0,
        };
        let rb = match &b.node_type {
            AinlNodeType::Semantic { semantic } => semantic.recurrence_count,
            _ => 0,
        };
        rb.cmp(&ra)
    });
    v.into_iter().take(limit).collect()
}

pub(crate) fn persona_snapshot_if_evolved(
    extractor: &GraphExtractorTask,
) -> Option<ainl_persona::PersonaSnapshot> {
    let snap = extractor.evolution_engine.snapshot();
    let defaults = default_axis_map(0.5);
    for axis in PersonaAxis::ALL {
        let s = snap.axes.get(&axis).map(|a| a.score).unwrap_or(0.5);
        let d = defaults.get(&axis).map(|a| a.score).unwrap_or(0.5);
        if (s - d).abs() > INGEST_SCORE_EPSILON {
            return Some(snap);
        }
    }
    None
}

pub(crate) fn compile_persona_from_nodes(
    nodes: &[AinlMemoryNode],
) -> Result<Option<String>, String> {
    if nodes.is_empty() {
        return Ok(None);
    }
    let mut lines = Vec::new();
    for n in nodes {
        if let AinlNodeType::Persona { persona } = &n.node_type {
            lines.push(format_persona_line(persona));
        }
    }
    if lines.is_empty() {
        Ok(None)
    } else {
        Ok(Some(lines.join("\n")))
    }
}

fn format_persona_line(p: &PersonaNode) -> String {
    format!(
        "- {} (strength {:.2}, layer {:?}, source {:?})",
        p.trait_name, p.strength, p.layer, p.source
    )
}

/// Canonical tool names for episodic storage: [`tag_tool_names`] → `TagNamespace::Tool` values,
/// deduplicated and sorted (lexicographic). Empty input yields `["turn"]` (same sentinel as before).
/// Refresh `{AINL_GRAPH_MEMORY_ARMARAOS_EXPORT}/{agent_id}_graph_export.json` when the env var is set.
pub(crate) fn try_export_graph_json_armaraos(
    store: &SqliteGraphStore,
    agent_id: &str,
) -> Result<(), String> {
    let trimmed = std::env::var("AINL_GRAPH_MEMORY_ARMARAOS_EXPORT").unwrap_or_default();
    let dir = trimmed.trim();
    if dir.is_empty() {
        return Ok(());
    }
    let dir_path = PathBuf::from(dir);
    std::fs::create_dir_all(&dir_path).map_err(|e| format!("export mkdir: {e}"))?;
    let path = dir_path.join(format!("{agent_id}_graph_export.json"));
    let snap = store.export_graph(agent_id)?;
    let v = serde_json::to_value(&snap).map_err(|e| format!("serialize: {e}"))?;
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&v).map_err(|e| format!("json encode: {e}"))?,
    )
    .map_err(|e| format!("write export: {e}"))?;
    Ok(())
}

pub(crate) fn normalize_tools_for_episode(tools_invoked: &[String]) -> Vec<String> {
    if tools_invoked.is_empty() {
        return vec!["turn".to_string()];
    }
    let tags = tag_tool_names(tools_invoked);
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for t in tags {
        if t.namespace == TagNamespace::Tool {
            seen.insert(t.value);
        }
    }
    if seen.is_empty() {
        vec!["turn".to_string()]
    } else {
        seen.into_iter().collect()
    }
}

pub(crate) fn record_turn_episode(
    memory: &ainl_memory::GraphMemory,
    agent_id: &str,
    input: &TurnInput,
    tools_invoked_canonical: &[String],
) -> Result<Uuid, String> {
    let turn_id = Uuid::new_v4();
    let timestamp = chrono::Utc::now().timestamp();
    let tools = tools_invoked_canonical.to_vec();
    let mut node = AinlMemoryNode::new_episode(
        turn_id,
        timestamp,
        tools.clone(),
        None,
        input.trace_event.clone(),
    );
    node.agent_id = agent_id.to_string();
    if let AinlNodeType::Episode { ref mut episodic } = node.node_type {
        episodic.user_message = Some(input.user_message.clone());
        episodic.tools_invoked = tools;
    }
    memory.write_node(&node)?;
    Ok(node.id)
}

#[cfg(feature = "async")]
#[path = "runtime_async.rs"]
mod runtime_async_impl;
