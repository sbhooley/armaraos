//! Shared learner payloads (trajectory steps, failure taxonomy, proposal envelopes).
//!
//! Consumed by `ainl-trajectory` / `ainl-failure-learning` crates; hosts embed these in graph nodes.

use serde::{Deserialize, Serialize};

use crate::vitals::CognitiveVitals;
use crate::{ContextFreshness, ImpactDecision};

/// One step in an execution trajectory (tool / adapter granularity).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrajectoryStep {
    pub step_id: String,
    /// Unix epoch milliseconds.
    pub timestamp_ms: i64,
    pub adapter: String,
    pub operation: String,
    #[serde(default)]
    pub inputs_preview: Option<String>,
    #[serde(default)]
    pub outputs_preview: Option<String>,
    pub duration_ms: u64,
    pub success: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub vitals: Option<CognitiveVitals>,
    #[serde(default)]
    pub freshness_at_step: Option<ContextFreshness>,
    /// Optional per-step state snapshot (host-defined JSON, e.g. turn counters, budget).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_vars: Option<serde_json::Value>,
    /// Optional structured tool telemetry (latency breakdown, I/O, HTTP, etc.).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_telemetry: Option<serde_json::Value>,
}

/// Overall outcome of a recorded trajectory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrajectoryOutcome {
    Success,
    PartialSuccess,
    Failure,
    Aborted,
}

/// Normalised failure kinds for learning + FTS indexing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "details")]
pub enum FailureKind {
    AdapterTypo {
        offered: String,
        #[serde(default)]
        suggestion: Option<String>,
    },
    ValidatorReject {
        rule: String,
    },
    AdapterTimeout {
        adapter: String,
        ms: u64,
    },
    ToolError {
        tool: String,
        message: String,
    },
    LoopGuardFire {
        tool: String,
        repeat_count: u32,
    },
    Other {
        message: String,
    },
}

/// Closed-loop improvement proposal envelope (validate → adopt).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProposalEnvelope {
    pub schema_version: u32,
    pub original_hash: String,
    pub proposed_hash: String,
    pub kind: String,
    pub rationale: String,
    pub freshness_at_proposal: ContextFreshness,
    pub impact_decision: ImpactDecision,
}
