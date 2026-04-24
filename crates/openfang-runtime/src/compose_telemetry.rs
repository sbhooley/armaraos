//! Side-channel for whole-prompt compression telemetry from
//! [`ainl_context_compiler`].
//!
//! ## Why a side channel?
//!
//! The agent loop is the only place where the assembled system prompt, conversation
//! history, and current user message are co-located in scope. The kernel needs the
//! resulting *whole-prompt* token figures (not just the user-message-only numbers
//! computed by `crate::prompt_compressor::compress_with_metrics`) to record accurate
//! [`openfang_memory::usage::CompressionUsageRecord`] rows — which in turn drive the
//! "TOKENS USED" / "USD NOT SPENT" tiles on the dashboard.
//!
//! Threading three new optional fields through [`crate::agent_loop::AgentLoopResult`]
//! would touch 13 construction sites at six different indentation levels (the early-exit
//! and fallback returns in `run_agent_loop` and `run_agent_loop_streaming`). To keep the
//! M1 change reversible we use the same in-memory hand-off pattern as
//! [`crate::eco_telemetry::record_turn`]: the agent loop calls
//! [`record_compose_turn`] once per turn and the kernel calls [`take_compose_turn`]
//! immediately after `run_agent_loop` returns and before persisting the compression
//! event.
//!
//! ## Lifecycle
//!
//! - `record_compose_turn` is called at most once per turn, after
//!   `ainl_context_compiler::ContextCompiler::compose` returns successfully.
//! - `take_compose_turn` is **destructive**: each value can only be consumed by exactly
//!   one reader (the kernel's compression-event recorder). This prevents stale data from
//!   bleeding into a later turn if the recorder is skipped due to an error path.
//! - Concurrent turns for different agents do not collide — keying is by
//!   `agent_id` string.
//! - The map is bounded only by the number of live agents; missed reads naturally evict
//!   on the next `record_compose_turn` for the same agent (overwrite semantics).
//!
//! ## Threading model
//!
//! Backed by [`dashmap::DashMap`] inside a [`std::sync::OnceLock`], identical to the
//! `eco_telemetry` module. No extra dependencies, no `Mutex` contention on the hot path.

use crate::graph_memory_writer::GraphMemoryWriter;
use ainl_context_compiler::{ContextCompiler, Role as CcRole, Segment, SegmentKind};
use dashmap::DashMap;
use openfang_types::message::{ContentBlock, Message, MessageContent, Role as MsgRole};
use std::sync::Arc;
use std::sync::OnceLock;
use tracing::{debug, warn};

