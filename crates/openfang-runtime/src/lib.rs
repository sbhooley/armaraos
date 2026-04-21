//! Agent runtime and execution environment.
//!
//! Manages the agent execution loop, LLM driver abstraction,
//! tool execution, and WASM sandboxing for untrusted skill/plugin code.

/// Compile-time **openfang-runtime** Cargo features for AINL crates (for `GET /api/status` handoff).
#[must_use]
pub fn ainl_integration_compile_flags() -> serde_json::Value {
    serde_json::json!({
        "ainl_extractor": cfg!(feature = "ainl-extractor"),
        "ainl_tagger": cfg!(feature = "ainl-tagger"),
        "ainl_persona_evolution": cfg!(feature = "ainl-persona-evolution"),
        "ainl_runtime_engine": cfg!(feature = "ainl-runtime-engine"),
    })
}

/// When `ARMARAOS_DISABLE_AINL_RUNTIME_ENGINE` is set to a truthy value (`1`, `true`, `yes`, `on`),
/// the embedded AINL runtime engine path is forced off for every agent, even if manifests opt in
/// or `AINL_RUNTIME_ENGINE=1` is set. Intended for emergency rollback without rebuilding.
#[must_use]
pub fn ainl_runtime_engine_env_disabled() -> bool {
    std::env::var("ARMARAOS_DISABLE_AINL_RUNTIME_ENGINE")
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

#[cfg(test)]
pub(crate) fn runtime_env_test_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// Runtime counters for ainl-runtime bridge cache behavior.
#[must_use]
pub fn ainl_runtime_bridge_metrics() -> serde_json::Value {
    #[cfg(feature = "ainl-runtime-engine")]
    {
        let (hits, misses, construct_failures, run_failures) =
            agent_loop::ainl_runtime_bridge_cache_metrics_snapshot();
        serde_json::json!({
            "cache_hits": hits,
            "cache_misses": misses,
            "construct_failures": construct_failures,
            "run_failures": run_failures,
        })
    }
    #[cfg(not(feature = "ainl-runtime-engine"))]
    {
        serde_json::json!({
            "cache_hits": 0,
            "cache_misses": 0,
            "construct_failures": 0,
            "run_failures": 0,
        })
    }
}

/// Default User-Agent header sent with all outgoing HTTP requests.
/// Some LLM providers (e.g. Moonshot, Qwen) reject requests without one.
pub const USER_AGENT: &str = "openfang/0.3.48";

pub mod a2a;
pub mod agent_loop;
pub mod ainl_graph_extractor_bridge;
#[cfg(feature = "ainl-runtime-engine")]
pub mod ainl_runtime_bridge;
pub mod ainl_semantic_tagger_bridge;

pub mod ainl_bundle_cron;
pub mod ainl_inbox_reader;
pub mod apply_patch;
pub mod audit;
pub mod auth_cooldown;
pub mod browser;
pub mod command_lane;
pub mod compactor;
pub mod context_budget;
pub mod context_overflow;
pub mod copilot_oauth;
pub mod docker_sandbox;
pub mod document_tools;
pub mod drivers;
pub mod eco_counterfactual;
pub mod eco_mode_resolver;
pub(crate) mod eco_telemetry;
pub mod embedding;
pub mod graceful_shutdown;
pub mod graph_extractor;
pub mod graph_memory_context;
pub mod graph_memory_writer;
pub mod hooks;
pub mod host_ainl_snapshot;
pub mod host_functions;
pub mod image_gen;
pub mod kernel_handle;
pub mod link_understanding;
pub mod llm_driver;
pub mod llm_errors;
pub mod loop_guard;
pub mod ainl_policy;
pub mod mcp;
pub mod mcp_readiness;
pub mod mcp_server;
pub mod media_understanding;
pub mod local_voice_bootstrap;
pub mod model_catalog;
pub mod plan_executor;
pub mod planner_metrics;
pub mod planner_mode;
pub mod persona_evolution;
pub mod process_manager;
pub mod prompt_builder;
pub mod prompt_compressor;
pub mod provider_health;
pub mod python_runtime;
pub mod reply_directives;
pub mod retry;
pub mod routing;
pub mod sandbox;
pub mod session_repair;
pub mod shell_bleed;
pub mod str_utils;
pub mod subprocess_sandbox;
#[cfg(all(test, feature = "ainl-extractor"))]
mod tests;
pub mod think_filter;
pub mod tool_policy;
pub mod tool_runner;
pub mod tts;
pub mod vitals_classifier;
pub mod web_cache;
pub mod web_content;
pub mod web_fetch;
pub mod web_search;
pub mod workspace_context;
pub mod workspace_sandbox;
