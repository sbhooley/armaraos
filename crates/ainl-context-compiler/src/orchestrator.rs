//! `ContextCompiler::compose` — orchestrates segment selection, budget allocation, compaction,
//! and per-segment compression, returning a [`ComposedPrompt`] plus telemetry.
//!
//! The Tier 0 path is deterministic and offline-safe: heuristic relevance scoring, greedy
//! budget fill, and per-segment [`ainl_compression`] passes. Tier ≥ 1 lights up automatically
//! when the host injects a [`Summarizer`] (M2) or [`crate::embedder::Embedder`] (M3) — but the
//! orchestrator never blocks or fails when those are absent.

use crate::budget::BudgetPolicy;
use crate::capability::CapabilityProbe;
use crate::metrics::ContextCompilerMetrics;
use crate::relevance::{HeuristicScorer, RelevanceScore, RelevanceScorer};
use crate::segment::{Role, Segment, SegmentKind};
use crate::summarizer::{AnchoredSummary, Summarizer};
use crate::{ContextCompilerEvent, ContextEmissionSink, SinkRef};
use ainl_compression::{compress, EfficientMode};
use ainl_contracts::CognitiveVitals;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, warn};

/// Result of one `compose()` call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposedPrompt {
    /// Segments to assemble into the final LLM input, in target order.
    pub segments: Vec<Segment>,
    /// The anchored summary state after this compose call (may be `is_empty()` at Tier 0).
    pub anchored_summary: AnchoredSummary,
    /// Aggregate + per-segment telemetry.
    pub telemetry: ContextCompilerMetrics,
}

/// Builder + entry point for context-window assembly.
///
/// Hosts construct one `ContextCompiler` per session (or per agent) and reuse it across turns.
/// Optional dependencies (`summarizer`, `sink`) can be set once and shared via `Arc`.
pub struct ContextCompiler {
    scorer: Arc<dyn RelevanceScorer>,
    budget: BudgetPolicy,
    summarizer: Option<Arc<dyn Summarizer>>,
    sink: SinkRef,
}

impl ContextCompiler {
    /// Construct a Tier 0 compiler with the supplied scorer and budget.
    #[must_use]
    pub fn new(scorer: Arc<dyn RelevanceScorer>, budget: BudgetPolicy) -> Self {
        Self {
            scorer,
            budget,
            summarizer: None,
            sink: None,
        }
    }

    /// Convenience: build with the default heuristic scorer and default budget.
    #[must_use]
    pub fn with_defaults() -> Self {
        Self::new(Arc::new(HeuristicScorer::new()), BudgetPolicy::default())
    }

    /// Inject a Tier 1 summarizer (M2). When absent, the orchestrator runs at Tier 0.
    #[must_use]
    pub fn with_summarizer(mut self, summarizer: Arc<dyn Summarizer>) -> Self {
        self.summarizer = Some(summarizer);
        self
    }

    /// Attach a structured-event sink (mirrors `ainl_compression::with_telemetry_callback`).
    #[must_use]
    pub fn with_sink(mut self, sink: Arc<dyn ContextEmissionSink>) -> Self {
        self.sink = Some(sink);
        self
    }

    /// Return the active capability probe based on injected dependencies.
    #[must_use]
    pub fn probe(&self) -> CapabilityProbe {
        CapabilityProbe {
            summarizer: self.summarizer.is_some(),
            embedder: false, // M3 hook
        }
    }

    fn emit(&self, event: ContextCompilerEvent) {
        if let Some(sink) = &self.sink {
            sink.emit(event);
        }
    }

