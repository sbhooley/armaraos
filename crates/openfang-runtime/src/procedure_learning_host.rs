//! ArmaraOS host bridge for reusable AINL procedure learning.
//!
//! The core learning logic lives in `ainl-procedure-learning`; this module only adapts OpenFang
//! recurrence events into portable proposals in the existing improvement-proposal ledger.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use ainl_contracts::{
    ContextFreshness, ExperienceBundle, ExperienceEvent, ImpactDecision, ProcedureArtifact,
    ProcedureReuseOutcome, ProposalEnvelope, TrajectoryOutcome, LEARNER_SCHEMA_VERSION,
};
use ainl_improvement_proposals::ImprovementProposalId;
use ainl_memory::{AinlNodeKind, AinlNodeType, GraphMemory, TrajectoryDetailRecord};

static PROCEDURE_AUTO_SUBMIT_OK: AtomicU64 = AtomicU64::new(0);
static PROCEDURE_AUTO_SUBMIT_DEDUP: AtomicU64 = AtomicU64::new(0);
static PROCEDURE_AUTO_SUBMIT_DISABLED: AtomicU64 = AtomicU64::new(0);
static PROCEDURE_AUTO_SUBMIT_ERROR: AtomicU64 = AtomicU64::new(0);
static PROCEDURE_PATCH_SUBMIT_OK: AtomicU64 = AtomicU64::new(0);
static PROCEDURE_PATCH_SUBMIT_DEDUP: AtomicU64 = AtomicU64::new(0);
static PROCEDURE_PATCH_SUBMIT_ERROR: AtomicU64 = AtomicU64::new(0);
static PROCEDURE_DRAFT_STAGE_OK: AtomicU64 = AtomicU64::new(0);
static PROCEDURE_DRAFT_STAGE_ERROR: AtomicU64 = AtomicU64::new(0);
static PROCEDURE_REUSE_OUTCOME_OK: AtomicU64 = AtomicU64::new(0);
static PROCEDURE_REUSE_OUTCOME_ERROR: AtomicU64 = AtomicU64::new(0);

#[must_use]
pub fn metrics_snapshot() -> serde_json::Value {
    serde_json::json!({
        "procedure_auto_submit_ok": PROCEDURE_AUTO_SUBMIT_OK.load(Ordering::Relaxed),
        "procedure_auto_submit_dedup": PROCEDURE_AUTO_SUBMIT_DEDUP.load(Ordering::Relaxed),
        "procedure_auto_submit_disabled": PROCEDURE_AUTO_SUBMIT_DISABLED.load(Ordering::Relaxed),
        "procedure_auto_submit_error": PROCEDURE_AUTO_SUBMIT_ERROR.load(Ordering::Relaxed),
        "procedure_patch_submit_ok": PROCEDURE_PATCH_SUBMIT_OK.load(Ordering::Relaxed),
        "procedure_patch_submit_dedup": PROCEDURE_PATCH_SUBMIT_DEDUP.load(Ordering::Relaxed),
        "procedure_patch_submit_error": PROCEDURE_PATCH_SUBMIT_ERROR.load(Ordering::Relaxed),
        "procedure_draft_stage_ok": PROCEDURE_DRAFT_STAGE_OK.load(Ordering::Relaxed),
        "procedure_draft_stage_error": PROCEDURE_DRAFT_STAGE_ERROR.load(Ordering::Relaxed),
        "procedure_reuse_outcome_ok": PROCEDURE_REUSE_OUTCOME_OK.load(Ordering::Relaxed),
        "procedure_reuse_outcome_error": PROCEDURE_REUSE_OUTCOME_ERROR.load(Ordering::Relaxed),
    })
}

pub fn build_procedure_patch_envelope(
    artifact: &ProcedureArtifact,
    failure_id: &str,
    failure_message: &str,
) -> Result<(ProposalEnvelope, String), String> {
    let patch = ainl_procedure_learning::patch_from_failure(artifact, failure_id, failure_message);
    let proposed_json = serde_json::to_string_pretty(&patch).map_err(|e| e.to_string())?;
    let proposed_hash = ainl_improvement_proposals::sha256_hex_lower(&proposed_json);
    let original_hash = ainl_improvement_proposals::sha256_hex_lower(
        &serde_json::to_string(artifact)
            .map_err(|e| format!("serialize source procedure artifact for hash: {e}"))?,
    );
    Ok((
        ProposalEnvelope {
            schema_version: LEARNER_SCHEMA_VERSION,
            original_hash,
            proposed_hash,
            kind: ainl_improvement_proposals::proposal_kind::PROCEDURE_PATCH.into(),
            rationale: patch.rationale.clone(),
            freshness_at_proposal: ContextFreshness::Unknown,
            impact_decision: ImpactDecision::RequireImpactFirst,
        },
        proposed_json,
    ))
}

