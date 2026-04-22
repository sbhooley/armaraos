//! FTS-backed search over `Failure` nodes, mapped to a small hit struct for prompts.

use ainl_memory::{AinlMemoryNode, AinlNodeType, GraphMemory};
use uuid::Uuid;

/// One failure match (newest-first order preserved from the store when possible).
#[derive(Debug, Clone, PartialEq)]
pub struct FailureRecallHit {
    /// Graph node id.
    pub id: Uuid,
    pub source: String,
    pub message: String,
    pub tool_name: Option<String>,
    /// Best-effort ranking score (1.0 today; room for rankers later).
    pub score: f32,
}

fn hit_from_node(node: AinlMemoryNode) -> Option<FailureRecallHit> {
    let AinlNodeType::Failure { failure } = node.node_type else {
        return None;
    };
    Some(FailureRecallHit {
        id: node.id,
        source: failure.source,
        message: failure.message,
        tool_name: failure.tool_name,
        score: 1.0,
    })
}

/// Search persisted failures for one agent (wraps `GraphMemory::search_failures_for_agent`).
pub fn search_failures_for_agent(
    memory: &GraphMemory,
    agent_id: &str,
    query: &str,
    limit: usize,
) -> Result<Vec<FailureRecallHit>, String> {
    let nodes = memory.search_failures_for_agent(agent_id, query, limit)?;
    Ok(nodes.into_iter().filter_map(hit_from_node).collect())
}