    /// Compose a prompt window from `segments`, scored against `latest_user_query`, within
    /// `self.budget`. `existing_summary` carries over from the prior turn (Tier ≥ 1 only).
    /// `vitals` (when supplied) caps compression aggressiveness on low-trust turns per
    /// SELF_LEARNING_INTEGRATION_MAP §15.3.
    ///
    /// Algorithm (per the plan):
    /// 1. Coarse selection — always-keep + heuristic scoring.
    /// 2. Budget allocation — apportion across kinds per `BudgetPolicy`.
    /// 3. Older-history compaction — `Summarizer` when present, else heuristic compression.
    /// 4. Tool-result fine-grained pruning.
    /// 5. Per-segment compression via `ainl_compression`.
    /// 6. Emit telemetry events.
    pub fn compose(
        &self,
        latest_user_query: &str,
        segments: Vec<Segment>,
        existing_summary: Option<&AnchoredSummary>,
        vitals: Option<&CognitiveVitals>,
    ) -> ComposedPrompt {
        let t0 = Instant::now();
        let probe = self.probe();
        let tier = probe.active_tier();
        self.emit(ContextCompilerEvent::TierSelected {
            tier,
            reason: probe.reason(),
        });

        let mut metrics = ContextCompilerMetrics::new(tier, self.budget.total_window);
        // Default mode by vitals: low-trust caps at Balanced, otherwise Balanced default.
        let default_mode = if self
            .budget
            .vitals_aware
            .then(|| vitals.is_some_and(|v| v.trust < 0.5))
            .unwrap_or(false)
        {
            EfficientMode::Balanced
        } else {
            EfficientMode::Balanced
        };

        // ── 1. Coarse selection ─────────────────────────────────────────────────────────
        // Score every segment, then split into always-keep and rankable.
        let mut scored: Vec<(usize, RelevanceScore)> = segments
            .iter()
            .enumerate()
            .map(|(idx, s)| (idx, self.scorer.score(s, latest_user_query, vitals)))
            .collect();
        // Sort highest-score first so the greedy fill picks most-relevant segments.
        scored.sort_by(|a, b| {
            b.1 .0
                .partial_cmp(&a.1 .0)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // ── 2. Budget allocation ────────────────────────────────────────────────────────
        // Always-keep slots come out of fixed reservations; the rest competes for `flexible_budget`.
        let mut flexible_budget = self.budget.flexible_budget();
        self.emit(ContextCompilerEvent::BudgetAllocated {
            total: self.budget.total_window,
            per_kind: vec![
                (SegmentKind::SystemPrompt, self.budget.system_budget()),
                (SegmentKind::ToolDefinitions, self.budget.tool_def_budget()),
                (SegmentKind::UserPrompt, self.budget.user_prompt_budget()),
            ],
        });

        // Recent-turns-keep-verbatim window: count from age_index = 0 upward; the N most recent
        // RecentTurn segments are pinned regardless of their heuristic score.
        let recent_pin_threshold = self.budget.recent_turns_keep_verbatim as u32;
        let pinned_idx: std::collections::HashSet<usize> = segments
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                s.kind.is_always_keep()
                    || (s.kind == SegmentKind::RecentTurn && s.age_index < recent_pin_threshold)
                    || s.kind == SegmentKind::ToolDefinitions
            })
            .map(|(i, _)| i)
            .collect();

        // ── 3+4+5. Greedy fill with per-segment compression ─────────────────────────────
        let mut keep: Vec<Option<Segment>> = (0..segments.len()).map(|_| None).collect();
        let mut summarizer_calls: u32 = 0;
        let mut summarizer_failures: u32 = 0;
        let mut dropped_for_summarization: Vec<Segment> = Vec::new();
        let mut anchored = existing_summary
            .cloned()
            .unwrap_or_else(AnchoredSummary::empty);

        // Pinned segments first (always-keep + recent-pinned).
        for &(idx, _score) in scored
            .iter()
            .filter(|(i, _)| pinned_idx.contains(i))
            .collect::<Vec<_>>()
            .iter()
        {
            let original = &segments[*idx];
            let original_tok = original.token_estimate();
            // Pinned segments are never compressed by default (system + user + tool defs + recent).
            keep[*idx] = Some(original.clone());
            metrics.record_segment(original.kind, original_tok, original_tok, false);
            self.emit(ContextCompilerEvent::BlockEmitted {
                source: source_label(original.kind),
                kind: original.kind,
                original_tokens: original_tok,
                kept_tokens: original_tok,
            });
        }

        // Now the rankable rest, in score order.
        for &(idx, _score) in scored.iter().filter(|(i, _)| !pinned_idx.contains(i)) {
            let seg = &segments[idx];
            let original_tok = seg.token_estimate();

            // Compression mode per kind: tool results get most aggressive treatment (highest
            // savings ratio per industry consensus 10:1-20:1). Older turns get Balanced. Memory
            // blocks stay verbatim or use Balanced if oversized.
            let mode = match seg.kind {
                SegmentKind::ToolResult => EfficientMode::Aggressive,
                SegmentKind::OlderTurn => default_mode,
                SegmentKind::MemoryBlock | SegmentKind::AnchoredSummaryRecall => default_mode,
                SegmentKind::RecentTurn => default_mode,
                _ => EfficientMode::Off,
            };

            // Try to compress first to see if it fits in the remaining flexible budget.
            let compressed = if mode == EfficientMode::Off {
                seg.content.clone()
            } else {
                compress(&seg.content, mode).text
            };
            let compressed_tok = ainl_compression::tokenize_estimate(&compressed);

            if compressed_tok <= flexible_budget {
                let mut kept = seg.clone();
                kept.content = compressed;
                keep[idx] = Some(kept);
                flexible_budget = flexible_budget.saturating_sub(compressed_tok);
                metrics.record_segment(seg.kind, original_tok, compressed_tok, false);
                self.emit(ContextCompilerEvent::BlockEmitted {
                    source: source_label(seg.kind),
                    kind: seg.kind,
                    original_tokens: original_tok,
                    kept_tokens: compressed_tok,
                });
            } else {
                // Doesn't fit — drop. If it's an older turn, queue it for summarization (Tier ≥ 1).
                if seg.kind == SegmentKind::OlderTurn {
                    dropped_for_summarization.push(seg.clone());
                }
                metrics.record_segment(seg.kind, original_tok, 0, true);
                debug!(
                    kind = ?seg.kind,
                    original_tok,
                    flexible_budget,
                    "context_compiler: dropped (over budget)"
                );
            }
        }

        // ── Tier ≥ 1: anchored summarization of dropped older turns ─────────────────────
        if let Some(summ) = &self.summarizer {
            if !dropped_for_summarization.is_empty() {
                let s0 = Instant::now();
                summarizer_calls += 1;
                match summ.summarize(&dropped_for_summarization, Some(&anchored)) {
                    Ok(new_summary) => {
                        let summary_tokens = ainl_compression::tokenize_estimate(
                            &new_summary.to_prompt_text(),
                        );
                        anchored = new_summary;
                        anchored.token_estimate = summary_tokens;
                        anchored.iteration = anchored.iteration.saturating_add(1);
                        self.emit(ContextCompilerEvent::SummarizerInvoked {
                            duration_ms: s0.elapsed().as_millis() as u64,
                            segments_in: dropped_for_summarization.len(),
                            summary_tokens,
                        });
                    }
                    Err(e) => {
                        summarizer_failures += 1;
                        warn!(error = %e, "context_compiler: summarizer failed, degrading to Tier 0 for this turn");
                        self.emit(ContextCompilerEvent::SummarizerFailed {
                            duration_ms: s0.elapsed().as_millis() as u64,
                            error_kind: e.kind(),
                        });
                    }
                }
            }
        }

        // Assemble in stable original order: SystemPrompt → MemoryBlock → AnchoredSummaryRecall
        // → OlderTurn (oldest→newest by age_index desc→asc) → RecentTurn (oldest→newest)
        // → ToolDefinitions → UserPrompt. We preserve the host's intended ordering for now by
        // emitting in original index order (the host arranges segments before calling compose).
        let mut composed: Vec<Segment> = keep.into_iter().flatten().collect();

        // If summarizer produced content, inject it as an AnchoredSummaryRecall segment near
        // the top so the LLM sees it before older turns.
        if !anchored.is_empty() {
            let recall = Segment {
                kind: SegmentKind::AnchoredSummaryRecall,
                role: Role::System,
                content: anchored.to_prompt_text(),
                age_index: 0,
                tool_name: None,
                base_importance: 1.5,
                #[cfg(feature = "freshness")]
                freshness: None,
            };
            // Insert after SystemPrompt + MemoryBlock segments so it precedes turns.
            let insert_at = composed
                .iter()
                .position(|s| {
                    !matches!(
                        s.kind,
                        SegmentKind::SystemPrompt | SegmentKind::MemoryBlock
                    )
                })
                .unwrap_or(composed.len());
            composed.insert(insert_at, recall);
        }

        // Safety net: if we still exceed the soft cap, emit BudgetExceeded so dashboards surface it.
        let total_kept_tokens: usize = composed.iter().map(|s| s.token_estimate()).sum();
        if total_kept_tokens > self.budget.soft_total_cap {
            self.emit(ContextCompilerEvent::BudgetExceeded {
                overage: total_kept_tokens.saturating_sub(self.budget.soft_total_cap),
            });
        }

        metrics.summarizer_calls = summarizer_calls;
        metrics.summarizer_failures = summarizer_failures;
        metrics.elapsed_ms = t0.elapsed().as_millis() as u64;

        ComposedPrompt {
            segments: composed,
            anchored_summary: anchored,
            telemetry: metrics,
        }
    }
}