pub fn submit_procedure_patch_from_failure(
    home_dir: &Path,
    agent_id: &str,
    artifact: &ProcedureArtifact,
    failure_id: &str,
    failure_message: &str,
) -> Result<Option<ImprovementProposalId>, String> {
    if !crate::improvement_proposals_host::env_enabled() {
        PROCEDURE_PATCH_SUBMIT_ERROR.fetch_add(1, Ordering::Relaxed);
        return Err("improvement proposals are disabled".to_string());
    }
    let (envelope, proposed_json) =
        build_procedure_patch_envelope(artifact, failure_id, failure_message)?;
    let existing = crate::improvement_proposals_host::list_proposals(home_dir, agent_id, 200)
        .unwrap_or_default();
    if existing.iter().any(|r| {
        r.proposed_hash == envelope.proposed_hash
            && r.kind == ainl_improvement_proposals::proposal_kind::PROCEDURE_PATCH
    }) {
        PROCEDURE_PATCH_SUBMIT_DEDUP.fetch_add(1, Ordering::Relaxed);
        return Ok(None);
    }
    match crate::improvement_proposals_host::submit(home_dir, agent_id, &envelope, &proposed_json) {
        Ok(id) => {
            PROCEDURE_PATCH_SUBMIT_OK.fetch_add(1, Ordering::Relaxed);
            Ok(Some(id))
        }
        Err(e) => {
            PROCEDURE_PATCH_SUBMIT_ERROR.fetch_add(1, Ordering::Relaxed);
            Err(e)
        }
    }
}

pub fn submit_patches_for_selected_procedures(
    home_dir: &Path,
    agent_id: &str,
    selected_procedure_ids: &[String],
    failure_id: &str,
    failure_message: &str,
) -> Result<Vec<ImprovementProposalId>, String> {
    if selected_procedure_ids.is_empty() {
        return Ok(Vec::new());
    }
    let db = graph_memory_db_path(home_dir, agent_id);
    let memory = GraphMemory::new(&db)?;
    let artifacts = memory.recall_procedure_artifacts()?;
    let mut submitted = Vec::new();
    for artifact in artifacts
        .into_iter()
        .filter(|artifact| selected_procedure_ids.iter().any(|id| id == &artifact.id))
    {
        if let Some(id) = submit_procedure_patch_from_failure(
            home_dir,
            agent_id,
            &artifact,
            failure_id,
            failure_message,
        )? {
            submitted.push(id);
        }
    }
    Ok(submitted)
}

pub fn maybe_submit_patches_for_selected_procedures(
    home_dir: &Path,
    agent_id: &str,
    selected_procedure_ids: &[String],
    failure_id: &str,
    failure_message: &str,
) {
    match submit_patches_for_selected_procedures(
        home_dir,
        agent_id,
        selected_procedure_ids,
        failure_id,
        failure_message,
    ) {
        Ok(ids) if !ids.is_empty() => tracing::debug!(
            agent_id = %agent_id,
            proposal_count = ids.len(),
            failure_id = %failure_id,
            "submitted procedure patch proposals after failed reuse"
        ),
        Ok(_) => {}
        Err(e) => tracing::warn!(
            agent_id = %agent_id,
            failure_id = %failure_id,
            error = %e,
            "failed to submit procedure patch proposals after failed reuse"
        ),
    }
}

