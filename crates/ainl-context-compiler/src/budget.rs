//! Token budget policy.
//!
//! Defaults are drawn from 2026 industry consensus (Factory.ai eval, Zylos research, Taskade
//! field guide):
//!
//! - System prompt: never compress (0:1)
//! - Recent 5-7 turns: never compress
//! - Older conversation history: 3:1 to 5:1 ratio target
//! - Tool outputs / observations: 10:1 to 20:1 ratio target
//! - Trigger compaction at 70% context utilization
//! - Recent context-rot research: keep total under ~30K tokens regardless of model max

use serde::{Deserialize, Serialize};

/// Token-budget allocation policy for [`crate::ContextCompiler::compose`].
///
/// All ratios are token estimates via [`ainl_compression::tokenize_estimate`] (heuristic
/// `chars / 4 + 1`). Hosts that need provider-accurate counts can rebuild their own
/// `BudgetPolicy` from a real tokenizer count and pass it in.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetPolicy {
    /// Total prompt window in token estimate (input side only; reserve for completion is excluded
    /// before this struct sees the budget).
    pub total_window: usize,
    /// Fraction reserved for the system prompt (kept verbatim regardless of mode).
    pub system_reserve_pct: f32,
    /// Fraction reserved for tool definitions.
    pub tool_def_reserve_pct: f32,
    /// Fraction reserved for the latest user prompt.
    pub user_prompt_reserve_pct: f32,
    /// Number of most-recent turns kept verbatim (no compaction).
    pub recent_turns_keep_verbatim: usize,
    /// Trigger compaction when used budget exceeds this fraction of `total_window`.
    pub trigger_compaction_at_pct: f32,
    /// Whether scoring should consult `ainl-context-freshness` to rank stale segments down.
    pub freshness_aware: bool,
    /// Whether scoring/aggressiveness should consult vitals (cap at Balanced when trust < 0.5).
    pub vitals_aware: bool,
    /// Soft cap on total assembled prompt (industry context-rot research suggests ~30K).
    pub soft_total_cap: usize,
}

impl Default for BudgetPolicy {
    fn default() -> Self {
        Self {
            // 30K is the practical sweet spot per recent context-rot research, even on 200K-window
            // models. Larger windows degrade attention quality non-linearly.
            total_window: 30_000,
            system_reserve_pct: 0.10,
            tool_def_reserve_pct: 0.10,
            user_prompt_reserve_pct: 0.05,
            recent_turns_keep_verbatim: 6,
            trigger_compaction_at_pct: 0.70,
            freshness_aware: true,
            vitals_aware: true,
            soft_total_cap: 30_000,
        }
    }
}

impl BudgetPolicy {
    /// Tokens reserved for system prompt content.
    #[must_use]
    pub fn system_budget(&self) -> usize {
        ((self.total_window as f32) * self.system_reserve_pct) as usize
    }

    /// Tokens reserved for tool definitions.
    #[must_use]
    pub fn tool_def_budget(&self) -> usize {
        ((self.total_window as f32) * self.tool_def_reserve_pct) as usize
    }

    /// Tokens reserved for the latest user prompt.
    #[must_use]
    pub fn user_prompt_budget(&self) -> usize {
        ((self.total_window as f32) * self.user_prompt_reserve_pct) as usize
    }

    /// Remaining budget for history + tool results + memory blocks after fixed reservations.
    #[must_use]
    pub fn flexible_budget(&self) -> usize {
        self.total_window
            .saturating_sub(self.system_budget())
            .saturating_sub(self.tool_def_budget())
            .saturating_sub(self.user_prompt_budget())
    }

    /// Whether to start compaction now given current used-token count.
    #[must_use]
    pub fn should_compact(&self, used_tokens: usize) -> bool {
        let trigger = ((self.total_window as f32) * self.trigger_compaction_at_pct) as usize;
        used_tokens >= trigger
    }

    /// Build a policy sized for a given model context window. Caller is responsible for
    /// subtracting the provider-side completion reservation before passing the input budget.
    #[must_use]
    pub fn for_window(input_window_tokens: usize) -> Self {
        let mut p = Self::default();
        p.total_window = input_window_tokens.min(p.soft_total_cap.max(input_window_tokens));
        // If the host gave us a smaller-than-default window, shrink the soft cap with it.
        p.soft_total_cap = p.soft_total_cap.min(input_window_tokens.max(1));
        p
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_reserves_sum_below_total() {
        let p = BudgetPolicy::default();
        let reserves =
            p.system_budget() + p.tool_def_budget() + p.user_prompt_budget();
        assert!(reserves < p.total_window);
        assert!(p.flexible_budget() > 0);
    }

    #[test]
    fn compaction_triggers_at_threshold() {
        let p = BudgetPolicy::default();
        assert!(!p.should_compact(0));
        assert!(p.should_compact((p.total_window as f32 * 0.71) as usize));
    }

    #[test]
    fn for_window_respects_soft_cap() {
        let p = BudgetPolicy::for_window(8_000);
        assert!(p.total_window <= 30_000);
        assert!(p.flexible_budget() > 0);
    }
}
