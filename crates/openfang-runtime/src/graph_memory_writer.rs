//! Bridge between openfang-runtime agent turns and ainl-memory graph store.
//!
//! Every agent turn that completes gets an EpisodeNode. Every tool call
//! result gets a SemanticNode (fact). Every A2A delegation gets an EpisodeNode
//! with delegation_to set.
//!
//! This is the wire that makes ainl-memory non-dead-code in the binary and
//! fulfills the architectural claim: execution IS the memory.
//!
//! **Export:** [`GraphMemoryWriter::export_graph_json`] and [`GraphMemoryWriter::export_graph_json_for_agent`]
//! call into **ainl-memory**’s graph export (same JSON shape as `AgentGraphSnapshot`). CLI:
//! `openfang memory graph-export <agent> --output path.json`. Python `ainl_graph_memory` can seed reads
//! via [`armaraos_graph_memory_export_json_path`] / `AINL_GRAPH_MEMORY_ARMARAOS_EXPORT` (see **ainativelang**
//! `docs/adapters/AINL_GRAPH_MEMORY.md`).

#[cfg(feature = "ainl-extractor")]
use ainl_graph_extractor::GraphExtractorTask;
#[cfg(all(feature = "ainl-extractor", feature = "ainl-persona-evolution"))]
use ainl_persona::PersonaAxis;
use ainl_memory::{AinlMemoryNode, AinlNodeType, GraphMemory};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, warn};
use uuid::Uuid;

/// Result of [`GraphMemoryWriter::run_persona_evolution_pass`].
#[cfg(feature = "ainl-extractor")]
pub type PersonaEvolutionExtractionReport = ainl_graph_extractor::ExtractionReport;

#[cfg(not(feature = "ainl-extractor"))]
#[derive(Debug, Clone)]
pub struct PersonaEvolutionExtractionReport {
    pub agent_id: String,
}

#[cfg(not(feature = "ainl-extractor"))]
impl PersonaEvolutionExtractionReport {
    pub fn has_errors(&self) -> bool {
        false
    }
}

/// Horizon for “any persona row exists” checks (per-agent DB; long window ≈ all history).
pub(crate) const PERSONA_PRIOR_LOOKBACK_SECS: i64 = 60 * 60 * 24 * 365 * 100;

/// JSON snapshot path for the Python `ainl_graph_memory` bridge (auto-refresh after persona evolution).
///
/// If `AINL_GRAPH_MEMORY_ARMARAOS_EXPORT` is set, it names a **directory**; the file is
/// `{dir}/{agent_id}_graph_export.json`. If unset, uses the agent data directory next to `ainl_memory.db`:
/// `{openfang_home_dir()}/agents/{agent_id}/ainl_graph_memory_export.json`.
pub fn armaraos_graph_memory_export_json_path(agent_id: &str) -> PathBuf {
    let trimmed = std::env::var("AINL_GRAPH_MEMORY_ARMARAOS_EXPORT")
        .unwrap_or_default()
        .trim()
        .to_string();
    if !trimmed.is_empty() {
        PathBuf::from(trimmed).join(format!("{agent_id}_graph_export.json"))
    } else {
        openfang_types::config::openfang_home_dir()
            .join("agents")
            .join(agent_id)
            .join("ainl_graph_memory_export.json")
    }
}

/// Thread-safe wrapper around GraphMemory for use in the async agent loop.
#[derive(Clone)]
pub struct GraphMemoryWriter {
    pub(crate) inner: Arc<Mutex<GraphMemory>>,
    pub(crate) agent_id: String,
    pub(crate) on_write: Option<Arc<dyn Fn(String, String) + Send + Sync>>,
}

impl GraphMemoryWriter {
    /// Agent id this writer is bound to (same as graph rows `agent_id`).
    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    /// Open or create the AINL graph memory DB for this agent.
    /// Path: ~/.armaraos/agents/<agent_id>/ainl_memory.db
    pub fn open(agent_id: &str) -> Result<Self, String> {
        Self::open_with_notify(agent_id, None)
    }