pub fn record_reuse_outcomes_for_selected_procedures(
    home_dir: &Path,
    agent_id: &str,
    selected_procedure_ids: &[String],
    outcome: TrajectoryOutcome,
    failure_id: Option<String>,
    notes: Option<String>,
) {
    if selected_procedure_ids.is_empty() {
        return;
    }
    let db = graph_memory_db_path(home_dir, agent_id);
    let Ok(memory) = GraphMemory::new(&db) else {
        PROCEDURE_REUSE_OUTCOME_ERROR.fetch_add(1, Ordering::Relaxed);
        return;
    };
    for procedure_id in selected_procedure_ids {
        let reuse = ProcedureReuseOutcome {
            procedure_id: procedure_id.clone(),
            outcome,
            failure_id: failure_id.clone(),
            notes: notes.clone(),
        };
        match memory.record_procedure_reuse_outcome_for_agent(agent_id, &reuse) {
            Ok(_) => {
                PROCEDURE_REUSE_OUTCOME_OK.fetch_add(1, Ordering::Relaxed);
            }
            Err(e) => {
                PROCEDURE_REUSE_OUTCOME_ERROR.fetch_add(1, Ordering::Relaxed);
                tracing::warn!(
                    agent_id = %agent_id,
                    procedure_id = %procedure_id,
                    error = %e,
                    "failed to record procedure reuse outcome"
                );
            }
        }
    }
}

#[must_use]
pub fn auto_submit_env_enabled() -> bool {
    match std::env::var("AINL_AUTO_SUBMIT_PROCEDURE_PROPOSALS") {
        Ok(s) => {
            let v = s.trim().to_ascii_lowercase();
            !(v == "0" || v == "false" || v == "no" || v == "off")
        }
        Err(_) => true,
    }
}

#[derive(Debug, Clone)]
pub struct ProcedureMintFromPattern<'a> {
    pub name: &'a str,
    pub tool_sequence: &'a [String],
    pub observation_count: u32,
    pub fitness: f32,
    pub freshness_at_proposal: Option<ContextFreshness>,
}

fn graph_memory_db_path(home_dir: &Path, agent_id: &str) -> std::path::PathBuf {
    home_dir
        .join("agents")
        .join(agent_id)
        .join("ainl_memory.db")
}

fn skills_staging_dir(home_dir: &Path, agent_id: &str, artifact_id: &str) -> std::path::PathBuf {
    home_dir
        .join("skills")
        .join("staging")
        .join(agent_id)
        .join(sanitize_path_component(artifact_id))
}

fn skills_promoted_dir(home_dir: &Path, agent_id: &str, artifact_id: &str) -> std::path::PathBuf {
    home_dir
        .join("skills")
        .join("promoted")
        .join(agent_id)
        .join(sanitize_path_component(artifact_id))
}

fn sanitize_path_component(raw: &str) -> String {
    let mut out = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    if out.is_empty() {
        out.push_str("procedure");
    }
    out
}

pub fn stage_procedure_artifact_draft(
    home_dir: &Path,
    agent_id: &str,
    artifact: &ProcedureArtifact,
) -> Result<std::path::PathBuf, String> {
    let dir = skills_staging_dir(home_dir, agent_id, &artifact.id);
    std::fs::create_dir_all(&dir).map_err(|e| format!("create procedure staging dir: {e}"))?;
    std::fs::write(
        dir.join("SKILL.md"),
        ainl_procedure_learning::render_markdown_skill(artifact),
    )
    .map_err(|e| format!("write SKILL.md: {e}"))?;
    std::fs::write(
        dir.join("skill.toml"),
        ainl_procedure_learning::render_openfang_skill_toml(artifact),
    )
    .map_err(|e| format!("write skill.toml: {e}"))?;
    std::fs::write(
        dir.join("HAND.toml"),
        ainl_procedure_learning::render_hand_metadata_toml(artifact),
    )
    .map_err(|e| format!("write HAND.toml: {e}"))?;
    std::fs::write(
        dir.join("procedure.json"),
        serde_json::to_vec_pretty(artifact).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("write procedure.json: {e}"))?;
    Ok(dir)
}

