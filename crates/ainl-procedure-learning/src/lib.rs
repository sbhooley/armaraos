//! Procedure learning for AINL hosts.
//!
//! This crate is the reusable “experience → procedure artifact → reuse → patch” core. Hosts such
//! as ArmaraOS provide storage, validation, and execution; this crate provides deterministic,
//! serializable learning decisions without depending on `openfang-*`.

use std::collections::{BTreeSet, HashSet};

use ainl_contracts::{
    ExperienceBundle, ProcedureArtifact, ProcedureArtifactFormat, ProcedureExecutionPlan,
    ProcedureExecutionStep, ProcedureLifecycle, ProcedurePatch, ProcedureStep, ProcedureStepKind,
    ProcedureVerification, TrajectoryOutcome, LEARNER_SCHEMA_VERSION,
};
use sha2::{Digest, Sha256};

pub mod proposal_kind {
    pub const PROCEDURE_MINT: &str = "procedure_mint";
    pub const PROCEDURE_PATCH: &str = "procedure_patch";
    pub const PROCEDURE_PROMOTE: &str = "procedure_promote";
    pub const PROCEDURE_DEPRECATE: &str = "procedure_deprecate";
    pub const GRAPH_PATCH_FROM_PROCEDURE: &str = "graph_patch_from_procedure";
}

#[derive(Debug, Clone)]
pub struct DistillPolicy {
    pub min_observations: u32,
    pub min_fitness: f32,
    pub require_success: bool,
}

