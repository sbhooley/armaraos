//! Per-turn trajectory assembly: [`TrajectoryTurnBuffer`] holds one slot per concurrent tool call.

use ainl_contracts::TrajectoryStep;
use std::sync::{Arc, Mutex};

/// One slot per pending tool in declaration order (see `agent_loop` + `execute_tool`).
#[derive(Debug)]
pub struct TrajectoryTurnBuffer {
    slots: Mutex<Vec<Option<TrajectoryStep>>>,
}

impl TrajectoryTurnBuffer {
    #[must_use]
    pub fn new(pending_tool_count: usize) -> Self {
        Self {
            slots: Mutex::new((0..pending_tool_count).map(|_| None).collect()),
        }
    }

    pub fn record_at(&self, slot: usize, step: TrajectoryStep) {
        if let Ok(mut g) = self.slots.lock() {
            if slot < g.len() {
                g[slot] = Some(step);
            }
        }
    }

    /// Returns ordered steps only if every slot was filled (one tool result per slot).
    pub fn take_steps_if_complete(self) -> Option<Vec<TrajectoryStep>> {
        let v = self.slots.into_inner().ok()?;
        let mut out = Vec::with_capacity(v.len());
        for s in v {
            out.push(s?);
        }
        Some(out)
    }
}

/// Append a step from a completed [`openfang_types::tool::ToolResult`].
pub fn record_trajectory_tool_step(
    capture: &Option<(Arc<TrajectoryTurnBuffer>, usize)>,
    tool_name: &str,
    tool_use_id: &str,
    started: std::time::Instant,
    tool_result: &openfang_types::tool::ToolResult,
) {
    let Some((buf, slot)) = capture else {
        return;
    };
    let duration_ms = started.elapsed().as_millis() as u64;
    let ts = chrono::Utc::now().timestamp_millis();
    let content_len = tool_result.content.chars().count();
    let tool_telemetry = serde_json::json!({
        "tool": tool_name,
        "tool_use_id": tool_use_id,
        "content_len": content_len,
        "is_error": tool_result.is_error,
        "duration_ms": duration_ms,
    });
    let frame_vars = std::env::var("AINL_TRAJECTORY_FRAME_JSON")
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok());
    let step = TrajectoryStep {
        step_id: format!("s_{slot}_{tool_use_id}"),
        timestamp_ms: ts,
        adapter: "builtin".into(),
        operation: tool_name.to_string(),
        inputs_preview: None,
        outputs_preview: None,
        duration_ms,
        success: !tool_result.is_error,
        error: if tool_result.is_error {
            Some(crate::str_utils::safe_truncate_str(&tool_result.content, 512).to_string())
        } else {
            None
        },
        vitals: None,
        freshness_at_step: None,
        frame_vars,
        tool_telemetry: Some(tool_telemetry),
    };
    buf.record_at(*slot, step);
}
