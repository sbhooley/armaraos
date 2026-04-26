//! Orchestration tracing types for multi-agent observability (API + debugging).

use crate::agent::AgentId;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// One observability event in a distributed orchestration trace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestrationTraceEvent {
    pub trace_id: String,
    pub orchestrator_id: AgentId,
    pub agent_id: AgentId,
    pub parent_agent_id: Option<AgentId>,
    pub event_type: TraceEventType,
    pub timestamp: DateTime<Utc>,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Summary row for `GET /api/orchestration/traces`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestrationTraceSummary {
    pub trace_id: String,
    pub orchestrator_id: AgentId,
    pub last_event_at: DateTime<Utc>,
    pub event_count: usize,
}

/// Node in a reconstructed call tree for `GET .../tree`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestrationTraceTreeNode {
    pub agent_id: AgentId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_agent_id: Option<AgentId>,
    pub children: Vec<OrchestrationTraceTreeNode>,
}

/// Per-agent cost line for `GET .../cost`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestrationTraceCostLine {
    pub agent_id: AgentId,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost_usd: f64,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestrationTraceCostSummary {
    pub trace_id: String,
    pub total_tokens: u64,
    pub total_cost_usd: f64,
    pub total_duration_ms: u64,
    pub by_agent: Vec<OrchestrationTraceCostLine>,
}

/// `GET /api/orchestration/quota-tree/:agent_id` — one node in the agent tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestrationQuotaTreeNode {
    pub agent_id: AgentId,
    pub name: String,
    pub quota: OrchestrationQuotaSnapshot,
    pub children: Vec<OrchestrationQuotaTreeNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestrationQuotaSnapshot {
    pub max_llm_tokens_per_hour: u64,
    pub used_llm_tokens: u64,
    pub max_tool_calls_per_minute: u32,
    pub max_cost_per_hour_usd: f64,
    #[serde(default)]
    pub inherits_parent: bool,
    /// `0` = unlimited. Token/cost metering for this agent uses [`Self::llm_token_billing_agent_id`] when set.
    #[serde(default)]
    pub max_subagents: u32,
    #[serde(default)]
    pub active_subagents: u32,
    /// `0` = unlimited. Current longest path to a descendant under this agent.
    #[serde(default)]
    pub max_spawn_depth: u32,
    #[serde(default)]
    pub spawn_subtree_height: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_token_billing_agent_id: Option<AgentId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TraceEventType {
    OrchestrationStart {
        pattern: String,
        initial_input: String,
    },
    AgentDelegated {
        target_agent: AgentId,
        task: String,
    },
    AgentCompleted {
        result_size: usize,
        tokens_in: u64,
        tokens_out: u64,
        duration_ms: u64,
        cost_usd: f64,
    },
    AgentFailed {
        error: String,
    },
    OrchestrationComplete {
        total_tokens: u64,
        total_cost_usd: f64,
        total_duration_ms: u64,
        agents_used: Vec<AgentId>,
    },
    /// Deterministic planner (`InferRequest` + `AgentSnapshot`) started executing a plan.
    PlanStarted {
        step_count: usize,
        confidence: f32,
        reasoning_step_ids: Vec<String>,
        /// Post-plan reminders from the inference control plane (`structured.follow_ups`), if any.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        planner_follow_ups: Vec<String>,
    },
    PlanStepStarted {
        step_id: String,
        tool: String,
    },
    PlanStepCompleted {
        step_id: String,
        tool: String,
    },
    PlanStepFailed {
        step_id: String,
        tool: String,
        error: String,
    },
    PlanReasoningReentry {
        step_id: String,
    },
    PlanLocalPatch {
        step_id: String,
        replan_attempt: u32,
    },
    PlanFallback {
        reason: String,
    },
}

impl TraceEventType {
    /// Stable snake_case name for filtering (matches JSON `type` tag).
    #[must_use]
    pub fn discriminant_name(&self) -> &'static str {
        match self {
            Self::OrchestrationStart { .. } => "orchestration_start",
            Self::AgentDelegated { .. } => "agent_delegated",
            Self::AgentCompleted { .. } => "agent_completed",
            Self::AgentFailed { .. } => "agent_failed",
            Self::OrchestrationComplete { .. } => "orchestration_complete",
            Self::PlanStarted { .. } => "plan_started",
            Self::PlanStepStarted { .. } => "plan_step_started",
            Self::PlanStepCompleted { .. } => "plan_step_completed",
            Self::PlanStepFailed { .. } => "plan_step_failed",
            Self::PlanReasoningReentry { .. } => "plan_reasoning_reentry",
            Self::PlanLocalPatch { .. } => "plan_local_patch",
            Self::PlanFallback { .. } => "plan_fallback",
        }
    }
}
