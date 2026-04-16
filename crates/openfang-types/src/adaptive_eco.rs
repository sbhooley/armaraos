//! Adaptive Ultra Cost-Efficient (eco) mode policy configuration and per-turn snapshots.
//!
//! Phase 1 (Milestone 2): shadow recommendations + reason codes; enforcement is opt-in via
//! [`AdaptiveEcoConfig::enforce`] so operators can validate behavior before auto-switching modes.

use serde::{Deserialize, Serialize};

/// Global and runtime knobs for adaptive eco mode (`[adaptive_eco]` in `config.toml`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AdaptiveEcoConfig {
    /// When true, run the adaptive resolver each turn and attach [`AdaptiveEcoTurnSnapshot`]
    /// under manifest metadata key `adaptive_eco`.
    pub enabled: bool,
    /// When false (default), the resolver does **not** change `efficient_mode` — recommendations
    /// are shadow-only. When true, [`AdaptiveEcoTurnSnapshot::effective_mode`] overwrites the
    /// injected `efficient_mode` for that turn.
    pub enforce: bool,
    /// When [`Self::enforce`] is true, require this many consecutive identical [`AdaptiveEcoTurnSnapshot::recommended_mode`]
    /// values before switching `efficient_mode` (reduces flip-flop).
    #[serde(default = "default_enforce_min_consecutive_turns")]
    pub enforce_min_consecutive_turns: u32,
    /// Allow recommending or enforcing `aggressive` when the user message looks structured
    /// (JSON/code/SQL-heavy). When false, cap at `balanced` when the base mode was `aggressive`.
    pub allow_aggressive_on_structured: bool,
    /// Minimum semantic preservation score (0.0–1.0) before recommending a step-down in future
    /// (reserved for rolling-window circuit breaker; included in config for one source of truth).
    pub semantic_floor: f32,
    /// When true (default), look at recent durable compression semantic scores in SQLite
    /// and step down compression when too many fall below [`Self::semantic_floor`].
    #[serde(default = "default_true")]
    pub circuit_breaker_enabled: bool,
    /// How many recent compression rows (with a semantic score) participate in the circuit breaker.
    #[serde(default = "default_circuit_breaker_window")]
    pub circuit_breaker_window: u32,
    /// Trip the breaker when at least this many samples in the window are strictly below [`Self::semantic_floor`].
    #[serde(default = "default_circuit_breaker_min_below_floor")]
    pub circuit_breaker_min_below_floor: u32,
    /// After a circuit-breaker step-down, block raising compression again for this many seconds (0 = disabled).
    #[serde(default)]
    pub post_circuit_cooldown_secs: u64,
    /// Minimum wall time between enforced mode changes per billing agent (0 = disabled).
    #[serde(default)]
    pub min_secs_between_enforced_changes: u64,
}

fn default_enforce_min_consecutive_turns() -> u32 {
    2
}

fn default_true() -> bool {
    true
}

fn default_circuit_breaker_window() -> u32 {
    12
}

fn default_circuit_breaker_min_below_floor() -> u32 {
    3
}

impl Default for AdaptiveEcoConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            enforce: false,
            enforce_min_consecutive_turns: default_enforce_min_consecutive_turns(),
            allow_aggressive_on_structured: false,
            semantic_floor: 0.82,
            circuit_breaker_enabled: default_true(),
            circuit_breaker_window: default_circuit_breaker_window(),
            circuit_breaker_min_below_floor: default_circuit_breaker_min_below_floor(),
            post_circuit_cooldown_secs: 0,
            min_secs_between_enforced_changes: 0,
        }
    }
}

/// Per-turn counterfactual compression comparison (actual applied mode vs baselines / recommendation).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EcoCounterfactualReceipt {
    pub applied_mode: String,
    pub original_tokens_est: u64,
    pub applied_compressed_tokens_est: u64,
    pub vs_off_tokens_saved: u64,
    pub vs_off_savings_pct: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recommended_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recommended_compressed_tokens_est: Option<u64>,
    /// Positive when the recommended mode would save **more** tokens than applied (shadow mismatch).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_saved_delta_recommended_minus_applied: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub balanced_compressed_tokens_est: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aggressive_extra_tokens_saved_vs_balanced: Option<u64>,
}

