//! Bridge between openfang-runtime agent turns and ainl-memory graph store.
//!
//! Every agent turn that completes gets an EpisodeNode. Every tool call
//! result gets a SemanticNode (fact). Every A2A delegation gets an EpisodeNode
//! with delegation_to set.
//!
//! This is the wire that makes ainl-memory non-dead-code in the binary and
//! fulfills the architectural claim: execution IS the memory.

use ainl_memory::GraphMemory;
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
}

impl GraphMemoryWriter {
    /// Open or create the AINL graph memory DB for this agent.
    /// Path: ~/.armaraos/agents/<agent_id>/ainl_memory.db
    pub fn open(agent_id: &str) -> Result<Self, String> {
        let path = Self::db_path(agent_id)?;
        std::fs::create_dir_all(path.parent().unwrap())
            .map_err(|e| format!("create dir: {e}"))?;
        let memory = GraphMemory::new(&path)
            .map_err(|e| format!("open graph memory: {e}"))?;
        Ok(Self {
            inner: Arc::new(Mutex::new(memory)),
            agent_id: agent_id.to_string(),
        })
    }

    fn db_path(agent_id: &str) -> Result<PathBuf, String> {
        let home = dirs::home_dir().ok_or("no home dir")?;
        Ok(home
            .join(".armaraos")
            .join("agents")
            .join(agent_id)
            .join("ainl_memory.db"))
    }

    /// Record a completed agent turn as an EpisodeNode.
    pub async fn record_turn(
        &self,
        tool_calls: Vec<String>,
        delegation_to: Option<String>,
        trace_json: Option<serde_json::Value>,
    ) {
        let inner = self.inner.lock().await;
        match inner.write_episode(tool_calls.clone(), delegation_to.clone(), trace_json) {
            Ok(id) => debug!(
                agent_id = %self.agent_id,
                episode_id = %id,
                tools = ?tool_calls,
                delegation_to = ?delegation_to,
                "AINL graph memory: episode written"
            ),
            Err(e) => warn!(
                agent_id = %self.agent_id,
                error = %e,
                "AINL graph memory: failed to write episode"
            ),
        }
    }

    /// Record a semantic fact learned during a turn.
    pub async fn record_fact(
        &self,
        fact: String,
        confidence: f32,
        source_turn_id: Uuid,
    ) {
        let inner = self.inner.lock().await;
        match inner.write_fact(fact.clone(), confidence, source_turn_id) {
            Ok(id) => debug!(
                agent_id = %self.agent_id,
                fact_id = %id,
                confidence = confidence,
                "AINL graph memory: fact written"
            ),
            Err(e) => warn!(
                agent_id = %self.agent_id,
                error = %e,
                "AINL graph memory: failed to write fact"
            ),
        }
    }

    /// Record an A2A delegation as an EpisodeNode with delegation_to set.
    pub async fn record_delegation(&self, target_agent_id: &str, tool_calls: Vec<String>) {
        self.record_turn(
            tool_calls,
            Some(target_agent_id.to_string()),
            None,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_graph_memory_writer_records_episode() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("test_ainl.db");

        // Use from_path directly for testing
        let memory = GraphMemory::new(&db_path).expect("open");
        let writer = GraphMemoryWriter {
            inner: Arc::new(Mutex::new(memory)),
            agent_id: "test-agent".to_string(),
        };

        writer.record_turn(
            vec!["web_search".to_string(), "file_read".to_string()],
            None,
            None,
        ).await;

        let recent = writer.recall_recent(60).await;
        assert_eq!(recent.len(), 1);

        if let ainl_memory::AinlNodeType::Episode { tool_calls, .. } = &recent[0].node_type {
            assert_eq!(tool_calls.len(), 2);
            assert!(tool_calls.contains(&"web_search".to_string()));
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
        };

        writer.record_delegation("agent-B", vec!["delegate".to_string()]).await;

        let recent = writer.recall_recent(60).await;
        assert_eq!(recent.len(), 1);
        if let ainl_memory::AinlNodeType::Episode { delegation_to, .. } = &recent[0].node_type {
            assert_eq!(delegation_to, &Some("agent-B".to_string()));
        } else {
            panic!("wrong node type");
        }
    }

    #[tokio::test]
    async fn test_recall_persona_returns_persona_nodes() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("persona_test.db");
        let memory = GraphMemory::new(&db_path).expect("open");
        let writer = GraphMemoryWriter {
            inner: Arc::new(Mutex::new(memory)),
            agent_id: "test".to_string(),
        };

        {
            let inner = writer.inner.lock().await;
            inner
                .write_persona("prefers_brevity", 0.9, vec![])
                .expect("write persona");
        }

        let nodes = writer.recall_persona(3600).await;
        assert_eq!(nodes.len(), 1);
        if let ainl_memory::AinlNodeType::Persona {
            trait_name,
            strength,
            ..
        } = &nodes[0].node_type
        {
            assert_eq!(trait_name, "prefers_brevity");
            assert!((strength - 0.9).abs() < 0.01);
        } else {
            panic!("wrong node type");
        }
    }
}
