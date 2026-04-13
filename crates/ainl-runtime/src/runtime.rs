//! Unified-graph orchestration runtime (v0.2): load, compile context, patch dispatch, record, emit, extract.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::time::Instant;

use ainl_graph_extractor::GraphExtractorTask;
use ainl_memory::{
    AinlMemoryNode, AinlNodeType, GraphStore, GraphValidationReport, PersonaNode, ProceduralNode,
    RuntimeStateNode, SqliteGraphStore,
};
use ainl_persona::axes::default_axis_map;
use ainl_persona::{EvolutionEngine, PersonaAxis, PersonaSnapshot, RawSignal, INGEST_SCORE_EPSILON};
use ainl_semantic_tagger::infer_topic_tags;
use ainl_semantic_tagger::tag_tool_names;
use ainl_semantic_tagger::TagNamespace;
use uuid::Uuid;

use crate::engine::{
    AinlGraphArtifact, MemoryContext, PatchDispatchResult, PatchSkipReason, TurnInput, TurnOutcome,
    TurnOutput, EMIT_TO_EDGE,
};
use crate::hooks::{NoOpHooks, TurnHooks};
use crate::RuntimeConfig;

/// Orchestrates ainl-memory, persona snapshot state, and graph extraction for one agent.
pub struct AinlRuntime {
    config: RuntimeConfig,
    memory: ainl_memory::GraphMemory,
    extractor: GraphExtractorTask,
    turn_count: u32,
    last_extraction_turn: u32,
    delegation_depth: u32,
    hooks: Box<dyn TurnHooks>,
    persona_cache: Option<String>,
    /// Test hook: when set, the next scheduled extraction pass is treated as failed (`PartialSuccess`).
    #[doc(hidden)]
    test_force_extraction_failure: bool,
}