const fn source_label(kind: SegmentKind) -> &'static str {
    match kind {
        SegmentKind::SystemPrompt => "system_prompt",
        SegmentKind::OlderTurn => "older_turn",
        SegmentKind::RecentTurn => "recent_turn",
        SegmentKind::ToolDefinitions => "tool_definitions",
        SegmentKind::ToolResult => "tool_result",
        SegmentKind::UserPrompt => "user_prompt",
        SegmentKind::AnchoredSummaryRecall => "anchored_summary_recall",
        SegmentKind::MemoryBlock => "memory_block",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::summarizer::SummarizerError;
    use std::sync::Mutex;

    #[derive(Default)]
    struct CapturingSink {
        events: Mutex<Vec<ContextCompilerEvent>>,
    }

    impl ContextEmissionSink for CapturingSink {
        fn emit(&self, event: ContextCompilerEvent) {
            self.events.lock().expect("lock").push(event);
        }
    }

    fn long_text(prefix: &str, n: usize) -> String {
        let mut out = String::new();
        for i in 0..n {
            out.push_str(prefix);
            out.push_str(&format!(" sentence {i}. "));
        }
        out
    }

    #[test]
    fn tier0_compose_keeps_system_and_user_verbatim() {
        let compiler = ContextCompiler::with_defaults();
        let segments = vec![
            Segment::system_prompt("You are a helpful assistant."),
            Segment::user_prompt("Help me debug a tokio runtime issue."),
        ];
        let out = compiler.compose("Help me debug a tokio runtime issue.", segments, None, None);
        assert_eq!(out.segments.len(), 2);
        assert!(out.segments.iter().any(|s| s.kind == SegmentKind::SystemPrompt));
        assert!(out.segments.iter().any(|s| s.kind == SegmentKind::UserPrompt));
        assert_eq!(out.telemetry.tier, "heuristic");
        assert_eq!(out.telemetry.summarizer_calls, 0);
    }

    #[test]
    fn tier0_compresses_long_older_turns_within_budget() {
        let mut budget = BudgetPolicy::default();
        budget.total_window = 4_000; // tight to force compression
        let compiler = ContextCompiler::new(Arc::new(HeuristicScorer::new()), budget);
        let segments = vec![
            Segment::system_prompt("system"),
            Segment::older_turn(Role::Assistant, long_text("rust borrow checker tokio", 200), 10),
            Segment::user_prompt("rust tokio"),
        ];
        let out = compiler.compose("rust tokio", segments, None, None);
        // System + user always survive.
        assert!(out.segments.iter().any(|s| s.kind == SegmentKind::SystemPrompt));
        assert!(out.segments.iter().any(|s| s.kind == SegmentKind::UserPrompt));
        // Older turn either compressed or dropped; metrics show non-zero original.
        assert!(out.telemetry.total_original_tokens > 0);
    }

    #[test]
    fn sink_receives_tier_and_block_events() {
        let sink = Arc::new(CapturingSink::default());
        let compiler = ContextCompiler::with_defaults().with_sink(sink.clone());
        let segments = vec![
            Segment::system_prompt("sys"),
            Segment::user_prompt("hi"),
        ];
        let _ = compiler.compose("hi", segments, None, None);
        let events = sink.events.lock().unwrap();
        assert!(events.iter().any(|e| matches!(e, ContextCompilerEvent::TierSelected { .. })));
        assert!(events.iter().any(|e| matches!(e, ContextCompilerEvent::BlockEmitted { .. })));
        assert!(events.iter().any(|e| matches!(e, ContextCompilerEvent::BudgetAllocated { .. })));
    }

    #[test]
    fn tier1_summarizer_invoked_on_dropped_older_turns() {
        struct MockSummarizer;
        impl Summarizer for MockSummarizer {
            fn summarize(
                &self,
                segments: &[Segment],
                _existing: Option<&AnchoredSummary>,
            ) -> Result<AnchoredSummary, SummarizerError> {
                let mut s = AnchoredSummary::empty();
                s.sections[0].content = format!("Summarized {} segments.", segments.len());
                Ok(s)
            }
        }
        let mut budget = BudgetPolicy::default();
        budget.total_window = 2_000;
        let compiler = ContextCompiler::new(Arc::new(HeuristicScorer::new()), budget)
            .with_summarizer(Arc::new(MockSummarizer));
        // Many large older turns, guaranteed to overflow → summarizer fires.
        let mut segments: Vec<Segment> = (0..30)
            .map(|i| Segment::older_turn(Role::Assistant, long_text("rust", 100), i + 5))
            .collect();
        segments.insert(0, Segment::system_prompt("sys"));
        segments.push(Segment::user_prompt("rust"));
        let out = compiler.compose("rust", segments, None, None);
        assert_eq!(out.telemetry.tier, "heuristic_summarization");
        assert!(out.telemetry.summarizer_calls > 0);
        assert!(!out.anchored_summary.is_empty());
        // Anchored summary should appear in the composed segments as a recall block.
        assert!(out
            .segments
            .iter()
            .any(|s| s.kind == SegmentKind::AnchoredSummaryRecall));
    }

    #[test]
    fn summarizer_failure_degrades_gracefully() {
        struct FailingSummarizer;
        impl Summarizer for FailingSummarizer {
            fn summarize(
                &self,
                _segments: &[Segment],
                _existing: Option<&AnchoredSummary>,
            ) -> Result<AnchoredSummary, SummarizerError> {
                Err(SummarizerError::Timeout)
            }
        }
        let sink = Arc::new(CapturingSink::default());
        let mut budget = BudgetPolicy::default();
        budget.total_window = 1_500;
        let compiler = ContextCompiler::new(Arc::new(HeuristicScorer::new()), budget)
            .with_summarizer(Arc::new(FailingSummarizer))
            .with_sink(sink.clone());
        let mut segments: Vec<Segment> = (0..20)
            .map(|i| Segment::older_turn(Role::Assistant, long_text("rust", 80), i + 5))
            .collect();
        segments.insert(0, Segment::system_prompt("sys"));
        segments.push(Segment::user_prompt("rust"));
        let out = compiler.compose("rust", segments, None, None);
        assert!(out.telemetry.summarizer_failures > 0);
        let events = sink.events.lock().unwrap();
        assert!(events
            .iter()
            .any(|e| matches!(e, ContextCompilerEvent::SummarizerFailed { .. })));
    }
}
