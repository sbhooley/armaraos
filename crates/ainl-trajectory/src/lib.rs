//! Execution trajectory helpers for the self-learning stack.
//!
//! Hosts (`openfang-runtime`, `ainl-runtime`, MCP tooling) share [`TrajectoryDraft`] and
//! [`replay::TrajectoryReplayLine`] JSONL for exports; persistence lives in `ainl-memory`.

#![forbid(unsafe_code)]

pub mod replay;

pub use ainl_contracts::{
    TrajectoryOutcome, TrajectoryStep,
};
pub use replay::{
    parse_jsonl, trajectory_replay_line, TrajectoryReplayLine, TRAJECTORY_REPLAY_SCHEMA_VERSION,
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
        }
    }

    pub fn push_step(&mut self, step: TrajectoryStep) {
        self.steps.push(step);
    }
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
        });
        let j = serde_json::to_string(&d).unwrap();
        let back: TrajectoryDraft = serde_json::from_str(&j).unwrap();
        assert_eq!(d, back);
    }
}