impl AinlRuntime {
    pub fn new(config: RuntimeConfig, store: SqliteGraphStore) -> Self {
        let agent_id = config.agent_id.clone();
        let memory = ainl_memory::GraphMemory::from_sqlite_store(store);
        let (init_turn_count, init_persona_cache, init_last_extraction_turn) = if agent_id.is_empty()
        {
            (0, None, 0)
        } else {
            match memory.sqlite_store().load_runtime_state(&agent_id) {
                Ok(Some(state)) => {
                    tracing::info!(
                        agent_id = %agent_id,
                        turn_count = state.turn_count,
                        "restored runtime state"
                    );
                    (
                        state.turn_count,
                        state.last_persona_prompt,
                        state.last_extraction_turn,
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
            last_extraction_turn: init_last_extraction_turn,
            delegation_depth: 0,
            hooks: Box::new(NoOpHooks),
            persona_cache: init_persona_cache,
            test_force_extraction_failure: false,
        }
    }

    #[doc(hidden)]
    pub fn test_turn_count(&self) -> u32 {
        self.turn_count
    }

    #[doc(hidden)]
    pub fn test_delegation_depth(&self) -> u32 {
        self.delegation_depth
    }

    #[doc(hidden)]
    pub fn test_set_delegation_depth(&mut self, depth: u32) {
        self.delegation_depth = depth;
    }

    #[doc(hidden)]
    pub fn test_set_force_extraction_failure(&mut self, fail: bool) {
        self.test_force_extraction_failure = fail;
    }

    pub fn with_hooks(mut self, hooks: impl TurnHooks + 'static) -> Self {
        self.hooks = Box::new(hooks);
        self
    }

    /// Borrow the backing SQLite store (same connection as graph memory).
    pub fn sqlite_store(&self) -> &SqliteGraphStore {
        self.memory.sqlite_store()
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
    pub fn evolution_engine_mut(&mut self) -> &mut EvolutionEngine {
        &mut self.extractor.evolution_engine
    }

    /// Ingest explicit [`RawSignal`]s without reading the graph (wrapper for [`EvolutionEngine::ingest_signals`]).
    pub fn apply_evolution_signals(&mut self, signals: Vec<RawSignal>) -> usize {
        self.extractor.evolution_engine.ingest_signals(signals)
    }

    /// Apply a host correction nudge on one axis ([`EvolutionEngine::correction_tick`]).
    pub fn evolution_correction_tick(&mut self, axis: PersonaAxis, correction: f32) {
        self.extractor.evolution_engine.correction_tick(axis, correction);
    }

    /// Snapshot current axis EMA state and persist the evolution persona bundle to the store.
    pub fn persist_evolution_snapshot(&mut self) -> Result<PersonaSnapshot, String> {
        let store = self.memory.sqlite_store();
        let snap = self.extractor.evolution_engine.snapshot();
        self.extractor
            .evolution_engine
            .write_persona_node(store, &snap)?;
        Ok(snap)
    }

    /// Graph-backed evolution only: extract signals from the store, ingest, write ([`EvolutionEngine::evolve`]).
    ///
    /// This does **not** run semantic `recurrence_count` bumps or the extractor’s `extract_pass`
    /// heuristics — use [`GraphExtractorTask::run_pass`] for the full scheduled pipeline.
    pub fn evolve_persona_from_graph_signals(&mut self) -> Result<PersonaSnapshot, String> {
        let store = self.memory.sqlite_store();
        self.extractor.evolution_engine.evolve(store)
    }

    /// Boot: export + validate the agent subgraph.
    pub fn load_artifact(&self) -> Result<AinlGraphArtifact, String> {
        AinlGraphArtifact::load(self.memory.sqlite_store(), &self.config.agent_id)
    }

    /// Same as [`Self::compile_memory_context_for`] with `user_message: None` (semantic relevance falls back
    /// to the latest episode’s `user_message` when present).
    pub fn compile_memory_context(&self) -> Result<MemoryContext, String> {
        self.compile_memory_context_for(None)
    }

    /// Build [`MemoryContext`] from the live store plus current extractor axis state.
    pub fn compile_memory_context_for(&self, user_message: Option<&str>) -> Result<MemoryContext, String> {
        if self.config.agent_id.is_empty() {
            return Err("RuntimeConfig.agent_id must be set".to_string());
        }
        let store = self.memory.sqlite_store();
        let q = store.query(&self.config.agent_id);
        let recent_episodes = q.recent_episodes(5)?;
        let effective_user = user_message
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| {
                recent_episodes.first().and_then(|n| {
                    if let AinlNodeType::Episode { episodic } = &n.node_type {
                        episodic.user_message.clone().filter(|m| !m.is_empty())
                    } else {
                        None
                    }
                })
            });
        let all_semantic = q.semantic_nodes()?;
        let relevant_semantic = match effective_user.as_deref() {
            Some(msg) => self.relevant_semantic_nodes(msg, all_semantic, 10),
            None => fallback_high_recurrence_semantic(all_semantic, 10),
        };
        let active_patches = q.active_patches()?;
        let persona_snapshot = persona_snapshot_if_evolved(&self.extractor);
        Ok(MemoryContext {
            recent_episodes,
            relevant_semantic,
            active_patches,
            persona_snapshot,
            compiled_at: chrono::Utc::now(),
        })
    }

    /// Route `EMIT_TO` edges from an episode to hook targets (host implements [`TurnHooks::on_emit`]).
    pub fn route_emit_edges(
        &self,
        episode_id: Uuid,
        turn_output_payload: &serde_json::Value,
    ) -> Result<(), String> {
        let store = self.memory.sqlite_store();
        let neighbors = store
            .query(&self.config.agent_id)
            .neighbors(episode_id, EMIT_TO_EDGE)?;
        for n in neighbors {
            let target = emit_target_name(&n);
            self.hooks.on_emit(&target, turn_output_payload);
        }
        Ok(())
    }

    /// Full single-turn orchestration (no LLM / no IR parse).
    pub fn run_turn(&mut self, input: TurnInput) -> Result<TurnOutput, String> {
        self.delegation_depth += 1;
        let rt_ptr = self as *mut Self;
        // Safety: `rt_ptr` aliases `self` for the synchronous body of `run_turn` only; the defer runs on
        // return before `self` is invalidated.
        // `scopeguard::defer!` only supports expression statements; use `guard` for the same drop semantics.
        let _depth_guard = scopeguard::guard((), |()| unsafe {
            if (*rt_ptr).delegation_depth > 0 {
                (*rt_ptr).delegation_depth -= 1;
            }
        });

        if self.delegation_depth > self.config.max_delegation_depth {
            let out = TurnOutput {
                outcome: TurnOutcome::DepthLimitExceeded,
                ..Default::default()
            };
            self.hooks.on_turn_complete(&out);
            return Ok(out);
        }

        if !self.config.enable_graph_memory {
            let memory_context = MemoryContext::default();
            let out = TurnOutput {
                memory_context,
                outcome: TurnOutcome::GraphMemoryDisabled,
                ..Default::default()
            };
            self.hooks.on_turn_complete(&out);
            return Ok(out);
        }

        if self.config.agent_id.is_empty() {
            return Err("RuntimeConfig.agent_id must be set for run_turn".to_string());
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
            .sqlite_store()
            .validate_graph(&self.config.agent_id)?;
        if !validation.is_valid {
            let mut msg = String::from("graph validation failed before turn");
            for d in &validation.dangling_edge_details {
                msg.push_str(&format!(
                    "; {} -> {} [{}]",
                    d.source_id, d.target_id, d.edge_type
                ));
            }
            return Err(msg);
        }

        self.hooks
            .on_artifact_loaded(&self.config.agent_id, validation.node_count);

        let mut patches_failed: Vec<String> = Vec::new();
        let mut warnings: Vec<String> = Vec::new();

        let t_persona = Instant::now();
        let persona_prompt_contribution = if let Some(cached) = &self.persona_cache {
            Some(cached.clone())
        } else {
            let nodes = self
                .memory
                .sqlite_store()
                .query(&self.config.agent_id)
                .persona_nodes()?;
            let compiled = compile_persona_from_nodes(&nodes)?;
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
        let memory_context = self.compile_memory_context_for(Some(&input.user_message))?;
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
                &mut patches_failed,
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
            let out = TurnOutput {
                patch_dispatch_results,
                memory_context,
                persona_prompt_contribution,
                steps_executed: dispatched_count,
                outcome: TurnOutcome::StepLimitExceeded {
                    steps_executed: dispatched_count,
                },
                ..Default::default()
            };
            self.hooks.on_turn_complete(&out);
            return Ok(out);
        }

        let t_episode = Instant::now();
        let tools_canonical = normalize_tools_for_episode(&input.tools_invoked);
        let episode_id = record_turn_episode(
            &self.memory,
            &self.config.agent_id,
            &input,
            &tools_canonical,
        )?;
        self.hooks.on_episode_recorded(episode_id);
        tracing::debug!(
            target: "ainl_runtime",
            duration_ms = t_episode.elapsed().as_millis() as u64,
            episode_id = %episode_id,
            "episode_record"
        );

        for &tid in &input.emit_targets {
            self.memory
                .sqlite_store()
                .insert_graph_edge_checked(episode_id, tid, EMIT_TO_EDGE)?;
        }

        let emit_payload = serde_json::json!({
            "episode_id": episode_id.to_string(),
            "user_message": input.user_message,
            "tools_invoked": tools_canonical,
            "persona_contribution": persona_prompt_contribution,
            "turn_count": self.turn_count.wrapping_add(1),
        });
        if let Err(e) = self.route_emit_edges(episode_id, &emit_payload) {
            tracing::warn!(error = %e, "emit routing failed — continuing");
            warnings.push(format!("emit_routing: {e}"));
        }

        self.turn_count = self.turn_count.wrapping_add(1);

        let should_extract = self.config.extraction_interval > 0
            && self
                .turn_count
                .saturating_sub(self.last_extraction_turn)
                >= self.config.extraction_interval;

        let t_extract = Instant::now();
        let (extraction_report, extraction_failed) = if should_extract {
            let force_fail = std::mem::take(&mut self.test_force_extraction_failure);

            let res = if force_fail {
                tracing::warn!(error = "test_forced", "extraction pass failed — continuing");
                tracing::debug!(
                    target: "ainl_runtime",
                    duration_ms = t_extract.elapsed().as_millis() as u64,
                    signals_ingested = 0u64,
                    skipped = false,
                    "extraction_pass"
                );
                (None, true)
            } else {
                match self.extractor.run_pass(self.memory.sqlite_store()) {
                    Ok(report) => {
                        tracing::info!(
                            agent_id = %report.agent_id,
                            signals_extracted = report.signals_extracted,
                            signals_applied = report.signals_applied,
                            semantic_nodes_updated = report.semantic_nodes_updated,
                            "ainl-graph-extractor pass completed (scheduled)"
                        );
                        self.hooks.on_extraction_complete(&report);
                        self.persona_cache = None;
                        tracing::debug!(
                            target: "ainl_runtime",
                            duration_ms = t_extract.elapsed().as_millis() as u64,
                            signals_ingested = report.signals_extracted as u64,
                            skipped = false,
                            "extraction_pass"
                        );
                        (Some(report), false)
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "extraction pass failed — continuing");
                        tracing::debug!(
                            target: "ainl_runtime",
                            duration_ms = t_extract.elapsed().as_millis() as u64,
                            signals_ingested = 0u64,
                            skipped = false,
                            "extraction_pass"
                        );
                        (None, true)
                    }
                }
            };
            self.last_extraction_turn = self.turn_count;
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

        let outcome = if extraction_failed
            || !patches_failed.is_empty()
            || !warnings.is_empty()
        {
            TurnOutcome::PartialSuccess {
                episode_recorded: true,
                extraction_failed,
                patches_failed,
                warnings,
            }
        } else {
            TurnOutcome::Success
        };

        let out = TurnOutput {
            episode_id,
            persona_prompt_contribution,
            memory_context,
            extraction_report,
            steps_executed: dispatched_count,
            outcome,
            patch_dispatch_results,
        };

        if !self.config.agent_id.is_empty() {
            let persist_state = RuntimeStateNode {
                agent_id: self.config.agent_id.clone(),
                turn_count: self.turn_count,
                last_extraction_turn: self.last_extraction_turn,
                last_persona_prompt: self.persona_cache.clone(),
                updated_at: chrono::Utc::now().to_rfc3339(),
            };
            if let Err(e) = self.memory.sqlite_store().save_runtime_state(&persist_state) {
                tracing::warn!(error = %e, "failed to persist runtime state — non-fatal");
            }
        }

        self.hooks.on_turn_complete(&out);
        Ok(out)
    }

    /// Score and rank semantic nodes for the current user text (`ainl-semantic-tagger` topic tags + recurrence).
    pub fn relevant_semantic_nodes(
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

        if user_topics.is_empty() {
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
                                let slug = rest.trim();
                                if user_topics.contains(slug) {
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
            if score > 0.0 {
                scored.push((score, rec, n));
            }
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
        let mut discarded = Vec::new();
        self.dispatch_patches_collect(patches, frame, &mut discarded)
    }

    fn dispatch_patches_collect(
        &mut self,
        patches: &[AinlMemoryNode],
        frame: &HashMap<String, serde_json::Value>,
        patches_failed: &mut Vec<String>,
    ) -> Vec<PatchDispatchResult> {
        let mut out = Vec::new();
        for node in patches {
            let res = self.dispatch_one_patch(node, frame);
            if let Some(PatchSkipReason::PersistFailed(ref e)) = res.skip_reason {
                tracing::warn!(label = %res.label, error = %e, "patch fitness write failed — continuing");
                patches_failed.push(res.label.clone());
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
                };
            }
        }

        let fitness_before = fitness_opt.unwrap_or(0.5);
        let fitness_after = 0.2_f32 * 1.0 + 0.8 * fitness_before;

        let store = self.memory.sqlite_store();
        let updated = match store.read_node(node.id) {
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
                    skip_reason: Some(PatchSkipReason::MissingDeclaredRead(
                        "node_row".into(),
                    )),
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
                };
            }
        };

        if let Err(e) = self.memory.write_node(&updated) {
            return PatchDispatchResult {
                label: label_src.clone(),
                patch_version: pv,
                fitness_before,
                fitness_after: fitness_before,
                dispatched: false,
                skip_reason: Some(PatchSkipReason::PersistFailed(e)),
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
        }
    }
}

fn emit_target_name(n: &AinlMemoryNode) -> String {
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

fn procedural_label(p: &ProceduralNode) -> String {
    if !p.label.is_empty() {
        p.label.clone()
    } else {
        p.pattern_name.clone()
    }
}

fn fallback_high_recurrence_semantic(all: Vec<AinlMemoryNode>, limit: usize) -> Vec<AinlMemoryNode> {
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

fn persona_snapshot_if_evolved(extractor: &GraphExtractorTask) -> Option<ainl_persona::PersonaSnapshot> {
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

fn compile_persona_from_nodes(nodes: &[AinlMemoryNode]) -> Result<Option<String>, String> {
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
fn normalize_tools_for_episode(tools_invoked: &[String]) -> Vec<String> {
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

fn record_turn_episode(
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
