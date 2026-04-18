//! Resolve whether native planner (`InferRequest` + `AgentSnapshot`) should run for this turn.
//!
//! See plan: metadata `planner_mode`, env overrides, model tier from catalog when provided.

use openfang_types::model_catalog::ModelTier;
use serde_json::Value;
use std::collections::HashMap;

/// Global kill-switch / force-on (`ARMARA_PLANNER_MODE=off|on`).
pub fn env_planner_override() -> Option<bool> {
    match std::env::var("ARMARA_PLANNER_MODE") {
        Ok(s) => {
            let t = s.trim().to_ascii_lowercase();
            if t == "off" || t == "0" || t == "false" {
                Some(false)
            } else if t == "on" || t == "1" || t == "true" {
                Some(true)
            } else {
                None
            }
        }
        Err(_) => None,
    }
}

fn metadata_planner_mode(metadata: &HashMap<String, Value>) -> Option<&str> {
    metadata
        .get("planner_mode")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
}

fn tier_planner_default(tier: ModelTier) -> bool {
    // Map catalog tiers to planner defaults: small/fast/local → on; medium → opt-in; large → opt-in.
    match tier {
        ModelTier::Fast | ModelTier::Local => true,
        ModelTier::Balanced | ModelTier::Smart => std::env::var("ARMARA_PLANNER_MEDIUM_MODELS")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false),
        ModelTier::Frontier | ModelTier::Custom => std::env::var("ARMARA_PLANNER_LARGE_MODELS")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false),
    }
}

/// Resolve whether planner mode is active for this agent/model.
///
/// `catalog_lookup`: if `Some`, `model_id` is resolved to a tier; if missing, `"auto"` treats as off
/// (safe default) unless env forces on.
pub fn resolve_planner_mode(
    metadata: &HashMap<String, Value>,
    model_id: &str,
    catalog_lookup: Option<ModelTier>,
) -> bool {
    if let Some(on) = env_planner_override() {
        return on;
    }
    match metadata_planner_mode(metadata) {
        Some("off") | Some("false") => false,
        Some("on") | Some("true") => true,
        Some("auto") | None => match catalog_lookup {
            Some(tier) => tier_planner_default(tier),
            None => {
                // Unknown tier: stay off unless explicitly opted in via metadata (handled above).
                let _ = model_id;
                false
            }
        },
        Some(_) => false,
    }
}

/// Base URL for `POST /armara/v1/infer` (scheme + host + optional port), **without** `/v1` suffix.
pub fn native_infer_base_url() -> Option<String> {
    std::env::var("ARMARA_NATIVE_INFER_URL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Strip trailing `/v1` from provider OpenAI-style URLs so native infer can append `/armara/v1/infer`.
pub fn normalize_infer_base_url(raw: &str) -> String {
    let t = raw.trim().trim_end_matches('/');
    t.trim_end_matches("/v1").to_string()
}
