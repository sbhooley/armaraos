//! Persist graph [`TrajectoryNode`] + `ainl_trajectories` row in one shot (OpenFang + ainl-runtime).

use crate::node::{AinlMemoryNode, TrajectoryNode};
use crate::trajectory_table::TrajectoryDetailRecord;
use crate::GraphMemory;

use ainl_contracts::{TrajectoryOutcome, TrajectoryStep};
use uuid::Uuid;

/// When **unset** or any non-falsy value, trajectory rows are written after each successful episode
/// (same opt-out semantics as `AINL_EXTRACTOR_ENABLED` in OpenFang: `0`, `false`, `no`, `off`).
#[must_use]
pub fn trajectory_env_enabled() -> bool {
    match std::env::var("AINL_TRAJECTORY_ENABLED") {
        Ok(s) => {
            let v = s.trim().to_ascii_lowercase();
            !(v == "0" || v == "false" || v == "no" || v == "off")
        }
        Err(_) => true,
    }
}

fn coarse_steps_from_tools(tools: &[String]) -> Vec<TrajectoryStep> {
    let base_ms = chrono::Utc::now().timestamp_millis();
    tools
        .iter()
        .enumerate()
        .map(|(i, name)| TrajectoryStep {
            step_id: format!("step_{i}"),
            timestamp_ms: base_ms + i as i64,
            adapter: "builtin".into(),
            operation: name.clone(),
            inputs_preview: None,
            outputs_preview: None,
            duration_ms: 0,
            success: true,
            error: None,
            vitals: None,
            freshness_at_step: None,
        })
        .collect()
}

/// Write trajectory graph node, `trajectory_of` edge, and `ainl_trajectories` detail row.
///
/// Returns `(graph_trajectory_node_id, detail_table_row_id)`.
pub fn persist_trajectory_for_episode(
    memory: &GraphMemory,
    agent_id: &str,
    episode_graph_id: Uuid,
    steps: Vec<TrajectoryStep>,
    outcome: TrajectoryOutcome,
    session_id: &str,
    project_id: Option<&str>,
    ainl_source_hash: Option<&str>,
    duration_ms: u64,
) -> Result<(Uuid, Uuid), String> {
    let recorded_at = chrono::Utc::now().timestamp();
    let traj_body = TrajectoryNode {
        episode_id: episode_graph_id,
        recorded_at,
        session_id: session_id.to_string(),
        project_id: project_id.map(str::to_string),
        ainl_source_hash: ainl_source_hash.map(str::to_string),
        outcome,
        steps: steps.clone(),
        duration_ms,
    };
    let node = AinlMemoryNode::new_trajectory(traj_body, agent_id);
    let graph_traj_id = node.id;
    memory.write_node(&node)?;
    memory.insert_graph_edge_checked(graph_traj_id, episode_graph_id, "trajectory_of")?;

    let detail_id = Uuid::new_v4();
    let row = TrajectoryDetailRecord {
        id: detail_id,
        episode_id: episode_graph_id,
        graph_trajectory_node_id: Some(graph_traj_id),
        agent_id: agent_id.to_string(),
        session_id: session_id.to_string(),
        project_id: project_id.map(str::to_string),
        recorded_at,
        outcome,
        ainl_source_hash: ainl_source_hash.map(str::to_string),
        duration_ms,
        steps,
    };
    memory.insert_trajectory_detail(&row)?;
    Ok((graph_traj_id, detail_id))
}

/// Convenience when only coarse tool names are known (no per-call timings).
#[inline]
pub fn persist_trajectory_coarse_tools(
    memory: &GraphMemory,
    agent_id: &str,
    episode_graph_id: Uuid,
    tools: &[String],
    outcome: TrajectoryOutcome,
    session_id: &str,
    project_id: Option<&str>,
    ainl_source_hash: Option<&str>,
) -> Result<(Uuid, Uuid), String> {
    let steps = coarse_steps_from_tools(tools);
    persist_trajectory_for_episode(
        memory,
        agent_id,
        episode_graph_id,
        steps,
        outcome,
        session_id,
        project_id,
        ainl_source_hash,
        0,
    )
}
