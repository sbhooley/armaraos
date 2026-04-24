//! Host-callable gate for when failure “auto-suggestion” / prevention text should be emitted.
//!
//! [`should_emit_failure_suggestion`] is the contract hook referenced in
//! `docs/SELF_LEARNING_INTEGRATION_MAP.md` §16: Phase 2 must reject suggestions when the capture-time
//! repo/context freshness was **stale** (mirrors the Python plugin’s guardrails).

use ainl_contracts::ContextFreshness;

/// When `freshness_at_failure` is [`ContextFreshness::Stale`], callers should **not** inject
/// automatic failure-prevention or recall lines into the next prompt.
#[must_use]
pub fn should_emit_failure_suggestion(freshness_at_failure: Option<ContextFreshness>) -> bool {
    !matches!(freshness_at_failure, Some(ContextFreshness::Stale))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_blocks_suggestion() {
        assert!(!should_emit_failure_suggestion(Some(
            ContextFreshness::Stale
        )));
    }

    #[test]
    fn fresh_or_unknown_allows() {
        assert!(should_emit_failure_suggestion(Some(
            ContextFreshness::Fresh
        )));
        assert!(should_emit_failure_suggestion(Some(
            ContextFreshness::Unknown
        )));
        assert!(should_emit_failure_suggestion(None));
    }
}
