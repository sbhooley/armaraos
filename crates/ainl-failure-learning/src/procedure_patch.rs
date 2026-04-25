//! Procedure patch candidates derived from recurrent failure evidence.

use ainl_contracts::{
    ProcedureArtifact, ProcedurePatch, ProcedureStep, ProcedureStepKind, LEARNER_SCHEMA_VERSION,
};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq)]
pub struct ProcedureFailureEvidence {
    pub failure_id: String,
    pub summary: String,
    pub recurrence_count: u32,
}

#[derive(Debug, Clone)]
pub struct ProcedurePatchPolicy {
    pub min_recurrence_count: u32,
}

impl Default for ProcedurePatchPolicy {
    fn default() -> Self {
        Self {
            min_recurrence_count: 2,
        }
    }
}

#[must_use]
pub fn failure_patch_candidates(
    artifact: &ProcedureArtifact,
    failures: &[ProcedureFailureEvidence],
    policy: &ProcedurePatchPolicy,
) -> Vec<ProcedurePatch> {
    failures
        .iter()
        .filter(|failure| failure.recurrence_count >= policy.min_recurrence_count)
        .map(|failure| ProcedurePatch {
            schema_version: LEARNER_SCHEMA_VERSION,
            patch_id: format!(
                "patch:{}",
                Uuid::new_v5(
                    &Uuid::NAMESPACE_OID,
                    format!("{}:{}:{}", artifact.id, failure.failure_id, failure.summary)
                        .as_bytes(),
                )
            ),
            procedure_id: artifact.id.clone(),
            rationale: format!(
                "Recurring failure observed {} times: {}",
                failure.recurrence_count, failure.summary
            ),
            add_steps: structured_steps_for_failure(&failure.summary),
            add_known_failures: vec![failure.summary.clone()],
            add_recovery: recovery_for_failure(&failure.summary),
            source_failure_ids: vec![failure.failure_id.clone()],
        })
        .collect()
}

fn structured_steps_for_failure(summary: &str) -> Vec<ProcedureStep> {
    let lower = summary.to_ascii_lowercase();
    let mut steps = vec![ProcedureStep {
        step_id: "failure-precheck".into(),
        title: "Check recurring failure before continuing".into(),
        kind: ProcedureStepKind::Validate {
            target: summary.to_string(),
        },
        rationale: Some("Added automatically from failure recurrence.".into()),
    }];
    if lower.contains("syntax") || lower.contains("validat") {
        steps.push(ProcedureStep {
            step_id: "validation-gate".into(),
            title: "Validate output before downstream actions".into(),
            kind: ProcedureStepKind::Validate {
                target: "tool response has semantic success, not just transport success".into(),
            },
            rationale: Some(
                "Blocks the anti-pattern of continuing after failed validation.".into(),
            ),
        });
    }
    if lower.contains("timeout") || lower.contains("rate") {
        steps.push(ProcedureStep {
            step_id: "retry-budget-gate".into(),
            title: "Apply bounded retry or backoff before changing strategy".into(),
            kind: ProcedureStepKind::Branch {
                condition: "transient timeout/rate limit and retry budget remains".into(),
            },
            rationale: Some("Avoids repeated unchanged calls during transient failures.".into()),
        });
    }
    if lower.contains("permission") || lower.contains("denied") || lower.contains("policy") {
        steps.push(ProcedureStep {
            step_id: "human-review-policy".into(),
            title: "Escalate policy-sensitive recovery".into(),
            kind: ProcedureStepKind::HumanReview {
                reason: "permission or policy failure recurred".into(),
            },
            rationale: Some("Prevents unsafe automatic retries after policy blocks.".into()),
        });
    }
    steps
}

fn recovery_for_failure(summary: &str) -> Vec<String> {
    let lower = summary.to_ascii_lowercase();
    let mut recovery = vec![
        "Change inputs or repair the failed precondition before retrying.".into(),
        "Do not continue downstream from a failed validation or blocked tool result.".into(),
    ];
    if lower.contains("syntax") || lower.contains("validat") {
        recovery.push(
            "Re-run validation and require semantic success before claiming completion.".into(),
        );
    }
    if lower.contains("timeout") || lower.contains("rate") {
        recovery.push(
            "Use bounded retry/backoff, then switch strategy instead of repeating the same call."
                .into(),
        );
    }
    recovery
}

#[cfg(test)]
mod tests {
    use super::*;
    use ainl_contracts::{ProcedureArtifactFormat, ProcedureLifecycle, ProcedureVerification};

    fn artifact() -> ProcedureArtifact {
        ProcedureArtifact {
            schema_version: LEARNER_SCHEMA_VERSION,
            id: "proc:test".into(),
            title: "Test".into(),
            intent: "test".into(),
            summary: "summary".into(),
            required_tools: vec![],
            required_adapters: vec![],
            inputs: vec![],
            outputs: vec![],
            preconditions: vec![],
            steps: vec![],
            verification: ProcedureVerification::default(),
            known_failures: vec![],
            recovery: vec![],
            source_trajectory_ids: vec![],
            source_failure_ids: vec![],
            fitness: 0.8,
            observation_count: 3,
            lifecycle: ProcedureLifecycle::Candidate,
            render_targets: vec![ProcedureArtifactFormat::PromptOnly],
        }
    }

    #[test]
    fn emits_patch_only_above_recurrence_threshold() {
        let patches = failure_patch_candidates(
            &artifact(),
            &[
                ProcedureFailureEvidence {
                    failure_id: "f1".into(),
                    summary: "timeout".into(),
                    recurrence_count: 1,
                },
                ProcedureFailureEvidence {
                    failure_id: "f2".into(),
                    summary: "syntax invalid".into(),
                    recurrence_count: 2,
                },
            ],
            &ProcedurePatchPolicy::default(),
        );
        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].source_failure_ids, vec!["f2"]);
        assert!(patches[0].add_steps.len() >= 2);
        assert!(patches[0]
            .add_recovery
            .iter()
            .any(|r| r.contains("validation")));
    }
}
