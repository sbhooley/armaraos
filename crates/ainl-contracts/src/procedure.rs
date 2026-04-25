//! Portable procedure-learning contracts.
//!
//! These types are intentionally host-neutral: ArmaraOS/OpenFang can render them as skills or
//! hands, the Python AINL runtime can render executable graph source, and other hosts can keep
//! them as JSON graph-shaped procedures.

use serde::{Deserialize, Serialize};

use crate::{CognitiveVitals, ContextFreshness, ImpactDecision, TrajectoryOutcome, TrajectoryStep};

/// Lifecycle state for a reusable procedure artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProcedureLifecycle {
    #[default]
    Draft,
    Candidate,
    Validated,
    Promoted,
    Deprecated,
}

/// Render/execution targets for a procedure artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcedureArtifactFormat {
    MarkdownSkill,
    OpenFangSkill,
    AinlGraph,
    Hand,
    PromptOnly,
}

/// One normalized event in an experience bundle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExperienceEvent {
    pub event_id: String,
    pub timestamp_ms: i64,
    pub tool_or_adapter: String,
    pub operation: String,
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_preview: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_preview: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default)]
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vitals: Option<CognitiveVitals>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freshness_at_step: Option<ContextFreshness>,
}

impl From<&TrajectoryStep> for ExperienceEvent {
    fn from(step: &TrajectoryStep) -> Self {
        Self {
            event_id: step.step_id.clone(),
            timestamp_ms: step.timestamp_ms,
            tool_or_adapter: step.adapter.clone(),
            operation: step.operation.clone(),
            success: step.success,
            input_preview: step.inputs_preview.clone(),
            output_preview: step.outputs_preview.clone(),
            error: step.error.clone(),
            duration_ms: step.duration_ms,
            vitals: step.vitals.clone(),
            freshness_at_step: step.freshness_at_step,
        }
    }
}

/// Host-neutral evidence packet used to mint or patch reusable procedures.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExperienceBundle {
    pub schema_version: u32,
    pub bundle_id: String,
    pub agent_id: String,
    pub intent: String,
    pub outcome: TrajectoryOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_outcome: Option<String>,
    pub observation_count: u32,
    pub fitness: f32,
    pub events: Vec<ExperienceEvent>,
    #[serde(default)]
    pub source_trajectory_ids: Vec<String>,
    #[serde(default)]
    pub source_failure_ids: Vec<String>,
    pub freshness: ContextFreshness,
    pub impact_decision: ImpactDecision,
}

/// Structured procedure step, rich enough to render to Markdown or graph forms.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ProcedureStepKind {
    ToolCall {
        tool: String,
        #[serde(default)]
        args_schema: serde_json::Value,
    },
    AdapterCall {
        adapter: String,
        op: String,
    },
    Validate {
        target: String,
    },
    Branch {
        condition: String,
    },
    HumanReview {
        reason: String,
    },
    Instruction {
        text: String,
    },
}

/// One step in a reusable procedure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProcedureStep {
    pub step_id: String,
    pub title: String,
    pub kind: ProcedureStepKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
}

/// Verification guidance attached to a procedure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ProcedureVerification {
    #[serde(default)]
    pub checks: Vec<String>,
    #[serde(default)]
    pub success_criteria: Vec<String>,
}

/// Canonical reusable procedure. Renderers produce SKILL.md, skill.toml, AINL graph skeletons, etc.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProcedureArtifact {
    pub schema_version: u32,
    pub id: String,
    pub title: String,
    pub intent: String,
    pub summary: String,
    #[serde(default)]
    pub required_tools: Vec<String>,
    #[serde(default)]
    pub required_adapters: Vec<String>,
    #[serde(default)]
    pub inputs: Vec<String>,
    #[serde(default)]
    pub outputs: Vec<String>,
    #[serde(default)]
    pub preconditions: Vec<String>,
    #[serde(default)]
    pub steps: Vec<ProcedureStep>,
    #[serde(default)]
    pub verification: ProcedureVerification,
    #[serde(default)]
    pub known_failures: Vec<String>,
    #[serde(default)]
    pub recovery: Vec<String>,
    #[serde(default)]
    pub source_trajectory_ids: Vec<String>,
    #[serde(default)]
    pub source_failure_ids: Vec<String>,
    pub fitness: f32,
    pub observation_count: u32,
    pub lifecycle: ProcedureLifecycle,
    #[serde(default)]
    pub render_targets: Vec<ProcedureArtifactFormat>,
}

