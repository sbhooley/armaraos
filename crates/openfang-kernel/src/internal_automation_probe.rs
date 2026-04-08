//! Internal automation / probe agents (dashboard: "Automation & probe chats").
//!
//! These names match `isInternalAutomationProbeChatAgentName` in the dashboard JS.
//! Tests and harnesses often use **unique suffixes** (`allowlist-probe-<uuid>`), so
//! duplicate-name cron dedupe never triggers — we **merge by family** to one agent
//! per prefix group, reassigning cron first. Leftovers with no jobs are removed.

use crate::OpenFangKernel;
use openfang_types::agent::{AgentEntry, AgentId};
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Same naming rule as `crates/openfang-api/static/js/app.js` (`isInternalAutomationProbeChatAgentName`).
pub(crate) fn is_internal_automation_probe_agent_name(name: &str) -> bool {
    let n = name.to_lowercase();
    n.starts_with("allowlist-probe")
        || n.starts_with("offline-cron")
        || n.starts_with("allow-ir-off")
}

/// Stable family key for collapsing many uniquely suffixed probe agents into one.
fn probe_family_key(name: &str) -> Option<&'static str> {
    let n = name.to_lowercase();
    if n.starts_with("allowlist-probe") {
        Some("allowlist-probe")
    } else if n.starts_with("offline-cron") {
        Some("offline-cron")
    } else if n.starts_with("allow-ir-off") {
        Some("allow-ir-off")
    } else {
        None
    }
}

fn safe_agent_dir_name(name: &str) -> bool {
    !name.is_empty() && !name.contains('/') && !name.contains('\\') && !name.contains("..")
}

/// Best-effort removal of `~/agents/<name>/` after a probe agent is purged from the DB.
fn remove_agent_workspace_best_effort(home_dir: &Path, agent_name: &str) {
    if !safe_agent_dir_name(agent_name) {
        return;
    }
    let dir = home_dir.join("agents").join(agent_name);
    if dir.is_dir() {
        if let Err(e) = std::fs::remove_dir_all(&dir) {
            tracing::debug!(
                path = %dir.display(),
                error = %e,
                "GC: could not remove agent workspace dir"
            );
        }
    }
}

/// Merge multiple internal probe agents that share the same **name prefix family**
/// into a single survivor (most cron jobs, then newest `created_at`). Cron jobs on
/// merged-away agents are reassigned to the survivor before those agents are killed.
///
/// Caps "Automation & probe chats" at **at most three** agents (one per family).
pub(crate) fn consolidate_internal_probe_agent_families(kernel: &OpenFangKernel) -> usize {
    let mut by_family: HashMap<&'static str, Vec<AgentEntry>> = HashMap::new();
    for entry in kernel.registry.list() {
        if let Some(key) = probe_family_key(&entry.name) {
            by_family.entry(key).or_default().push(entry);
        }
    }

    let mut killed = 0usize;
    for (_key, group) in by_family {
        if group.len() <= 1 {
            continue;
        }
        let survivor_id = group
            .iter()
            .max_by_key(|e| {
                let n = kernel.cron_scheduler.list_jobs(e.id).len();
                (n, e.created_at)
            })
            .map(|e| e.id)
            .expect("non-empty group");
        let survivor_name = group
            .iter()
            .find(|e| e.id == survivor_id)
            .map(|e| e.name.as_str())
            .unwrap_or("?");

        for entry in &group {
            if entry.id == survivor_id {
                continue;
            }
            let migrated = kernel
                .cron_scheduler
                .reassign_agent_jobs(entry.id, survivor_id);
            if migrated > 0 {
                tracing::info!(
                    from = %entry.name,
                    from_id = %entry.id,
                    to = %survivor_name,
                    to_id = %survivor_id,
                    migrated,
                    "Reassigned cron jobs while consolidating probe agent family"
                );
            }
            match kernel.kill_agent(entry.id) {
                Ok(()) => {
                    remove_agent_workspace_best_effort(&kernel.config.home_dir, &entry.name);
                    killed += 1;
                    tracing::info!(
                        agent = %entry.name,
                        id = %entry.id,
                        "Removed duplicate internal probe agent (family consolidated)"
                    );
                }
                Err(e) => tracing::warn!(
                    agent = %entry.name,
                    id = %entry.id,
                    error = %e,
                    "Probe family consolidation kill failed"
                ),
            }
        }
    }
    killed
}

/// Remove internal probe agents that are not referenced by any cron job.
///
/// Called once per boot after agents are restored from SQLite. Keeps agents that
/// still have at least one scheduled job targeting their id.
pub(crate) fn gc_unreferenced_internal_probe_agents(kernel: &OpenFangKernel) -> usize {
    let referenced: HashSet<AgentId> = kernel
        .cron_scheduler
        .list_all_jobs()
        .into_iter()
        .map(|j| j.agent_id)
        .collect();

    let to_kill: Vec<(AgentId, String)> = kernel
        .registry
        .list()
        .into_iter()
        .filter(|e| is_internal_automation_probe_agent_name(&e.name))
        .filter(|e| !referenced.contains(&e.id))
        .map(|e| (e.id, e.name))
        .collect();

    let mut killed = 0usize;
    for (id, name) in to_kill {
        match kernel.kill_agent(id) {
            Ok(()) => {
                remove_agent_workspace_best_effort(&kernel.config.home_dir, &name);
                killed += 1;
                tracing::info!(
                    agent = %name,
                    id = %id,
                    "Garbage-collected unreferenced internal automation/probe agent"
                );
            }
            Err(e) => {
                tracing::warn!(
                    agent = %name,
                    id = %id,
                    error = %e,
                    "GC internal probe agent failed"
                );
            }
        }
    }
    killed
}

#[cfg(test)]
mod tests {
    use super::{is_internal_automation_probe_agent_name, probe_family_key};

    #[test]
    fn probe_family_groups_unique_suffixes() {
        assert_eq!(
            probe_family_key("allowlist-probe-abc"),
            Some("allowlist-probe")
        );
        assert_eq!(probe_family_key("OFFLINE-CRON-UUID"), Some("offline-cron"));
        assert_eq!(probe_family_key("allow-ir-off-9"), Some("allow-ir-off"));
        assert_eq!(probe_family_key("assistant"), None);
    }

    #[test]
    fn probe_name_matches_dashboard_rule() {
        assert!(is_internal_automation_probe_agent_name(
            "allowlist-probe-abc"
        ));
        assert!(is_internal_automation_probe_agent_name("ALLOWLIST-PROBE-1"));
        assert!(is_internal_automation_probe_agent_name("offline-cron-uuid"));
        assert!(is_internal_automation_probe_agent_name("allow-ir-off-test"));
        assert!(!is_internal_automation_probe_agent_name("assistant"));
        assert!(!is_internal_automation_probe_agent_name(
            "allowlist-regular"
        ));
    }
}
