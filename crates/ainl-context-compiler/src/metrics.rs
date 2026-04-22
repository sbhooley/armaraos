//! Per-call telemetry struct emitted alongside [`crate::ComposedPrompt`].

use crate::capability::Tier;
use crate::segment::SegmentKind;
use serde::{Deserialize, Serialize};

/// Per-segment compression accounting.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SegmentMetrics {
    /// Segment kind classifier.
    pub kind: String,
    /// Token estimate before pruning / compression.
    pub original_tokens: usize,
    /// Token estimate after pruning / compression.
    pub compressed_tokens: usize,
    /// Whether the segment was dropped entirely.
    pub dropped: bool,
}

impl SegmentMetrics {
    /// Tokens saved by this segment's compression / drop.
    #[must_use]
    pub fn saved(&self) -> usize {
        self.original_tokens.saturating_sub(self.compressed_tokens)
    }
}

/// Aggregate telemetry for one [`crate::ContextCompiler::compose`] call.
///
/// This is the struct that powers the dashboard's whole-prompt savings widgets — it replaces
/// the previous user-message-only `CompressionMetrics` accounting at the kernel layer.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContextCompilerMetrics {
    /// Active tier for this call.
    pub tier: String,
    /// Total token budget configured for this call.
    pub total_budget: usize,
    /// Total tokens before any pruning / compression.
    pub total_original_tokens: usize,
    /// Total tokens emitted into the composed prompt.
    pub total_compressed_tokens: usize,
    /// Total tokens saved (= original − compressed).
    pub total_saved_tokens: usize,
    /// Savings ratio in `[0.0, 100.0]`.
    pub savings_ratio_pct: f32,
    /// Per-segment breakdown (one row per kept or dropped segment).
    pub per_segment: Vec<SegmentMetrics>,
    /// Number of summarizer invocations during this call (Tier ≥ 1).
    pub summarizer_calls: u32,
    /// Number of summarizer failures (auto-degraded for this call).
    pub summarizer_failures: u32,
    /// Wall-clock duration of `compose()` in milliseconds.
    pub elapsed_ms: u64,
}

impl ContextCompilerMetrics {
    /// Build from a populated tier + budget; per-segment rows added incrementally.
    #[must_use]
    pub fn new(tier: Tier, total_budget: usize) -> Self {
        Self {
            tier: tier.as_str().to_string(),
            total_budget,
            ..Default::default()
        }
    }

    /// Append per-segment accounting and update aggregate totals.
    pub fn record_segment(&mut self, kind: SegmentKind, original: usize, compressed: usize, dropped: bool) {
        self.per_segment.push(SegmentMetrics {
            kind: kind.as_str().to_string(),
            original_tokens: original,
            compressed_tokens: compressed,
            dropped,
        });
        self.total_original_tokens = self.total_original_tokens.saturating_add(original);
        self.total_compressed_tokens = self.total_compressed_tokens.saturating_add(compressed);
        self.total_saved_tokens = self
            .total_original_tokens
            .saturating_sub(self.total_compressed_tokens);
        self.savings_ratio_pct = if self.total_original_tokens == 0 {
            0.0
        } else {
            (self.total_saved_tokens as f32 * 100.0) / self.total_original_tokens as f32
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn savings_ratio_recomputes_after_each_segment() {
        let mut m = ContextCompilerMetrics::new(Tier::Heuristic, 10_000);
        m.record_segment(SegmentKind::OlderTurn, 1000, 200, false);
        m.record_segment(SegmentKind::ToolResult, 500, 100, false);
        assert_eq!(m.total_original_tokens, 1500);
        assert_eq!(m.total_compressed_tokens, 300);
        assert_eq!(m.total_saved_tokens, 1200);
        assert!((m.savings_ratio_pct - 80.0).abs() < 0.01);
    }

    #[test]
    fn dropped_segment_counts_as_full_savings() {
        let mut m = ContextCompilerMetrics::new(Tier::Heuristic, 10_000);
        m.record_segment(SegmentKind::OlderTurn, 800, 0, true);
        assert_eq!(m.total_saved_tokens, 800);
    }
}
