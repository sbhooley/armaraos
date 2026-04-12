//! Configurable caps for the agent loop, inter-agent tools, and workflow run retention.
//!
//! On-disk schema: `[runtime_limits]` in `config.toml`. Per-agent overrides use
//! `runtime_*` keys in agent manifest `[metadata]` (see `EffectiveRuntimeLimits`).

use crate::agent::AgentManifest;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use tracing::warn;

/// Hard ceilings when unbounded mode is **off** (config + env double opt-in not both active).
pub const BOUNDED_CEILING_CONTINUE: u32 = 64;
pub const BOUNDED_CEILING_ITERATIONS: u32 = 64;
pub const BOUNDED_CEILING_AGENT_DEPTH: u32 = 64;
pub const BOUNDED_CEILING_HISTORY_MESSAGES: usize = 1024;
pub const BOUNDED_CEILING_WORKFLOW_RUNS: usize = 512;

/// Absolute maxima when unbounded mode **is** on (`allow_unbounded_agent_loop` + `ARMARAOS_UNBOUNDED=1`).
pub const UNBOUNDED_MAX_CONTINUE: u32 = 512;
pub const UNBOUNDED_MAX_ITERATIONS: u32 = 512;
pub const UNBOUNDED_MAX_AGENT_DEPTH: u32 = 512;
pub const UNBOUNDED_MAX_HISTORY_MESSAGES: usize = 4096;
pub const UNBOUNDED_MAX_WORKFLOW_RUNS: usize = 10_000;
/// Cap TTL even in unbounded mode (90 days).
pub const UNBOUNDED_MAX_WORKFLOW_TTL_SECS: u64 = 90 * 24 * 3600;

/// `true` when config allows and the process environment opts in.
pub fn unbounded_runtime_mode_active(cfg: &RuntimeLimitsConfig) -> bool {
    cfg.allow_unbounded_agent_loop
        && std::env::var("ARMARAOS_UNBOUNDED").ok().as_deref() == Some("1")
}

/// `[runtime_limits]` table in `config.toml`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct RuntimeLimitsConfig {
    /// Max tool/LLM iterations per user turn when manifest has no `[autonomous].max_iterations`.
    pub max_iterations: u32,
    /// Max consecutive `StopReason::MaxTokens` continuations before returning a partial reply.
    pub max_continuations: u32,
    /// Trim session to this many messages before the LLM call (safety valve).
    pub max_history_messages: usize,
    /// Max nested `agent_send` depth (A→B→C…).
    pub max_agent_call_depth: u32,
    /// Max workflow run records kept in memory (completed/failed evicted first).
    pub workflow_max_retained_runs: usize,
    /// Drop completed/failed runs older than this many seconds (None = time-based eviction off).
    pub workflow_run_ttl_secs: Option<u64>,
    /// When `true` **and** `ARMARAOS_UNBOUNDED=1`, apply [`UNBOUNDED_*`] ceilings instead of tight [`BOUNDED_*`] caps.
    pub allow_unbounded_agent_loop: bool,
    /// When set, new [`OrchestrationContext`](crate::orchestration::OrchestrationContext) roots that do not
    /// already have `remaining_budget_ms` get this wall-clock budget (e.g. spawn + orchestration triggers).
    /// `None` leaves the budget unset (no wall-clock cap). TOML: `[runtime_limits] orchestration_default_budget_ms`.
    #[serde(default)]
    pub orchestration_default_budget_ms: Option<u64>,
}

impl Default for RuntimeLimitsConfig {
    fn default() -> Self {
        Self {
            max_iterations: 80,
            max_continuations: 5,
            max_history_messages: 60,
            max_agent_call_depth: 5,
            workflow_max_retained_runs: 200,
            workflow_run_ttl_secs: None,
            allow_unbounded_agent_loop: false,
            orchestration_default_budget_ms: None,
        }
    }
}

impl RuntimeLimitsConfig {
    /// Clamp file-backed limits to safe platform bounds; logs when values are adjusted.
    pub fn clamp_bounds(&mut self) {
        let unbounded = unbounded_runtime_mode_active(self);
        let before = self.clone();
        self.sanitize_minima();
        self.apply_ceiling(unbounded);
        if &before != self {
            warn!(
                ?before,
                after = ?self,
                unbounded,
                "runtime_limits clamped to safe bounds"
            );
        }
    }

    fn sanitize_minima(&mut self) {
        if self.max_iterations == 0 {
            self.max_iterations = 1;
        }
        if self.max_continuations == 0 {
            self.max_continuations = 1;
        }
        if self.max_history_messages == 0 {
            self.max_history_messages = 1;
        }
        if self.max_agent_call_depth == 0 {
            self.max_agent_call_depth = 1;
        }
        if self.workflow_max_retained_runs == 0 {
            self.workflow_max_retained_runs = 1;
        }
        if let Some(0) = self.workflow_run_ttl_secs {
            self.workflow_run_ttl_secs = None;
        }
        if let Some(0) = self.orchestration_default_budget_ms {
            self.orchestration_default_budget_ms = None;
        }
    }