pub fn write_procedure_promotion_outputs(
    home_dir: &Path,
    agent_id: &str,
    artifact: &ProcedureArtifact,
) -> Result<std::path::PathBuf, String> {
    let dir = skills_promoted_dir(home_dir, agent_id, &artifact.id);
    std::fs::create_dir_all(&dir).map_err(|e| format!("create promoted procedure dir: {e}"))?;
    std::fs::write(
        dir.join("SKILL.md"),
        ainl_procedure_learning::render_markdown_skill(artifact),
    )
    .map_err(|e| format!("write promoted SKILL.md: {e}"))?;
    std::fs::write(
        dir.join("skill.toml"),
        ainl_procedure_learning::render_openfang_skill_toml(artifact),
    )
    .map_err(|e| format!("write promoted skill.toml: {e}"))?;
    std::fs::write(
        dir.join("HAND.toml"),
        ainl_procedure_learning::render_hand_metadata_toml(artifact),
    )
    .map_err(|e| format!("write promoted HAND.toml: {e}"))?;
    std::fs::write(
        dir.join("procedure.ainl"),
        ainl_procedure_learning::render_ainl_compact_skeleton(artifact, "learned_procedure"),
    )
    .map_err(|e| format!("write promoted procedure.ainl: {e}"))?;
    std::fs::write(
        dir.join("execution_plan.json"),
        serde_json::to_vec_pretty(&ainl_procedure_learning::render_execution_plan(artifact))
            .map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("write promoted execution_plan.json: {e}"))?;
    std::fs::write(
        dir.join("procedure.json"),
        serde_json::to_vec_pretty(artifact).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("write promoted procedure.json: {e}"))?;
    Ok(dir)
}

#[must_use]
pub fn build_experience_bundle_from_pattern(
    agent_id: &str,
    spec: &ProcedureMintFromPattern<'_>,
) -> ExperienceBundle {
    let now = chrono::Utc::now().timestamp_millis();
    ExperienceBundle {
        schema_version: LEARNER_SCHEMA_VERSION,
        bundle_id: format!(
            "experience:{}",
            ainl_procedure_learning::sha256_hex_lower(&format!(
                "{}:{}:{}",
                agent_id,
                spec.name,
                spec.tool_sequence.join("->")
            ))
        ),
        agent_id: agent_id.to_string(),
        intent: spec.name.trim().to_string(),
        outcome: TrajectoryOutcome::Success,
        host_outcome: None,
        observation_count: spec.observation_count,
        fitness: spec.fitness,
        events: spec
            .tool_sequence
            .iter()
            .enumerate()
            .map(|(idx, tool)| ExperienceEvent {
                event_id: format!("event-{:02}", idx + 1),
                timestamp_ms: now + idx as i64,
                tool_or_adapter: "tool".into(),
                operation: tool.clone(),
                success: true,
                input_preview: None,
                output_preview: None,
                error: None,
                duration_ms: 0,
                vitals: None,
                freshness_at_step: None,
            })
            .collect(),
        source_trajectory_ids: Vec::new(),
        source_failure_ids: Vec::new(),
        freshness: spec
            .freshness_at_proposal
            .unwrap_or(ContextFreshness::Unknown),
        impact_decision: ImpactDecision::AllowExecute,
    }
}

#[must_use]
pub fn build_experience_bundle_from_memory(
    home_dir: &Path,
    agent_id: &str,
    spec: &ProcedureMintFromPattern<'_>,
) -> ExperienceBundle {
    let db = graph_memory_db_path(home_dir, agent_id);
    let Ok(memory) = GraphMemory::new(&db) else {
        return build_experience_bundle_from_pattern(agent_id, spec);
    };
    let Ok(rows) = memory.list_trajectories_for_agent(agent_id, 100, None) else {
        return build_experience_bundle_from_pattern(agent_id, spec);
    };
    let matching = rows
        .into_iter()
        .filter(|row| {
            row.outcome == TrajectoryOutcome::Success
                && trajectory_matches_tools(row, spec.tool_sequence)
        })
        .collect::<Vec<_>>();
    if matching.is_empty() {
        return build_experience_bundle_from_pattern(agent_id, spec);
    }
    let first = &matching[0];
    let mut bundle = ExperienceBundle {
        schema_version: LEARNER_SCHEMA_VERSION,
        bundle_id: format!(
            "experience:{}",
            ainl_procedure_learning::sha256_hex_lower(&format!(
                "{}:{}:{}",
                agent_id,
                spec.name,
                matching
                    .iter()
                    .map(|row| row.id.to_string())
                    .collect::<Vec<_>>()
                    .join(":")
            ))
        ),
        agent_id: agent_id.to_string(),
        intent: spec.name.trim().to_string(),
        outcome: TrajectoryOutcome::Success,
        host_outcome: Some("agent_loop_completed".into()),
        observation_count: spec.observation_count.max(matching.len() as u32),
        fitness: fitness_from_trajectory_rows(&matching, spec.fitness),
        events: first.steps.iter().map(ExperienceEvent::from).collect(),
        source_trajectory_ids: matching.iter().map(|row| row.id.to_string()).collect(),
        source_failure_ids: Vec::new(),
        freshness: first
            .steps
            .iter()
            .filter_map(|step| step.freshness_at_step)
            .next()
            .unwrap_or_else(|| {
                spec.freshness_at_proposal
                    .unwrap_or(ContextFreshness::Unknown)
            }),
        impact_decision: ImpactDecision::AllowExecute,
    };
    bundle.source_failure_ids = related_failure_ids(&memory, agent_id, spec.tool_sequence);
    bundle
}

