//! Bounded snapshot construction — never calls `export_graph()`.

use crate::{AgentSnapshot, PolicyCaps, SnapshotPolicy, SNAPSHOT_SCHEMA_VERSION};
use ainl_memory::{AinlNodeKind, AinlMemoryNode, GraphMemory};

#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    #[error("graph memory: {0}")]
    Graph(String),
}

/// Build a bounded [`AgentSnapshot`] using typed recall queries (not full export).
pub fn build_snapshot(
    gm: &GraphMemory,
    agent_id: &str,
    policy: &SnapshotPolicy,
    tool_allowlist: Vec<String>,
    policy_caps: PolicyCaps,
) -> Result<AgentSnapshot, SnapshotError> {
    let filter_agent = |nodes: Vec<AinlMemoryNode>| -> Vec<AinlMemoryNode> {
        nodes
            .into_iter()
            .filter(|n| n.agent_id == agent_id || n.agent_id.is_empty())
            .collect()
    };

    let episodic_raw = gm
        .recall_by_type(AinlNodeKind::Episode, policy.episodic_window_secs)
        .map_err(SnapshotError::Graph)?;
    let episodic = filter_agent(episodic_raw)
        .into_iter()
        .take(policy.episodic_max)
        .collect();

    // Non-episodic window is operator-configurable; defaults to 30 days.
    let long_window_secs: i64 = policy.non_episodic_window_secs;
    let semantic_raw = gm
        .recall_by_type(AinlNodeKind::Semantic, long_window_secs)
        .map_err(SnapshotError::Graph)?;
    let semantic = filter_agent(semantic_raw)
        .into_iter()
        .take(policy.semantic_top_n)
        .collect();

    let procedural_raw = gm
        .recall_by_type(AinlNodeKind::Procedural, long_window_secs)
        .map_err(SnapshotError::Graph)?;
    let procedural = filter_agent(procedural_raw)
        .into_iter()
        .take(policy.procedural_top_n)
        .collect();

    let persona_raw = gm
        .recall_by_type(AinlNodeKind::Persona, long_window_secs)
        .map_err(SnapshotError::Graph)?;
    let persona = filter_agent(persona_raw)
        .into_iter()
        .take(policy.persona_top_n)
        .collect();

    Ok(AgentSnapshot {
        agent_id: agent_id.to_string(),
        snapshot_version: SNAPSHOT_SCHEMA_VERSION,
        persona,
        episodic,
        semantic,
        procedural,
        tool_allowlist,
        policy_caps,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PolicyCaps, SnapshotPolicy, SNAPSHOT_SCHEMA_VERSION};
    use uuid::Uuid;

    #[test]
    fn build_snapshot_includes_semantic_for_agent() {
        let path = std::env::temp_dir().join(format!("ainl_snapshot_build_{}.db", Uuid::new_v4()));
        let _ = std::fs::remove_file(&path);
        let gm = GraphMemory::new(&path).expect("open gm");
        let ag = "agent-snap-test";
        let mut fact = AinlMemoryNode::new_fact("hello world".into(), 0.9, Uuid::new_v4());
        fact.agent_id = ag.into();
        gm.write_node(&fact).expect("write");

        let policy = SnapshotPolicy::default();
        let caps = PolicyCaps::default();
        let snap = build_snapshot(&gm, ag, &policy, vec!["file_read".into()], caps.clone())
            .expect("snap");
        assert_eq!(snap.snapshot_version, SNAPSHOT_SCHEMA_VERSION);
        assert_eq!(snap.agent_id, ag);
        assert_eq!(snap.tool_allowlist, vec!["file_read"]);
        assert_eq!(snap.policy_caps, caps);
        assert_eq!(snap.semantic.len(), 1);
    }
}