impl Default for DistillPolicy {
    fn default() -> Self {
        Self {
            min_observations: 3,
            min_fitness: 0.70,
            require_success: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReuseScore {
    pub procedure_id: String,
    pub score: f32,
    pub reasons: Vec<String>,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ProcedureLearningError {
    #[error("insufficient observations: {observed} < {required}")]
    InsufficientObservations { observed: u32, required: u32 },
    #[error("fitness below threshold: {fitness:.3} < {required:.3}")]
    FitnessBelowThreshold { fitness: f32, required: f32 },
    #[error("experience outcome is not successful")]
    NonSuccessfulOutcome,
    #[error("experience has no events")]
    EmptyExperience,
}

#[must_use]
pub fn sha256_hex_lower(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))
}

#[must_use]
pub fn procedure_fingerprint(bundle: &ExperienceBundle) -> String {
    let mut canonical = String::new();
    canonical.push_str(bundle.intent.trim());
    canonical.push('\n');
    for event in &bundle.events {
        canonical.push_str(&event.tool_or_adapter);
        canonical.push(':');
        canonical.push_str(&event.operation);
        canonical.push(':');
        canonical.push_str(if event.success { "ok" } else { "err" });
        canonical.push('\n');
    }
    sha256_hex_lower(&canonical)
}

pub fn distill_procedure(
    bundle: &ExperienceBundle,
    policy: &DistillPolicy,
) -> Result<ProcedureArtifact, ProcedureLearningError> {
    if bundle.events.is_empty() {
        return Err(ProcedureLearningError::EmptyExperience);
    }
    if bundle.observation_count < policy.min_observations {
        return Err(ProcedureLearningError::InsufficientObservations {
            observed: bundle.observation_count,
            required: policy.min_observations,
        });
    }
    if bundle.fitness < policy.min_fitness {
        return Err(ProcedureLearningError::FitnessBelowThreshold {
            fitness: bundle.fitness,
            required: policy.min_fitness,
        });
    }
    if policy.require_success && bundle.outcome != TrajectoryOutcome::Success {
        return Err(ProcedureLearningError::NonSuccessfulOutcome);
    }

    let fingerprint = procedure_fingerprint(bundle);
    let mut required_tools = BTreeSet::new();
    let mut required_adapters = BTreeSet::new();
    let mut known_failures = Vec::new();
    let steps = bundle
        .events
        .iter()
        .enumerate()
        .map(|(idx, event)| {
            if event.success {
                required_tools.insert(event.operation.clone());
            } else if let Some(err) = &event.error {
                known_failures.push(format!("{}: {}", event.operation, err));
            }
            if event.tool_or_adapter != "tool" {
                required_adapters.insert(event.tool_or_adapter.clone());
            }
            ProcedureStep {
                step_id: format!("step-{:02}", idx + 1),
                title: event.operation.clone(),
                kind: ProcedureStepKind::ToolCall {
                    tool: event.operation.clone(),
                    args_schema: serde_json::json!({"type":"object"}),
                },
                rationale: event.output_preview.clone(),
            }
        })
        .collect::<Vec<_>>();

    Ok(ProcedureArtifact {
        schema_version: LEARNER_SCHEMA_VERSION,
        id: format!("proc:{fingerprint}"),
        title: title_from_intent(&bundle.intent),
        intent: bundle.intent.clone(),
        summary: format!(
            "Learned from {} observations with fitness {:.2}.",
            bundle.observation_count, bundle.fitness
        ),
        required_tools: required_tools.into_iter().collect(),
        required_adapters: required_adapters.into_iter().collect(),
        inputs: Vec::new(),
        outputs: Vec::new(),
        preconditions: vec![
            "Use this procedure only when the user task matches the intent.".into(),
        ],
        steps,
        verification: ProcedureVerification {
            checks: vec![
                "Confirm all required tool calls completed successfully.".into(),
                "Summarize any errors instead of claiming success.".into(),
            ],
            success_criteria: vec![
                "The requested workflow is completed or a safe failure is reported.".into(),
            ],
        },
        known_failures,
        recovery: vec!["If any step fails, stop and inspect the failure before retrying.".into()],
        source_trajectory_ids: bundle.source_trajectory_ids.clone(),
        source_failure_ids: bundle.source_failure_ids.clone(),
        fitness: bundle.fitness,
        observation_count: bundle.observation_count,
        lifecycle: ProcedureLifecycle::Candidate,
        render_targets: vec![
            ProcedureArtifactFormat::MarkdownSkill,
            ProcedureArtifactFormat::AinlGraph,
            ProcedureArtifactFormat::PromptOnly,
        ],
    })
}

#[must_use]
pub fn score_reuse(
    artifact: &ProcedureArtifact,
    user_intent: &str,
    available_tools: &[String],
) -> ReuseScore {
    let intent_l = user_intent.to_ascii_lowercase();
    let mut score = 0.0_f32;
    let mut reasons = Vec::new();
    for token in artifact
        .intent
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| t.len() >= 4)
    {
        if intent_l.contains(&token.to_ascii_lowercase()) {
            score += 0.15;
        }
    }
    let available: HashSet<&str> = available_tools.iter().map(String::as_str).collect();
    let required = artifact.required_tools.len().max(1) as f32;
    let have = artifact
        .required_tools
        .iter()
        .filter(|t| available.contains(t.as_str()))
        .count() as f32;
    let tool_score = have / required;
    score += tool_score * 0.45;
    if tool_score >= 1.0 {
        reasons.push("all_required_tools_available".into());
    } else {
        reasons.push("some_required_tools_missing".into());
    }
    score += artifact.fitness.clamp(0.0, 1.0) * 0.30;
    score += (artifact.observation_count.min(10) as f32 / 10.0) * 0.10;
    ReuseScore {
        procedure_id: artifact.id.clone(),
        score: score.clamp(0.0, 1.0),
        reasons,
    }
}

#[must_use]
pub fn patch_from_failure(
    artifact: &ProcedureArtifact,
    failure_id: impl Into<String>,
    failure_message: impl Into<String>,
) -> ProcedurePatch {
    let failure_id = failure_id.into();
    let failure_message = failure_message.into();
    let patch_hash = sha256_hex_lower(&format!("{}:{failure_id}:{failure_message}", artifact.id));
    ProcedurePatch {
        schema_version: LEARNER_SCHEMA_VERSION,
        patch_id: format!("patch:{patch_hash}"),
        procedure_id: artifact.id.clone(),
        rationale: format!("Patch learned from failed reuse: {failure_message}"),
        add_steps: vec![ProcedureStep {
            step_id: "recovery-check".into(),
            title: "Check prior failure before retry".into(),
            kind: ProcedureStepKind::Validate {
                target: "previous failure is addressed".into(),
            },
            rationale: Some(failure_message.clone()),
        }],
        add_known_failures: vec![failure_message],
        add_recovery: vec!["Do not retry unchanged inputs after this failure.".into()],
        source_failure_ids: vec![failure_id],
    }
}

#[must_use]
pub fn apply_patch(artifact: &ProcedureArtifact, patch: &ProcedurePatch) -> ProcedureArtifact {
    let mut next = artifact.clone();
    next.steps.extend(patch.add_steps.clone());
    next.known_failures.extend(patch.add_known_failures.clone());
    next.recovery.extend(patch.add_recovery.clone());
    next.source_failure_ids
        .extend(patch.source_failure_ids.clone());
    next.lifecycle = ProcedureLifecycle::Candidate;
    next
}

#[must_use]
pub fn render_execution_plan(artifact: &ProcedureArtifact) -> ProcedureExecutionPlan {
    let mut prior_step: Option<String> = None;
    let steps = artifact
        .steps
        .iter()
        .map(|step| {
            let (executor, operation, args_schema) = match &step.kind {
                ProcedureStepKind::ToolCall { tool, args_schema } => {
                    ("tool".to_string(), tool.clone(), args_schema.clone())
                }
                ProcedureStepKind::AdapterCall { adapter, op } => (
                    "adapter".to_string(),
                    format!("{adapter}.{op}"),
                    serde_json::Value::Null,
                ),
                ProcedureStepKind::Validate { target } => (
                    "validate".to_string(),
                    target.clone(),
                    serde_json::Value::Null,
                ),
                ProcedureStepKind::Branch { condition } => (
                    "branch".to_string(),
                    condition.clone(),
                    serde_json::Value::Null,
                ),
                ProcedureStepKind::HumanReview { reason } => (
                    "human_review".to_string(),
                    reason.clone(),
                    serde_json::Value::Null,
                ),
                ProcedureStepKind::Instruction { text } => (
                    "instruction".to_string(),
                    text.clone(),
                    serde_json::Value::Null,
                ),
            };
            let depends_on = prior_step.iter().cloned().collect::<Vec<_>>();
            prior_step = Some(step.step_id.clone());
            ProcedureExecutionStep {
                step_id: step.step_id.clone(),
                title: step.title.clone(),
                executor,
                operation,
                args_schema,
                depends_on,
                on_error: "abort_and_patch".into(),
            }
        })
        .collect();
    ProcedureExecutionPlan {
        procedure_id: artifact.id.clone(),
        schema_version: LEARNER_SCHEMA_VERSION,
        steps,
        verification: artifact.verification.clone(),
    }
}

#[must_use]
pub fn render_markdown_skill(artifact: &ProcedureArtifact) -> String {
    let mut out = String::new();
    out.push_str("# ");
    out.push_str(&artifact.title);
    out.push_str("\n\n## Intent\n\n");
    out.push_str(&artifact.intent);
    out.push_str("\n\n## Summary\n\n");
    out.push_str(&artifact.summary);
    if !artifact.required_tools.is_empty() {
        out.push_str("\n\n## Required Tools\n\n");
        for tool in &artifact.required_tools {
            out.push_str("- `");
            out.push_str(tool);
            out.push_str("`\n");
        }
    }
    out.push_str("\n## Procedure\n\n");
    for step in &artifact.steps {
        out.push_str("- ");
        out.push_str(&step.title);
        if let Some(r) = &step.rationale {
            out.push_str(" — ");
            out.push_str(r);
        }
        out.push('\n');
    }
    if !artifact.known_failures.is_empty() {
        out.push_str("\n## Known Failures\n\n");
        for failure in &artifact.known_failures {
            out.push_str("- ");
            out.push_str(failure);
            out.push('\n');
        }
    }
    if !artifact.verification.checks.is_empty() {
        out.push_str("\n## Verification\n\n");
        for check in &artifact.verification.checks {
            out.push_str("- ");
            out.push_str(check);
            out.push('\n');
        }
    }
    out
}

#[must_use]
pub fn render_ainl_compact_skeleton(artifact: &ProcedureArtifact, graph_name: &str) -> String {
    let graph = sanitize_graph_name(graph_name);
    let mut out = format!("# generated from {}\n{}:\n", artifact.id, graph);
    out.push_str("  in: task\n");
    out.push_str("  # Procedure intent: ");
    out.push_str(&artifact.intent.replace('\n', " "));
    out.push('\n');
    for step in &artifact.steps {
        out.push_str("  # ");
        out.push_str(&step.title.replace('\n', " "));
        out.push('\n');
    }
    out.push_str("  out \"procedure_skeleton:");
    out.push_str(&artifact.id.replace('"', ""));
    out.push_str("\"\n");
    out
}

#[must_use]
pub fn render_openfang_skill_toml(artifact: &ProcedureArtifact) -> String {
    let mut out = String::new();
    out.push_str("[skill]\n");
    out.push_str("id = \"");
    out.push_str(&toml_escape(&artifact.id));
    out.push_str("\"\nname = \"");
    out.push_str(&toml_escape(&artifact.title));
    out.push_str("\"\ndescription = \"");
    out.push_str(&toml_escape(&artifact.summary));
    out.push_str("\"\nlifecycle = \"");
    out.push_str(match artifact.lifecycle {
        ProcedureLifecycle::Draft => "draft",
        ProcedureLifecycle::Candidate => "candidate",
        ProcedureLifecycle::Validated => "validated",
        ProcedureLifecycle::Promoted => "promoted",
        ProcedureLifecycle::Deprecated => "deprecated",
    });
    out.push_str("\"\n\n[procedure]\nintent = \"");
    out.push_str(&toml_escape(&artifact.intent));
    out.push_str("\"\nrequired_tools = [");
    out.push_str(&quoted_toml_list(&artifact.required_tools));
    out.push_str("]\nobservation_count = ");
    out.push_str(&artifact.observation_count.to_string());
    out.push_str("\nfitness = ");
    out.push_str(&format!("{:.3}", artifact.fitness));
    out.push('\n');
    out
}

#[must_use]
pub fn render_hand_metadata_toml(artifact: &ProcedureArtifact) -> String {
    let mut out = String::new();
    out.push_str("[hand]\n");
    out.push_str("schema_version = \"1\"\nname = \"");
    out.push_str(&toml_escape(&artifact.title));
    out.push_str("\"\ndescription = \"");
    out.push_str(&toml_escape(&artifact.summary));
    out.push_str("\"\n\n[ainl_procedure]\nid = \"");
    out.push_str(&toml_escape(&artifact.id));
    out.push_str("\"\nintent = \"");
    out.push_str(&toml_escape(&artifact.intent));
    out.push_str("\"\nrendered_from = \"procedure_artifact\"\n");
    out
}

fn title_from_intent(intent: &str) -> String {
    let t = intent.trim();
    if t.is_empty() {
        "Learned Procedure".into()
    } else {
        let first = t.lines().next().unwrap_or(t);
        let mut s = first.chars().take(80).collect::<String>();
        if s.len() < first.len() {
            s.push_str("...");
        }
        s
    }
}

fn quoted_toml_list(values: &[String]) -> String {
    values
        .iter()
        .map(|v| format!("\"{}\"", toml_escape(v)))
        .collect::<Vec<_>>()
        .join(", ")
}

fn toml_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', " ")
}

