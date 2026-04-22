//! Capability tier auto-detection.
//!
//! Tiers light up additively as the host injects optional dependencies. The orchestrator
//! consults the active tier per-call and auto-degrades on any failure — never blocks startup,
//! never fails a turn because Tier 1/2 was unavailable.

use serde::{Deserialize, Serialize};

/// Active capability tier for one [`crate::ContextCompiler::compose`] call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    /// Heuristic-only (always available, offline-safe).
    #[default]
    Heuristic,
    /// Heuristic + LLM-driven anchored summarization (M2).
    HeuristicSummarization,
    /// Heuristic + summarization + embedding-based relevance rerank (M3).
    HeuristicSummarizationEmbedding,
}

impl Tier {
    /// Stable label for telemetry.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Heuristic => "heuristic",
            Self::HeuristicSummarization => "heuristic_summarization",
            Self::HeuristicSummarizationEmbedding => "heuristic_summarization_embedding",
        }
    }
}

/// Probe result describing which optional capabilities the host wired in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CapabilityProbe {
    /// Whether a [`crate::Summarizer`] is available.
    pub summarizer: bool,
    /// Whether an embedder is available (Tier 2).
    pub embedder: bool,
}

impl CapabilityProbe {
    /// Construct a probe with no optional capabilities (offline default).
    #[must_use]
    pub fn offline() -> Self {
        Self::default()
    }

    /// Highest tier the probe authorizes.
    #[must_use]
    pub fn active_tier(self) -> Tier {
        match (self.summarizer, self.embedder) {
            (true, true) => Tier::HeuristicSummarizationEmbedding,
            (true, false) => Tier::HeuristicSummarization,
            (false, true) => Tier::HeuristicSummarizationEmbedding,
            (false, false) => Tier::Heuristic,
        }
    }

    /// Human-readable reason for telemetry (`reason` field of `TierSelected` event).
    #[must_use]
    pub fn reason(self) -> &'static str {
        match (self.summarizer, self.embedder) {
            (true, true) => "summarizer_and_embedder_present",
            (true, false) => "summarizer_present",
            (false, true) => "embedder_present",
            (false, false) => "heuristic_only",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_is_heuristic() {
        assert_eq!(CapabilityProbe::offline().active_tier(), Tier::Heuristic);
    }

    #[test]
    fn summarizer_only_unlocks_tier1() {
        let p = CapabilityProbe {
            summarizer: true,
            embedder: false,
        };
        assert_eq!(p.active_tier(), Tier::HeuristicSummarization);
    }

    #[test]
    fn both_unlocks_tier2() {
        let p = CapabilityProbe {
            summarizer: true,
            embedder: true,
        };
        assert_eq!(p.active_tier(), Tier::HeuristicSummarizationEmbedding);
    }

    #[test]
    fn embedder_only_unlocks_embedding_tier() {
        let p = CapabilityProbe {
            summarizer: false,
            embedder: true,
        };
        assert_eq!(p.active_tier(), Tier::HeuristicSummarizationEmbedding);
    }
}