    /// Same as [`Self::open`], with an optional hook invoked after each successful write.
    /// Arguments to the hook: `(agent_id, kind)` where `kind` is `episode`, `delegation`, or `fact`.
    pub fn open_with_notify(
        agent_id: &str,
        on_write: Option<Arc<dyn Fn(String, String) + Send + Sync>>,
    ) -> Result<Self, String> {
        let path = Self::db_path(agent_id)?;
        std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| format!("create dir: {e}"))?;
        let memory = GraphMemory::new(&path).map_err(|e| format!("open graph memory: {e}"))?;
        Ok(Self {
            inner: Arc::new(Mutex::new(memory)),
            agent_id: agent_id.to_string(),
            on_write,
        })
    }

    /// Test-only: bind a caller-supplied [`GraphMemory`] (e.g. temp-file SQLite) instead of `~/.armaraos/...`.
    #[cfg(test)]
    pub(crate) fn from_memory_for_tests(
        memory: GraphMemory,
        agent_id: impl Into<String>,
        on_write: Option<Arc<dyn Fn(String, String) + Send + Sync>>,
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(memory)),
            agent_id: agent_id.into(),
            on_write,
        }
    }

    fn db_path(agent_id: &str) -> Result<PathBuf, String> {
        // Must match kernel agent dirs (`KernelConfig::home_dir` / `openfang_home_dir`) and
        // `GET /api/graph-memory` — not `dirs::home_dir().join(".armaraos")` alone (breaks `ARMARAOS_HOME`).
        Ok(openfang_types::config::openfang_home_dir()
            .join("agents")
            .join(agent_id)
            .join("ainl_memory.db"))
    }

    fn fire_write_hook(&self, kind: &str) {
        if let Some(f) = &self.on_write {
            f(self.agent_id.clone(), kind.to_string());
        }
    }

    /// Record a completed agent turn as an EpisodeNode.
    ///
    /// On success, returns the new episode **node** id (same id space as
    /// [`Self::record_fact`] `source_turn_id` in existing call sites).
    pub async fn record_turn(
        &self,
        tool_calls: Vec<String>,
        delegation_to: Option<String>,
        trace_json: Option<serde_json::Value>,
        episode_tags: &[String],
    ) -> Option<Uuid> {
        let kind = if delegation_to.is_some() {
            "delegation"
        } else {
            "episode"
        };
        let res = {
            let inner = self.inner.lock().await;
            let prev_id = inner
                .recall_recent(60 * 60 * 24 * 7)
                .ok()
                .and_then(|nodes| nodes.into_iter().next())
                .map(|n| n.id);
            let turn_id = Uuid::new_v4();
            let timestamp = chrono::Utc::now().timestamp();
            let mut node = AinlMemoryNode::new_episode(
                turn_id,
                timestamp,
                tool_calls.clone(),
                delegation_to.clone(),
                trace_json,
            );
            node.agent_id = self.agent_id.clone();
            if let AinlNodeType::Episode { ref mut episodic } = node.node_type {
                if let Some(prev) = prev_id {
                    episodic.follows_episode_id = Some(prev.to_string());
                }
                for t in episode_tags {
                    if !episodic.tags.iter().any(|x| x == t) {
                        episodic.tags.push(t.clone());
                    }
                }
            }
            let new_id = node.id;
            match inner.write_node(&node) {
                Ok(()) => {
                    if let Some(prev) = prev_id {
                        let _ = inner.write_edge(new_id, prev, "follows");
                    }
                    Ok(new_id)
                }
                Err(e) => Err(e),
            }
        };
        match res {
            Ok(id) => {
                debug!(
                    agent_id = %self.agent_id,
                    episode_id = %id,
                    tools = ?tool_calls,
                    delegation_to = ?delegation_to,
                    "AINL graph memory: episode written"
                );
                self.fire_write_hook(kind);
                Some(id)
            }
            Err(e) => {
                warn!(
                    agent_id = %self.agent_id,
                    error = %e,
                    "AINL graph memory: failed to write episode"
                );
                None
            }
        }
    }

    /// Record a procedural pattern node (named tool workflow).
    ///
    /// When `trace_id` is set, it is stored on [`ainl_memory::ProceduralNode::trace_id`] for export / Python bridge correlation.
    pub async fn record_pattern(
        &self,
        name: &str,
        tool_sequence: Vec<String>,
        confidence: f32,
        trace_id: Option<String>,
    ) {
        let res = {
            let inner = self.inner.lock().await;
            let mut node = AinlMemoryNode::new_procedural_tools(
                name.to_string(),
                tool_sequence,
                confidence,
            );
            node.agent_id = self.agent_id.clone();
            if let AinlNodeType::Procedural { ref mut procedural } = node.node_type {
                procedural.trace_id = trace_id;
            }
            let id = node.id;
            inner.write_node(&node).map(|()| id)
        };
        match res {
            Ok(id) => {
                debug!(
                    agent_id = %self.agent_id,
                    pattern_id = %id,
                    pattern_name = %name,
                    "AINL graph memory: procedural pattern written"
                );
                self.fire_write_hook("procedural");
            }
            Err(e) => warn!(
                agent_id = %self.agent_id,
                error = %e,
                "AINL graph memory: failed to write procedural pattern"
            ),
        }
    }

    /// Record a semantic fact learned during a turn.
    pub async fn record_fact(&self, fact: String, confidence: f32, source_turn_id: Uuid) {
        self.record_fact_with_tags(fact, confidence, source_turn_id, &[])
            .await;
    }

    /// Like [`Self::record_fact`], with extra semantic tags (e.g. `trace_id:<uuid>` for correlation).
    pub async fn record_fact_with_tags(
        &self,
        fact: String,
        confidence: f32,
        source_turn_id: Uuid,
        extra_tags: &[String],
    ) {
        let res = {
            let inner = self.inner.lock().await;
            let mut node = AinlMemoryNode::new_fact(fact.clone(), confidence, source_turn_id);
            node.agent_id = self.agent_id.clone();
            if let AinlNodeType::Semantic { ref mut semantic } = node.node_type {
                semantic.source_episode_id = source_turn_id.to_string();
                for t in extra_tags {
                    if !semantic.tags.iter().any(|x| x == t) {
                        semantic.tags.push(t.clone());
                    }
                }
            }
            let id = node.id;
            inner.write_node(&node).map(|()| id)
        };
        match res {
            Ok(id) => {
                debug!(
                    agent_id = %self.agent_id,
                    fact_id = %id,
                    confidence = confidence,
                    "AINL graph memory: fact written"
                );
                self.fire_write_hook("fact");
            }
            Err(e) => warn!(
                agent_id = %self.agent_id,
                error = %e,
                "AINL graph memory: failed to write fact"
            ),
        }
    }

    /// Export this agent’s subgraph as JSON (same shape as `ainl_memory::AgentGraphSnapshot`).
    pub async fn export_graph_json(&self) -> Result<serde_json::Value, String> {
        let inner = self.inner.lock().await;
        let snap = inner.export_graph(&self.agent_id)?;
        serde_json::to_value(&snap).map_err(|e| format!("export_graph json: {e}"))
    }

    /// Read `~/.armaraos/agents/<agent_id>/ainl_memory.db` and export the agent subgraph (blocking-friendly).
    pub fn export_graph_json_for_agent(agent_id: &str) -> Result<serde_json::Value, String> {
        let path = Self::db_path(agent_id)?;
        if !path.is_file() {
            return Err(format!(
                "AINL graph memory DB not found at {} (expected per-agent SQLite)",
                path.display()
            ));
        }
        let memory = GraphMemory::new(&path).map_err(|e| format!("open graph memory: {e}"))?;
        let snap = memory.export_graph(agent_id)?;
        serde_json::to_value(&snap).map_err(|e| format!("export_graph json: {e}"))
    }

    /// Record an A2A delegation as an EpisodeNode with delegation_to set.
    pub async fn record_delegation(&self, target_agent_id: &str, tool_calls: Vec<String>) {
        let _ = self
            .record_turn(
                tool_calls,
                Some(target_agent_id.to_string()),
                None,
                &[],
            )
            .await;
    }

    /// Recall recent episodes (last N seconds).
    pub async fn recall_recent(&self, seconds_ago: i64) -> Vec<ainl_memory::AinlMemoryNode> {
        let inner = self.inner.lock().await;
        inner.recall_recent(seconds_ago).unwrap_or_default()
    }

    /// Recall PersonaNode entries from AINL graph memory within the last N seconds.
    pub async fn recall_persona(&self, seconds_ago: i64) -> Vec<ainl_memory::AinlMemoryNode> {
        let inner = self.inner.lock().await;
        inner
            .recall_by_type(ainl_memory::AinlNodeKind::Persona, seconds_ago)
            .unwrap_or_default()
    }

    /// Persona payloads for `agent_id` within the lookback window (same SQL path as
    /// [`Self::recall_persona`], filtered to matching rows).
    pub async fn recall_persona_for_agent(
        &self,
        agent_id: &str,
        lookback_secs: i64,
    ) -> Vec<ainl_memory::PersonaNode> {
        self.recall_persona(lookback_secs)
            .await
            .into_iter()
            .filter(|n| n.agent_id == agent_id)
            .filter_map(|n| match n.node_type {
                ainl_memory::AinlNodeType::Persona { persona } => Some(persona),
                _ => None,
            })
            .collect()
    }

    /// Post-turn persona evolution: run `ainl-graph-extractor`’s [`GraphExtractorTask::run_pass`]
    /// (semantic recurrence, graph `RawSignal`s, `extract_pass`, ingest, optional evolution write).
    ///
    /// When the merged signal batch is empty (cold graph / no extractable cues), applies a
    /// lightweight [`ainl_persona::EvolutionEngine::correction_tick`] on every axis toward `0.5`
    /// and persists **only if** this agent already had at least one [`PersonaNode`] in the store
    /// before this pass — skipping the meaningless neutral bootstrap on a brand-new graph.
    ///
    /// **ArmaraOS / openfang-runtime:** this method is the **active** evolution write path for
    /// agents backed by `~/.armaraos/agents/<id>/ainl_memory.db`. Any other host that persists the
    /// same [`ainl_persona::EVOLUTION_TRAIT_NAME`] row for that DB (for example the `ainl-runtime`
    /// crate’s `AinlRuntime::persist_evolution_snapshot` / `evolve_persona_from_graph_signals`) must
    /// coordinate so those calls are not concurrent with this pass, or disable the other writer
    /// (see the `ainl-runtime` README, including optional `async` / `run_turn_async` and why the graph
    /// uses `Arc<std::sync::Mutex<GraphMemory>>` instead of `tokio::sync::Mutex`).
    ///
    /// Call after episode + fact writes so `GraphExtractor` sees fresh nodes. Intended to be
    /// `tokio::spawn`’d from the agent loop so the user-visible turn is not blocked.
    pub async fn run_persona_evolution_pass(&self) -> PersonaEvolutionExtractionReport {
        #[cfg(feature = "ainl-extractor")]
        {
            let inner = self.inner.lock().await;
            let store = inner.sqlite_store();
            let had_persona_before_pass = inner
                .recall_by_type(ainl_memory::AinlNodeKind::Persona, PERSONA_PRIOR_LOOKBACK_SECS)
                .unwrap_or_default()
                .iter()
                .any(|n| n.agent_id == self.agent_id);
            let mut task = GraphExtractorTask::new(&self.agent_id);
            let mut report = task.run_pass(store);

            if let Some(ref e) = report.extract_error {
                warn!(
                    agent_id = %self.agent_id,
                    error = %e,
                    "graph extractor signal merge failed"
                );
            }
            if let Some(ref e) = report.pattern_error {
                warn!(
                    agent_id = %self.agent_id,
                    error = %e,
                    "graph extractor pattern persistence failed"
                );
            }
            if let Some(ref e) = report.persona_error {
                warn!(
                    agent_id = %self.agent_id,
                    error = %e,
                    "graph extractor persona evolution write failed"
                );
            }

            if report.merged_signals.is_empty() {
                #[cfg(feature = "ainl-persona-evolution")]
                {
                    for ax in PersonaAxis::ALL {
                        task.evolution_engine.correction_tick(ax, 0.5);
                    }
                    let snapshot = task.evolution_engine.snapshot();
                    if had_persona_before_pass {
                        if let Err(e) = task.evolution_engine.write_persona_node(store, &snapshot) {
                            warn!(
                                agent_id = %self.agent_id,
                                error = %e,
                                "graph extractor persona evolution write failed"
                            );
                            merge_persona_err(&mut report.persona_error, e);
                        }
                    }
                }
            }
            report
        }
        #[cfg(not(feature = "ainl-extractor"))]
        {
            PersonaEvolutionExtractionReport {
                agent_id: self.agent_id.clone(),
            }
        }
    }
}

