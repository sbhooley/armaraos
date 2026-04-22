//! JSONL replay encoding (one JSON object per line) for trajectory exports and tooling.
//!
//! Mirrors the plugin’s `trajectory_cli.py` export shape at a stable `schema_version`.

use std::io::{self, Write};

use ainl_contracts::{TrajectoryOutcome, TrajectoryStep};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Bump when the JSONL envelope gains breaking fields.
pub const TRAJECTORY_REPLAY_SCHEMA_VERSION: u32 = 1;

/// One trajectory row suitable for JSONL export (aligned with `ainl_memory::TrajectoryDetailRecord`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrajectoryReplayLine {
    pub schema_version: u32,
    pub id: String,
    pub episode_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph_trajectory_node_id: Option<String>,
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
}

impl TrajectoryReplayLine {
    /// Serialize this record as a single JSONL line (trailing newline).
    pub fn to_jsonl_string(&self) -> Result<String, serde_json::Error> {
        let mut s = serde_json::to_string(self)?;
        s.push('\n');
        Ok(s)
    }

    /// Write one JSONL line to `w`.
    pub fn write_jsonl_to<W: Write>(&self, w: &mut W) -> io::Result<()> {
        serde_json::to_writer(&mut *w, self)?;
        w.write_all(b"\n")
    }
}

/// Parse JSONL from a string (ignores empty lines).
pub fn parse_jsonl(s: &str) -> Result<Vec<TrajectoryReplayLine>, serde_json::Error> {
    s.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(serde_json::from_str)
        .collect()
}

/// Build a replay line from discrete fields (hosts that do not use `ainl-memory` types).
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn trajectory_replay_line(
    id: Uuid,
    episode_id: Uuid,
    graph_trajectory_node_id: Option<Uuid>,
    agent_id: &str,
    session_id: &str,
    project_id: Option<&str>,
    recorded_at: i64,
    outcome: TrajectoryOutcome,
    ainl_source_hash: Option<&str>,
    duration_ms: u64,
    steps: Vec<TrajectoryStep>,
) -> TrajectoryReplayLine {
    TrajectoryReplayLine {
        schema_version: TRAJECTORY_REPLAY_SCHEMA_VERSION,
        id: id.to_string(),
        episode_id: episode_id.to_string(),
        graph_trajectory_node_id: graph_trajectory_node_id.map(|u| u.to_string()),
        agent_id: agent_id.to_string(),
        session_id: session_id.to_string(),
        project_id: project_id.map(str::to_string),
        recorded_at,
        outcome,
        ainl_source_hash: ainl_source_hash.map(str::to_string),
        duration_ms,
        steps,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn roundtrip_jsonl_single_line() {
        let line = trajectory_replay_line(
            Uuid::nil(),
            Uuid::nil(),
            None,
            "agent-a",
            "sess-1",
            Some("proj"),
            1700000000,
            TrajectoryOutcome::Success,
            Some("sha256:abc"),
            12,
            vec![TrajectoryStep {
                step_id: "0".into(),
                timestamp_ms: 1,
                adapter: "builtin".into(),
                operation: "noop".into(),
                inputs_preview: None,
                outputs_preview: None,
                duration_ms: 1,
                success: true,
                error: None,
                vitals: None,
                freshness_at_step: None,
            }],
        );
        let encoded = line.to_jsonl_string().unwrap();
        let parsed = parse_jsonl(&encoded).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0], line);
    }
}