/// Pre-enforcement replay report (`GET /api/usage/adaptive-eco/replay`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdaptiveEcoReplayReport {
    pub window: String,
    pub adaptive_eco_events: u64,
    pub shadow_mismatch_turns: u64,
    pub shadow_mismatch_rate: f64,
    pub circuit_breaker_trips: u64,
    pub hysteresis_blocks: u64,
    pub eco_compression_turns: u64,
    pub compression_semantic_p50: Option<f64>,
    pub compression_semantic_p95: Option<f64>,
    pub compression_semantic_mean: Option<f64>,
}

/// Heuristic 0.0–1.0 confidence for adaptive policy for this turn (dashboard / receipts).
#[must_use]
pub fn compute_adaptive_confidence(
    snap: &AdaptiveEcoTurnSnapshot,
    semantic_preservation: Option<f32>,
) -> f32 {
    let sem = semantic_preservation.unwrap_or(0.88).clamp(0.0, 1.0);
    let mut score = 0.58 * sem;
    if snap.circuit_breaker_tripped {
        score -= 0.12;
    }
    if snap.hysteresis_blocked {
        score -= 0.06;
    }
    if snap.shadow_only && snap.recommended_mode != snap.effective_mode {
        score -= 0.07;
    }
    match snap.cache_capability.as_str() {
        "explicit_prompt_cache" | "implicit_automatic" => score += 0.08,
        "routed_provider_dependent" => score += 0.02,
        _ => {}
    }
    score.clamp(0.0, 1.0)
}

/// Serializable snapshot attached to manifest metadata (`adaptive_eco`) for telemetry and graph traces.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdaptiveEcoTurnSnapshot {
    /// Mode after policy resolution (may match base when `enforce` is false).
    pub effective_mode: String,
    /// Resolver recommendation (may differ from `effective_mode` when `enforce` is false).
    pub recommended_mode: String,
    /// Efficient mode before optional circuit-breaker step-down (user/orchestration/global base).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_mode_before_circuit: Option<String>,
    /// True when recent semantic scores tripped the breaker and the working base was stepped down.
    #[serde(default)]
    pub circuit_breaker_tripped: bool,
    /// True when [`AdaptiveEcoConfig::enforce`] is on but hysteresis deferred a mode change.
    #[serde(default)]
    pub hysteresis_blocked: bool,
    /// Human-readable machine codes for debugging and dashboards.
    pub reason_codes: Vec<String>,
    /// Provider id from the agent manifest `[model].provider`.
    pub provider: String,
    /// Model id from the agent manifest `[model].model`.
    pub model: String,
    /// Provider-specific cache capability label (see `eco_mode_resolver` in `openfang-runtime`).
    pub cache_capability: String,
    /// Input price per million tokens from the model catalog when resolvable.
    pub input_price_per_million: Option<f64>,
    /// True when the policy did not apply `recommended_mode` as `efficient_mode`.
    pub shadow_only: bool,
    /// Copy of [`AdaptiveEcoConfig::enforce`] for consumers that only read metadata.
    pub enforce: bool,
}

/// Per billing-agent hysteresis state for adaptive eco enforcement (in-memory; cleared on restart).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AdaptiveEcoHysteresisState {
    pub pending_target: Option<String>,
    pub streak: u32,
}

/// Durable row for SQLite (`adaptive_eco_events`), including optional per-turn semantic score.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdaptiveEcoUsageRecord {
    pub agent_id: crate::agent::AgentId,
    pub effective_mode: String,
    pub recommended_mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_mode_before_circuit: Option<String>,
    #[serde(default)]
    pub circuit_breaker_tripped: bool,
    #[serde(default)]
    pub hysteresis_blocked: bool,
    pub shadow_only: bool,
    pub enforce: bool,
    pub provider: String,
    pub model: String,
    pub cache_capability: String,
    pub input_price_per_million: Option<f64>,
    pub reason_codes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_preservation_score: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adaptive_confidence: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub counterfactual: Option<EcoCounterfactualReceipt>,
}

/// Lightweight aggregates for dashboards / `GET /api/usage/adaptive-eco`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdaptiveEcoUsageSummary {
    pub window: String,
    pub events: u64,
    pub shadow_mismatch_turns: u64,
    pub circuit_breaker_trips: u64,
    pub hysteresis_blocks: u64,
}
