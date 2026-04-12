//! Shared task queue types (memory substrate + tools + API).

use serde::{Deserialize, Serialize};

/// How the memory substrate `task_claim` operation selects the next pending task.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskClaimStrategy {
    /// Sticky trace first (when `prefer_orchestration_trace_id` is set), then priority / FIFO.
    #[default]
    Default,
    /// Prefer unassigned pool tasks before tasks assigned to a specific agent.
    PreferUnassigned,
    /// Only claim tasks matching the sticky trace filter; do not fall back to the general queue.
    /// If no trace id is preferred, returns no task.
    StickyOnly,
}
