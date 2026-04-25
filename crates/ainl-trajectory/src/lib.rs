//! Execution trajectory helpers for the self-learning stack.
//!
//! Hosts (`openfang-runtime`, `ainl-runtime`, MCP tooling) share [`TrajectoryDraft`] and
//! [`replay::TrajectoryReplayLine`] JSONL for exports; persistence lives in `ainl-memory`.

#![forbid(unsafe_code)]

pub mod replay;

pub use ainl_contracts::{TrajectoryOutcome, TrajectoryStep};
pub use replay::{
    parse_jsonl, trajectory_replay_line, TrajectoryReplayLine, TRAJECTORY_REPLAY_SCHEMA_VERSION,
};

use std::collections::{BTreeMap, BTreeSet};

use ainl_contracts::{
    ContextFreshness, ExperienceBundle, ExperienceEvent, ImpactDecision, LEARNER_SCHEMA_VERSION,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// In-memory trajectory being assembled before commit to `ainl_memory` as [`ainl_memory::TrajectoryNode`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrajectoryDraft {
    pub episode_id: Uuid,
    pub session_id: String,
    pub project_id: Option<String>,
    pub ainl_source_hash: Option<String>,
    pub outcome: TrajectoryOutcome,
    pub steps: Vec<TrajectoryStep>,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_vars: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fitness_delta: Option<f32>,
}

impl TrajectoryDraft {
    #[must_use]
    pub fn new(episode_id: Uuid, outcome: TrajectoryOutcome) -> Self {
        Self {
            episode_id,
            session_id: String::new(),
            project_id: None,
            ainl_source_hash: None,
            outcome,
            steps: Vec::new(),
            duration_ms: 0,
            frame_vars: None,
            fitness_delta: None,
        }
    }