/// Patch proposal against an existing procedure artifact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProcedurePatch {
    pub schema_version: u32,
    pub patch_id: String,
    pub procedure_id: String,
    pub rationale: String,
    #[serde(default)]
    pub add_steps: Vec<ProcedureStep>,
    #[serde(default)]
    pub add_known_failures: Vec<String>,
    #[serde(default)]
    pub add_recovery: Vec<String>,
    #[serde(default)]
    pub source_failure_ids: Vec<String>,
}

/// Outcome of trying to reuse a validated procedure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProcedureReuseOutcome {
    pub procedure_id: String,
    pub outcome: TrajectoryOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

/// Portable deterministic execution payload derived from a [`ProcedureArtifact`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProcedureExecutionPlan {
    pub procedure_id: String,
    pub schema_version: u32,
    #[serde(default)]
    pub steps: Vec<ProcedureExecutionStep>,
    #[serde(default)]
    pub verification: ProcedureVerification,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProcedureExecutionStep {
    pub step_id: String,
    pub title: String,
    pub executor: String,
    pub operation: String,
    #[serde(default)]
    pub args_schema: serde_json::Value,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub on_error: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{TrajectoryOutcome, LEARNER_SCHEMA_VERSION};

    #[test]
    fn procedure_artifact_roundtrips_json() {
        let p = ProcedureArtifact {
            schema_version: LEARNER_SCHEMA_VERSION,
            id: "proc:demo".into(),
            title: "Demo".into(),
            intent: "Do a demo".into(),
            summary: "Reusable demo flow".into(),
            required_tools: vec!["file_read".into()],
            required_adapters: vec![],
            inputs: vec!["path".into()],
            outputs: vec!["summary".into()],
            preconditions: vec!["Workspace exists".into()],
            steps: vec![ProcedureStep {
                step_id: "step-1".into(),
                title: "Read file".into(),
                kind: ProcedureStepKind::ToolCall {
                    tool: "file_read".into(),
                    args_schema: serde_json::json!({"type":"object"}),
                },
                rationale: None,
            }],
            verification: ProcedureVerification {
                checks: vec!["Confirm output is non-empty".into()],
                success_criteria: vec!["Output summarizes file".into()],
            },
            known_failures: vec![],
            recovery: vec![],
            source_trajectory_ids: vec!["traj-1".into()],
            source_failure_ids: vec![],
            fitness: 0.9,
            observation_count: 3,
            lifecycle: ProcedureLifecycle::Candidate,
            render_targets: vec![ProcedureArtifactFormat::MarkdownSkill],
        };
        let j = serde_json::to_value(&p).unwrap();
        let back: ProcedureArtifact = serde_json::from_value(j).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn experience_event_from_trajectory_step() {
        let step = TrajectoryStep {
            step_id: "s1".into(),
            timestamp_ms: 10,
            adapter: "tool".into(),
            operation: "file_read".into(),
            inputs_preview: Some("in".into()),
            outputs_preview: Some("out".into()),
            duration_ms: 5,
            success: true,
            error: None,
            vitals: None,
            freshness_at_step: None,
            frame_vars: None,
            tool_telemetry: None,
        };
        let event = ExperienceEvent::from(&step);
        assert_eq!(event.operation, "file_read");
        assert!(event.success);
        assert_eq!(event.duration_ms, 5);
    }

    #[test]
    fn reuse_outcome_serializes_failure() {
        let outcome = ProcedureReuseOutcome {
            procedure_id: "proc:demo".into(),
            outcome: TrajectoryOutcome::Failure,
            failure_id: Some("failure-1".into()),
            notes: None,
        };
        let j = serde_json::to_string(&outcome).unwrap();
        assert!(j.contains("failure"));
    }
}
