//! Bounded agent graph snapshot + deterministic plan types for the planner protocol.
//!
//! Shared between ArmaraOS and `ainl-inference-server` via path or published crate dependency.

mod builder;

pub use builder::{build_snapshot, SnapshotError};

use ainl_memory::AinlMemoryNode;
use serde::{Deserialize, Serialize};

/// Schema version for [`AgentSnapshot::snapshot_version`]; server rejects unknown versions.
pub const SNAPSHOT_SCHEMA_VERSION: u32 = 1;

/// Default total plan wall-clock cap (ms).
pub const DEFAULT_MAX_WALL_MS: u64 = 60_000;
/// Default max `LocalPatch` replans per plan execution.
pub const DEFAULT_MAX_REPLAN_CALLS: u32 = 3;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentSnapshot {
    pub agent_id: String,
    pub snapshot_version: u32,
    #[serde(default)]
    pub persona: Vec<AinlMemoryNode>,
    #[serde(default)]
    pub episodic: Vec<AinlMemoryNode>,
    #[serde(default)]
    pub semantic: Vec<AinlMemoryNode>,
    #[serde(default)]
    pub procedural: Vec<AinlMemoryNode>,
    #[serde(default)]
    pub tool_allowlist: Vec<String>,
    #[serde(default)]
    pub policy_caps: PolicyCaps,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PolicyCaps {
    #[serde(default = "default_max_steps")]
    pub max_steps: u32,
    #[serde(default = "default_max_depth")]
    pub max_depth: u32,
    #[serde(default = "default_max_wall_ms")]
    pub max_wall_ms: u64,
    #[serde(default = "default_max_replan_calls")]
    pub max_replan_calls: u32,
    #[serde(default)]
    pub deny_tools: Vec<String>,
}

fn default_max_steps() -> u32 {
    32
}
fn default_max_depth() -> u32 {
    8
}
fn default_max_wall_ms() -> u64 {
    DEFAULT_MAX_WALL_MS
}
fn default_max_replan_calls() -> u32 {
    DEFAULT_MAX_REPLAN_CALLS
}

impl Default for PolicyCaps {
    fn default() -> Self {
        Self {
            max_steps: default_max_steps(),
            max_depth: default_max_depth(),
            max_wall_ms: default_max_wall_ms(),
            max_replan_calls: default_max_replan_calls(),
            deny_tools: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RepairContext {
    pub failed_step_id: String,
    pub failed_step_tool: String,
    pub error_msg: String,
    pub prior_outputs: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeterministicPlan {
    #[serde(default)]
    pub steps: Vec<PlanStep>,
    #[serde(default)]
    pub graph_writes: Vec<GraphWrite>,
    #[serde(default)]
    pub confidence: f32,
    #[serde(default)]
    pub reasoning_required_at: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlanStep {
    pub id: String,
    pub tool: String,
    #[serde(default)]
    pub args: serde_json::Value,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub on_error: OnErrorPolicy,
    #[serde(default)]
    pub idempotency_key: Option<String>,
    #[serde(default)]
    pub optional: bool,
    #[serde(default)]
    pub expected_output_schema: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum OnErrorPolicy {
    RetryOnce,
    LocalPatch,
    #[default]
    Abort,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GraphWrite {
    pub node_type: String,
    pub label: String,
    #[serde(default)]
    pub payload: serde_json::Value,
    #[serde(default)]
    pub fitness_delta: Option<f32>,
}

/// Typed tool-step failure for escalation without string parsing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, thiserror::Error)]
pub enum PlanStepError {
    #[error("tool not found: {0}")]
    ToolNotFound(String),
    #[error("policy blocked: {reason}")]
    PolicyBlocked { reason: String },
    #[error("transient: {0}")]
    Transient(String),
    #[error("deterministic: {0}")]
    Deterministic(String),
    #[error("timeout")]
    Timeout,
}

impl PlanStepError {
    pub fn to_message(&self) -> String {
        self.to_string()
    }
}

/// Lookup window (seconds) for non-episodic snapshot types (semantic, procedural, persona).
/// 30 days is the default; operators can override via `[runtime_limits] snapshot_non_episodic_window_secs`.
pub const DEFAULT_NON_EPISODIC_WINDOW_SECS: i64 = 86_400 * 30;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SnapshotPolicy {
    pub episodic_window_secs: i64,
    pub episodic_max: usize,
    pub semantic_top_n: usize,
    pub procedural_top_n: usize,
    pub persona_top_n: usize,
    /// Lookup window (seconds) for semantic, procedural, and persona node types.
    /// Defaults to [`DEFAULT_NON_EPISODIC_WINDOW_SECS`] (30 days).
    pub non_episodic_window_secs: i64,
}

impl Default for SnapshotPolicy {
    fn default() -> Self {
        Self {
            episodic_window_secs: 1800,
            episodic_max: 10,
            semantic_top_n: 20,
            procedural_top_n: 10,
            persona_top_n: 5,
            non_episodic_window_secs: DEFAULT_NON_EPISODIC_WINDOW_SECS,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GraphWriteError {
    #[error("invalid node_type for graph write: {0}")]
    InvalidNodeType(String),
    #[error("episodic and patch writes are not allowed via apply_graph_writes")]
    DisallowedKind,
    #[error("failed to build node: {0}")]
    Build(String),
}

/// Map planner graph writes to concrete memory nodes for `GraphMemory::write_node`.
///
/// Rejects `episode` / `episodic` / `patch` — those paths are owned by the executor / dispatch_patches.
pub fn apply_graph_writes(
    writes: &[GraphWrite],
    agent_id: &str,
    now_ms: i64,
) -> Result<Vec<AinlMemoryNode>, GraphWriteError> {
    use ainl_memory::AinlMemoryNode;
    use uuid::Uuid;

    let mut out = Vec::with_capacity(writes.len());
    for w in writes {
        let nt = w.node_type.to_lowercase();
        match nt.as_str() {
            "episode" | "episodic" | "patch" => return Err(GraphWriteError::DisallowedKind),
            "semantic" => {
                let fact = w
                    .payload
                    .get("fact")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&w.label)
                    .to_string();
                let confidence = w
                    .payload
                    .get("confidence")
                    .and_then(|v| v.as_f64())
                    .map(|f| f as f32)
                    .unwrap_or(0.8);
                let source_turn_id = w
                    .payload
                    .get("source_turn_id")
                    .and_then(|v| v.as_str())
                    .and_then(|s| Uuid::parse_str(s).ok())
                    .unwrap_or_else(Uuid::new_v4);
                let mut node = AinlMemoryNode::new_fact(fact, confidence, source_turn_id);
                node.id = Uuid::new_v4();
                node.agent_id = agent_id.to_string();
                out.push(node);
            }
            "persona" => {
                let trait_name = w
                    .payload
                    .get("trait_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&w.label)
                    .to_string();
                let strength = w
                    .payload
                    .get("strength")
                    .and_then(|v| v.as_f64())
                    .map(|f| f as f32)
                    .unwrap_or(0.7);
                let learned_from = w
                    .payload
                    .get("learned_from")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|x| x.as_str().and_then(|s| Uuid::parse_str(s).ok()))
                            .collect()
                    })
                    .unwrap_or_default();
                let mut node = AinlMemoryNode::new_persona(trait_name, strength, learned_from);
                node.id = Uuid::new_v4();
                node.agent_id = agent_id.to_string();
                out.push(node);
            }
            "procedural" => {
                let pattern_name = w
                    .payload
                    .get("pattern_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&w.label)
                    .to_string();
                let tool_sequence: Vec<String> = w
                    .payload
                    .get("tool_sequence")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|x| x.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                let confidence = w
                    .payload
                    .get("confidence")
                    .and_then(|v| v.as_f64())
                    .map(|f| f as f32)
                    .unwrap_or(0.75);
                let mut node = if tool_sequence.is_empty() {
                    let compiled = w
                        .payload
                        .get("compiled_graph")
                        .and_then(|v| v.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|x| x.as_u64().map(|u| u as u8))
                                .collect()
                        })
                        .unwrap_or_default();
                    AinlMemoryNode::new_pattern(pattern_name, compiled)
                } else {
                    AinlMemoryNode::new_procedural_tools(pattern_name, tool_sequence, confidence)
                };
                node.id = Uuid::new_v4();
                node.agent_id = agent_id.to_string();
                if let Some(fd) = w.fitness_delta {
                    if let ainl_memory::AinlNodeType::Procedural { ref mut procedural } =
                        node.node_type
                    {
                        procedural.fitness = Some(
                            procedural.fitness.unwrap_or(0.5) + fd,
                        );
                    }
                }
                let _ = now_ms;
                out.push(node);
            }
            other => return Err(GraphWriteError::InvalidNodeType(other.to_string())),
        }
    }
    Ok(out)
}

/// JSON discriminator for structured planner output (`InferOutput.structured`).
pub const STRUCTURED_KIND_DETERMINISTIC_PLAN: &str = "deterministic_plan";
/// Structured response when server-side plan validation fails after repair attempt.
pub const STRUCTURED_KIND_PLANNER_INVALID_PLAN: &str = "planner_invalid_plan";
