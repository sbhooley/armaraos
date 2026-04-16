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
    /// Allow recommending or enforcing `aggressive` when the user message looks structured
    /// (JSON/code/SQL-heavy). When false, cap at `balanced` when the base mode was `aggressive`.
    pub allow_aggressive_on_structured: bool,
    /// Minimum semantic preservation score (0.0–1.0) before recommending a step-down in future
    /// (reserved for rolling-window circuit breaker; included in config for one source of truth).
    pub semantic_floor: f32,
}

impl Default for AdaptiveEcoConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            enforce: false,
            allow_aggressive_on_structured: false,
            semantic_floor: 0.82,
        }
    }
}

/// Serializable snapshot attached to manifest metadata (`adaptive_eco`) for telemetry and graph traces.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdaptiveEcoTurnSnapshot {
    /// Mode after policy resolution (may match base when `enforce` is false).
    pub effective_mode: String,
    /// Resolver recommendation (may differ from `effective_mode` when `enforce` is false).
    pub recommended_mode: String,
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