fn trajectory_matches_tools(row: &TrajectoryDetailRecord, tools: &[String]) -> bool {
    if tools.is_empty() || row.steps.is_empty() {
        return false;
    }
    let ops = row
        .steps
        .iter()
        .map(|step| step.operation.trim())
        .collect::<Vec<_>>();
    let want = tools.iter().map(|tool| tool.trim()).collect::<Vec<_>>();
    if ops == want {
        return true;
    }
    want.iter().all(|tool| ops.iter().any(|op| op == tool))
}

fn fitness_from_trajectory_rows(rows: &[TrajectoryDetailRecord], fallback: f32) -> f32 {
    if rows.is_empty() {
        return fallback.clamp(0.0, 1.0);
    }
    let success_ratio = rows
        .iter()
        .filter(|row| row.outcome == TrajectoryOutcome::Success)
        .count() as f32
        / rows.len() as f32;
    let avg_vitals_trust = rows
        .iter()
        .flat_map(|row| row.steps.iter())
        .filter_map(|step| step.vitals.as_ref().map(|v| v.trust))
        .collect::<Vec<_>>();
    let vitals_score = if avg_vitals_trust.is_empty() {
        fallback.clamp(0.0, 1.0)
    } else {
        avg_vitals_trust.iter().sum::<f32>() / avg_vitals_trust.len() as f32
    };
    ((success_ratio * 0.65) + (vitals_score.clamp(0.0, 1.0) * 0.35)).clamp(0.0, 1.0)
}

fn related_failure_ids(memory: &GraphMemory, agent_id: &str, tools: &[String]) -> Vec<String> {
    let Ok(nodes) = memory.recall_by_type(AinlNodeKind::Failure, 60 * 60 * 24 * 90) else {
        return Vec::new();
    };
    nodes
        .into_iter()
        .filter(|node| node.agent_id == agent_id)
        .filter_map(|node| {
            let AinlNodeType::Failure { failure } = &node.node_type else {
                return None;
            };
            let matches_tool = failure
                .tool_name
                .as_deref()
                .or(failure.source_tool.as_deref())
                .is_some_and(|name| tools.iter().any(|tool| tool == name));
            if matches_tool {
                Some(node.id.to_string())
            } else {
                None
            }
        })
        .take(20)
        .collect()
}

pub fn build_procedure_mint_envelope(
    agent_id: &str,
    spec: &ProcedureMintFromPattern<'_>,
) -> Result<(ProposalEnvelope, String), String> {
    let bundle = build_experience_bundle_from_pattern(agent_id, spec);
    build_procedure_mint_envelope_from_bundle(&bundle, spec)
}

pub fn build_procedure_mint_envelope_from_bundle(
    bundle: &ExperienceBundle,
    spec: &ProcedureMintFromPattern<'_>,
) -> Result<(ProposalEnvelope, String), String> {
    let artifact = ainl_procedure_learning::distill_procedure(
        bundle,
        &ainl_procedure_learning::DistillPolicy::default(),
    )
    .map_err(|e| e.to_string())?;
    let proposed_json = serde_json::to_string_pretty(&artifact).map_err(|e| e.to_string())?;
    let proposed_hash = ainl_improvement_proposals::sha256_hex_lower(&proposed_json);
    let original_hash = ainl_improvement_proposals::sha256_hex_lower(&format!(
        "procedure_mint:{}:{}",
        bundle.agent_id, bundle.bundle_id
    ));
    Ok((
        ProposalEnvelope {
            schema_version: LEARNER_SCHEMA_VERSION,
            original_hash,
            proposed_hash,
            kind: ainl_improvement_proposals::proposal_kind::PROCEDURE_MINT.into(),
            rationale: format!(
                "Auto-mint reusable procedure from recurrent pattern `{}` ({} observations, fitness {:.2}).",
                spec.name, bundle.observation_count, bundle.fitness
            ),
            freshness_at_proposal: bundle.freshness,
            impact_decision: bundle.impact_decision,
        },
        proposed_json,
    ))
}

