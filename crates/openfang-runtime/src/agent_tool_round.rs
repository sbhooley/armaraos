//! Shared policy for one LLM-declared tool round.
//!
//! The heavy execution plumbing still lives in `agent_loop` because it owns many borrowed runtime
//! handles. This module centralizes the parts that must stay identical between streaming and
//! non-streaming paths: lane classification, side-effect-safe deduplication, and ordered batching.

/// Execution lane for a tool call within one assistant `tool_use` round.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolExecutionLane {
    /// Read-only calls can run concurrently with adjacent read-only calls.
    ReadParallel,
    /// Workspace/file mutations must stay in LLM-declared order.
    WorkspaceMutationSerial,
    /// Shell/process lifecycle tools must stay in order to avoid races.
    ProcessSerial,
    /// External side effects (messages, posts, AINL runs, delegated agents) must stay in order.
    ExternalSideEffectSerial,
}

impl ToolExecutionLane {
    #[must_use]
    pub fn is_parallel_read(self) -> bool {
        matches!(self, Self::ReadParallel)
    }
}

pub struct ToolRoundExecutor;

impl ToolRoundExecutor {
    #[must_use]
    pub fn classify_lane(tool_name: &str) -> ToolExecutionLane {
        match tool_name {
            "file_read"
            | "file_list"
            | "document_extract"
            | "mcp_resource_read"
            | "web_search"
            | "web_fetch"
            | "web_scrape"
            | "web_get"
            | "mcp_ainl_ainl_validate"
            | "mcp_ainl_ainl_compile"
            | "mcp_ainl_ainl_capabilities"
            | "mcp_ainl_ainl_security_report"
            | "mcp_ainl_ainl_ir_diff"
            | "mcp_ainl_ainl_ptc_signature_check" => ToolExecutionLane::ReadParallel,
            "file_write" | "apply_patch" | "workspace_action" | "script_run" => {
                ToolExecutionLane::WorkspaceMutationSerial
            }
            "shell_exec" | "process_start" | "process_kill" | "process_list" => {
                ToolExecutionLane::ProcessSerial
            }
            _ => {
                if tool_name.starts_with("channel_")
                    || tool_name.starts_with("agent_")
                    || tool_name == "mcp_ainl_ainl_run"
                    || tool_name.contains("send")
                    || tool_name.contains("post")
                    || tool_name.contains("publish")
                {
                    ToolExecutionLane::ExternalSideEffectSerial
                } else {
                    ToolExecutionLane::ReadParallel
                }
            }
        }
    }

    #[must_use]
    pub fn safe_to_deduplicate(tool_name: &str) -> bool {
        Self::classify_lane(tool_name).is_parallel_read()
    }

    /// Build ordered batches. Consecutive read-only calls share one batch; every side-effecting
    /// call is isolated in its own batch. Batch order always matches the original tool order.
    #[must_use]
    pub fn ordered_batches<'a, T, F>(items: &'a [T], mut lane_of: F) -> Vec<std::ops::Range<usize>>
    where
        F: FnMut(&'a T) -> ToolExecutionLane,
    {
        let mut batches = Vec::new();
        let mut idx = 0;
        while idx < items.len() {
            let lane = lane_of(&items[idx]);
            if lane.is_parallel_read() {
                let start = idx;
                idx += 1;
                while idx < items.len() && lane_of(&items[idx]).is_parallel_read() {
                    idx += 1;
                }
                batches.push(start..idx);
            } else {
                batches.push(idx..idx + 1);
                idx += 1;
            }
        }
        batches
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_mutations_and_processes_as_serial() {
        assert_eq!(
            ToolRoundExecutor::classify_lane("file_write"),
            ToolExecutionLane::WorkspaceMutationSerial
        );
        assert_eq!(
            ToolRoundExecutor::classify_lane("shell_exec"),
            ToolExecutionLane::ProcessSerial
        );
        assert_eq!(
            ToolRoundExecutor::classify_lane("channel_send"),
            ToolExecutionLane::ExternalSideEffectSerial
        );
    }

    #[test]
    fn batches_adjacent_reads_but_isolates_side_effects() {
        let tools = vec![
            "file_read",
            "mcp_ainl_ainl_validate",
            "file_write",
            "file_read",
            "shell_exec",
            "web_fetch",
            "web_search",
        ];
        let batches = ToolRoundExecutor::ordered_batches(&tools, |name| {
            ToolRoundExecutor::classify_lane(name)
        });
        assert_eq!(batches, vec![0..2, 2..3, 3..4, 4..5, 5..7]);
    }

    #[test]
    fn dedup_policy_only_allows_read_parallel_tools() {
        assert!(ToolRoundExecutor::safe_to_deduplicate("file_read"));
        assert!(ToolRoundExecutor::safe_to_deduplicate(
            "mcp_ainl_ainl_validate"
        ));
        assert!(!ToolRoundExecutor::safe_to_deduplicate("file_write"));
        assert!(!ToolRoundExecutor::safe_to_deduplicate("mcp_ainl_ainl_run"));
    }
}
