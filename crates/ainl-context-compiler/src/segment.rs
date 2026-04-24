//! Prompt-window segments.
//!
//! A `Segment` is one logical chunk of the LLM input. The orchestrator scores, prunes, and
//! compresses segments according to a [`crate::BudgetPolicy`] and emits a sequence of segments
//! ready to assemble into the final prompt.

use serde::{Deserialize, Serialize};

#[cfg(feature = "freshness")]
use ainl_contracts::ContextFreshness;

/// Speaker / origin role for a segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// System / instruction author.
    System,
    /// End user.
    User,
    /// Model assistant.
    Assistant,
    /// Tool / function output.
    Tool,
}

/// Coarse classification used for budget allocation, telemetry grouping, and dashboard widgets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SegmentKind {
    /// System prompt (always-keep, never compressed by default).
    SystemPrompt,
    /// A turn old enough to be eligible for summarization / compaction.
    OlderTurn,
    /// A turn within the verbatim window (kept as-is).
    RecentTurn,
    /// Tool / function definitions appended to the prompt.
    ToolDefinitions,
    /// Output of a tool / function call.
    ToolResult,
    /// The user's latest message (always-keep, never compressed).
    UserPrompt,
    /// Recalled `AnchoredSummary` from a prior turn (Tier ≥ 1).
    AnchoredSummaryRecall,
    /// A graph-memory-derived prompt block (e.g. `recent_attempts`, `known_facts`).
    MemoryBlock,
}

impl SegmentKind {
    /// Stable lowercase string label for telemetry / dashboards.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SystemPrompt => "system_prompt",
            Self::OlderTurn => "older_turn",
            Self::RecentTurn => "recent_turn",
            Self::ToolDefinitions => "tool_definitions",
            Self::ToolResult => "tool_result",
            Self::UserPrompt => "user_prompt",
            Self::AnchoredSummaryRecall => "anchored_summary_recall",
            Self::MemoryBlock => "memory_block",
        }
    }

    /// Whether the orchestrator must keep this segment verbatim by default.
    #[must_use]
    pub fn is_always_keep(self) -> bool {
        matches!(self, Self::SystemPrompt | Self::UserPrompt)
    }
}

/// One logical segment of the prompt window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Segment {
    /// Coarse classification.
    pub kind: SegmentKind,
    /// Speaker / origin.
    pub role: Role,
    /// Raw text content (the orchestrator may compress this).
    pub content: String,
    /// 0 = newest, larger = older. Used by recency scoring.
    pub age_index: u32,
    /// Optional tool name for `ToolResult` / `ToolDefinitions` segments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    /// Caller-provided base importance hint (default 1.0). Boosts always-keep segments
    /// implicitly via [`SegmentKind::is_always_keep`] regardless of this value.
    #[serde(default = "one")]
    pub base_importance: f32,
    /// Freshness signal at the time this segment was assembled (e.g. repo-knowledge currency).
    /// Used by [`crate::HeuristicScorer`] to rank stale segments lower per
    /// SELF_LEARNING_INTEGRATION_MAP §15.2.
    #[cfg(feature = "freshness")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freshness: Option<ContextFreshness>,
}

const fn one() -> f32 {
    1.0
}

impl Segment {
    /// Construct a fresh user-prompt segment (the latest user message).
    #[must_use]
    pub fn user_prompt(content: impl Into<String>) -> Self {
        Self {
            kind: SegmentKind::UserPrompt,
            role: Role::User,
            content: content.into(),
            age_index: 0,
            tool_name: None,
            base_importance: 2.0,
            #[cfg(feature = "freshness")]
            freshness: None,
        }
    }

    /// Construct a system-prompt segment.
    #[must_use]
    pub fn system_prompt(content: impl Into<String>) -> Self {
        Self {
            kind: SegmentKind::SystemPrompt,
            role: Role::System,
            content: content.into(),
            age_index: u32::MAX, // Pinned, recency does not apply.
            tool_name: None,
            base_importance: 1.5,
            #[cfg(feature = "freshness")]
            freshness: None,
        }
    }

    /// Construct a recent-turn segment with explicit age.
    #[must_use]
    pub fn recent_turn(role: Role, content: impl Into<String>, age_index: u32) -> Self {
        Self {
            kind: SegmentKind::RecentTurn,
            role,
            content: content.into(),
            age_index,
            tool_name: None,
            base_importance: 1.0,
            #[cfg(feature = "freshness")]
            freshness: None,
        }
    }

    /// Construct an older-turn segment (eligible for compaction).
    #[must_use]
    pub fn older_turn(role: Role, content: impl Into<String>, age_index: u32) -> Self {
        Self {
            kind: SegmentKind::OlderTurn,
            role,
            content: content.into(),
            age_index,
            tool_name: None,
            base_importance: 0.7,
            #[cfg(feature = "freshness")]
            freshness: None,
        }
    }

    /// Construct a tool-result segment.
    #[must_use]
    pub fn tool_result(
        tool_name: impl Into<String>,
        content: impl Into<String>,
        age_index: u32,
    ) -> Self {
        Self {
            kind: SegmentKind::ToolResult,
            role: Role::Tool,
            content: content.into(),
            age_index,
            tool_name: Some(tool_name.into()),
            base_importance: 0.8,
            #[cfg(feature = "freshness")]
            freshness: None,
        }
    }

    /// Construct a tool-definitions segment.
    #[must_use]
    pub fn tool_definitions(content: impl Into<String>) -> Self {
        Self {
            kind: SegmentKind::ToolDefinitions,
            role: Role::System,
            content: content.into(),
            age_index: u32::MAX,
            tool_name: None,
            base_importance: 1.2,
            #[cfg(feature = "freshness")]
            freshness: None,
        }
    }

    /// Construct a memory-block segment (graph-memory-derived).
    #[must_use]
    pub fn memory_block(label: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            kind: SegmentKind::MemoryBlock,
            role: Role::System,
            content: content.into(),
            age_index: 0,
            tool_name: Some(label.into()),
            base_importance: 1.0,
            #[cfg(feature = "freshness")]
            freshness: None,
        }
    }

    /// Token estimate via the shared [`ainl_compression::tokenize_estimate`] heuristic.
    #[must_use]
    pub fn token_estimate(&self) -> usize {
        ainl_compression::tokenize_estimate(&self.content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn always_keep_classification() {
        assert!(SegmentKind::SystemPrompt.is_always_keep());
        assert!(SegmentKind::UserPrompt.is_always_keep());
        assert!(!SegmentKind::OlderTurn.is_always_keep());
        assert!(!SegmentKind::ToolResult.is_always_keep());
    }

    #[test]
    fn segment_token_estimate_nonzero() {
        let s = Segment::user_prompt("Hello world this is a test");
        assert!(s.token_estimate() > 0);
    }

    #[test]
    fn segment_kind_label_stable() {
        assert_eq!(SegmentKind::SystemPrompt.as_str(), "system_prompt");
        assert_eq!(SegmentKind::ToolResult.as_str(), "tool_result");
    }
}