pub fn auto_submit_procedure_mint_from_pattern(
    home_dir: &Path,
    agent_id: &str,
    spec: &ProcedureMintFromPattern<'_>,
) -> Result<Option<ImprovementProposalId>, String> {
    if !crate::improvement_proposals_host::env_enabled() || !auto_submit_env_enabled() {
        PROCEDURE_AUTO_SUBMIT_DISABLED.fetch_add(1, Ordering::Relaxed);
        return Ok(None);
    }
    if spec.tool_sequence.is_empty() {
        PROCEDURE_AUTO_SUBMIT_DEDUP.fetch_add(1, Ordering::Relaxed);
        return Ok(None);
    }
    let bundle = build_experience_bundle_from_memory(home_dir, agent_id, spec);
    let (envelope, proposed_json) = build_procedure_mint_envelope_from_bundle(&bundle, spec)?;
    let artifact_for_stage = serde_json::from_str::<ProcedureArtifact>(&proposed_json).ok();
    if let Some(artifact) = artifact_for_stage.as_ref() {
        if procedure_candidate_already_exists(home_dir, agent_id, artifact, &envelope.proposed_hash)
        {
            PROCEDURE_AUTO_SUBMIT_DEDUP.fetch_add(1, Ordering::Relaxed);
            return Ok(None);
        }
    }
    let existing = crate::improvement_proposals_host::list_proposals(home_dir, agent_id, 200)
        .unwrap_or_default();
    if existing.iter().any(|r| {
        r.proposed_hash == envelope.proposed_hash
            && r.kind == ainl_improvement_proposals::proposal_kind::PROCEDURE_MINT
    }) {
        PROCEDURE_AUTO_SUBMIT_DEDUP.fetch_add(1, Ordering::Relaxed);
        return Ok(None);
    }
    match crate::improvement_proposals_host::submit(home_dir, agent_id, &envelope, &proposed_json) {
        Ok(id) => {
            PROCEDURE_AUTO_SUBMIT_OK.fetch_add(1, Ordering::Relaxed);
            if let Some(artifact) = artifact_for_stage.as_ref() {
                match stage_procedure_artifact_draft(home_dir, agent_id, artifact) {
                    Ok(path) => {
                        PROCEDURE_DRAFT_STAGE_OK.fetch_add(1, Ordering::Relaxed);
                        tracing::debug!(
                            agent_id = %agent_id,
                            artifact_id = %artifact.id,
                            staging_path = %path.display(),
                            "staged procedure artifact draft"
                        )
                    }
                    Err(e) => {
                        PROCEDURE_DRAFT_STAGE_ERROR.fetch_add(1, Ordering::Relaxed);
                        tracing::warn!(
                            agent_id = %agent_id,
                            artifact_id = %artifact.id,
                            error = %e,
                            "failed to stage procedure artifact draft"
                        );
                    }
                }
            }
            Ok(Some(id))
        }
        Err(e) => {
            PROCEDURE_AUTO_SUBMIT_ERROR.fetch_add(1, Ordering::Relaxed);
            Err(e)
        }
    }
}

