//! Optional **failure recall** → memory-block segment (`sources-failure-warnings` feature).
//!
//! Search with [`ainl_failure_learning::search_failures_for_agent`], then turn hits into
//! a [`Segment::memory_block`](crate::segment::Segment::memory_block) for the context compiler.

use crate::segment::Segment;
use ainl_failure_learning::{format_failure_prevention_block, search_failures_for_agent, FailureRecallHit};
use ainl_memory::GraphMemory;
use std::cmp::min;

const MAX_QUERY_CHARS: usize = 240;

/// Build a memory-block segment from search hits, or `None` if `hits` is empty.
#[must_use]
pub fn memory_block_from_failure_hits(
    segment_label: impl Into<String>,
    block_title: &str,
    hits: &[FailureRecallHit],
) -> Option<Segment> {
    if hits.is_empty() {
        return None;
    }
    let body = format_failure_prevention_block(block_title, hits);
    if body.trim().is_empty() {
        return None;
    }
    Some(Segment::memory_block(segment_label, body))
}

/// FTS search for failures matching `user_query`, then build a memory-block segment.
///
/// `user_query` is truncated for FTS stability. Returns `None` when the query is empty, on search
/// error, or when there are no hits.
#[must_use]
pub fn memory_block_for_user_query(
    memory: &GraphMemory,
    agent_id: &str,
    user_query: &str,
    limit: usize,
) -> Option<Segment> {
    let q = user_query.trim();
    if q.is_empty() || agent_id.trim().is_empty() {
        return None;
    }
    let cap = min(limit.max(1), 20);
    let q_short: String = q.chars().take(MAX_QUERY_CHARS).collect();
    let hits = search_failures_for_agent(memory, agent_id, &q_short, cap).ok()?;
    memory_block_from_failure_hits("failure_recall_fts", "Failure recall (compose)", &hits)
}