    fn apply_ceiling(&mut self, unbounded: bool) {
        if unbounded {
            self.max_iterations = self.max_iterations.clamp(1, UNBOUNDED_MAX_ITERATIONS);
            self.max_continuations = self.max_continuations.clamp(1, UNBOUNDED_MAX_CONTINUE);
            self.max_history_messages = self
                .max_history_messages
                .clamp(1, UNBOUNDED_MAX_HISTORY_MESSAGES);
            self.max_agent_call_depth = self
                .max_agent_call_depth
                .clamp(1, UNBOUNDED_MAX_AGENT_DEPTH);
            self.workflow_max_retained_runs = self
                .workflow_max_retained_runs
                .clamp(1, UNBOUNDED_MAX_WORKFLOW_RUNS);
            if let Some(ttl) = self.workflow_run_ttl_secs {
                self.workflow_run_ttl_secs = Some(ttl.clamp(1, UNBOUNDED_MAX_WORKFLOW_TTL_SECS));
            }
            if let Some(ms) = self.orchestration_default_budget_ms {
                const MAX_MS: u64 = 90 * 24 * 3600 * 1000;
                self.orchestration_default_budget_ms = Some(ms.min(MAX_MS));
            }
        } else {
            self.max_iterations = self.max_iterations.clamp(1, BOUNDED_CEILING_ITERATIONS);
            self.max_continuations = self.max_continuations.clamp(1, BOUNDED_CEILING_CONTINUE);
            self.max_history_messages = self
                .max_history_messages
                .clamp(1, BOUNDED_CEILING_HISTORY_MESSAGES);
            self.max_agent_call_depth = self
                .max_agent_call_depth
                .clamp(1, BOUNDED_CEILING_AGENT_DEPTH);
            self.workflow_max_retained_runs = self
                .workflow_max_retained_runs
                .clamp(1, BOUNDED_CEILING_WORKFLOW_RUNS);
            if let Some(ttl) = self.workflow_run_ttl_secs {
                // Bounded mode: still allow long TTL but cap absurd values.
                self.workflow_run_ttl_secs = Some(ttl.clamp(1, UNBOUNDED_MAX_WORKFLOW_TTL_SECS));
            }
            if let Some(ms) = self.orchestration_default_budget_ms {
                const MAX_MS: u64 = 90 * 24 * 3600 * 1000;
                self.orchestration_default_budget_ms = Some(ms.min(MAX_MS));
            }
        }
    }
}

/// Snapshot used for one agent turn (global `[runtime_limits]` + manifest `[metadata]` overrides).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EffectiveRuntimeLimits {
    pub max_iterations: u32,
    pub max_continuations: u32,
    pub max_history_messages: usize,
    pub max_agent_call_depth: u32,
    pub workflow_max_retained_runs: usize,
    pub workflow_run_ttl_secs: Option<u64>,
}

impl EffectiveRuntimeLimits {
    /// Same numeric defaults as pre–Section 2 hard-coded constants.
    pub const fn legacy_defaults() -> Self {
        Self {
            max_iterations: 80,
            max_continuations: 5,
            max_history_messages: 60,
            max_agent_call_depth: 5,
            workflow_max_retained_runs: 200,
            workflow_run_ttl_secs: None,
        }
    }

    pub fn from_global_only(global: &RuntimeLimitsConfig) -> Self {
        Self {
            max_iterations: global.max_iterations,
            max_continuations: global.max_continuations,
            max_history_messages: global.max_history_messages,
            max_agent_call_depth: global.max_agent_call_depth,
            workflow_max_retained_runs: global.workflow_max_retained_runs,
            workflow_run_ttl_secs: global.workflow_run_ttl_secs,
        }
    }

    /// Merge `[runtime_limits]` with per-agent `runtime_*` metadata, then clamp to effective ceilings.
    pub fn from_global_and_manifest(
        global: &RuntimeLimitsConfig,
        manifest: &AgentManifest,
    ) -> Self {
        let mut e = Self::from_global_only(global);
        e.apply_manifest_metadata(&manifest.metadata);
        e.clamp_merged(global.allow_unbounded_agent_loop);
        e
    }

    fn apply_manifest_metadata(&mut self, metadata: &HashMap<String, Value>) {
        if let Some(v) = meta_u32(metadata, "runtime_max_iterations") {
            self.max_iterations = v;
        }
        if let Some(v) = meta_u32(metadata, "runtime_max_continuations") {
            self.max_continuations = v;
        }
        if let Some(v) = meta_usize(metadata, "runtime_max_history_messages") {
            self.max_history_messages = v;
        }
        if let Some(v) = meta_u32(metadata, "runtime_max_agent_call_depth") {
            self.max_agent_call_depth = v;
        }
        if let Some(v) = meta_usize(metadata, "runtime_workflow_max_retained_runs") {
            self.workflow_max_retained_runs = v;
        }
        if let Some(v) = meta_u64_optional(metadata, "runtime_workflow_run_ttl_secs") {
            self.workflow_run_ttl_secs = v;
        }
    }

