//! Normalize MCP `tools/list` style inventories into [`RepoIntelCapabilityProfile`](ainl_contracts::RepoIntelCapabilityProfile).

use ainl_contracts::{
    RepoIntelCapabilityProfile, RepoIntelCapabilityState, RepoIntelToolClass,
    CONTRACT_SCHEMA_VERSION,
};
use std::collections::{HashMap, HashSet};

/// Stable readiness check id for ArmaraOS / API (`readiness.checks`).
pub const CHECK_ID_REPO_INTELLIGENCE: &str = "repo_intelligence";

/// One tool row (namespaced name allowed).
#[derive(Debug, Clone)]
pub struct McpToolRow {
    pub server_name: String,
    pub tool_name: String,
    pub description: String,
}

/// Heuristic: does this tool belong to class `c`?
#[must_use]
pub fn tool_class_matches(tool_name: &str, description: &str, class: RepoIntelToolClass) -> bool {
    let n = format!("{} {}", tool_name, description).to_ascii_lowercase();
    match class {
        RepoIntelToolClass::Query => {
            n.contains("query")
                && (n.contains("search") || n.contains("hybrid") || n.contains("bm25"))
                || n.contains("gitnexus") && n.contains("query")
                || n.ends_with("query")
        }
        RepoIntelToolClass::Context => {
            n.contains("context") && n.contains("symbol")
                || n.contains("360")
                || n.contains("callee")
                || n.contains("caller")
        }
        RepoIntelToolClass::Impact => {
            n.contains("impact") || n.contains("blast") || n.contains("radius")
        }
        RepoIntelToolClass::DetectChanges => {
            n.contains("detect_changes")
                || n.contains("detectchanges")
                || (n.contains("diff") && n.contains("impact"))
        }
        RepoIntelToolClass::Cypher => n.contains("cypher"),
    }
}

/// Build a profile from discovered tools.
#[must_use]
pub fn classify_inventory(rows: &[McpToolRow]) -> RepoIntelCapabilityProfile {
    let mut classes: HashSet<RepoIntelToolClass> = HashSet::new();
    for row in rows {
        for c in [
            RepoIntelToolClass::Query,
            RepoIntelToolClass::Context,
            RepoIntelToolClass::Impact,
            RepoIntelToolClass::DetectChanges,
            RepoIntelToolClass::Cypher,
        ] {
            if tool_class_matches(&row.tool_name, &row.description, c) {
                classes.insert(c);
            }
        }
    }

    let mut class_vec: Vec<RepoIntelToolClass> = classes.into_iter().collect();
    class_vec.sort_by_key(|c| format!("{c:?}"));

    let has_impact = class_vec.contains(&RepoIntelToolClass::Impact);
    let has_query_or_ctx = class_vec.contains(&RepoIntelToolClass::Query)
        || class_vec.contains(&RepoIntelToolClass::Context);

    let state = if has_impact && has_query_or_ctx {
        RepoIntelCapabilityState::Ready
    } else if !class_vec.is_empty() {
        RepoIntelCapabilityState::Degraded
    } else {
        RepoIntelCapabilityState::Absent
    };

    let note = match state {
        RepoIntelCapabilityState::Ready => None,
        RepoIntelCapabilityState::Degraded => Some(
            "Some repo-intelligence tools detected; prefer impact+context or impact+query for full workflow."
                .into(),
        ),
        RepoIntelCapabilityState::Absent => Some(
            "No repo-intelligence MCP tools detected (query/context/impact). Install a GitNexus-class indexer or similar."
                .into(),
        ),
    };

    RepoIntelCapabilityProfile {
        schema_version: CONTRACT_SCHEMA_VERSION,
        state,
        classes_present: class_vec,
        note,
    }
}

/// Per-server summary for API payload.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ServerRepoIntelSummary {
    pub server_name: String,
    pub profile: RepoIntelCapabilityProfile,
}

#[must_use]
pub fn summarize_per_server(rows: &[McpToolRow]) -> Vec<ServerRepoIntelSummary> {
    let mut by_server: HashMap<String, Vec<McpToolRow>> = HashMap::new();
    for r in rows {
        by_server
            .entry(r.server_name.clone())
            .or_default()
            .push(r.clone());
    }
    let mut names: Vec<_> = by_server.keys().cloned().collect();
    names.sort();
    names
        .into_iter()
        .map(|name| ServerRepoIntelSummary {
            profile: classify_inventory(by_server.get(&name).map(|v| v.as_slice()).unwrap_or(&[])),
            server_name: name,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gitnex_class_like_tools_ready() {
        let rows = vec![
            McpToolRow {
                server_name: "gitnexus".into(),
                tool_name: "mcp_gitnexus_query".into(),
                description: "Hybrid search".into(),
            },
            McpToolRow {
                server_name: "gitnexus".into(),
                tool_name: "mcp_gitnexus_impact".into(),
                description: "Blast radius".into(),
            },
        ];
        let p = classify_inventory(&rows);
        assert_eq!(p.state, RepoIntelCapabilityState::Ready);
    }
}
