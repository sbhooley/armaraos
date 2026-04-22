//! Relevance scoring (Tier 0 heuristic; Tier 2 embedding rerank in `embedder.rs`).
//!
//! The Tier 0 [`HeuristicScorer`] uses a question-token-overlap × recency × freshness formula
//! aligned with the LongLLMLingua coarse-to-fine pattern and Towards Data Science 2026 survey:
//!
//! ```text
//! effective = base_importance × recency × freshness × vitals_cap + relevance_boost
//! relevance_boost = (|query_tokens ∩ segment_tokens| / |query_tokens|) × 0.35
//! ```

use crate::segment::{Segment, SegmentKind};
use ainl_contracts::CognitiveVitals;
use std::collections::HashSet;

#[cfg(feature = "freshness")]
use ainl_contracts::ContextFreshness;

/// Bounded relevance score in `[0.0, +inf)`. Higher = more relevant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RelevanceScore(pub f32);

impl RelevanceScore {
    /// Floor used for "always-keep" segments — beats any computed score.
    pub const ALWAYS_KEEP: Self = Self(f32::MAX);
}

/// Pluggable scoring strategy. Implement once per tier (heuristic / embedding / future hybrid).
pub trait RelevanceScorer: Send + Sync {
    /// Score a single segment in the context of the latest user query and optional vitals.
    fn score(
        &self,
        segment: &Segment,
        latest_user_query: &str,
        vitals: Option<&CognitiveVitals>,
    ) -> RelevanceScore;
}

/// Tier 0 heuristic scorer (no ML, no embeddings).
///
/// Always available; safe to use offline. Composes additional signals when their cargo features
/// are enabled (`freshness`, `tagger`).
#[derive(Debug, Clone, Default)]
pub struct HeuristicScorer {
    /// Multiplier applied to `relevance_boost` (default 0.35 per the TDS 2026 formula).
    pub relevance_boost_weight: f32,
    /// Decay factor applied per `age_index` step (default 0.9).
    pub recency_decay: f32,
}

impl HeuristicScorer {
    /// Construct a heuristic scorer with sensible defaults.
    #[must_use]
    pub fn new() -> Self {
        Self {
            relevance_boost_weight: 0.35,
            recency_decay: 0.9,
        }
    }

    fn tokens(s: &str) -> HashSet<String> {
        s.split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
            .map(|t| t.trim().to_ascii_lowercase())
            .filter(|t| t.len() >= 3)
            .collect()
    }

    fn recency_factor(&self, age_index: u32) -> f32 {
        if age_index == u32::MAX {
            // Pinned segments (system prompt, tool definitions) ignore recency.
            1.0
        } else {
            self.recency_decay.powi(age_index as i32)
        }
    }

    #[cfg(feature = "freshness")]
    fn freshness_factor(segment: &Segment) -> f32 {
        match segment.freshness {
            Some(ContextFreshness::Stale) => 0.5,
            Some(ContextFreshness::Unknown) => 0.85,
            Some(ContextFreshness::Fresh) | None => 1.0,
        }
    }

    #[cfg(not(feature = "freshness"))]
    fn freshness_factor(_segment: &Segment) -> f32 {
        1.0
    }

    /// Per integration-map §15.3: cap aggressiveness when vitals.trust < 0.5
    /// — model uncertainty should not compound information loss.
    fn vitals_cap(vitals: Option<&CognitiveVitals>) -> f32 {
        match vitals {
            Some(v) if v.trust < 0.5 => 0.85,
            _ => 1.0,
        }
    }
}

