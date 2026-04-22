//! Opt-in `project_id` on graph memory rows for multi-workspace isolation (see
//! `docs/SELF_LEARNING_INTEGRATION_MAP.md` §1/§2).
//!
//! When `AINL_MEMORY_PROJECT_SCOPE` is truthy, OpenFang tags new [`ainl_memory::AinlMemoryNode`]
//! rows with `metadata.project_id` from the agent manifest (when set). Reads can filter on this
//! column (see `ainl_memory::GraphMemory::search_all_nodes_fts`).

use openfang_types::agent::AgentManifest;

/// When set (`1` / `true` / `yes` / `on`), new graph nodes may carry `project_id` from the manifest.
#[must_use]
pub fn env_ainl_memory_project_scope() -> bool {
    std::env::var("AINL_MEMORY_PROJECT_SCOPE")
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

/// Resolved project key for memory writes this turn, or `None` (unscoped) when the env is off
/// or `metadata.project_id` is missing/empty.
#[must_use]
pub fn effective_memory_project_id(manifest: &AgentManifest) -> Option<String> {
    if !env_ainl_memory_project_scope() {
        return None;
    }
    manifest
        .metadata
        .get("project_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// When a manifest is not in scope, match trajectory’s `AINL_MEMORY_PROJECT_ID` convention:
/// if `AINL_MEMORY_PROJECT_SCOPE` is on, use that env (trimmed, non-empty) as the project key.
#[must_use]
pub fn memory_project_id_from_process_env() -> Option<String> {
    if !env_ainl_memory_project_scope() {
        return None;
    }
    std::env::var("AINL_MEMORY_PROJECT_ID")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[inline]
pub(crate) fn apply_memory_project_id_to_node(
    node: &mut ainl_memory::AinlMemoryNode,
    memory_project_id: Option<&str>,
) {
    if let Some(s) = memory_project_id.map(str::trim).filter(|s| !s.is_empty()) {
        node.project_id = Some(s.to_string());
    }
}
