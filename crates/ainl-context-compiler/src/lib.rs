//! # AINL Context Compiler — LLM context-window assembly
//!
//! Phase 6 of [`SELF_LEARNING_INTEGRATION_MAP.md`](../../../docs/SELF_LEARNING_INTEGRATION_MAP.md).
//!
//! Multi-segment, role-aware, question-aware prompt orchestration with progressive-enhancement
//! capability tiers (heuristic → LLM-driven anchored summarization → embedding-based relevance).
//!
//! ## Boundary vs `ainl-context-freshness`
//!
//! These two crates **share a name prefix but solve different problems**:
//!
//! | Crate | Lifecycle phase | "Context" means |
//! |---|---|---|
//! | [`ainl-context-freshness`](https://docs.rs/ainl-context-freshness) | **Pre-tool execution policy gate** | the agent's *knowledge of the world* (repo/index state vs HEAD) |
//! | `ainl-context-compiler` (this crate) | **Prompt assembly / window management** | the *LLM's input context window* (prompt bytes about to be sent) |
//!
//! `ainl-context-compiler` *consumes* `ainl-context-freshness` as a per-segment rank-down signal
//! (stale segments are ranked lower) — see [`relevance::HeuristicScorer`].
//!
//! See [`docs/ainl-crates-overview.md`](../../../docs/ainl-crates-overview.md) for the broader
//! `ainl-*` family map.
//!
//! ## Design tiers (auto-detected at runtime)
//!
//! - **Tier 0 — Heuristic** (always available): question-token overlap × recency × freshness;
//!   per-segment compression via [`ainl_compression`].
//! - **Tier 1 — Anchored summarization** (M2): when a [`summarizer::Summarizer`] is injected,
//!   older history collapses into a structured `AnchoredSummary` (Factory.ai pattern).
//! - **Tier 2 — Embedding rerank** (M3): when an [`embedder::Embedder`] is injected, segments are
//!   reranked by cosine similarity to the latest user message.
//!
//! Each tier auto-degrades on per-call failure; the system never blocks on optional capabilities.
//!
//! ## Telemetry sink (canonical pattern from §15.4)
//!
//! Mirrors [`ainl_compression::CompressionTelemetrySink`] exactly. Hosts implement
//! [`ContextEmissionSink`] once and pass it via [`orchestrator::ContextCompiler::with_sink`];
//! everything downstream just emits structured events.
//!
//! Telemetry field names come from
//! [`ainl_contracts::telemetry`](ainl_contracts::telemetry) constants prefixed `CONTEXT_COMPILER_*`
//! so dashboards, Prometheus exporters, and CI gates reference them consistently across hosts.

#![warn(missing_docs)]

pub mod budget;
pub mod capability;
pub mod mcp_ainl_prompt;
pub mod metrics;
pub mod orchestrator;
pub mod relevance;
pub mod segment;
pub mod summarizer;

pub mod embedder;
#[cfg(feature = "sources-failure-warnings")]
pub mod failure_recall;
#[cfg(feature = "sources-trajectory-recap")]
pub mod trajectory_recap;

pub use budget::BudgetPolicy;
pub use capability::{CapabilityProbe, Tier};
pub use embedder::{cosine, Embedder, EmbedderError, PlaceholderEmbedder};
#[cfg(feature = "sources-failure-warnings")]
pub use failure_recall::memory_block_for_user_query;
pub use mcp_ainl_prompt::{
    mcp_ainl_run_adapters_cheatsheet_segment, MCP_AINL_RUN_ADAPTERS_CHEATSHEET,
};
pub use metrics::{ContextCompilerMetrics, SegmentMetrics};
pub use orchestrator::{ComposedPrompt, ContextCompiler};
pub use relevance::{HeuristicScorer, RelevanceScore, RelevanceScorer};
pub use segment::{Role, Segment, SegmentKind};
pub use summarizer::{AnchoredSummary, AnchoredSummarySection, Summarizer, SummarizerError};
#[cfg(feature = "sources-trajectory-recap")]
pub use trajectory_recap::format_trajectory_recap_lines;

use std::sync::Arc;

/// Optional structured telemetry sink for context-compiler events.
///
/// Mirrors [`ainl_compression::CompressionTelemetrySink`] in shape. Hosts (e.g. `openfang-runtime`)
/// provide a single sink impl that bridges these events to their event bus / SSE stream / audit
/// log. Other AINL hosts (`ainl-inference-server`, `ainativelang` MCP) can pass `None` to opt out
/// entirely.
///
/// See `SELF_LEARNING_INTEGRATION_MAP.md` §15.4 for the canonical pattern.
pub trait ContextEmissionSink: Send + Sync {
    /// Emit a single context-compiler event.
    fn emit(&self, event: ContextCompilerEvent);
}

/// Structured event emitted by [`ContextCompiler`] during prompt composition.
///
/// All variants are intentionally cheap to construct (no large captures) so emission stays under
/// 1 ms even under high-frequency turns.
#[derive(Debug, Clone)]
pub enum ContextCompilerEvent {
    /// A single segment survived selection and was emitted into the composed prompt.
    BlockEmitted {
        /// Source identifier (e.g. `"system_prompt"`, `"recent_turn"`, `"tool_result"`).
        source: &'static str,
        /// Coarse segment kind for dashboard grouping.
        kind: SegmentKind,
        /// Original token estimate (pre-compression).
        original_tokens: usize,
        /// Token estimate after per-segment compression / pruning.
        kept_tokens: usize,
    },
    /// Total budget allocated and the per-kind reservation.
    BudgetAllocated {
        /// Total prompt-window budget in token estimate.
        total: usize,
        /// Per-kind reserved tokens (sum may be ≤ `total`).
        per_kind: Vec<(SegmentKind, usize)>,
    },
    /// Capability tier selected for this `compose()` call.
    TierSelected {
        /// Which tier the orchestrator activated.
        tier: Tier,
        /// Short reason code (e.g. `"summarizer_present"`, `"heuristic_only"`).
        reason: &'static str,
    },
    /// Summarizer was invoked successfully (Tier ≥ 1).
    SummarizerInvoked {
        /// Wall-clock duration of the summarizer call.
        duration_ms: u64,
        /// Number of segments fed into the summarizer.
        segments_in: usize,
        /// Token estimate of the resulting summary.
        summary_tokens: usize,
    },
    /// Summarizer call failed; orchestrator auto-degraded to heuristic for this turn.
    SummarizerFailed {
        /// Wall-clock duration of the failed call.
        duration_ms: u64,
        /// Short error kind classifier (e.g. `"timeout"`, `"http"`, `"parse"`).
        error_kind: &'static str,
    },
    /// Total budget was exceeded even after compaction; safety-net truncation applied.
    BudgetExceeded {
        /// Tokens over budget after best-effort compaction.
        overage: usize,
    },
}

/// Convenience type alias used throughout the crate.
pub type SinkRef = Option<Arc<dyn ContextEmissionSink>>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct CapturingSink {
        events: Mutex<Vec<ContextCompilerEvent>>,
    }

    impl ContextEmissionSink for CapturingSink {
        fn emit(&self, event: ContextCompilerEvent) {
            self.events.lock().expect("lock").push(event);
        }
    }

    #[test]
    fn sink_trait_is_object_safe() {
        let sink: Arc<dyn ContextEmissionSink> = Arc::new(CapturingSink {
            events: Mutex::new(Vec::new()),
        });
        sink.emit(ContextCompilerEvent::TierSelected {
            tier: Tier::Heuristic,
            reason: "test",
        });
    }
}