fn procedure_candidate_already_exists(
    home_dir: &Path,
    agent_id: &str,
    artifact: &ProcedureArtifact,
    proposed_hash: &str,
) -> bool {
    let db = graph_memory_db_path(home_dir, agent_id);
    if let Ok(memory) = GraphMemory::new(&db) {
        if memory
            .recall_procedure_artifacts()
            .unwrap_or_default()
            .into_iter()
            .any(|existing| existing.id == artifact.id)
        {
            return true;
        }
    }
    let staged = skills_staging_dir(home_dir, agent_id, &artifact.id).join("procedure.json");
    if staged.exists() {
        return true;
    }
    let existing = crate::improvement_proposals_host::list_proposals(home_dir, agent_id, 500)
        .unwrap_or_default();
    existing.iter().any(|r| {
        r.proposed_hash == proposed_hash
            || (r.kind == ainl_improvement_proposals::proposal_kind::PROCEDURE_MINT
                && r.original_hash.contains(&artifact.id))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ainl_contracts::{CognitiveVitals, VitalsGate};
    use ainl_memory::{AinlMemoryNode, TrajectoryDetailRecord};
    use uuid::Uuid;

    #[test]
    fn builds_procedure_mint_envelope_with_matching_hash() {
        let tools = vec!["file_read".to_string(), "shell_exec".to_string()];
        let spec = ProcedureMintFromPattern {
            name: "Review pull request",
            tool_sequence: &tools,
            observation_count: 3,
            fitness: 0.8,
            freshness_at_proposal: Some(ContextFreshness::Fresh),
        };
        let (env, text) = build_procedure_mint_envelope("agent", &spec).unwrap();
        assert_eq!(
            env.kind,
            ainl_improvement_proposals::proposal_kind::PROCEDURE_MINT
        );
        assert!(ainl_improvement_proposals::proposed_hash_matches(
            &env, &text
        ));
        assert!(text.contains("\"Review pull request\""));
    }

    #[test]
    fn builds_procedure_patch_envelope_with_matching_hash() {
        let tools = vec!["file_read".to_string(), "shell_exec".to_string()];
        let spec = ProcedureMintFromPattern {
            name: "Review pull request",
            tool_sequence: &tools,
            observation_count: 3,
            fitness: 0.8,
            freshness_at_proposal: Some(ContextFreshness::Fresh),
        };
        let (_, text) = build_procedure_mint_envelope("agent", &spec).unwrap();
        let mut artifact: ProcedureArtifact = serde_json::from_str(&text).unwrap();
        artifact.lifecycle = ainl_contracts::ProcedureLifecycle::Validated;
        let (env, patch_text) =
            build_procedure_patch_envelope(&artifact, "failure-1", "validation failed").unwrap();
        assert_eq!(
            env.kind,
            ainl_improvement_proposals::proposal_kind::PROCEDURE_PATCH
        );
        assert!(ainl_improvement_proposals::proposed_hash_matches(
            &env,
            &patch_text
        ));
        let patch: ainl_contracts::ProcedurePatch = serde_json::from_str(&patch_text).unwrap();
        assert_eq!(patch.procedure_id, artifact.id);
    }

    #[test]
    fn stages_procedure_artifact_draft_files() {
        let tools = vec!["file_read".to_string(), "shell_exec".to_string()];
        let spec = ProcedureMintFromPattern {
            name: "Review pull request",
            tool_sequence: &tools,
            observation_count: 3,
            fitness: 0.8,
            freshness_at_proposal: Some(ContextFreshness::Fresh),
        };
        let (_, text) = build_procedure_mint_envelope("agent", &spec).unwrap();
        let mut artifact: ProcedureArtifact = serde_json::from_str(&text).unwrap();
        artifact.lifecycle = ainl_contracts::ProcedureLifecycle::Validated;
        let tmp = tempfile::tempdir().unwrap();
        let dir = stage_procedure_artifact_draft(tmp.path(), "agent", &artifact).unwrap();
        assert!(dir.join("SKILL.md").exists());
        assert!(dir.join("skill.toml").exists());
        assert!(dir.join("HAND.toml").exists());
        assert!(dir.join("procedure.json").exists());
    }

    #[test]
    fn dedupes_existing_staged_procedure_candidate() {
        let tools = vec!["file_read".to_string(), "shell_exec".to_string()];
        let spec = ProcedureMintFromPattern {
            name: "Review pull request",
            tool_sequence: &tools,
            observation_count: 3,
            fitness: 0.8,
            freshness_at_proposal: Some(ContextFreshness::Fresh),
        };
        let (_, text) = build_procedure_mint_envelope("agent", &spec).unwrap();
        let artifact: ProcedureArtifact = serde_json::from_str(&text).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        stage_procedure_artifact_draft(tmp.path(), "agent", &artifact).unwrap();
        assert!(procedure_candidate_already_exists(
            tmp.path(),
            "agent",
            &artifact,
            "hash"
        ));
    }

    #[test]
    fn builds_experience_bundle_from_real_trajectory_and_failure_rows() {
        let tmp = tempfile::tempdir().unwrap();
        let agent_id = "agent-rich";
        let gm_path = graph_memory_db_path(tmp.path(), agent_id);
        std::fs::create_dir_all(gm_path.parent().unwrap()).unwrap();
        let memory = GraphMemory::new(&gm_path).unwrap();
        let episode_id = memory
            .write_episode(vec!["file_read".into()], None, None)
            .unwrap();
        let row = TrajectoryDetailRecord {
            id: Uuid::new_v4(),
            episode_id,
            graph_trajectory_node_id: None,
            agent_id: agent_id.into(),
            session_id: "session-1".into(),
            project_id: Some("project".into()),
            recorded_at: chrono::Utc::now().timestamp(),
            outcome: TrajectoryOutcome::Success,
            ainl_source_hash: Some("source-hash".into()),
            duration_ms: 25,
            steps: vec![ainl_contracts::TrajectoryStep {
                step_id: "s1".into(),
                timestamp_ms: 10,
                adapter: "tool".into(),
                operation: "file_read".into(),
                inputs_preview: Some("path".into()),
                outputs_preview: Some("contents".into()),
                duration_ms: 25,
                success: true,
                error: None,
                vitals: Some(CognitiveVitals {
                    gate: VitalsGate::Pass,
                    phase: "reasoning:0.9".into(),
                    trust: 0.9,
                    mean_logprob: -0.2,
                    entropy: 0.1,
                    sample_tokens: 12,
                }),
                freshness_at_step: Some(ContextFreshness::Fresh),
                frame_vars: None,
                tool_telemetry: None,
            }],
            frame_vars: None,
            fitness_delta: Some(0.05),
        };
        memory.insert_trajectory_detail(&row).unwrap();
        let mut failure = AinlMemoryNode::new_tool_execution_failure(
            "file_read",
            "ENOENT once",
            Some("session-1"),
        );
        failure.agent_id = agent_id.into();
        memory.write_node(&failure).unwrap();

        let tools = vec!["file_read".to_string()];
        let spec = ProcedureMintFromPattern {
            name: "Read files safely",
            tool_sequence: &tools,
            observation_count: 3,
            fitness: 0.7,
            freshness_at_proposal: None,
        };
        let bundle = build_experience_bundle_from_memory(tmp.path(), agent_id, &spec);
        assert_eq!(bundle.source_trajectory_ids, vec![row.id.to_string()]);
        assert_eq!(bundle.source_failure_ids, vec![failure.id.to_string()]);
        assert_eq!(bundle.events[0].duration_ms, 25);
        assert!(bundle.events[0].vitals.is_some());
        assert_eq!(bundle.freshness, ContextFreshness::Fresh);
    }

    #[test]
    fn submits_patch_for_selected_procedure_artifact() {
        let tmp = tempfile::tempdir().unwrap();
        let agent_id = "agent-patch";
        let gm_path = graph_memory_db_path(tmp.path(), agent_id);
        std::fs::create_dir_all(gm_path.parent().unwrap()).unwrap();
        let memory = GraphMemory::new(&gm_path).unwrap();
        let tools = vec!["file_read".to_string(), "shell_exec".to_string()];
        let spec = ProcedureMintFromPattern {
            name: "Review pull request",
            tool_sequence: &tools,
            observation_count: 3,
            fitness: 0.8,
            freshness_at_proposal: Some(ContextFreshness::Fresh),
        };
        let (_, text) = build_procedure_mint_envelope(agent_id, &spec).unwrap();
        let mut artifact: ProcedureArtifact = serde_json::from_str(&text).unwrap();
        artifact.lifecycle = ainl_contracts::ProcedureLifecycle::Validated;
        memory
            .write_procedure_artifact_for_agent(agent_id, &artifact)
            .unwrap();
        assert_eq!(memory.recall_procedure_artifacts().unwrap().len(), 1);
        let ids = submit_patches_for_selected_procedures(
            tmp.path(),
            agent_id,
            std::slice::from_ref(&artifact.id),
            "failure-1",
            "Agent loop ended with FailedSafely",
        )
        .unwrap();
        assert_eq!(ids.len(), 1);
        let proposals =
            crate::improvement_proposals_host::list_proposals(tmp.path(), agent_id, 20).unwrap();
        assert_eq!(proposals.len(), 1);
        assert_eq!(
            proposals[0].kind,
            ainl_improvement_proposals::proposal_kind::PROCEDURE_PATCH
        );
    }
}
