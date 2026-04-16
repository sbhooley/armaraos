//! Adaptive eco mode resolution: provider cache capability matrix + model catalog pricing.
//!
//! Default Milestone 2 behavior is **shadow-only** — see [`openfang_types::adaptive_eco::AdaptiveEcoConfig::enforce`].

use openfang_types::adaptive_eco::{
    AdaptiveEcoConfig, AdaptiveEcoHysteresisState, AdaptiveEcoTurnSnapshot,
};
use openfang_types::agent::AgentManifest;

use crate::model_catalog::ModelCatalog;

/// Normalize user/config eco strings to `off` | `balanced` | `aggressive`.
#[must_use]
pub fn normalize_efficient_mode(raw: &str) -> &'static str {
    let s = raw.trim().to_ascii_lowercase();
    if s.contains("aggressive") {
        "aggressive"
    } else if s.contains("balanced") {
        "balanced"
    } else if s.contains("off") || s.is_empty() {
        "off"
    } else {
        "balanced"
    }
}

/// Provider → durable `cache_capability` label (used for policy, telemetry, and TTL dampening).
#[must_use]
pub fn cache_capability_label(provider: &str) -> &'static str {
    let p = provider.trim().to_ascii_lowercase();
    match p.as_str() {
        "anthropic" => "explicit_prompt_cache",
        "openai" => "implicit_automatic",
        "google" | "gemini" => "limited_or_none",
        "groq" | "ollama" | "nvidia" | "lmstudio" | "vllm" => "none_local",
        "openrouter" => "routed_provider_dependent",
        _ => "unknown",
    }
}

/// True when the snapshot label indicates vendor prompt caching that shares a TTL window with billing.
#[must_use]
pub fn prompt_cache_capability_label(label: &str) -> bool {
    matches!(
        label.trim(),
        "explicit_prompt_cache" | "implicit_automatic" | "routed_provider_dependent"
    )
}

/// Relative compression aggressiveness for policy comparisons (higher = more compression).
#[must_use]
pub fn compression_tier_rank(mode: &str) -> u8 {
    match normalize_efficient_mode(mode) {
        "aggressive" => 2,
        "balanced" => 1,
        _ => 0,
    }
}

/// Step down one compression tier: aggressive → balanced → off.
#[must_use]
pub fn step_down_efficient_mode(mode: &str) -> &'static str {
    let s = mode.trim().to_ascii_lowercase();
    if s.contains("aggressive") {
        "balanced"
    } else {
        "off"
    }
}

/// If enough recent semantic scores fall below `cfg.semantic_floor`, return a more conservative mode.
#[must_use]
pub fn circuit_breaker_adjust_base(
    base_mode: &str,
    cfg: &AdaptiveEcoConfig,
    recent_scores_newest_first: &[f32],
) -> (String, bool) {
    if !cfg.circuit_breaker_enabled {
        return (base_mode.to_string(), false);
    }
    let window = cfg.circuit_breaker_window.max(1) as usize;
    let slice = if recent_scores_newest_first.len() > window {
        &recent_scores_newest_first[..window]
    } else {
        recent_scores_newest_first
    };
    if slice.is_empty() {
        return (base_mode.to_string(), false);
    }
    let floor = cfg.semantic_floor;
    let need = cfg.circuit_breaker_min_below_floor.max(1) as usize;
    let below = slice.iter().filter(|&&s| s < floor).count();
    if below < need {
        return (base_mode.to_string(), false);
    }
    let stepped = step_down_efficient_mode(base_mode);
    let tripped = stepped != normalize_efficient_mode(base_mode);
    (stepped.to_string(), tripped)
}

fn structured_payload_heavy(message: &str) -> bool {
    if message.len() < 120 {
        return false;
    }
    let code_ticks = message.matches("```").count();
    let braces = message.matches('{').count() + message.matches('}').count();
    let sqlish = message.to_uppercase().contains("SELECT ")
        && (message.contains(';') || message.contains("WHERE "));
    code_ticks >= 2 || braces >= 10 || sqlish
}

/// Resolve adaptive eco mode for this turn. Callers should set manifest `adaptive_eco` metadata from the result.
#[must_use]
pub fn resolve_adaptive_eco_turn(
    cfg: &AdaptiveEcoConfig,
    manifest: &AgentManifest,
    user_message: &str,
    catalog: &ModelCatalog,
) -> AdaptiveEcoTurnSnapshot {
    let provider = manifest.model.provider.trim().to_string();
    let model = manifest.model.model.trim().to_string();
    let cache_capability = cache_capability_label(&provider).to_string();

    let input_price_per_million = catalog.pricing(&model).map(|(inp, _)| inp);

    let base = normalize_efficient_mode(
        manifest
            .metadata
            .get("efficient_mode")
            .and_then(|v| v.as_str())
            .unwrap_or("off"),
    );

    let mut reason_codes: Vec<String> = Vec::new();
    reason_codes.push("adaptive_eco:v1".to_string());
    reason_codes.push(format!("provider_capability:{cache_capability}"));

    if let Some(p) = input_price_per_million {
        reason_codes.push("pricing_from_model_catalog".to_string());
        reason_codes.push(format!("input_price_per_million:{p:.6}"));
    } else {
        reason_codes.push("pricing_unresolved_for_model".to_string());
    }

    let mut recommended = base.to_string();

    if structured_payload_heavy(user_message) && recommended == "aggressive" && !cfg.allow_aggressive_on_structured {
        recommended = "balanced".to_string();
        reason_codes.push("structured_payload_guard:cap_aggressive".to_string());
    }

    // Effective mode for this turn starts at the working base; the kernel applies
    // enforcement + hysteresis when `cfg.enforce` is true.
    let effective_mode = base.to_string();

    if !cfg.enforce {
        reason_codes.push("shadow_only:enforce_off".to_string());
    } else {
        reason_codes.push("enforce:on".to_string());
    }

    let shadow_only = !cfg.enforce;

    AdaptiveEcoTurnSnapshot {
        effective_mode,
        recommended_mode: recommended,
        base_mode_before_circuit: None,
        circuit_breaker_tripped: false,
        hysteresis_blocked: false,
        reason_codes,
        provider,
        model,
        cache_capability,
        input_price_per_million,
        shadow_only,
        enforce: cfg.enforce,
    }
}