#[cfg(feature = "ainl-extractor")]
fn merge_persona_err(slot: &mut Option<String>, e: String) {
    match slot {
        None => *slot = Some(e),
        Some(prev) => {
            prev.push_str("; correction write: ");
            prev.push_str(&e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ainl_persona::EVOLUTION_TRAIT_NAME;
    use serde_json::json;

    #[tokio::test]
    #[cfg(feature = "ainl-persona-evolution")]
    async fn correction_tick_all_axes_no_panic() {
        let mut engine = ainl_persona::EvolutionEngine::new("tick-test-agent");
        for ax in ainl_persona::PersonaAxis::ALL {
            engine.correction_tick(ax, 0.5);
        }
        let snap = engine.snapshot();
        assert_eq!(snap.agent_id, "tick-test-agent");
    }

    #[tokio::test]
    #[cfg(feature = "ainl-extractor")]
    async fn persona_evolution_writes_evolution_node_after_tool_turn() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("evo_persona.db");
        let memory = GraphMemory::new(&db_path).expect("open");
        let writer = GraphMemoryWriter {
            inner: Arc::new(Mutex::new(memory)),
            agent_id: "evo-agent".to_string(),
            on_write: None,
        };

        {
            let inner = writer.inner.lock().await;
            inner
                .write_persona("seed_trait", 0.6, vec![])
                .expect("seed baseline persona so evolution can persist on cold signal passes");
        }

        assert!(
            writer
                .record_turn(
                    vec!["shell_exec".into()],
                    None,
                    Some(json!({ "outcome": "success" })),
                    &[],
                )
                .await
                .is_some()
        );
        let evolve_report = writer.run_persona_evolution_pass().await;
        assert!(
            !evolve_report.has_errors(),
            "unexpected extraction errors: {evolve_report:?}"
        );

        let nodes = writer.recall_persona(3600).await;
        let evo = nodes.iter().find(|n| {
            matches!(
                &n.node_type,
                ainl_memory::AinlNodeType::Persona { persona }
                    if persona.trait_name == EVOLUTION_TRAIT_NAME
            )
        });
        assert!(
            evo.is_some(),
            "expected evolution PersonaNode, got {nodes:?}"
        );
        let ainl_memory::AinlNodeType::Persona { persona } = &evo.unwrap().node_type else {
            panic!();
        };
        assert!(
            !persona.axis_scores.is_empty(),
            "axis_scores should be populated"
        );
    }

    #[tokio::test]
    #[cfg(feature = "ainl-extractor")]
    async fn persona_evolution_cycle_increments_over_two_passes() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("evo_cycle.db");
        let memory = GraphMemory::new(&db_path).expect("open");
        let writer = GraphMemoryWriter {
            inner: Arc::new(Mutex::new(memory)),
            agent_id: "evo-cycle-agent".to_string(),
            on_write: None,
        };

        {
            let inner = writer.inner.lock().await;
            inner
                .write_persona("seed_trait", 0.6, vec![])
                .expect("seed baseline persona so evolution can persist on cold signal passes");
        }

        async fn evolution_cycle(w: &GraphMemoryWriter) -> u32 {
            let nodes = w.recall_persona(3600).await;
            nodes
                .iter()
                .find_map(|n| {
                    if let ainl_memory::AinlNodeType::Persona { persona } = &n.node_type {
                        (persona.trait_name == EVOLUTION_TRAIT_NAME)
                            .then_some(persona.evolution_cycle)
                    } else {
                        None
                    }
                })
                .unwrap_or(0)
        }

        writer
            .record_turn(
                vec!["shell_exec".into()],
                None,
                Some(json!({ "outcome": "success" })),
                &[],
            )
            .await;
        let r1 = writer.run_persona_evolution_pass().await;
        assert!(!r1.has_errors(), "{r1:?}");
        let c1 = evolution_cycle(&writer).await;
        assert!(c1 >= 1);

        writer
            .record_turn(
                vec!["web_search".into()],
                None,
                Some(json!({ "outcome": "success" })),
                &[],
            )
            .await;
        let r2_pass = writer.run_persona_evolution_pass().await;
        assert!(!r2_pass.has_errors(), "{r2_pass:?}");
        let c2 = evolution_cycle(&writer).await;
        assert!(
            c2 > c1,
            "evolution_cycle should increase, got {c1} then {c2}"
        );
    }

    #[tokio::test]
    async fn test_graph_memory_writer_records_episode() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("test_ainl.db");

        // Use from_path directly for testing
        let memory = GraphMemory::new(&db_path).expect("open");
        let writer = GraphMemoryWriter {
            inner: Arc::new(Mutex::new(memory)),
            agent_id: "test-agent".to_string(),
            on_write: None,
        };

        assert!(
            writer
                .record_turn(
                    vec!["web_search".to_string(), "file_read".to_string()],
                    None,
                    None,
                    &[],
                )
                .await
                .is_some()
        );

        let recent = writer.recall_recent(60).await;
        assert_eq!(recent.len(), 1);

        if let ainl_memory::AinlNodeType::Episode { episodic } = &recent[0].node_type {
            assert_eq!(episodic.tool_calls.len(), 2);
            assert!(episodic.tool_calls.contains(&"web_search".to_string()));
        } else {
            panic!("wrong node type");
        }
    }

    #[tokio::test]
    async fn test_graph_memory_writer_records_delegation() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("test_ainl_deleg.db");
        let memory = GraphMemory::new(&db_path).expect("open");
        let writer = GraphMemoryWriter {
            inner: Arc::new(Mutex::new(memory)),
            agent_id: "test-agent".to_string(),
            on_write: None,
        };

        writer
            .record_delegation("agent-B", vec!["delegate".to_string()])
            .await;

        let recent = writer.recall_recent(60).await;
        assert_eq!(recent.len(), 1);
        if let ainl_memory::AinlNodeType::Episode { episodic } = &recent[0].node_type {
            assert_eq!(episodic.delegation_to, Some("agent-B".to_string()));
        } else {
            panic!("wrong node type");
        }
    }

    #[tokio::test]
    async fn export_graph_json_round_trips_after_episode_write() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("export_rt.db");
        let memory = GraphMemory::new(&db_path).expect("open");
        let writer = GraphMemoryWriter {
            inner: Arc::new(Mutex::new(memory)),
            agent_id: "export-agent".to_string(),
            on_write: None,
        };
        let trace = json!({"agent_id": "export-agent", "trace_id": "tr-abc"});
        assert!(
            writer
                .record_turn(vec!["shell_exec".into()], None, Some(trace.clone()), &[])
                .await
                .is_some()
        );
        let v = writer.export_graph_json().await.expect("export");
        assert_eq!(v["agent_id"], "export-agent");
        assert_eq!(v["schema_version"], "1.0");
        let nodes = v["nodes"].as_array().expect("nodes array");
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0]["agent_id"], "export-agent");
        let nt = &nodes[0]["node_type"];
        assert_eq!(nt["type"], "episode");
        assert_eq!(nt["trace_event"], trace);
    }

    #[tokio::test]
    async fn test_record_turn_writes_follows_edge_between_episodes() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("follows_test.db");
        let memory = GraphMemory::new(&db_path).expect("open");
        let writer = GraphMemoryWriter {
            inner: Arc::new(Mutex::new(memory)),
            agent_id: "test-agent".to_string(),
            on_write: None,
        };

        let id1 = writer
            .record_turn(vec!["a".to_string()], None, None, &[])
            .await
            .expect("ep1");
        let id2 = writer
            .record_turn(vec!["b".to_string()], None, None, &[])
            .await
            .expect("ep2");

        let store = GraphMemory::new(&db_path).expect("reopen");
        let prev = store.store().walk_edges(id2, "follows").expect("walk");
        assert_eq!(prev.len(), 1);
        assert_eq!(prev[0].id, id1);
    }

    #[tokio::test]
    async fn test_recall_persona_returns_persona_nodes() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("persona_test.db");
        let memory = GraphMemory::new(&db_path).expect("open");
        let writer = GraphMemoryWriter {
            inner: Arc::new(Mutex::new(memory)),
            agent_id: "test".to_string(),
            on_write: None,
        };

        {
            let inner = writer.inner.lock().await;
            inner
                .write_persona("prefers_brevity", 0.9, vec![])
                .expect("write persona");
        }

        let nodes = writer.recall_persona(3600).await;
        assert_eq!(nodes.len(), 1);
        if let ainl_memory::AinlNodeType::Persona { persona } = &nodes[0].node_type {
            assert_eq!(persona.trait_name, "prefers_brevity");
            assert!((persona.strength - 0.9).abs() < 0.01);
        } else {
            panic!("wrong node type");
        }
    }

    #[tokio::test]
    async fn record_pattern_with_trace_id_stores_trace() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("record_pat_trace.db");
        let memory = GraphMemory::new(&db_path).expect("open");
        let writer = GraphMemoryWriter {
            inner: Arc::new(Mutex::new(memory)),
            agent_id: "pat-trace-agent".to_string(),
            on_write: None,
        };
        writer
            .record_pattern(
                "demo_pat",
                vec!["tool_a".into(), "tool_b".into()],
                0.85,
                Some("trace-z99".into()),
            )
            .await;
        let v = writer.export_graph_json().await.expect("export");
        let nodes = v["nodes"].as_array().expect("nodes");
        let proc_json = nodes
            .iter()
            .find(|n| n["node_type"]["type"] == "procedural")
            .expect("procedural in export");
        assert_eq!(proc_json["node_type"]["trace_id"], "trace-z99");
    }
}
