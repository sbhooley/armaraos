//! Orchestration context for multi-agent workflows (call chains, patterns, tracing).

use crate::agent::AgentId;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

/// Orchestration context passed through agent calls to provide hierarchical awareness.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestrationContext {
    /// Root orchestrator agent ID (top of the call tree)
    pub orchestrator_id: AgentId,
    /// Full lineage: root → … → current callee
    pub call_chain: Vec<AgentId>,
    /// Current depth in the call tree (0 = root orchestrator turn)
    pub depth: u32,
    /// Shared context variables accessible across the orchestration tree
    pub shared_vars: HashMap<String, serde_json::Value>,
    /// Type of orchestration pattern being used
    pub pattern: OrchestrationPattern,
    /// Timeout budget remaining for the entire orchestration (milliseconds)
    pub remaining_budget_ms: Option<u64>,
    /// Distributed trace ID for observability
    pub trace_id: String,
    /// Creation timestamp
    pub created_at: DateTime<Utc>,
    /// Efficient mode setting inherited from parent agent (off/balanced/aggressive)
    /// Child agents and swarms inherit this to prevent cost overruns
    pub efficient_mode: Option<String>,
}

/// Callback invoked when live orchestration state should be pushed (e.g. trace UI / SSE).
pub type OrchestrationLiveSink = Arc<dyn Fn(&OrchestrationContext) + Send + Sync>;

impl OrchestrationContext {
    /// Create a new root orchestration context.
    pub fn new_root(
        orchestrator_id: AgentId,
        pattern: OrchestrationPattern,
        efficient_mode: Option<String>,
    ) -> Self {
        Self {
            orchestrator_id,
            call_chain: vec![orchestrator_id],
            depth: 0,
            shared_vars: HashMap::new(),
            pattern,
            remaining_budget_ms: None,
            trace_id: Uuid::new_v4().to_string(),
            created_at: Utc::now(),
            efficient_mode,
        }
    }

    /// Create a child context for a sub-agent call (callee id is the child).
    pub fn child(&self, child_id: AgentId) -> Self {
        let mut call_chain = self.call_chain.clone();
        call_chain.push(child_id);
        Self {
            orchestrator_id: self.orchestrator_id,
            call_chain,
            depth: self.depth + 1,
            shared_vars: self.shared_vars.clone(),
            pattern: self.pattern.clone(),
            remaining_budget_ms: self.remaining_budget_ms,
            trace_id: self.trace_id.clone(),
            created_at: self.created_at,
            efficient_mode: self.efficient_mode.clone(),
        }
    }

    /// When a wall-clock budget is set, returns true if no time remains.
    #[must_use]
    pub fn budget_exhausted(&self) -> bool {
        matches!(self.remaining_budget_ms, Some(0))
    }

    /// Subtract wall-clock milliseconds from the orchestration budget (saturating at zero).
    pub fn spend_wall_ms(&mut self, ms: u64) {
        if let Some(b) = self.remaining_budget_ms.as_mut() {
            *b = b.saturating_sub(ms);
        }
    }

    /// Merge keys into [`Self::shared_vars`] (later keys overwrite).
    pub fn merge_shared_vars(&mut self, patch: HashMap<String, serde_json::Value>) {
        for (k, v) in patch {
            self.shared_vars.insert(k, v);
        }
    }

    /// Lines appended under `## Orchestration context` in the agent system prompt (design doc §1).
    #[must_use]
    pub fn system_prompt_appendix(&self, max_agent_call_depth: u32) -> String {
        let mut s = String::new();
        s.push_str("You are operating as part of a larger orchestration");
        s.push_str(&format!(" (trace_id={}).\n", self.trace_id));
        s.push_str(&format!("- Orchestrator: {}\n", self.orchestrator_id));
        s.push_str(&format!(
            "- Your role: {}\n",
            self.pattern.description_for_prompt()
        ));
        s.push_str(&format!(
            "- Call depth: {}/{}\n",
            self.depth, max_agent_call_depth
        ));
        if self.depth > 0 && self.call_chain.len() >= 2 {
            let parent = self.call_chain[self.call_chain.len() - 2];
            s.push_str(&format!("- Parent agent: {parent}\n"));
        }
        if let Some(b) = self.remaining_budget_ms {
            s.push_str(&format!(
                "- Orchestration wall-clock budget remaining (approx.): {b} ms\n"
            ));
        }
        if !self.shared_vars.is_empty() {
            s.push_str(&format!(
                "- Shared variables: {} entries (merge via orchestration tools as needed)\n",
                self.shared_vars.len()
            ));
        }
        s
    }
}

/// Workflow reference for orchestration (UUID matches `WorkflowId` in the kernel).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrchestrationWorkflowId(pub Uuid);

