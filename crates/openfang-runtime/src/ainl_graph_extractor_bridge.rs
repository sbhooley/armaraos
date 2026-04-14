//! Bridge to the published [`ainl-graph-extractor`](https://crates.io/crates/ainl-graph-extractor)
//! crate for turn-scoped semantic tags and optional tool-sequence pattern hints.
//!
//! **Runtime toggle:** when this crate is built with the `ainl-extractor` feature, set
//! `AINL_EXTRACTOR_ENABLED=1` (or `true` / `yes` / `on`) so the agent loop uses this bridge for
//! fact/pattern extraction instead of the legacy [`crate::graph_extractor`] path.

/// Separator between user message and assistant message in [`format_turn_payload`].
pub const TURN_USER_ASSISTANT_SEP: &str = "\n\n---\n\n";
const TOOLS_MARKER: &str = "\n\n__AINL_TOOLS__\n";

/// `true` when `AINL_EXTRACTOR_ENABLED` is set to a truthy value (`1`, `true`, `yes`, `on`).
pub fn ainl_extractor_runtime_enabled() -> bool {
    #[cfg(feature = "ainl-extractor")]
    {
        std::env::var("AINL_EXTRACTOR_ENABLED")
            .map(|v| {
                matches!(
                    v.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or(false)
    }
    #[cfg(not(feature = "ainl-extractor"))]
    {
        false
    }
}

/// Build the canonical turn payload consumed by [`AinlExtractorBridge::extract_facts`].
pub fn format_turn_payload(user_message: &str, assistant_response: &str, tools: &[String]) -> String {
    let mut s = String::with_capacity(
        user_message.len() + assistant_response.len() + 32 + tools.len() * 16,
    );
    s.push_str(user_message);
    s.push_str(TURN_USER_ASSISTANT_SEP);
    s.push_str(assistant_response);
    if !tools.is_empty() {
        s.push_str(TOOLS_MARKER);
        for t in tools {
            s.push_str(t);
            s.push('\n');
        }
    }
    s
}

#[cfg(feature = "ainl-extractor")]
fn parse_turn_payload(turn_text: &str) -> (&str, Option<&str>, Vec<String>) {
    let (rest, tools) = if let Some(idx) = turn_text.rfind(TOOLS_MARKER) {
        let body = &turn_text[..idx];
        let tool_block = &turn_text[idx + TOOLS_MARKER.len()..];
        let tools = tool_block
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>();
        (body, tools)
    } else {
        (turn_text, Vec::new())
    };

    if let Some(pos) = rest.find(TURN_USER_ASSISTANT_SEP) {
        let u = rest[..pos].trim_end();
        let a = rest[pos + TURN_USER_ASSISTANT_SEP.len()..].trim_start();
        (u, Some(a), tools)
    } else {
        (rest.trim(), None, tools)
    }
}

/// If `tool_sequence` contains an immediate repeated run of length ≥ 2 (e.g. `a,b,a,b`),
/// returns a synthetic pattern name for graph memory.
#[cfg(feature = "ainl-extractor")]
fn repeated_subsequence_pattern(tool_sequence: &[String]) -> Option<String> {
    let n = tool_sequence.len();
    if n < 4 {
        return None;
    }
    for w in 2..=n / 2 {
        if tool_sequence[..w] == tool_sequence[w..w + w] {
            return Some(format!("repeated_{w}_tool_cycle"));
        }
    }
    None
}

/// Wraps structured extraction from `ainl-graph-extractor` when the `ainl-extractor` feature is on.
pub struct AinlExtractorBridge;

impl AinlExtractorBridge {
    /// Deterministic semantic facts as `(text, confidence)` pairs (topic / preference / etc.).
    pub fn extract_facts(turn_text: &str, _agent_id: &str) -> Vec<(String, f32)> {
        #[cfg(feature = "ainl-extractor")]
        {
            let (user, assistant, tools) = parse_turn_payload(turn_text);
            let tags = ainl_graph_extractor::extract_turn_semantic_tags_for_memory(
                user,
                assistant,
                &tools,
            );
            tags.into_iter()
                .map(|t| {
                    (
                        format!("{}: {}", t.namespace.prefix(), t.value),
                        t.confidence,
                    )
                })
                .collect()
        }
        #[cfg(not(feature = "ainl-extractor"))]
        {
            let _ = (turn_text, _agent_id);
            Vec::new()
        }
    }

    /// Returns a pattern name when a repeated tool subsequence is detected or when the legacy
    /// workflow heuristics match (via [`crate::graph_extractor::extract_pattern`]).
    pub fn extract_pattern(tool_sequence: &[String], _agent_id: &str) -> Option<String> {
        #[cfg(feature = "ainl-extractor")]
        {
            repeated_subsequence_pattern(tool_sequence).or_else(|| {
                crate::graph_extractor::extract_pattern(tool_sequence).map(|p| p.name)
            })
        }
        #[cfg(not(feature = "ainl-extractor"))]
        {
            let _ = (tool_sequence, _agent_id);
            None
        }
    }
}

#[cfg(feature = "ainl-extractor")]
fn pattern_from_bridge_turn(
    turn_tool_names: &[String],
    agent_id: &str,
) -> Option<crate::graph_extractor::ExtractedPattern> {
    let name = AinlExtractorBridge::extract_pattern(turn_tool_names, agent_id)?;
    if let Some(p) = crate::graph_extractor::extract_pattern(turn_tool_names) {
        if p.name == name {
            return Some(p);
        }
    }
    Some(crate::graph_extractor::ExtractedPattern {
        name,
        tool_sequence: turn_tool_names.to_vec(),
        confidence: 0.78,
    })
}

/// Facts + procedural pattern for one graph-memory turn (respects `AINL_EXTRACTOR_ENABLED`).
pub fn graph_memory_turn_extraction(
    user_message: &str,
    assistant_response: &str,
    tools_for_episode: &[String],
    turn_tool_names: &[String],
    agent_id: &str,
) -> (
    Vec<crate::graph_extractor::ExtractedFact>,
    Option<crate::graph_extractor::ExtractedPattern>,
) {
    let use_structured = ainl_extractor_runtime_enabled();
    let facts = if cfg!(feature = "ainl-extractor") && use_structured {
        let turn = format_turn_payload(user_message, assistant_response, tools_for_episode);
        let mut out: Vec<crate::graph_extractor::ExtractedFact> =
            AinlExtractorBridge::extract_facts(&turn, agent_id)
                .into_iter()
                .map(|(text, confidence)| crate::graph_extractor::ExtractedFact {
                    text,
                    confidence,
                })
                .collect();
        if out.is_empty() {
            out = crate::graph_extractor::extract_facts_for_turn(
                user_message,
                assistant_response,
                tools_for_episode,
            );
        }
        out
    } else {
        crate::graph_extractor::extract_facts_for_turn(
            user_message,
            assistant_response,
            tools_for_episode,
        )
    };

    let pattern = {
        #[cfg(feature = "ainl-extractor")]
        {
            if use_structured {
                pattern_from_bridge_turn(turn_tool_names, agent_id)
            } else {
                crate::graph_extractor::extract_pattern(turn_tool_names)
            }
        }
        #[cfg(not(feature = "ainl-extractor"))]
        {
            let _ = use_structured;
            crate::graph_extractor::extract_pattern(turn_tool_names)
        }
    };

    (facts, pattern)
}