    fn clamp_merged(&mut self, allow_unbounded_cfg: bool) {
        let mut tmp = RuntimeLimitsConfig {
            max_iterations: self.max_iterations,
            max_continuations: self.max_continuations,
            max_history_messages: self.max_history_messages,
            max_agent_call_depth: self.max_agent_call_depth,
            workflow_max_retained_runs: self.workflow_max_retained_runs,
            workflow_run_ttl_secs: self.workflow_run_ttl_secs,
            allow_unbounded_agent_loop: allow_unbounded_cfg,
            orchestration_default_budget_ms: None,
        };
        tmp.sanitize_minima();
        let unbounded = unbounded_runtime_mode_active(&tmp);
        tmp.apply_ceiling(unbounded);
        self.max_iterations = tmp.max_iterations;
        self.max_continuations = tmp.max_continuations;
        self.max_history_messages = tmp.max_history_messages;
        self.max_agent_call_depth = tmp.max_agent_call_depth;
        self.workflow_max_retained_runs = tmp.workflow_max_retained_runs;
        self.workflow_run_ttl_secs = tmp.workflow_run_ttl_secs;
    }

    /// Re-apply global ceilings after caller-side numeric overrides (e.g. workflow adaptive step).
    pub fn re_clamp_after_override(&mut self, global: &RuntimeLimitsConfig) {
        self.clamp_merged(global.allow_unbounded_agent_loop);
    }
}

/// Limits passed into workflow run creation / eviction (no manifest in some call paths).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkflowRetentionLimits {
    pub max_retained_runs: usize,
    pub run_ttl_secs: Option<u64>,
}

impl WorkflowRetentionLimits {
    pub fn from_global_config(cfg: &RuntimeLimitsConfig) -> Self {
        Self {
            max_retained_runs: cfg.workflow_max_retained_runs,
            run_ttl_secs: cfg.workflow_run_ttl_secs,
        }
    }

    pub fn from_effective(e: &EffectiveRuntimeLimits) -> Self {
        Self {
            max_retained_runs: e.workflow_max_retained_runs,
            run_ttl_secs: e.workflow_run_ttl_secs,
        }
    }

    /// Pre–Section 2 defaults (200 runs, no TTL).
    pub const fn legacy_default() -> Self {
        Self {
            max_retained_runs: 200,
            run_ttl_secs: None,
        }
    }
}

fn meta_u32(metadata: &HashMap<String, Value>, key: &str) -> Option<u32> {
    let v = metadata.get(key)?;
    if let Some(n) = v.as_u64() {
        return u32::try_from(n).ok();
    }
    if let Some(s) = v.as_str() {
        return s.trim().parse().ok();
    }
    None
}

fn meta_usize(metadata: &HashMap<String, Value>, key: &str) -> Option<usize> {
    let v = metadata.get(key)?;
    if let Some(n) = v.as_u64() {
        return usize::try_from(n).ok();
    }
    if let Some(s) = v.as_str() {
        return s.trim().parse().ok();
    }
    None
}

fn meta_u64_optional(metadata: &HashMap<String, Value>, key: &str) -> Option<Option<u64>> {
    let v = metadata.get(key)?;
    if v.is_null() {
        return Some(None);
    }
    if let Some(n) = v.as_u64() {
        return Some(Some(n));
    }
    if let Some(s) = v.as_str() {
        let t = s.trim();
        if t.is_empty() || t.eq_ignore_ascii_case("none") || t == "null" {
            return Some(None);
        }
        return Some(Some(t.parse().ok()?));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_clamp_caps_high_values() {
        let mut c = RuntimeLimitsConfig {
            max_iterations: 10_000,
            max_continuations: 200,
            max_history_messages: 50_000,
            max_agent_call_depth: 99,
            workflow_max_retained_runs: 99_999,
            workflow_run_ttl_secs: Some(999_999_999),
            allow_unbounded_agent_loop: false,
            orchestration_default_budget_ms: None,
        };
        c.clamp_bounds();
        assert_eq!(c.max_iterations, BOUNDED_CEILING_ITERATIONS);
        assert_eq!(c.max_continuations, BOUNDED_CEILING_CONTINUE);
        assert_eq!(c.max_history_messages, BOUNDED_CEILING_HISTORY_MESSAGES);
        assert_eq!(c.max_agent_call_depth, BOUNDED_CEILING_AGENT_DEPTH);
        assert_eq!(c.workflow_max_retained_runs, BOUNDED_CEILING_WORKFLOW_RUNS);
    }

    #[test]
    fn manifest_overrides_merge_then_clamp() {
        let global = RuntimeLimitsConfig {
            max_iterations: 40,
            ..Default::default()
        };
        let mut m = AgentManifest::default();
        m.metadata
            .insert("runtime_max_iterations".to_string(), Value::from(100_i64));
        let eff = EffectiveRuntimeLimits::from_global_and_manifest(&global, &m);
        assert_eq!(eff.max_iterations, BOUNDED_CEILING_ITERATIONS);
    }
}