    pub fn push_step(&mut self, step: TrajectoryStep) {
        self.steps.push(step);
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExperienceCluster {
    pub fingerprint: String,
    pub intent: String,
    pub outcome: TrajectoryOutcome,
    pub trajectories: Vec<TrajectoryDraft>,
    #[serde(default)]
    pub success_ratio: f32,
    #[serde(default)]
    pub avg_duration_ms: u64,
    #[serde(default)]
    pub avg_vitals_trust: Option<f32>,
}

#[derive(Debug, Clone)]
pub struct ClusterPolicy {
    pub min_success_ratio: f32,
    pub min_steps: usize,
    pub max_avg_duration_ms: Option<u64>,
    pub min_avg_vitals_trust: Option<f32>,
}

impl Default for ClusterPolicy {
    fn default() -> Self {
        Self {
            min_success_ratio: 1.0,
            min_steps: 1,
            max_avg_duration_ms: Some(10 * 60 * 1000),
            min_avg_vitals_trust: Some(0.55),
        }
    }
}

#[must_use]
pub fn trajectory_fingerprint(record: &TrajectoryDraft) -> String {
    let mut s = String::new();
    s.push_str(record.project_id.as_deref().unwrap_or("global"));
    s.push(':');
    if let Some(source_hash) = record.ainl_source_hash.as_deref() {
        s.push_str(source_hash);
    }
    for step in &record.steps {
        s.push('|');
        s.push_str(&step.adapter);
        s.push('.');
        s.push_str(&step.operation);
    }
    Uuid::new_v5(&Uuid::NAMESPACE_OID, s.as_bytes()).to_string()
}

#[must_use]
pub fn cluster_experiences(records: &[TrajectoryDraft]) -> Vec<ExperienceCluster> {
    cluster_experiences_with_policy(records, &ClusterPolicy::default())
}

#[must_use]
pub fn cluster_experiences_with_policy(
    records: &[TrajectoryDraft],
    policy: &ClusterPolicy,
) -> Vec<ExperienceCluster> {
    let mut clusters: BTreeMap<String, ExperienceCluster> = BTreeMap::new();
    for record in records {
        if record.steps.len() < policy.min_steps {
            continue;
        }
        let fingerprint = trajectory_fingerprint(record);
        let entry = clusters
            .entry(fingerprint.clone())
            .or_insert_with(|| ExperienceCluster {
                fingerprint,
                intent: record
                    .project_id
                    .clone()
                    .unwrap_or_else(|| "repeated agent workflow".into()),
                outcome: record.outcome,
                trajectories: Vec::new(),
                success_ratio: 0.0,
                avg_duration_ms: 0,
                avg_vitals_trust: None,
            });
        entry.trajectories.push(record.clone());
    }
    clusters
        .into_values()
        .filter_map(|mut cluster| {
            enrich_cluster_stats(&mut cluster);
            if cluster.success_ratio < policy.min_success_ratio {
                return None;
            }
            if let Some(max_duration) = policy.max_avg_duration_ms {
                if cluster.avg_duration_ms > max_duration {
                    return None;
                }
            }
            if let (Some(min_trust), Some(avg_trust)) =
                (policy.min_avg_vitals_trust, cluster.avg_vitals_trust)
            {
                if avg_trust < min_trust {
                    return None;
                }
            }
            Some(cluster)
        })
        .collect()
}

fn enrich_cluster_stats(cluster: &mut ExperienceCluster) {
    let count = cluster.trajectories.len().max(1);
    let success = cluster
        .trajectories
        .iter()
        .filter(|t| t.outcome == TrajectoryOutcome::Success && t.steps.iter().all(|s| s.success))
        .count();
    cluster.success_ratio = success as f32 / count as f32;
    cluster.avg_duration_ms = cluster
        .trajectories
        .iter()
        .map(|t| t.duration_ms)
        .sum::<u64>()
        / count as u64;
    let trusts = cluster
        .trajectories
        .iter()
        .flat_map(|t| t.steps.iter())
        .filter_map(|step| step.vitals.as_ref().map(|v| v.trust))
        .collect::<Vec<_>>();
    if !trusts.is_empty() {
        cluster.avg_vitals_trust = Some(trusts.iter().sum::<f32>() / trusts.len() as f32);
    }
}

#[must_use]
pub fn build_experience_bundle(cluster: &ExperienceCluster) -> ExperienceBundle {
    let observation_count = cluster.trajectories.len() as u32;
    let source_trajectory_ids = cluster
        .trajectories
        .iter()
        .map(|t| t.episode_id.to_string())
        .collect::<Vec<_>>();
    let events = cluster
        .trajectories
        .first()
        .map(|t| {
            t.steps
                .iter()
                .map(ExperienceEvent::from)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let fitness = if observation_count == 0 {
        0.0
    } else {
        let base = cluster.success_ratio;
        let delta = cluster
            .trajectories
            .iter()
            .filter_map(|t| t.fitness_delta)
            .sum::<f32>();
        (base + (delta / observation_count as f32)).clamp(0.0, 1.0)
    };
    ExperienceBundle {
        schema_version: LEARNER_SCHEMA_VERSION,
        bundle_id: format!("experience:{}", cluster.fingerprint),
        agent_id: "unknown".into(),
        intent: cluster.intent.clone(),
        outcome: cluster.outcome,
        host_outcome: None,
        observation_count,
        fitness,
        events,
        source_trajectory_ids,
        source_failure_ids: Vec::new(),
        freshness: ContextFreshness::Unknown,
        impact_decision: ImpactDecision::AllowExecute,
    }
}

#[must_use]
pub fn stable_tool_sequence(records: &[TrajectoryDraft]) -> Vec<String> {
    let mut seqs = records
        .iter()
        .map(|record| {
            record
                .steps
                .iter()
                .map(|step| step.operation.clone())
                .collect::<Vec<_>>()
        })
        .collect::<BTreeSet<_>>();
    seqs.pop_first().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draft_roundtrip_json() {
        let mut d = TrajectoryDraft::new(Uuid::nil(), TrajectoryOutcome::Success);
        d.duration_ms = 42;
        d.steps.push(TrajectoryStep {
            step_id: "a".into(),
            timestamp_ms: 1,
            adapter: "http".into(),
            operation: "GET".into(),
            inputs_preview: None,
            outputs_preview: None,
            duration_ms: 3,
            success: true,
            error: None,
            vitals: None,
            freshness_at_step: None,
            frame_vars: None,
            tool_telemetry: None,
        });
        let j = serde_json::to_string(&d).unwrap();
        let back: TrajectoryDraft = serde_json::from_str(&j).unwrap();
        assert_eq!(d, back);
    }

    #[test]
    fn clusters_repeated_trajectories_into_experience_bundle() {
        let mut a = TrajectoryDraft::new(Uuid::new_v4(), TrajectoryOutcome::Success);
        a.project_id = Some("review".into());
        a.steps.push(TrajectoryStep {
            step_id: "a".into(),
            timestamp_ms: 1,
            adapter: "tool".into(),
            operation: "file_read".into(),
            inputs_preview: None,
            outputs_preview: None,
            duration_ms: 3,
            success: true,
            error: None,
            vitals: None,
            freshness_at_step: None,
            frame_vars: None,
            tool_telemetry: None,
        });
        let mut b = a.clone();
        b.episode_id = Uuid::new_v4();
        let clusters = cluster_experiences(&[a, b]);
        assert_eq!(clusters.len(), 1);
        let bundle = build_experience_bundle(&clusters[0]);
        assert_eq!(bundle.observation_count, 2);
        assert_eq!(bundle.events.len(), 1);
        assert_eq!(bundle.fitness, 1.0);
    }

    #[test]
    fn clustering_suppresses_failed_or_sparse_trajectories() {
        let mut failed = TrajectoryDraft::new(Uuid::new_v4(), TrajectoryOutcome::Failure);
        failed.project_id = Some("review".into());
        failed.steps.push(TrajectoryStep {
            step_id: "a".into(),
            timestamp_ms: 1,
            adapter: "tool".into(),
            operation: "file_read".into(),
            inputs_preview: None,
            outputs_preview: None,
            duration_ms: 3,
            success: false,
            error: Some("missing file".into()),
            vitals: None,
            freshness_at_step: None,
            frame_vars: None,
            tool_telemetry: None,
        });
        assert!(cluster_experiences(&[failed]).is_empty());

        let sparse = TrajectoryDraft::new(Uuid::new_v4(), TrajectoryOutcome::Success);
        let policy = ClusterPolicy {
            min_steps: 1,
            ..ClusterPolicy::default()
        };
        assert!(cluster_experiences_with_policy(&[sparse], &policy).is_empty());
    }
}