/// High-level orchestration pattern.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OrchestrationPattern {
    /// Simple agent_send / ad-hoc delegation
    AdHoc,
    /// Part of a workflow execution
    Workflow {
        workflow_id: OrchestrationWorkflowId,
        step_index: usize,
        step_name: String,
    },
    /// Map-reduce job
    MapReduce {
        job_id: String,
        phase: MapReducePhase,
        item_index: Option<usize>,
    },
    /// Supervised execution
    Supervisor {
        supervisor_id: AgentId,
        task_type: String,
    },
    /// Capability-based delegation
    Delegation {
        delegator_id: AgentId,
        capability_required: String,
    },
    /// Coordinated multi-agent execution
    Coordination {
        coordinator_id: AgentId,
        task_id: String,
    },
}

impl OrchestrationPattern {
    /// One-line description for system prompt injection (human-readable, not `Debug`).
    #[must_use]
    pub fn description_for_prompt(&self) -> String {
        match self {
            Self::AdHoc => "Ad-hoc delegation (no formal orchestration pattern).".to_string(),
            Self::Workflow {
                workflow_id,
                step_index,
                step_name,
            } => format!(
                "Workflow step {step_index} ({step_name}); workflow id {}.",
                workflow_id.0
            ),
            Self::MapReduce {
                job_id,
                phase,
                item_index,
            } => {
                let phase_s = match phase {
                    MapReducePhase::Map => "map",
                    MapReducePhase::Reduce => "reduce",
                };
                match item_index {
                    Some(i) => format!("Map-reduce job {job_id}, {phase_s} phase, item {i}."),
                    None => format!("Map-reduce job {job_id}, {phase_s} phase."),
                }
            }
            Self::Supervisor {
                supervisor_id,
                task_type,
            } => format!("Supervised task ({task_type}) under supervisor {supervisor_id}."),
            Self::Delegation {
                delegator_id,
                capability_required,
            } => format!(
                "Delegation for capability \"{capability_required}\" from delegator {delegator_id}."
            ),
            Self::Coordination {
                coordinator_id,
                task_id,
            } => format!("Coordinated task \"{task_id}\" under coordinator {coordinator_id}."),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MapReducePhase {
    Map,
    Reduce,
}

/// Strategy for kernel agent selection (`select_agent_for_task`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SelectionStrategy {
    RoundRobin,
    LeastBusy,
    CostEfficient,
    #[default]
    BestMatch,
    Random,
}

/// Options for host `select_agent_for_task` / `agent_delegate` ranking.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DelegateSelectionOptions {
    /// When true and the host has an embedding driver, blend cosine similarity between the task
    /// and each candidate’s profile text into the match score.
    #[serde(default = "default_true")]
    pub semantic_ranking: bool,
    /// If set, automatically spawn a worker from this pool when all matching agents are busy.
    /// Requires the pool to be configured in `[[agent_pools]]`.
    #[serde(default)]
    pub auto_spawn_pool: Option<String>,
    /// Minimum in-flight tasks for an agent to be considered "busy" for auto-spawn.
    /// Default: 1 (any in-flight work means busy).
    #[serde(default = "default_auto_spawn_threshold")]
    pub auto_spawn_threshold: u32,
}

fn default_auto_spawn_threshold() -> u32 {
    1
}

fn default_true() -> bool {
    true
}

/// Build an [`OrchestrationContext`] from a claimed task JSON (memory substrate shape) for the claiming agent.
///
/// Returns `None` when the task payload has no `orchestration.trace_id` (plain queue items).
#[must_use]
pub fn orchestration_context_from_claimed_task(
    task: &serde_json::Value,
    claimant: AgentId,
) -> Option<OrchestrationContext> {
    let payload = task.get("payload")?;
    let orch = payload.get("orchestration")?;
    let trace_id = orch.get("trace_id")?.as_str()?.to_string();

    let orchestrator_id = orch
        .get("orchestrator_id")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<AgentId>().ok())
        .or_else(|| {
            task.get("created_by")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse().ok())
        })
        .unwrap_or(claimant);

    let mut ctx = if orchestrator_id == claimant {
        let mut c = OrchestrationContext::new_root(claimant, OrchestrationPattern::AdHoc, None);
        c.trace_id = trace_id;
        c
    } else {
        let mut root =
            OrchestrationContext::new_root(orchestrator_id, OrchestrationPattern::AdHoc, None);
        root.trace_id = trace_id;
        root.child(claimant)
    };

    if let Some(id) = task.get("id").and_then(|v| v.as_str()) {
        ctx.shared_vars
            .insert("task_queue_task_id".to_string(), serde_json::json!(id));
    }
    if let Some(t) = task.get("title").and_then(|v| v.as_str()) {
        ctx.shared_vars
            .insert("task_queue_title".to_string(), serde_json::json!(t));
    }
    if let Some(d) = task.get("description").and_then(|v| v.as_str()) {
        ctx.shared_vars
            .insert("task_queue_description".to_string(), serde_json::json!(d));
    }

    Some(ctx)
}