impl RelevanceScorer for HeuristicScorer {
    fn score(
        &self,
        segment: &Segment,
        latest_user_query: &str,
        vitals: Option<&CognitiveVitals>,
    ) -> RelevanceScore {
        if segment.kind.is_always_keep() {
            return RelevanceScore::ALWAYS_KEEP;
        }
        let query_tokens = Self::tokens(latest_user_query);
        if query_tokens.is_empty() {
            // Fall back to recency × freshness × importance × vitals.
            let base = segment.base_importance
                * self.recency_factor(segment.age_index)
                * Self::freshness_factor(segment)
                * Self::vitals_cap(vitals);
            return RelevanceScore(base);
        }
        let segment_tokens = Self::tokens(&segment.content);
        let overlap = query_tokens
            .iter()
            .filter(|t| segment_tokens.contains(*t))
            .count();
        let relevance_boost = if query_tokens.is_empty() {
            0.0
        } else {
            (overlap as f32 / query_tokens.len() as f32) * self.relevance_boost_weight
        };
        let base = segment.base_importance
            * self.recency_factor(segment.age_index)
            * Self::freshness_factor(segment)
            * Self::vitals_cap(vitals);
        // Tool results get an additional implicit boost when they came from a tool name the
        // user just mentioned — this is what lets coarse selection keep the right tool output.
        let tool_name_boost = match (segment.kind, &segment.tool_name) {
            (SegmentKind::ToolResult, Some(name)) => {
                let lq = latest_user_query.to_ascii_lowercase();
                if lq.contains(&name.to_ascii_lowercase()) {
                    0.5
                } else {
                    0.0
                }
            }
            _ => 0.0,
        };
        RelevanceScore(base + relevance_boost + tool_name_boost)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::segment::{Role, Segment};

    #[test]
    fn always_keep_floor() {
        let scorer = HeuristicScorer::new();
        let seg = Segment::user_prompt("anything");
        let s = scorer.score(&seg, "anything", None);
        assert_eq!(s.0, f32::MAX);
    }

    #[test]
    fn overlap_boost_helps_relevant_history() {
        let scorer = HeuristicScorer::new();
        let relevant =
            Segment::older_turn(Role::Assistant, "discussing rust borrow checker tokio", 5);
        let irrelevant =
            Segment::older_turn(Role::Assistant, "weather report and lunch options", 5);
        let q = "rust tokio borrow";
        let r = scorer.score(&relevant, q, None);
        let i = scorer.score(&irrelevant, q, None);
        assert!(r.0 > i.0, "relevant > irrelevant; got {r:?} vs {i:?}");
    }

    #[test]
    fn recency_decay_lowers_old_scores() {
        let scorer = HeuristicScorer::new();
        let new = Segment::older_turn(Role::Assistant, "topic words here", 1);
        let old = Segment::older_turn(Role::Assistant, "topic words here", 30);
        let q = "topic";
        let n = scorer.score(&new, q, None);
        let o = scorer.score(&old, q, None);
        assert!(n.0 > o.0);
    }

    #[test]
    fn vitals_low_trust_caps_score() {
        let scorer = HeuristicScorer::new();
        let seg = Segment::older_turn(Role::Assistant, "irrelevant text", 2);
        let high_trust = ainl_contracts::CognitiveVitals {
            gate: ainl_contracts::VitalsGate::Pass,
            phase: "reasoning:0.9".into(),
            trust: 0.9,
            mean_logprob: -0.1,
            entropy: 0.05,
            sample_tokens: 20,
        };
        let low_trust = ainl_contracts::CognitiveVitals {
            trust: 0.3,
            ..high_trust.clone()
        };
        let h = scorer.score(&seg, "topic", Some(&high_trust));
        let l = scorer.score(&seg, "topic", Some(&low_trust));
        assert!(h.0 >= l.0);
    }

    #[cfg(feature = "freshness")]
    #[test]
    fn stale_freshness_lowers_score() {
        let scorer = HeuristicScorer::new();
        let mut fresh = Segment::older_turn(Role::Assistant, "discussing rust topic words", 3);
        fresh.freshness = Some(ainl_contracts::ContextFreshness::Fresh);
        let mut stale = fresh.clone();
        stale.freshness = Some(ainl_contracts::ContextFreshness::Stale);
        let f = scorer.score(&fresh, "rust topic", None);
        let s = scorer.score(&stale, "rust topic", None);
        assert!(f.0 > s.0, "fresh > stale; got {f:?} vs {s:?}");
    }

    #[test]
    fn tool_name_match_boosts_tool_result() {
        let scorer = HeuristicScorer::new();
        let matching = Segment::tool_result("file_read", "contents...", 4);
        let other = Segment::tool_result("web_search", "contents...", 4);
        let q = "what did file_read return?";
        let m = scorer.score(&matching, q, None);
        let o = scorer.score(&other, q, None);
        assert!(m.0 > o.0);
    }
}
