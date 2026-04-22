use ainl_context_compiler::{AnchoredSummary, Segment, Summarizer, SummarizerError};

/// In-process “Tier 1” [`Summarizer`] for [`crate::compose_telemetry::context_compiler_for_telemetry`]:
/// rolls dropped `OlderTurn` (and any other) segments from the compiler into a structured
/// [`AnchoredSummary`] `current_state` section **without** an extra LLM round-trip.
/// Intended for wiring tests and low-latency ops; swap for an LLM-backed implementation later.
#[derive(Debug, Default, Clone, Copy)]
pub struct HeuristicAnchorSummarizer;

impl Summarizer for HeuristicAnchorSummarizer {
    fn summarize(
        &self,
        segments: &[Segment],
        existing: Option<&AnchoredSummary>,
    ) -> Result<AnchoredSummary, SummarizerError> {
        let mut s = existing.cloned().unwrap_or_else(AnchoredSummary::empty);
        let joined: String = segments
            .iter()
            .map(|seg| format!("[{}] {}", seg.kind.as_str(), seg.content))
            .collect::<Vec<_>>()
            .join("\n\n");
        for section in s.sections.iter_mut() {
            if section.id == "current_state" {
                section.content = joined;
                break;
            }
        }
        s.token_estimate = ainl_compression::tokenize_estimate(&s.to_prompt_text());
        s.iteration = s.iteration.saturating_add(1);
        Ok(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ainl_context_compiler::{Role as CRole, Segment};

    #[test]
    fn heuristic_fills_current_state() {
        let s = HeuristicAnchorSummarizer;
        let segs = [Segment::older_turn(
            CRole::Assistant,
            "dropped",
            2,
        )];
        let out = s.summarize(&segs, None).expect("ok");
        let st = out
            .sections
            .iter()
            .find(|x| x.id == "current_state")
            .expect("section");
        assert!(st.content.contains("older_turn"), "{}", st.content);
        assert!(st.content.contains("dropped"), "{}", st.content);
    }
}
