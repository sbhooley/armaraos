//! Update `SemanticNode::recurrence_count` from `reference_count` deltas (max +1 per pass).

use ainl_memory::{AinlNodeType, GraphStore, SqliteGraphStore};

/// For each semantic node for `agent_id`, if `reference_count` increased since the last extractor
/// snapshot, increment `recurrence_count` by **at most 1** and refresh `_last_ref_snapshot`.
///
/// Returns how many nodes were written because of a positive delta.
pub fn update_semantic_recurrence(
    store: &SqliteGraphStore,
    agent_id: &str,
) -> Result<usize, String> {
    let candidates = store.find_by_type("semantic")?;
    let mut updated = 0usize;

    for cand in candidates {
        if cand.agent_id != agent_id {
            continue;
        }
        let id = cand.id;
        let Some(mut node) = store.read_node(id)? else {
            continue;
        };

        let AinlNodeType::Semantic { ref mut semantic } = node.node_type else {
            continue;
        };

        let current_ref = semantic.reference_count;
        let snapshot = semantic.last_ref_snapshot;
        let delta = (current_ref as i64) - (snapshot as i64);
        if delta > 0 {
            semantic.recurrence_count = semantic.recurrence_count.saturating_add(1);
            semantic.last_ref_snapshot = current_ref;
            store.write_node(&node)?;
            updated += 1;
        }
    }

    Ok(updated)
}