/// When set to `1` / `true` / `yes` / `on`, search [`GraphMemoryWriter`] for matching failures
/// and insert a [`Segment::memory_block`] after the system segment in the compose path. When
/// **unset** or any other value, the extra segment is **not** added (avoids duplicating
/// `graph_memory_context` failure text that may already be in the system prompt).
#[must_use]
fn compose_failure_recall_from_graph_enabled() -> bool {
    std::env::var("AINL_COMPOSE_FAILURE_RECALL")
        .map(|s| {
            matches!(
                s.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

/// When set to `1` / `true` / `yes` / `on`, whole-prompt telemetry uses **system = kernel/manifest
/// only** plus [`crate::graph_memory_context::PromptMemoryContext::to_memory_block_segments`]
/// as separate [`SegmentKind::MemoryBlock`] rows (after optional FTS failure segment). The LLM still
/// receives the legacy single string until M2+ migration changes the driver; this path measures the
/// compiler’s view of the window for dashboards.
#[must_use]
pub fn compose_graph_memory_as_segments_from_env() -> bool {
    std::env::var("AINL_COMPOSE_GRAPH_MEMORY_AS_SEGMENTS")
        .map(|s| {
            matches!(
                s.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

/// Load a failure-recall memory block for [`process_compose_telemetry_for_turn`] (async: locks graph
/// memory). Returns `None` when disabled, when there is no graph, or when FTS finds nothing.
pub async fn load_failure_recall_segment_for_compose(
    graph_memory: Option<&GraphMemoryWriter>,
    agent_id: &str,
    user_message: &str,
) -> Option<Segment> {
    if !compose_failure_recall_from_graph_enabled() {
        return None;
    }
    let gm = graph_memory?;
    let mem = gm.inner.lock().await;
    ainl_context_compiler::memory_block_for_user_query(&*mem, agent_id, user_message, 8)
}

/// When `1` / `true` / `yes` / `on`, replace the LLM-bound system + messages with the
/// `ainl_context_compiler` composed output (Phase 6 M2). Default: off (measurement-only M1).
#[must_use]
pub fn env_ainl_context_compose_apply() -> bool {
    std::env::var("AINL_CONTEXT_COMPOSE_APPLY")
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

/// When truthy, runs [`ainl_context_compiler::ContextCompiler`] with Tier 1
/// [`crate::context_compiler_summarizer::HeuristicAnchorSummarizer`] (no extra LLM; see README).
pub fn env_ainl_context_compose_summarizer() -> bool {
    std::env::var("AINL_CONTEXT_COMPOSE_SUMMARIZER")
        .map(|s| {
            matches!(
                s.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

/// When truthy, use [`ainl_context_compiler::PlaceholderEmbedder`] for M3 segment rerank in `compose()`.
pub fn env_ainl_context_compose_embed() -> bool {
    std::env::var("AINL_CONTEXT_COMPOSE_EMBED")
        .map(|s| {
            matches!(
                s.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn context_compiler_for_telemetry() -> ContextCompiler {
    let mut c = ContextCompiler::with_defaults();
    if env_ainl_context_compose_summarizer() {
        c = c.with_summarizer(Arc::new(
            crate::context_compiler_summarizer::HeuristicAnchorSummarizer,
        ));
    }
    if env_ainl_context_compose_embed() {
        c = c.with_embedder(Arc::new(ainl_context_compiler::PlaceholderEmbedder::new()));
    }
    c
}

fn map_cc_role_to_message(role: CcRole, content: String) -> Message {
    match role {
        CcRole::System => Message::system(content),
        CcRole::User => Message::user(content),
        CcRole::Assistant => Message::assistant(content),
        // Compiler tool segments are user-visible context in the window.
        CcRole::Tool => Message::user(content),
    }
}

/// Convert composed segments to OpenFang `system` + `messages` for the driver.
/// Returns `None` if nothing usable was produced.
fn map_composed_to_system_and_messages(
    composed: &ainl_context_compiler::ComposedPrompt,
    system_fallback: &str,
) -> Option<(String, Vec<Message>)> {
    let mut system_parts: Vec<String> = Vec::new();
    let mut out: Vec<Message> = Vec::new();
    for seg in &composed.segments {
        let t = seg.content.trim();
        if t.is_empty() {
            continue;
        }
        let c = seg.content.clone();
        if seg.kind == SegmentKind::SystemPrompt {
            system_parts.push(c);
            continue;
        }
        // Non-system segment: if role is System (shouldn't happen), fold into system parts.
        if seg.role == CcRole::System {
            system_parts.push(c);
            continue;
        }
        out.push(map_cc_role_to_message(seg.role, c));
    }
    let system = if system_parts.is_empty() {
        system_fallback.to_string()
    } else {
        system_parts.join("\n\n")
    };
    if system.is_empty() && out.is_empty() {
        return None;
    }
    Some((system, out))
}

/// Number of most recent turns kept verbatim before older turns are eligible for compaction.
/// Mirrors [`ainl_context_compiler::BudgetPolicy::recent_turns_keep_verbatim`] default — kept
/// in sync deliberately so the segment classification matches what the compiler scores against.
const RECENT_TURN_THRESHOLD: usize = 4;

/// Snapshot of one turn's whole-prompt compression telemetry.
///
/// Field semantics mirror [`ainl_context_compiler::ContextCompilerMetrics`] so the
/// kernel's recorder can pass these numbers straight to
/// [`openfang_memory::usage::CompressionUsageRecord`] (which is `u64`-typed for both
/// counters).
#[derive(Debug, Clone, Copy, Default)]
pub struct ComposeSnapshot {
    /// Estimated input tokens *before* compression, summed across all segments
    /// (system prompt, history, tool definitions/results, current user message).
    pub original_tokens: u64,
    /// Estimated input tokens *after* per-segment compression.
    pub compressed_tokens: u64,
}

impl ComposeSnapshot {
    /// Tokens removed by the compiler this turn (saturating; never negative).
    #[inline]
    #[must_use]
    pub fn tokens_saved(&self) -> u64 {
        self.original_tokens.saturating_sub(self.compressed_tokens)
    }
}

/// Active compiler tier captured for diagnostics + dashboard "tier" badge.
///
/// String-typed (rather than the compiler's `Tier` enum) because the kernel side
/// already serializes this directly into telemetry JSON / SQL columns.
#[derive(Debug, Clone)]
pub struct ComposeTurn {
    pub snapshot: ComposeSnapshot,
    /// e.g. `"heuristic"`, `"heuristic_summarization"`, `"heuristic_summarization_embedding"`.
    pub tier: String,
}

fn store() -> &'static DashMap<String, ComposeTurn> {
    static MAP: OnceLock<DashMap<String, ComposeTurn>> = OnceLock::new();
    MAP.get_or_init(DashMap::new)
}

/// Record one turn's whole-prompt compression telemetry. Overwrites any prior un-consumed
/// snapshot for the same `agent_id` (the most recent turn always wins).
pub fn record_compose_turn(
    agent_id: &str,
    original_tokens: u64,
    compressed_tokens: u64,
    tier: &str,
) {
    store().insert(
        agent_id.to_string(),
        ComposeTurn {
            snapshot: ComposeSnapshot {
                original_tokens,
                compressed_tokens,
            },
            tier: tier.to_string(),
        },
    );
}

/// Consume the most recent compose snapshot for `agent_id`, removing it from the
/// in-memory store. Returns `None` when no compose telemetry was recorded this turn
/// (e.g. compiler skipped, error path, or recorder ran twice).
#[must_use]
pub fn take_compose_turn(agent_id: &str) -> Option<ComposeTurn> {
    store().remove(agent_id).map(|(_, v)| v)
}

/// Map an `openfang-types` message role to the compiler's role enum.
fn map_role(role: MsgRole) -> CcRole {
    match role {
        MsgRole::System => CcRole::System,
        MsgRole::User => CcRole::User,
        MsgRole::Assistant => CcRole::Assistant,
    }
}

/// Build a fresh `Vec<Segment>` from a system prompt and the assembled LLM message list.
///
/// `latest_user_query` is intentionally accepted separately (rather than reading the last
/// `User` message) because callers usually have the **un-compressed** original in scope and
/// it produces better relevance scores than the compressed text already inside `messages`.
///
/// Classification rules:
/// - The system prompt → [`SegmentKind::SystemPrompt`].
/// - Each historical message: [`SegmentKind::RecentTurn`] vs [`SegmentKind::OlderTurn`] from the
///   last `RECENT_TURN_THRESHOLD` *messages* (message index, not per expanded segment).
/// - Plain [`MessageContent::Text`] → one turn segment. [`MessageContent::Blocks`] is expanded
///   into one segment per `Text` run (coalesced in message order) plus
///   [`SegmentKind::ToolResult`] for tool I/O, recent/older for `Thinking`, and a short
///   placeholder for images so the compiler accounts for their presence in the window.
/// - The trailing user message is replaced with [`SegmentKind::UserPrompt`] from
///   `latest_user_query` when set.
fn build_segments(
    system_prompt: &str,
    messages: &[Message],
    latest_user_query: &str,
) -> Vec<Segment> {
    let mut segments: Vec<Segment> = Vec::with_capacity(messages.len() + 2);
    if !system_prompt.is_empty() {
        segments.push(Segment::system_prompt(system_prompt));
    }

    history_core_segments(messages, latest_user_query, &mut segments);
    segments
}

/// History + current user, without a leading system segment.
fn build_history_user_segments(messages: &[Message], latest_user_query: &str) -> Vec<Segment> {
    let mut segments: Vec<Segment> = Vec::with_capacity(messages.len() + 1);
    history_core_segments(messages, latest_user_query, &mut segments);
    segments
}

/// The last message is almost always the current user turn; it is dropped from `history` and
/// re-added as [`SegmentKind::UserPrompt`] with `latest_user_query` when set.
fn history_core_segments(messages: &[Message], latest_user_query: &str, out: &mut Vec<Segment>) {
    let trailing_is_user = messages
        .last()
        .map(|m| m.role == MsgRole::User)
        .unwrap_or(false);
    let history_end = if trailing_is_user {
        messages.len().saturating_sub(1)
    } else {
        messages.len()
    };

    let history = &messages[..history_end];
    let recent_start = history.len().saturating_sub(RECENT_TURN_THRESHOLD);

    for (idx, msg) in history.iter().enumerate() {
        push_message_segments_for_history(msg, idx, history.len(), recent_start, out);
    }

    if !latest_user_query.is_empty() {
        out.push(Segment::user_prompt(latest_user_query));
    } else if trailing_is_user {
        if let Some(last) = messages.last() {
            let text = last.content.text_content();
            if !text.is_empty() {
                out.push(Segment::user_prompt(text));
            }
        }
    }
}

/// Expand one historical message to one or more compiler segments.
fn push_message_segments_for_history(
    msg: &Message,
    msg_idx: usize,
    history_len: usize,
    recent_start: usize,
    out: &mut Vec<Segment>,
) {
    let age_index = (history_len - 1 - msg_idx) as u32;
    let is_recent = msg_idx >= recent_start;

    match &msg.content {
        MessageContent::Text(s) => {
            if s.is_empty() {
                return;
            }
            let role = map_role(msg.role);
            out.push(if is_recent {
                Segment::recent_turn(role, s.as_str(), age_index)
            } else {
                Segment::older_turn(role, s.as_str(), age_index)
            });
        }
        MessageContent::Blocks(blocks) => {
            let mut text_run = String::new();
            let flush_text = |acc: &mut String, out: &mut Vec<Segment>| {
                if acc.trim().is_empty() {
                    acc.clear();
                    return;
                }
                let role = map_role(msg.role);
                let t = std::mem::take(acc);
                out.push(if is_recent {
                    Segment::recent_turn(role, t, age_index)
                } else {
                    Segment::older_turn(role, t, age_index)
                });
            };

            for b in blocks {
                match b {
                    ContentBlock::Text { text, .. } => {
                        if !text.is_empty() {
                            if !text_run.is_empty() {
                                text_run.push_str("\n\n");
                            }
                            text_run.push_str(text);
                        }
                    }
                    ContentBlock::ToolResult {
                        content, tool_name, ..
                    } => {
                        flush_text(&mut text_run, out);
                        let name = if tool_name.is_empty() {
                            "tool"
                        } else {
                            tool_name.as_str()
                        };
                        out.push(Segment::tool_result(name, content.as_str(), age_index));
                    }
                    ContentBlock::ToolUse { name, input, .. } => {
                        flush_text(&mut text_run, out);
                        out.push(Segment::tool_result(
                            if name.is_empty() {
                                "tool_use"
                            } else {
                                name.as_str()
                            },
                            input.to_string(),
                            age_index,
                        ));
                    }
                    ContentBlock::Thinking { thinking } => {
                        flush_text(&mut text_run, out);
                        if thinking.trim().is_empty() {
                            continue;
                        }
                        let role = CcRole::Assistant;
                        out.push(if is_recent {
                            Segment::recent_turn(role, thinking.as_str(), age_index)
                        } else {
                            Segment::older_turn(role, thinking.as_str(), age_index)
                        });
                    }
                    ContentBlock::Image { .. } => {
                        flush_text(&mut text_run, out);
                        out.push(if is_recent {
                            Segment::recent_turn(
                                map_role(msg.role),
                                "[image attachment]",
                                age_index,
                            )
                        } else {
                            Segment::older_turn(map_role(msg.role), "[image attachment]", age_index)
                        });
                    }
                    ContentBlock::Unknown => {
                        flush_text(&mut text_run, out);
                    }
                }
            }
            flush_text(&mut text_run, out);
        }
    }
}

/// `system_base` = manifest/kernel only. `graph` = graph-memory blocks. Order: system → optional
/// FTS `failure_recall` → `graph` → history → user.
fn build_segments_compiler_root(
    system_base: &str,
    graph_memory_blocks: &[Segment],
    messages: &[Message],
    latest_user_query: &str,
    failure_recall: Option<Segment>,
) -> Vec<Segment> {
    let mut segments: Vec<Segment> =
        Vec::with_capacity(2 + graph_memory_blocks.len() + messages.len());
    if !system_base.is_empty() {
        segments.push(Segment::system_prompt(system_base));
    }
    if let Some(fr) = failure_recall {
        insert_failure_recall_after_system(&mut segments, fr);
    }
    for g in graph_memory_blocks {
        segments.push(g.clone());
    }
    segments.extend(build_history_user_segments(messages, latest_user_query));
    segments
}

fn insert_failure_recall_after_system(segments: &mut Vec<Segment>, seg: Segment) {
    if matches!(
        segments.first().map(|s| s.kind),
        Some(SegmentKind::SystemPrompt)
    ) {
        segments.insert(1, seg);
    } else {
        segments.insert(0, seg);
    }
}

/// Run the Tier 0 [`ContextCompiler`], record [`record_compose_turn`], and optionally (M2) replace
/// `system_prompt` + `messages` with the composed window when
/// [`env_ainl_context_compose_apply`] is on and the transcript is plain text (no `Blocks` content).
pub fn process_compose_telemetry_for_turn(
    agent_id: &str,
    system_prompt: &mut String,
    messages: &mut Vec<Message>,
    latest_user_query: &str,
    failure_recall: Option<Segment>,
    graph_memory_block_segments: Option<Vec<Segment>>,
    system_for_compose_base: Option<String>,
) {
    let use_compiler_root = compose_graph_memory_as_segments_from_env()
        && system_for_compose_base.is_some()
        && graph_memory_block_segments
            .as_ref()
            .is_some_and(|v| !v.is_empty());

    let text_only = messages
        .iter()
        .all(|m| matches!(&m.content, MessageContent::Text(_)));
    let segments = if use_compiler_root {
        build_segments_compiler_root(
            system_for_compose_base.as_deref().unwrap_or(""),
            graph_memory_block_segments.as_deref().unwrap_or(&[]),
            messages,
            latest_user_query,
            failure_recall,
        )
    } else {
        let mut s = build_segments(system_prompt.as_str(), messages, latest_user_query);
        if let Some(fr) = failure_recall {
            insert_failure_recall_after_system(&mut s, fr);
        }
        s
    };
    if segments.is_empty() {
        debug!(agent_id, "compose_telemetry: no segments to score");
        return;
    }

    let compiler = context_compiler_for_telemetry();
    let composed = compiler.compose(latest_user_query, segments, None, None);
    let original = composed.telemetry.total_original_tokens as u64;
    let compressed = composed.telemetry.total_compressed_tokens as u64;
    let tier = composed.telemetry.tier.clone();

    if original == 0 {
        // Pathological — segments produced no token estimate. Don't record a misleading row.
        warn!(
            agent_id,
            "compose_telemetry: original token estimate was zero, skipping record"
        );
        return;
    }

    debug!(
        agent_id,
        original_tokens = original,
        compressed_tokens = compressed,
        tier = %tier,
        "compose_telemetry: recorded whole-prompt snapshot"
    );
    record_compose_turn(agent_id, original, compressed, &tier);

    if !env_ainl_context_compose_apply() || !text_only {
        if !text_only {
            debug!(
                agent_id,
                "compose M2: skipping prompt swap (non-text `MessageContent` in history)"
            );
        }
        return;
    }

    let fallback = system_prompt.clone();
    let Some((s, m)) = map_composed_to_system_and_messages(&composed, &fallback) else {
        debug!(agent_id, "compose M2: empty mapping, keeping host prompt");
        return;
    };
    if m.is_empty() {
        debug!(
            agent_id,
            "compose M2: no messages in composed output, keeping host prompt"
        );
        return;
    }
    *system_prompt = s;
    *messages = m;
}

/// Back-compat helper for call sites that only need M1 side-channel recording (no in-place apply).
pub fn measure_and_record(
    agent_id: &str,
    system_prompt: &str,
    messages: &[Message],
    latest_user_query: &str,
) {
    measure_and_record_with_options(
        agent_id,
        system_prompt,
        messages,
        latest_user_query,
        None,
        None,
        None,
    );
}

/// M1 side-channel with optional pre-built failure-recall segment (see
/// [`load_failure_recall_segment_for_compose`]) and optional graph-memory
/// blocks + base system (see [`compose_graph_memory_as_segments_from_env`]).
pub fn measure_and_record_with_options(
    agent_id: &str,
    system_prompt: &str,
    messages: &[Message],
    latest_user_query: &str,
    failure_recall: Option<Segment>,
    graph_memory_block_segments: Option<Vec<Segment>>,
    system_for_compose_base: Option<String>,
) {
    let mut s = system_prompt.to_string();
    let mut m = messages.to_vec();
    process_compose_telemetry_for_turn(
        agent_id,
        &mut s,
        &mut m,
        latest_user_query,
        failure_recall,
        graph_memory_block_segments,
        system_for_compose_base,
    );
}

/// Back-compat: failure segment only, legacy compose layout.
pub fn measure_and_record_with_failure_recall(
    agent_id: &str,
    system_prompt: &str,
    messages: &[Message],
    latest_user_query: &str,
    failure_recall: Option<Segment>,
) {
    measure_and_record_with_options(
        agent_id,
        system_prompt,
        messages,
        latest_user_query,
        failure_recall,
        None,
        None,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_then_take_returns_snapshot_once() {
        let id = "00000000-0000-0000-0000-0000000000a1";
        record_compose_turn(id, 1000, 400, "heuristic");

        let first = take_compose_turn(id).expect("first take should return value");
        assert_eq!(first.snapshot.original_tokens, 1000);
        assert_eq!(first.snapshot.compressed_tokens, 400);
        assert_eq!(first.snapshot.tokens_saved(), 600);
        assert_eq!(first.tier, "heuristic");

        assert!(
            take_compose_turn(id).is_none(),
            "second take should return None (consumed)"
        );
    }

    #[test]
    fn overwrite_semantics_keep_latest_only() {
        let id = "00000000-0000-0000-0000-0000000000a2";
        record_compose_turn(id, 100, 50, "heuristic");
        record_compose_turn(id, 200, 80, "heuristic_summarization");

        let snap = take_compose_turn(id).expect("snapshot present");
        assert_eq!(snap.snapshot.original_tokens, 200);
        assert_eq!(snap.snapshot.compressed_tokens, 80);
        assert_eq!(snap.tier, "heuristic_summarization");
    }

    #[test]
    fn agents_are_isolated() {
        let a = "00000000-0000-0000-0000-0000000000a3";
        let b = "00000000-0000-0000-0000-0000000000a4";
        record_compose_turn(a, 10, 5, "heuristic");
        record_compose_turn(b, 20, 7, "heuristic");

        let snap_a = take_compose_turn(a).expect("a present");
        let snap_b = take_compose_turn(b).expect("b present");
        assert_eq!(snap_a.snapshot.original_tokens, 10);
        assert_eq!(snap_b.snapshot.original_tokens, 20);
    }

    #[test]
    fn build_segments_classifies_recent_vs_older_turns() {
        // 6 history messages: indexes 0..1 should be older, 2..5 recent (RECENT_TURN_THRESHOLD = 4).
        // The trailing user message is dropped from history and re-added as UserPrompt.
        let history: Vec<Message> = vec![
            Message::user("very old user 1"),
            Message::assistant("very old assistant 1"),
            Message::user("recent user 1"),
            Message::assistant("recent assistant 1"),
            Message::user("recent user 2"),
            Message::assistant("recent assistant 2"),
            Message::user("current user query"),
        ];
        let segs = build_segments("system instructions here", &history, "current user query");

        // System prompt + 6 history segments + 1 UserPrompt (trailing user dropped from history).
        assert_eq!(
            segs.len(),
            8,
            "expected system + 6 history + 1 user_prompt, got {segs:?}"
        );
        assert_eq!(
            segs.first().expect("system seg").kind,
            ainl_context_compiler::SegmentKind::SystemPrompt
        );
        assert_eq!(
            segs.last().expect("user seg").kind,
            ainl_context_compiler::SegmentKind::UserPrompt
        );
        assert_eq!(segs.last().expect("user seg").content, "current user query");

        // Skip system + 2 oldest, then check recent classification.
        let middle = &segs[1..segs.len() - 1];
        let older_count = middle
            .iter()
            .filter(|s| s.kind == ainl_context_compiler::SegmentKind::OlderTurn)
            .count();
        let recent_count = middle
            .iter()
            .filter(|s| s.kind == ainl_context_compiler::SegmentKind::RecentTurn)
            .count();
        assert_eq!(older_count, 2, "expected 2 older turns: {middle:?}");
        assert_eq!(recent_count, 4, "expected 4 recent turns: {middle:?}");
    }

    #[test]
    fn build_segments_expands_tool_result_blocks() {
        let history: Vec<Message> = vec![
            Message {
                role: openfang_types::message::Role::Assistant,
                content: MessageContent::Blocks(vec![
                    ContentBlock::Text {
                        text: "Calling tool".into(),
                        provider_metadata: None,
                    },
                    ContentBlock::ToolResult {
                        tool_use_id: "1".into(),
                        tool_name: "bash".into(),
                        content: "ok".into(),
                        is_error: false,
                    },
                ]),
                orchestration_ctx: None,
            },
            Message::user("current"),
        ];
        let segs = build_segments("sys", &history, "current");
        let tool_segs: Vec<_> = segs
            .iter()
            .filter(|s| s.kind == ainl_context_compiler::SegmentKind::ToolResult)
            .collect();
        assert!(
            !tool_segs.is_empty(),
            "expected a ToolResult segment, got {segs:?}"
        );
        assert!(
            segs.iter()
                .any(|s| s.content.contains("Calling tool") || s.content.contains("bash")),
            "expected text + tool, got {segs:?}"
        );
    }

    #[test]
    fn build_segments_drops_empty_text_messages() {
        let history: Vec<Message> = vec![
            Message::user(""),
            Message::assistant("real reply"),
            Message::user("current"),
        ];
        let segs = build_segments("sys", &history, "current");
        // System + 1 non-empty history (the empty one is dropped) + 1 user_prompt = 3.
        assert_eq!(segs.len(), 3, "got {segs:?}");
    }

    #[test]
    fn measure_and_record_records_nonzero_for_realistic_prompt() {
        let agent = "00000000-0000-0000-0000-0000000000aa";
        let _ = take_compose_turn(agent); // clear any prior leak

        let history: Vec<Message> = vec![
            Message::user("Earlier I asked about tokio runtimes."),
            Message::assistant("You did. We discussed tokio::spawn semantics."),
            Message::user("Now: how do I structure a graceful shutdown?"),
        ];
        let system_prompt = "You are a Rust expert. Explain concepts clearly with examples.";
        measure_and_record(
            agent,
            system_prompt,
            &history,
            "Now: how do I structure a graceful shutdown?",
        );

        let snap = take_compose_turn(agent).expect("snapshot recorded");
        assert!(
            snap.snapshot.original_tokens > 0,
            "original tokens should be non-zero, got {snap:?}"
        );
        assert!(
            snap.snapshot.compressed_tokens > 0,
            "compressed tokens should be non-zero, got {snap:?}"
        );
        // For Tier 0, system + user are pinned verbatim, so compressed ≤ original always.
        assert!(snap.snapshot.compressed_tokens <= snap.snapshot.original_tokens);
        assert_eq!(snap.tier, "heuristic");
    }
}
