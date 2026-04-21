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
/// `catalog_lookup`: if `Some`, `model_id` is resolved to a tier; if missing, `"auto"` treats as
/// `provider_is_inference_server` (i.e. on when the agent's provider IS the AINL inference
/// server, off otherwise). Without that signal, `"auto"` + unknown tier defaults to off so we
/// stay safely opt-in for hosted/free-tier OpenAI-style providers.
///
/// **Why `provider_is_inference_server` overrides the unknown-tier default:** if the operator
/// explicitly chose the AINL inference server as the provider, they want the GraphPatch planner
/// pipeline. The model name (e.g. `Qwen/Qwen2.5-0.5B-Instruct-GGUF` proxied via llama.cpp) is
/// usually not in the public model catalog, so a strict tier check would silently fall through
/// to the legacy tool loop — exactly the failure mode that motivated this autowire.
pub fn resolve_planner_mode(
    metadata: &HashMap<String, Value>,
    model_id: &str,
    catalog_lookup: Option<ModelTier>,
    provider_is_inference_server: bool,
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
                // Unknown tier: opt in iff the provider IS the AINL inference server,
                // since that explicit selection is itself the strongest opt-in signal.
                let _ = model_id;
                provider_is_inference_server
            }
        },
        Some(_) => false,
    }
}

/// Env-only base URL override for `POST /armara/v1/infer` (scheme + host + optional port),
/// **without** `/v1` suffix. Prefer [`effective_native_infer_base_url`] which also detects
/// the inference-server URL from the agent's configured provider.
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

/// Detect whether `(provider_name, optional configured URL)` refers to the
/// AINL `ainl-inference-server` Rust control plane.
///
/// True for any of:
///   * A canonical provider name string (`ainl-inference`, `ainl_inference`,
///     `ainl`, `armara-inference`, `armara_inference`, `armara-infer`, `armara`).
///   * A configured URL containing the native planner path (`/armara/v1`).
///   * A configured URL on the inference server's default port (`:8787`).
///
/// Used by [`effective_native_infer_base_url`] (autowire `ARMARA_NATIVE_INFER_URL`)
/// and by [`resolve_planner_mode`] (treat the explicit provider choice as the
/// strongest auto-on signal regardless of model tier).
pub fn provider_is_ainl_inference_server(provider: &str, url: Option<&str>) -> bool {
    let p = provider.trim().to_ascii_lowercase().replace('_', "-");
    if matches!(
        p.as_str(),
        "ainl"
            | "ainl-inference"
            | "ainl-infer"
            | "ainl-inference-server"
            | "armara"
            | "armara-inference"
            | "armara-infer"
    ) {
        return true;
    }
    if let Some(u) = url {
        let lu = u.to_ascii_lowercase();
        if lu.contains("/armara/v1") {
            return true;
        }
        if lu.contains(":8787") {
            return true;
        }
    }
    false
}

/// Effective `/armara/v1/infer` base URL, in priority order:
///   1. `ARMARA_NATIVE_INFER_URL` env override (operator explicit, wins).
///   2. The configured provider URL when the provider IS the AINL inference server
///      (auto-derived from `[provider_urls]` / dashboard `set_provider_url`).
///
/// Returns the URL with any `/v1` suffix stripped so the native infer driver can
/// safely append `/armara/v1/infer`.
pub fn effective_native_infer_base_url(
    provider: &str,
    provider_url: Option<&str>,
) -> Option<String> {
    if let Some(env) = native_infer_base_url() {
        return Some(env);
    }
    if provider_is_ainl_inference_server(provider, provider_url) {
        if let Some(u) = provider_url {
            return Some(normalize_infer_base_url(u));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn empty_meta() -> HashMap<String, Value> {
        HashMap::new()
    }

    #[test]
    fn provider_name_variants_map_to_inference_server() {
        for p in [
            "ainl",
            "ainl-inference",
            "ainl_inference",
            "AINL-Inference",
            "armara",
            "armara-infer",
            "armara_inference",
        ] {
            assert!(
                provider_is_ainl_inference_server(p, None),
                "expected provider {p:?} to be recognized as AINL inference server"
            );
        }
    }

    #[test]
    fn provider_url_with_native_path_or_default_port_is_inference_server() {
        assert!(provider_is_ainl_inference_server(
            "openai",
            Some("http://127.0.0.1:8787/armara/v1/infer")
        ));
        assert!(provider_is_ainl_inference_server(
            "openai",
            Some("http://localhost:8787/v1")
        ));
        assert!(!provider_is_ainl_inference_server(
            "openai",
            Some("https://api.openai.com/v1")
        ));
        assert!(!provider_is_ainl_inference_server("openai", None));
    }

    /// Combined env-touching test: cargo runs `#[test]` functions in parallel
    /// across a binary by default, and `serial_test` isn't a dependency of this
    /// crate, so all assertions that mutate `ARMARA_NATIVE_INFER_URL` /
    /// `ARMARA_PLANNER_MODE` live in a single function to guarantee strict
    /// sequencing and a single save/restore cycle per env var.
    #[test]
    fn env_aware_resolution_and_url_fallback() {
        let prev_url = std::env::var("ARMARA_NATIVE_INFER_URL").ok();
        let prev_mode = std::env::var("ARMARA_PLANNER_MODE").ok();
        std::env::remove_var("ARMARA_NATIVE_INFER_URL");
        std::env::remove_var("ARMARA_PLANNER_MODE");

        // 1. No env, non-inference provider URL → effective URL is None.
        assert_eq!(
            effective_native_infer_base_url("openai", Some("https://api.openai.com/v1")),
            None
        );

        // 2. No env, ainl-inference provider with URL → strip /v1, return base.
        assert_eq!(
            effective_native_infer_base_url(
                "ainl-inference",
                Some("http://127.0.0.1:8787/v1")
            ),
            Some("http://127.0.0.1:8787".to_string())
        );

        // 3. Env override always wins.
        std::env::set_var("ARMARA_NATIVE_INFER_URL", "http://override:9000");
        assert_eq!(
            effective_native_infer_base_url(
                "ainl-inference",
                Some("http://127.0.0.1:8787/v1")
            ),
            Some("http://override:9000".to_string())
        );
        std::env::remove_var("ARMARA_NATIVE_INFER_URL");

        // 4. Unknown tier + inference-server hint → planner ON; without the hint → OFF.
        assert!(!resolve_planner_mode(&empty_meta(), "qwen-0.5b", None, false));
        assert!(resolve_planner_mode(&empty_meta(), "qwen-0.5b", None, true));

        // 5. Explicit metadata still wins over the autowire hint.
        let mut meta = HashMap::new();
        meta.insert("planner_mode".into(), Value::String("off".into()));
        assert!(!resolve_planner_mode(&meta, "qwen-0.5b", None, true));

        // 6. Global env kill-switch overrides everything.
        std::env::set_var("ARMARA_PLANNER_MODE", "off");
        assert!(!resolve_planner_mode(&empty_meta(), "qwen-0.5b", None, true));

        match prev_url {
            Some(v) => std::env::set_var("ARMARA_NATIVE_INFER_URL", v),
            None => std::env::remove_var("ARMARA_NATIVE_INFER_URL"),
        }
        match prev_mode {
            Some(v) => std::env::set_var("ARMARA_PLANNER_MODE", v),
            None => std::env::remove_var("ARMARA_PLANNER_MODE"),
        }
    }
}