/// When [`AdaptiveEcoConfig::enforce`] is true, require `min_n` consecutive matching recommendations
/// before switching modes. Uses `billing_id` as the stable key (matches usage rows).
pub fn hysteresis_resolve_adaptive_effective(
    map: &dashmap::DashMap<openfang_types::agent::AgentId, AdaptiveEcoHysteresisState>,
    billing_id: openfang_types::agent::AgentId,
    current_mode: &str,
    recommended: &str,
    min_n: u32,
) -> (String, bool) {
    let min_n = min_n.max(1);
    let cur = normalize_efficient_mode(current_mode);
    let rec = normalize_efficient_mode(recommended);
    if rec == cur {
        map.remove(&billing_id);
        return (cur.to_string(), false);
    }
    let rec_owned = rec.to_string();
    if let Some(mut existing) = map.get_mut(&billing_id) {
        if existing.pending_target.as_ref() == Some(&rec_owned) {
            existing.streak = existing.streak.saturating_add(1);
        } else {
            existing.pending_target = Some(rec_owned.clone());
            existing.streak = 1;
        }
        let streak = existing.streak;
        drop(existing);
        if streak >= min_n {
            map.remove(&billing_id);
            return (rec_owned, false);
        }
        return (cur.to_string(), true);
    }
    map.insert(
        billing_id,
        AdaptiveEcoHysteresisState {
            pending_target: Some(rec_owned.clone()),
            streak: 1,
        },
    );
    if 1 >= min_n {
        map.remove(&billing_id);
        return (rec_owned, false);
    }
    (cur.to_string(), true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use openfang_types::agent::AgentManifest;
    use openfang_types::adaptive_eco::AdaptiveEcoConfig;

    fn manifest_with(mode: &str, provider: &str, model: &str) -> AgentManifest {
        let mut m = AgentManifest::default();
        m.model.provider = provider.to_string();
        m.model.model = model.to_string();
        m.metadata.insert(
            "efficient_mode".to_string(),
            serde_json::Value::String(mode.to_string()),
        );
        m
    }

    #[test]
    fn shadow_keeps_base_mode() {
        let cat = ModelCatalog::new();
        let mut cfg = AdaptiveEcoConfig::default();
        cfg.enabled = true;
        cfg.enforce = false;
        let man = manifest_with("balanced", "openrouter", "claude-sonnet-4-20250514");
        let snap = resolve_adaptive_eco_turn(&cfg, &man, "plain short", &cat);
        assert_eq!(snap.effective_mode, "balanced");
        assert!(snap.shadow_only);
        assert!(snap.reason_codes.iter().any(|s| s.contains("shadow_only")));
    }

    #[test]
    fn circuit_breaker_steps_down_when_semantics_bad() {
        let mut cfg = AdaptiveEcoConfig::default();
        cfg.circuit_breaker_enabled = true;
        cfg.circuit_breaker_window = 8;
        cfg.circuit_breaker_min_below_floor = 2;
        cfg.semantic_floor = 0.9;
        let scores = vec![0.5_f32, 0.4_f32];
        let (m, trip) = circuit_breaker_adjust_base("aggressive", &cfg, &scores);
        assert_eq!(m, "balanced");
        assert!(trip);
    }

    #[test]
    fn hysteresis_requires_streak() {
        let map = dashmap::DashMap::new();
        let id = openfang_types::agent::AgentId::new();
        let (m1, b1) = hysteresis_resolve_adaptive_effective(&map, id, "balanced", "off", 2);
        assert_eq!(m1, "balanced");
        assert!(b1);
        let (m2, b2) = hysteresis_resolve_adaptive_effective(&map, id, "balanced", "off", 2);
        assert_eq!(m2, "off");
        assert!(!b2);
        assert!(map.is_empty());
    }

    #[test]
    fn tier_rank_and_cache_labels_sanity() {
        assert_eq!(compression_tier_rank("off"), 0);
        assert_eq!(compression_tier_rank("balanced"), 1);
        assert_eq!(compression_tier_rank("aggressive"), 2);
        assert!(prompt_cache_capability_label("explicit_prompt_cache"));
        assert!(!prompt_cache_capability_label("none_local"));
    }

    #[test]
    fn structured_guard_caps_aggressive_recommendation() {
        let cat = ModelCatalog::new();
        let mut cfg = AdaptiveEcoConfig::default();
        cfg.enabled = true;
        cfg.allow_aggressive_on_structured = false;
        let man = manifest_with("aggressive", "anthropic", "claude-sonnet-4-20250514");
        let msg = format!(
            "{}\n```json\n{{\"x\":1}}\n```\n```sql\nSELECT 1;\n```\n",
            "{\"a\":1,\"b\":2,\"c\":3,\"d\":4,\"e\":5,\"f\":6,\"g\":7,\"h\":8}".repeat(8)
        );
        let snap = resolve_adaptive_eco_turn(&cfg, &man, msg.as_str(), &cat);
        assert_eq!(snap.recommended_mode, "balanced");
        assert!(snap
            .reason_codes
            .iter()
            .any(|s| s.starts_with("structured_payload_guard")));
    }
}