impl DelegateSelectionOptions {
    /// Keyword-only ranking (no embedding API calls, no auto-spawn).
    pub const KEYWORDS_ONLY: Self = Self {
        semantic_ranking: false,
        auto_spawn_pool: None,
        auto_spawn_threshold: 1,
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_lineage() {
        let a = AgentId::new();
        let b = AgentId::new();
        let c = AgentId::new();
        let root = OrchestrationContext::new_root(a, OrchestrationPattern::AdHoc, None);
        assert_eq!(root.call_chain.len(), 1);
        let to_b = root.child(b);
        assert_eq!(to_b.depth, 1);
        assert_eq!(to_b.call_chain, vec![a, b]);
        let to_c = to_b.child(c);
        assert_eq!(to_c.depth, 2);
        assert_eq!(to_c.orchestrator_id, a);
        assert_eq!(to_c.trace_id, root.trace_id);
    }

    #[test]
    fn budget_spend_and_exhausted() {
        let a = AgentId::new();
        let mut ctx = OrchestrationContext::new_root(a, OrchestrationPattern::AdHoc, None);
        ctx.remaining_budget_ms = Some(100);
        assert!(!ctx.budget_exhausted());
        ctx.spend_wall_ms(40);
        assert_eq!(ctx.remaining_budget_ms, Some(60));
        ctx.spend_wall_ms(100);
        assert_eq!(ctx.remaining_budget_ms, Some(0));
        assert!(ctx.budget_exhausted());
    }

    #[test]
    fn system_prompt_appendix_matches_design_shape() {
        let a = AgentId::new();
        let b = AgentId::new();
        let mut ctx = OrchestrationContext::new_root(a, OrchestrationPattern::AdHoc, None);
        ctx = ctx.child(b);
        let appendix = ctx.system_prompt_appendix(5);
        assert!(appendix.contains("trace_id="));
        assert!(appendix.contains("- Orchestrator:"));
        assert!(appendix.contains("- Your role:"));
        assert!(appendix.contains("- Call depth: 1/5"));
        assert!(appendix.contains("- Parent agent:"));
    }

    #[test]
    fn description_for_prompt_not_debug() {
        let wid = OrchestrationWorkflowId(Uuid::nil());
        let s = OrchestrationPattern::Workflow {
            workflow_id: wid,
            step_index: 2,
            step_name: "fetch".to_string(),
        }
        .description_for_prompt();
        assert!(s.contains("Workflow step 2 (fetch)"));
        assert!(!s.contains("OrchestrationWorkflowId"));
    }

    #[test]
    fn orchestration_from_claimed_task_worker_branch() {
        let orch = AgentId::new();
        let worker = AgentId::new();
        let task = serde_json::json!({
            "id": "task-uuid",
            "title": "Do thing",
            "description": "Details",
            "created_by": orch.to_string(),
            "payload": {
                "orchestration": {
                    "trace_id": "trace-xyz",
                    "orchestrator_id": orch.to_string(),
                }
            }
        });
        let ctx = orchestration_context_from_claimed_task(&task, worker).expect("ctx");
        assert_eq!(ctx.trace_id, "trace-xyz");
        assert_eq!(ctx.orchestrator_id, orch);
        assert_eq!(ctx.depth, 1);
        assert_eq!(ctx.call_chain, vec![orch, worker]);
        assert_eq!(
            ctx.shared_vars
                .get("task_queue_task_id")
                .and_then(|v| v.as_str()),
            Some("task-uuid")
        );
    }

    #[test]
    fn orchestration_from_claimed_task_same_agent_as_orchestrator() {
        let a = AgentId::new();
        let task = serde_json::json!({
            "id": "t1",
            "title": "x",
            "description": "y",
            "created_by": a.to_string(),
            "payload": {
                "orchestration": {
                    "trace_id": "tr",
                    "orchestrator_id": a.to_string(),
                }
            }
        });
        let ctx = orchestration_context_from_claimed_task(&task, a).expect("ctx");
        assert_eq!(ctx.depth, 0);
        assert_eq!(ctx.call_chain, vec![a]);
    }

    #[test]
    fn orchestration_from_claimed_task_plain_payload_returns_none() {
        let task = serde_json::json!({ "id": "x", "payload": {} });
        assert!(orchestration_context_from_claimed_task(&task, AgentId::new()).is_none());
    }
}
