//! Per-agent last vitals trust for end-of-trajectory **fitness delta** (current − previous).

use dashmap::DashMap;
use std::sync::OnceLock;

fn last_trust_by_agent() -> &'static DashMap<String, f32> {
    static M: OnceLock<DashMap<String, f32>> = OnceLock::new();
    M.get_or_init(DashMap::new)
}

/// Returns `current - previous` when a previous value exists; updates the stored trust for the next
/// turn. On the first observation for an agent, stores `current` and returns `None` (no delta).
#[must_use]
pub fn vitals_trust_fitness_delta(agent_id: &str, current: Option<f32>) -> Option<f32> {
    let c = current?;
    let m = last_trust_by_agent();
    let key = agent_id.to_string();
    let delta = m.get(&key).map(|prev| c - *prev);
    m.insert(key, c);
    delta
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_turn_has_delta() {
        let id = "a1";
        assert!(vitals_trust_fitness_delta(id, Some(0.5)).is_none());
        let d = vitals_trust_fitness_delta(id, Some(0.6))
            .expect("delta on second call");
        assert!((d - 0.1f32).abs() < 1e-4);
    }
}
