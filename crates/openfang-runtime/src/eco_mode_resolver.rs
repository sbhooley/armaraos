//! Adaptive eco mode resolution: provider cache capability matrix + model catalog pricing.
//!
//! Default Milestone 2 behavior is **shadow-only** — see [`openfang_types::adaptive_eco::AdaptiveEcoConfig::enforce`].

use openfang_types::adaptive_eco::{AdaptiveEcoConfig, AdaptiveEcoTurnSnapshot};
use openfang_types::agent::AgentManifest;

use crate::model_catalog::ModelCatalog;

fn normalize_mode(raw: &str) -> &'static str {
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

fn cache_capability_label(provider: &str) -> &'static str {
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

    let base = normalize_mode(
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

    let effective_mode = if cfg.enforce {
        recommended.clone()
    } else {
        base.to_string()
    };

    if !cfg.enforce {
        reason_codes.push("shadow_only:enforce_off".to_string());
    } else {
        reason_codes.push("enforce:on".to_string());
    }

    let shadow_only = !cfg.enforce;

    AdaptiveEcoTurnSnapshot {
        effective_mode,
        recommended_mode: recommended,
        reason_codes,
        provider,
        model,
        cache_capability,
        input_price_per_million,
        shadow_only,
        enforce: cfg.enforce,
    }
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