fn sanitize_graph_name(name: &str) -> String {
    let mut out = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    if out.is_empty() {
        out.push_str("learned_procedure");
    }
    if out.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ainl_contracts::{ContextFreshness, ExperienceEvent, ImpactDecision};

    fn sample_bundle() -> ExperienceBundle {
        ExperienceBundle {
            schema_version: LEARNER_SCHEMA_VERSION,
            bundle_id: "bundle-1".into(),
            agent_id: "agent-1".into(),
            intent: "Review a pull request".into(),
            outcome: TrajectoryOutcome::Success,
            host_outcome: None,
            observation_count: 3,
            fitness: 0.8,
            events: vec![
                ExperienceEvent {
                    event_id: "e1".into(),
                    timestamp_ms: 1,
                    tool_or_adapter: "tool".into(),
                    operation: "file_read".into(),
                    success: true,
                    input_preview: None,
                    output_preview: Some("read diff".into()),
                    error: None,
                    duration_ms: 10,
                    vitals: None,
                    freshness_at_step: None,
                },
                ExperienceEvent {
                    event_id: "e2".into(),
                    timestamp_ms: 2,
                    tool_or_adapter: "tool".into(),
                    operation: "shell_exec".into(),
                    success: true,
                    input_preview: None,
                    output_preview: Some("tests pass".into()),
                    error: None,
                    duration_ms: 20,
                    vitals: None,
                    freshness_at_step: None,
                },
            ],
            source_trajectory_ids: vec!["traj-1".into()],
            source_failure_ids: vec![],
            freshness: ContextFreshness::Fresh,
            impact_decision: ImpactDecision::AllowExecute,
        }
    }

    #[test]
    fn distills_successful_recurrent_bundle() {
        let artifact = distill_procedure(&sample_bundle(), &DistillPolicy::default()).unwrap();
        assert_eq!(artifact.lifecycle, ProcedureLifecycle::Candidate);
        assert!(artifact.required_tools.contains(&"file_read".to_string()));
        assert_eq!(artifact.steps.len(), 2);
    }

    #[test]
    fn rejects_low_observation_bundle() {
        let mut b = sample_bundle();
        b.observation_count = 1;
        let err = distill_procedure(&b, &DistillPolicy::default()).unwrap_err();
        assert!(matches!(
            err,
            ProcedureLearningError::InsufficientObservations { .. }
        ));
    }

    #[test]
    fn scores_reuse_from_intent_tools_and_fitness() {
        let artifact = distill_procedure(&sample_bundle(), &DistillPolicy::default()).unwrap();
        let score = score_reuse(
            &artifact,
            "Please review this pull request",
            &["file_read".into(), "shell_exec".into()],
        );
        assert!(score.score > 0.7, "{score:?}");
    }

    #[test]
    fn failure_patch_applies_to_artifact() {
        let artifact = distill_procedure(&sample_bundle(), &DistillPolicy::default()).unwrap();
        let patch = patch_from_failure(&artifact, "f1", "shell timed out");
        let next = apply_patch(&artifact, &patch);
        assert!(next.known_failures.iter().any(|f| f.contains("timed out")));
        assert!(next.steps.len() > artifact.steps.len());
    }

    #[test]
    fn renders_markdown_and_ainl_skeleton() {
        let artifact = distill_procedure(&sample_bundle(), &DistillPolicy::default()).unwrap();
        let md = render_markdown_skill(&artifact);
        assert!(md.contains("## Procedure"));
        let ainl = render_ainl_compact_skeleton(&artifact, "review-pr");
        assert!(ainl.contains("review_pr:"));
        assert!(ainl.contains("procedure_skeleton"));
        assert!(!ainl.contains("out {"));
        let skill_toml = render_openfang_skill_toml(&artifact);
        assert!(skill_toml.contains("[skill]"));
        assert!(skill_toml.contains("required_tools"));
        let hand_toml = render_hand_metadata_toml(&artifact);
        assert!(hand_toml.contains("[hand]"));
        assert!(hand_toml.contains("[ainl_procedure]"));
    }

    #[test]
    fn crate_manifest_has_no_openfang_dependency() {
        let manifest = include_str!("../Cargo.toml");
        assert!(!manifest.contains("openfang-"));
    }

    #[test]
    fn renders_portable_execution_plan() {
        let artifact = distill_procedure(&sample_bundle(), &DistillPolicy::default()).unwrap();
        let plan = render_execution_plan(&artifact);
        assert_eq!(plan.procedure_id, artifact.id);
        assert_eq!(plan.steps.len(), artifact.steps.len());
        assert_eq!(plan.steps[0].executor, "tool");
    }
}
