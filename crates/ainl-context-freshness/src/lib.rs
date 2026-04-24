//! Pure functions for [`ContextFreshness`](ainl_contracts::ContextFreshness) and execution gating.
//!
//! See also: [`ainl-context-compiler`](https://docs.rs/ainl-context-compiler) for LLM
//! context-window assembly (different lifecycle phase — this crate gates *tool execution* based
//! on repo-knowledge currency; the compiler crate assembles *prompt bytes* sent to the LLM).
//! Both crates can be used independently or together.

use ainl_contracts::{ContextFreshness, ImpactDecision};

/// Inputs for freshness evaluation (extend without breaking callers).
#[derive(Debug, Clone, Default)]
pub struct FreshnessInputs {
    /// When true, index/repo graph is known stale vs HEAD.
    pub index_stale_vs_head: Option<bool>,
    /// When true, freshness could not be determined.
    pub unknown: bool,
}

/// Decide freshness from explicit inputs.
#[must_use]
pub fn evaluate_freshness(i: &FreshnessInputs) -> ContextFreshness {
    if i.unknown {
        return ContextFreshness::Unknown;
    }
    match i.index_stale_vs_head {
        Some(true) => ContextFreshness::Stale,
        Some(false) => ContextFreshness::Fresh,
        None => ContextFreshness::Unknown,
    }
}

/// Policy: when to block execution until context is refreshed.
#[must_use]
pub fn impact_decision_strict(f: ContextFreshness, repo_intel_ready: bool) -> ImpactDecision {
    match f {
        ContextFreshness::Stale => {
            if repo_intel_ready {
                ImpactDecision::RequireImpactFirst
            } else {
                ImpactDecision::BlockUntilFresh
            }
        }
        ContextFreshness::Unknown => {
            if repo_intel_ready {
                ImpactDecision::RequireImpactFirst
            } else {
                ImpactDecision::RequireImpactFirst
            }
        }
        ContextFreshness::Fresh => ImpactDecision::AllowExecute,
    }
}

/// Lenient policy: never block, only suggest impact when stale/unknown if repo intel exists.
#[must_use]
pub fn impact_decision_balanced(f: ContextFreshness, repo_intel_ready: bool) -> ImpactDecision {
    match f {
        ContextFreshness::Fresh => ImpactDecision::AllowExecute,
        ContextFreshness::Stale | ContextFreshness::Unknown => {
            if repo_intel_ready {
                ImpactDecision::RequireImpactFirst
            } else {
                ImpactDecision::AllowExecute
            }
        }
    }
}

/// Combine with compile/run gate for AINL MCP.
#[must_use]
pub fn can_execute_with_context(f: ContextFreshness, strict: bool, repo_intel_ready: bool) -> bool {
    let d = if strict {
        impact_decision_strict(f, repo_intel_ready)
    } else {
        impact_decision_balanced(f, repo_intel_ready)
    };
    matches!(d, ImpactDecision::AllowExecute)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_allows() {
        assert!(can_execute_with_context(
            ContextFreshness::Fresh,
            true,
            false
        ));
    }
}
