//! Persistence helpers for [`ainl-context-compiler`](https://docs.rs/ainl-context-compiler)
//! anchored summaries (Phase 6 of `SELF_LEARNING_INTEGRATION_MAP.md`).
//!
//! Anchored summaries are stored as **semantic-graph nodes** with a stable per-agent UUIDv5
//! id. Writes use `INSERT OR REPLACE` (via [`crate::store::SqliteGraphStore::write_node`]) so
//! repeated upserts overwrite the prior summary in place — no schema migration required.
//!
//! The summary payload itself is opaque JSON (the `AnchoredSummary` struct from the compiler
//! crate) stored in the [`crate::node::SemanticNode::fact`] field, tagged
//! `anchored_summary` for analytics filtering.
//!
//! Round-trip is therefore: caller serialises `AnchoredSummary` → `upsert_anchored_summary`
//! stores by stable id → `fetch_anchored_summary` returns the most recent payload string.

use crate::node::{AinlMemoryNode, AinlNodeType, MemoryCategory, SemanticNode};
use crate::GraphMemory;
use uuid::Uuid;

/// Tag applied to all anchored-summary semantic nodes for downstream filtering.
pub const ANCHORED_SUMMARY_TAG: &str = "anchored_summary";

/// UUIDv5 namespace used to derive stable per-agent ids.
///
/// Constant; do not change — collisions with prior on-disk rows would lose history.
const ANCHORED_SUMMARY_NS: Uuid = Uuid::from_bytes([
    0x9e, 0x4f, 0x4d, 0xb8, 0x1c, 0x1c, 0x4a, 0x6e, 0xa0, 0x77, 0xe2, 0x55, 0x12, 0x67, 0x3d, 0x2c,
]);

/// Stable UUIDv5 id for a given agent's anchored-summary row.
///
/// Pure function; safe to call without a memory handle (e.g. when constructing test fixtures).
#[must_use]
pub fn anchored_summary_id(agent_id: &str) -> Uuid {
    Uuid::new_v5(&ANCHORED_SUMMARY_NS, agent_id.as_bytes())
}

impl GraphMemory {
    /// Upsert a serialized anchored summary for `agent_id`.
    ///
    /// `summary_payload` should be a JSON-serialized `AnchoredSummary` from the
    /// `ainl-context-compiler` crate. Returns the stable node id (same value for repeated calls
    /// with the same `agent_id`).
    pub fn upsert_anchored_summary(
        &self,
        agent_id: &str,
        summary_payload: &str,
    ) -> Result<Uuid, String> {
        let id = anchored_summary_id(agent_id);
        let semantic = SemanticNode {
            fact: summary_payload.to_string(),
            confidence: 1.0,
            source_turn_id: id,
            topic_cluster: None,
            source_episode_id: String::new(),
            contradiction_ids: Vec::new(),
            last_referenced_at: chrono::Utc::now().timestamp() as u64,
            reference_count: 0,
            decay_eligible: false,
            tags: vec![ANCHORED_SUMMARY_TAG.to_string()],
            recurrence_count: 0,
            last_ref_snapshot: 0,
        };
        let node = AinlMemoryNode {
            id,
            memory_category: MemoryCategory::Semantic,
            importance_score: 1.0,
            agent_id: agent_id.to_string(),
            project_id: None,
            node_type: AinlNodeType::Semantic { semantic },
            edges: Vec::new(),
        };
        self.write_node(&node)?;
        Ok(id)
    }

    /// Fetch the most recent anchored-summary payload for `agent_id`, if any.
    ///
    /// Returns the raw JSON string (caller deserializes into `AnchoredSummary`).
    pub fn fetch_anchored_summary(&self, agent_id: &str) -> Result<Option<String>, String> {
        let id = anchored_summary_id(agent_id);
        let node = self.store().read_node(id)?;
        Ok(node.and_then(|n| match n.node_type {
            AinlNodeType::Semantic { semantic } => Some(semantic.fact),
            _ => None,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchored_summary_id_is_stable_per_agent() {
        let a = anchored_summary_id("agent-alpha");
        let b = anchored_summary_id("agent-alpha");
        let c = anchored_summary_id("agent-beta");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn upsert_then_fetch_roundtrip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("anchored_summary_smoke.db");
        let memory = GraphMemory::new(&db_path).expect("graph memory");

        // First upsert
        let payload_v1 = r#"{"schema_version":1,"sections":[{"id":"intent","label":"Intent","content":"v1"}],"token_estimate":1,"iteration":1}"#;
        let id1 = memory
            .upsert_anchored_summary("agent-rt", payload_v1)
            .expect("upsert v1");
        let fetched_v1 = memory
            .fetch_anchored_summary("agent-rt")
            .expect("fetch v1")
            .expect("payload present");
        assert_eq!(fetched_v1, payload_v1);

        // Second upsert REPLACES (same id)
        let payload_v2 = r#"{"schema_version":1,"sections":[{"id":"intent","label":"Intent","content":"v2"}],"token_estimate":1,"iteration":2}"#;
        let id2 = memory
            .upsert_anchored_summary("agent-rt", payload_v2)
            .expect("upsert v2");
        assert_eq!(id1, id2, "same agent must reuse the same node id");
        let fetched_v2 = memory
            .fetch_anchored_summary("agent-rt")
            .expect("fetch v2")
            .expect("payload present");
        assert_eq!(fetched_v2, payload_v2);
    }

    #[test]
    fn fetch_missing_returns_none() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("anchored_summary_missing.db");
        let memory = GraphMemory::new(&db_path).expect("graph memory");
        let result = memory
            .fetch_anchored_summary("nonexistent-agent")
            .expect("query ok");
        assert!(result.is_none());
    }

    #[test]
    fn distinct_agents_isolated() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("anchored_summary_isolation.db");
        let memory = GraphMemory::new(&db_path).expect("graph memory");
        memory
            .upsert_anchored_summary("agent-a", r#"{"a":1}"#)
            .expect("upsert a");
        memory
            .upsert_anchored_summary("agent-b", r#"{"b":2}"#)
            .expect("upsert b");
        let a = memory.fetch_anchored_summary("agent-a").unwrap().unwrap();
        let b = memory.fetch_anchored_summary("agent-b").unwrap().unwrap();
        assert_eq!(a, r#"{"a":1}"#);
        assert_eq!(b, r#"{"b":2}"#);
    }
}
