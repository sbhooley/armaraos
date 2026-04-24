//! Gate for when an extracted **tool-sequence** pattern is treated as a **reusable, prompt-visible**
//! procedure (Phase 3 — pattern promotion; see `docs/SELF_LEARNING_INTEGRATION_MAP.md`).
//!
//! A pattern must reach at least `MIN_OBSERVATIONS` reinforcements and have EMA fitness ≥
//! `FITNESS_FLOOR` (default 0.7) before it may appear in graph-memory `SuggestedProcedure` output.

use std::sync::OnceLock;

use ainl_contracts::vitals::VitalsGate;

/// Default minimum independent observations of the same normalized tool sequence before promotion.
pub const DEFAULT_MIN_OBSERVATIONS: u32 = 3;
/// Default minimum EMA fitness (0–1) for promotion, aligned with the self-learning map.
pub const DEFAULT_FITNESS_FLOOR: f32 = 0.7;
/// EMA α for per-turn confidence updates.
pub const EMA_ALPHA: f32 = 0.3;

fn min_observations() -> u32 {
    static V: OnceLock<u32> = OnceLock::new();
    *V.get_or_init(|| {
        std::env::var("AINL_PATTERN_PROMOTION_MIN_OBSERVATIONS")
            .ok()
            .and_then(|s| s.parse().ok())
            .filter(|&n| n > 0)
            .unwrap_or(DEFAULT_MIN_OBSERVATIONS)
    })
}

fn fitness_floor() -> f32 {
    static V: OnceLock<f32> = OnceLock::new();
    *V.get_or_init(|| {
        std::env::var("AINL_PATTERN_PROMOTION_FITNESS_FLOOR")
            .ok()
            .and_then(|s| s.parse().ok())
            .filter(|f| (0.0..=1.0).contains(f))
            .unwrap_or(DEFAULT_FITNESS_FLOOR)
    })
}

/// Returns whether a pattern with this observation count and EMA fitness should be shown in
/// SuggestedProcedure-style blocks and treated as “promoted”.
#[must_use]
pub fn should_promote_with_policy(
    pattern_observation_count: u32,
    ema_fitness: f32,
    min_obs: u32,
    floor: f32,
) -> bool {
    pattern_observation_count >= min_obs && ema_fitness >= floor
}

#[must_use]
pub fn should_promote(pattern_observation_count: u32, ema_fitness: f32) -> bool {
    should_promote_with_policy(
        pattern_observation_count,
        ema_fitness,
        min_observations(),
        fitness_floor(),
    )
}

/// EMA for fitness: `ema = (1-α) * previous + α * this_turn_confidence` with `None` previous →
/// the first observation value.
#[must_use]
pub fn ema_fitness_update(previous_ema: Option<f32>, this_turn_confidence: f32) -> f32 {
    let c = this_turn_confidence.clamp(0.0, 1.0);
    match previous_ema {
        None => c,
        Some(p) => ((1.0 - EMA_ALPHA) * p.clamp(0.0, 1.0) + EMA_ALPHA * c).clamp(0.0, 1.0),
    }
}

/// When `true`, the host should not persist or reinforce **extracted** tool-sequence patterns for
/// this turn. Maps integration-map “block on bad vitals” to `VitalsGate::Fail` (doc “Block” ↔ fail
/// gate; there is no separate `Block` variant).
#[must_use]
pub fn should_skip_pattern_persist_for_vitals(gate: Option<VitalsGate>) -> bool {
    matches!(gate, Some(VitalsGate::Fail))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ainl_contracts::vitals::VitalsGate;

    #[test]
    fn ema_starts_at_first_observation() {
        let e = ema_fitness_update(None, 0.8);
        assert!((e - 0.8).abs() < 0.0001);
    }

    #[test]
    fn promotion_requires_both_reach_and_fitness() {
        assert!(!should_promote_with_policy(2, 0.9, 3, 0.7));
        assert!(!should_promote_with_policy(3, 0.69, 3, 0.7));
        assert!(should_promote_with_policy(3, 0.7, 3, 0.7));
    }

    #[test]
    fn skip_persist_only_on_fail_gate() {
        assert!(!should_skip_pattern_persist_for_vitals(None));
        assert!(!should_skip_pattern_persist_for_vitals(Some(
            VitalsGate::Pass
        )));
        assert!(!should_skip_pattern_persist_for_vitals(Some(
            VitalsGate::Warn
        )));
        assert!(should_skip_pattern_persist_for_vitals(Some(
            VitalsGate::Fail
        )));
    }
}
