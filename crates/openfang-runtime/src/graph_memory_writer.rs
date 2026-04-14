//! Bridge between openfang-runtime agent turns and ainl-memory graph store.
//!
//! Every agent turn that completes gets an EpisodeNode. Every tool call
//! result gets a SemanticNode (fact). Every A2A delegation gets an EpisodeNode
//! with delegation_to set.
//!
//! This is the wire that makes ainl-memory non-dead-code in the binary and
//! fulfills the architectural claim: execution IS the memory.

use ainl_graph_extractor::GraphExtractorTask;
use ainl_memory::{AinlMemoryNode, AinlNodeType, GraphMemory};
use ainl_persona::PersonaAxis;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, warn};
use uuid::Uuid;

/// Thread-safe wrapper around GraphMemory for use in the async agent loop.
#[derive(Clone)]
pub struct GraphMemoryWriter {
    inner: Arc<Mutex<GraphMemory>>,
    agent_id: String,
    on_write: Option<Arc<dyn Fn(String, String) + Send + Sync>>,
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
            if let Some(prev) = prev_id {
                if let AinlNodeType::Episode { ref mut episodic } = node.node_type {
                    episodic.follows_episode_id = Some(prev.to_string());
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
    pub async fn record_pattern(&self, name: &str, tool_sequence: Vec<String>, confidence: f32) {
        let res = {
            let inner = self.inner.lock().await;
            let mut node = AinlMemoryNode::new_procedural_tools(
                name.to_string(),
                tool_sequence,
                confidence,
            );
            node.agent_id = self.agent_id.clone();
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
        let res = {
            let inner = self.inner.lock().await;
            let mut node = AinlMemoryNode::new_fact(fact.clone(), confidence, source_turn_id);
            node.agent_id = self.agent_id.clone();
            if let AinlNodeType::Semantic { ref mut semantic } = node.node_type {
                semantic.source_episode_id = source_turn_id.to_string();
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

    /// Record an A2A delegation as an EpisodeNode with delegation_to set.
    pub async fn record_delegation(&self, target_agent_id: &str, tool_calls: Vec<String>) {
        let _ = self
            .record_turn(tool_calls, Some(target_agent_id.to_string()), None)
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

    /// Post-turn persona evolution: run `ainl-graph-extractor`’s [`GraphExtractorTask::run_pass`]
    /// (semantic recurrence, graph `RawSignal`s, `extract_pass`, ingest, write evolution persona).
    ///
    /// When the merged signal batch is empty (cold graph / no extractable cues), applies a
    /// lightweight [`ainl_persona::EvolutionEngine::correction_tick`] on every axis toward `0.5`
    /// and persists again so axes still drift slowly over time.
    ///
    /// Call after episode + fact writes so `GraphExtractor` sees fresh nodes. Intended to be
    /// `tokio::spawn`’d from the agent loop so the user-visible turn is not blocked.
    pub async fn run_persona_evolution_pass(&self) -> Result<(), String> {
        let inner = self.inner.lock().await;
        let store = inner.sqlite_store();
        let mut task = GraphExtractorTask::new(&self.agent_id);
        let report = task.run_pass(store)?;
        if report.merged_signals.is_empty() {
            for ax in PersonaAxis::ALL {
                task.evolution_engine.correction_tick(ax, 0.5);
            }
            let snapshot = task.evolution_engine.snapshot();
            task.evolution_engine.write_persona_node(store, &snapshot)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ainl_persona::EVOLUTION_TRAIT_NAME;
    use serde_json::json;

    #[tokio::test]
    async fn correction_tick_all_axes_no_panic() {
        let mut engine = ainl_persona::EvolutionEngine::new("tick-test-agent");
        for ax in ainl_persona::PersonaAxis::ALL {
            engine.correction_tick(ax, 0.5);
        }
        let snap = engine.snapshot();
        assert_eq!(snap.agent_id, "tick-test-agent");
    }

    #[tokio::test]
    async fn persona_evolution_writes_evolution_node_after_tool_turn() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("evo_persona.db");
        let memory = GraphMemory::new(&db_path).expect("open");
        let writer = GraphMemoryWriter {
            inner: Arc::new(Mutex::new(memory)),
            agent_id: "evo-agent".to_string(),
            on_write: None,
        };

        assert!(
            writer
                .record_turn(
                    vec!["shell_exec".into()],
                    None,
                    Some(json!({ "outcome": "success" })),
                )
                .await
                .is_some()
        );
        writer.run_persona_evolution_pass().await.expect("evolve");

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
    async fn persona_evolution_cycle_increments_over_two_passes() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("evo_cycle.db");
        let memory = GraphMemory::new(&db_path).expect("open");
        let writer = GraphMemoryWriter {
            inner: Arc::new(Mutex::new(memory)),
            agent_id: "evo-cycle-agent".to_string(),
            on_write: None,
        };

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
            )
            .await;
        writer.run_persona_evolution_pass().await.unwrap();
        let c1 = evolution_cycle(&writer).await;
        assert!(c1 >= 1);

        writer
            .record_turn(
                vec!["web_search".into()],
                None,
                Some(json!({ "outcome": "success" })),
            )
            .await;
        writer.run_persona_evolution_pass().await.unwrap();
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
            .record_turn(vec!["a".to_string()], None, None)
            .await
            .expect("ep1");
        let id2 = writer
            .record_turn(vec!["b".to_string()], None, None)
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
}
