//! Summarizer trait + structured anchored-summary types.
//!
//! M1 ships the *trait surface* and the data types so callers can already pass `Some(summarizer)`;
//! the actual `LlmDriverSummarizer` adapter (in `openfang-runtime/src/context_summarizer.rs`)
//! lands in M2. With no summarizer injected, the orchestrator transparently runs at Tier 0
//! (heuristic compression only).
//!
//! The structured-section design is taken from Factory.ai's anchored iterative summarization
//! pattern: a fixed schema of sections that the LLM repeatedly re-populates, instead of a free-form
//! summary that drifts across turns.

use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fmt;

use crate::segment::Segment;

/// One section of an [`AnchoredSummary`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnchoredSummarySection {
    /// Stable section id (e.g. `"intent"`, `"decisions"`).
    pub id: String,
    /// Human-readable label for dashboards.
    pub label: String,
    /// Section content. May be empty.
    pub content: String,
}

/// Structured running summary that anchors to fixed sections across turns.
///
/// The Factory.ai pattern: rather than re-writing a free-form blob each compaction, the
/// summarizer is given the prior summary plus the dropped segments and asked to update each named
/// section. This is what keeps coherence over 30+ turn conversations without drift.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnchoredSummary {
    /// Schema version for forward compatibility.
    pub schema_version: u32,
    /// The fixed section list. Order is meaningful for prompt assembly.
    pub sections: Vec<AnchoredSummarySection>,
    /// Total token estimate (kept on the struct so dashboards don't recompute).
    pub token_estimate: usize,
    /// Iteration counter — bumped each time the summarizer re-anchors.
    pub iteration: u32,
}

impl AnchoredSummary {
    /// Current schema version for `AnchoredSummary` payloads.
    pub const SCHEMA_VERSION: u32 = 1;

    /// Default fixed-section schema (Factory.ai-derived).
    #[must_use]
    pub fn empty() -> Self {
        Self {
            schema_version: Self::SCHEMA_VERSION,
            sections: vec![
                AnchoredSummarySection {
                    id: "intent".into(),
                    label: "Intent".into(),
                    content: String::new(),
                },
                AnchoredSummarySection {
                    id: "decisions".into(),
                    label: "Decisions".into(),
                    content: String::new(),
                },
                AnchoredSummarySection {
                    id: "files_touched".into(),
                    label: "Files touched".into(),
                    content: String::new(),
                },
                AnchoredSummarySection {
                    id: "pending_tasks".into(),
                    label: "Pending tasks".into(),
                    content: String::new(),
                },
                AnchoredSummarySection {
                    id: "current_state".into(),
                    label: "Current state".into(),
                    content: String::new(),
                },
            ],
            token_estimate: 0,
            iteration: 0,
        }
    }

    /// Whether all sections are empty (no content yet).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sections.iter().all(|s| s.content.trim().is_empty())
    }

    /// Render the summary as a single text block for prompt injection.
    #[must_use]
    pub fn to_prompt_text(&self) -> String {
        let mut out = String::new();
        for section in &self.sections {
            if section.content.trim().is_empty() {
                continue;
            }
            if !out.is_empty() {
                out.push_str("\n\n");
            }
            out.push_str("## ");
            out.push_str(&section.label);
            out.push('\n');
            out.push_str(section.content.trim());
        }
        out
    }
}

/// Errors a [`Summarizer`] implementation may return.
#[derive(Debug)]
pub enum SummarizerError {
    /// The summarizer call timed out.
    Timeout,
    /// Network / transport failure.
    Transport(String),
    /// Parsing the LLM's structured response failed.
    Parse(String),
    /// Catch-all.
    Other(String),
}

impl fmt::Display for SummarizerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timeout => f.write_str("summarizer timed out"),
            Self::Transport(m) => write!(f, "summarizer transport: {m}"),
            Self::Parse(m) => write!(f, "summarizer parse: {m}"),
            Self::Other(m) => write!(f, "summarizer: {m}"),
        }
    }
}

impl Error for SummarizerError {}

impl SummarizerError {
    /// Short stable kind tag for telemetry.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::Transport(_) => "transport",
            Self::Parse(_) => "parse",
            Self::Other(_) => "other",
        }
    }
}

/// Trait implemented by the M2 `openfang-runtime` adapter (`LlmDriverSummarizer`) — wraps the
/// existing `LlmDriver` so the compiler crate stays openfang-free.
///
/// Returns `Result<AnchoredSummary, SummarizerError>` so the orchestrator can auto-degrade to
/// Tier 0 on any failure without bringing down the turn.
///
/// Marked `Send + Sync` so a single summarizer instance can be shared via `Arc`.
pub trait Summarizer: Send + Sync {
    /// Re-anchor `existing_summary` (or build from scratch when `None`) using the dropped
    /// `segments`. Implementations should be idempotent across retries.
    ///
    /// The trait method is intentionally **synchronous** at the interface level so M1 can ship
    /// without an async runtime dependency in this crate. The host adapter wraps any async work
    /// (e.g. `LlmDriver` HTTP) using `tokio::runtime::Handle::block_on` or by exposing a
    /// non-async wrapper.
    fn summarize(
        &self,
        segments: &[Segment],
        existing_summary: Option<&AnchoredSummary>,
    ) -> Result<AnchoredSummary, SummarizerError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_summary_renders_nothing() {
        let s = AnchoredSummary::empty();
        assert!(s.is_empty());
        assert_eq!(s.to_prompt_text(), "");
    }

    #[test]
    fn populated_summary_renders_sections() {
        let mut s = AnchoredSummary::empty();
        s.sections[0].content = "Build the context compiler.".into();
        s.sections[1].content = "Reuse ainl-compression for fine pruning.".into();
        let text = s.to_prompt_text();
        assert!(text.contains("## Intent"));
        assert!(text.contains("Build the context compiler."));
        assert!(text.contains("## Decisions"));
        assert!(!text.contains("## Files touched")); // empty section omitted
    }

    #[test]
    fn summarizer_error_kinds_unique() {
        let kinds = [
            SummarizerError::Timeout.kind(),
            SummarizerError::Transport("x".into()).kind(),
            SummarizerError::Parse("x".into()).kind(),
            SummarizerError::Other("x".into()).kind(),
        ];
        for i in 0..kinds.len() {
            for j in (i + 1)..kinds.len() {
                assert_ne!(kinds[i], kinds[j]);
            }
        }
    }

    #[test]
    fn json_roundtrip() {
        let mut s = AnchoredSummary::empty();
        s.sections[0].content = "hello".into();
        s.token_estimate = 3;
        s.iteration = 1;
        let j = serde_json::to_value(&s).unwrap();
        let back: AnchoredSummary = serde_json::from_value(j).unwrap();
        assert_eq!(s, back);
    }
}
