//! Row-oriented trajectory storage in `ainl_trajectories` (sibling to graph nodes).
//!
//! Full step payloads live here so `ainl_graph_nodes` trajectory rows can stay small for recall.

use ainl_contracts::{TrajectoryOutcome, TrajectoryStep};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// One persisted trajectory run: episode-linked, JSON-step payload, optional graph node cross-ref.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrajectoryDetailRecord {
    pub id: Uuid,
    pub episode_id: Uuid,
    /// Corresponding `AinlNodeType::Trajectory` node id when also written to the graph.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph_trajectory_node_id: Option<Uuid>,
    pub agent_id: String,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub recorded_at: i64,
    pub outcome: TrajectoryOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ainl_source_hash: Option<String>,
    pub duration_ms: u64,
    pub steps: Vec<TrajectoryStep>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_vars: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fitness_delta: Option<f32>,
}

impl TrajectoryDetailRecord {
    /// Map this DB row to a [`ainl_trajectory::TrajectoryReplayLine`] for JSONL export.
    #[must_use]
    pub fn to_replay_line(&self) -> ainl_trajectory::TrajectoryReplayLine {
        ainl_trajectory::trajectory_replay_line(
            self.id,
            self.episode_id,
            self.graph_trajectory_node_id,
            &self.agent_id,
            &self.session_id,
            self.project_id.as_deref(),
            self.recorded_at,
            self.outcome,
            self.ainl_source_hash.as_deref(),
            self.duration_ms,
            self.steps.clone(),
            self.frame_vars.clone(),
            self.fitness_delta,
        )
    }

    /// One JSON object + newline (JSONL) for tooling / replay files.
    pub fn to_replay_jsonl(&self) -> Result<String, serde_json::Error> {
        self.to_replay_line().to_jsonl_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn detail_record_replay_jsonl_roundtrip() {
        let r = TrajectoryDetailRecord {
            id: Uuid::nil(),
            episode_id: Uuid::nil(),
            graph_trajectory_node_id: None,
            agent_id: "a".into(),
            session_id: "s".into(),
            project_id: None,
            recorded_at: 42,
            outcome: TrajectoryOutcome::PartialSuccess,
            ainl_source_hash: Some("h1".into()),
            duration_ms: 7,
            steps: vec![],
            frame_vars: None,
            fitness_delta: None,
        };
        let line = r.to_replay_jsonl().unwrap();
        let rows = ainl_trajectory::parse_jsonl(&line).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].agent_id, "a");
        assert_eq!(rows[0].outcome, TrajectoryOutcome::PartialSuccess);
    }
}
