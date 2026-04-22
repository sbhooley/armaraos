//! Optional **trajectory recap** lines for prompt injection (`sources-trajectory-recap` feature).
//!
//! Hosts read recent [`ainl_memory::trajectory_table::TrajectoryDetailRecord`] rows and pass them
//! here to produce short bullet lines for a `## TrajectoryRecap` block (opt-in in OpenFang via
//! `AINL_MEMORY_INCLUDE_TRAJECTORY_RECAP`).

use ainl_memory::TrajectoryDetailRecord;

/// Build human-readable one-line summaries for recent trajectories (newest rows first in `rows`).
#[must_use]
pub fn format_trajectory_recap_lines(
    rows: &[TrajectoryDetailRecord],
    max_rows: usize,
    max_ops: usize,
) -> Vec<String> {
    let cap = max_rows.max(1);
    let op_cap = max_ops.max(1);
    rows.iter()
        .take(cap)
        .map(|r| {
            let mut ops: Vec<String> = r
                .steps
                .iter()
                .take(op_cap)
                .map(|s| s.operation.clone())
                .collect();
            if r.steps.len() > op_cap {
                ops.push("…".to_string());
            }
            let ops_s = if ops.is_empty() {
                "no_steps".to_string()
            } else {
                ops.join(" → ")
            };
            let fd = r
                .fitness_delta
                .map(|d| format!("{d:.3}"))
                .unwrap_or_else(|| "n/a".to_string());
            format!(
                "traj={} ep={} outcome={:?} steps={} ms={} fitnessΔ={} ops: {}",
                short_id(&r.id),
                short_id(&r.episode_id),
                r.outcome,
                r.steps.len(),
                r.duration_ms,
                fd,
                ops_s
            )
        })
        .collect()
}

fn short_id(id: &uuid::Uuid) -> String {
    let s = id.to_string();
    s.chars().take(8).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ainl_contracts::TrajectoryOutcome;
    use uuid::Uuid;

    #[test]
    fn format_skips_empty_steps_with_placeholder() {
        let r = TrajectoryDetailRecord {
            id: Uuid::nil(),
            episode_id: Uuid::nil(),
            graph_trajectory_node_id: None,
            agent_id: "a".into(),
            session_id: "s".into(),
            project_id: None,
            recorded_at: 1,
            outcome: TrajectoryOutcome::Success,
            ainl_source_hash: None,
            duration_ms: 9,
            steps: vec![],
            frame_vars: None,
            fitness_delta: Some(0.1),
        };
        let lines = format_trajectory_recap_lines(&[r], 2, 3);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("no_steps"), "{}", lines[0]);
    }
}
