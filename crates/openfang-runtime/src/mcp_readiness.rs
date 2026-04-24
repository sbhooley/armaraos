//! MCP / tool **readiness** evaluation — shared by API, doctor, UI, and agent loop.
//!
//! Calendar heuristics remain local; **repo intelligence** classification is delegated to
//! [`ainl_repo_intel`] so ArmaraOS stays aligned with `ainl-contracts`.

use crate::mcp::McpConnection;
use ainl_contracts::{ContextFreshness, RepoIntelCapabilityState};
use ainl_repo_intel::{self, McpToolRow};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// JSON `readiness.version` — bump when the check map shape changes incompatibly.
pub const READINESS_SCHEMA_VERSION: u32 = 1;

/// Stable id for the calendar/events integration check.
pub const CHECK_ID_CALENDAR: &str = "calendar";

/// Stable id for GitNexus-class repo intelligence tools (see `ainl-repo-intel`).
pub const CHECK_ID_REPO_INTELLIGENCE: &str = ainl_repo_intel::CHECK_ID_REPO_INTELLIGENCE;

/// One discovered MCP tool (namespaced name + description as seen by the runtime).
#[derive(Debug, Clone)]
pub struct McpToolSnapshot {
    pub server_name: String,
    pub tool_name: String,
    pub description: String,
}

/// Provider hints for the calendar check (server *name* heuristics, not tool-level).
#[derive(Debug, Clone, Serialize)]
pub struct CalendarProviderHints {
    pub google_like_server_connected: bool,
    pub apple_like_server_connected: bool,
    pub caldav_like_server_connected: bool,
}

/// Result for a single readiness check (serialized to API / doctor).
#[derive(Debug, Clone, Serialize)]
pub struct ReadinessCheckResult {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub ready: bool,
    pub severity: &'static str,
    /// Human-readable default when `ready` is false (string or null in JSON).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub missing_reason: Option<String>,
    pub matched_servers: Vec<String>,
    pub matched_tools: Vec<String>,
    pub provider_hints: CalendarProviderHints,
    pub remediation: String,
}

/// Full readiness report (`readiness` object in `GET /api/mcp/servers`).
#[derive(Debug, Clone, Serialize)]
pub struct ReadinessReport {
    pub version: u32,
    pub checks: BTreeMap<String, ReadinessCheckResult>,
}

/// Legacy `calendar_readiness` payload (field names preserved for one release cycle).
#[derive(Debug, Clone, Serialize)]
pub struct CalendarReadinessAlias {
    pub ready: bool,
    pub connected_servers_with_calendar_tools: Vec<String>,
    pub calendar_tool_names: Vec<String>,
    pub provider_hints: CalendarProviderHints,
    pub missing_reason: serde_json::Value,
}

#[derive(Debug, Clone, Default)]
pub struct ToolReadinessFlags {
    /// Check ids that matched this tool (e.g. `"calendar"`).
    pub check_ids: BTreeSet<String>,
}

/// Evaluate readiness + per-tool flags from live MCP connections.
#[must_use]
pub fn evaluate_from_connections(connections: &[McpConnection]) -> EvaluatedMcpReadiness {
    let snapshots = collect_snapshots(connections);
    evaluate_from_snapshots(&snapshots)
}

fn collect_snapshots(connections: &[McpConnection]) -> Vec<McpToolSnapshot> {
    let mut out = Vec::new();
    for c in connections {
        let server = c.name().to_string();
        for t in c.tools() {
            out.push(McpToolSnapshot {
                server_name: server.clone(),
                tool_name: t.name.clone(),
                description: t.description.clone(),
            });
        }
    }
    out
}

/// Evaluate from tool rows only (tests + API when connections are already flattened).
#[must_use]
pub fn evaluate_from_snapshots(snapshots: &[McpToolSnapshot]) -> EvaluatedMcpReadiness {
    let mut tool_flags: Vec<(McpToolSnapshot, ToolReadinessFlags)> = snapshots
        .iter()
        .cloned()
        .map(|s| (s, ToolReadinessFlags::default()))
        .collect();

    let mut report = ReadinessReport {
        version: READINESS_SCHEMA_VERSION,
        checks: BTreeMap::new(),
    };

    // --- calendar check ---
    let cal = evaluate_calendar_check(snapshots, &mut tool_flags);
    report
        .checks
        .insert(CHECK_ID_CALENDAR.to_string(), cal.result.clone());

    let ri = evaluate_repo_intelligence_check(snapshots, &mut tool_flags);
    report
        .checks
        .insert(CHECK_ID_REPO_INTELLIGENCE.to_string(), ri.result);

    let calendar_alias = CalendarReadinessAlias {
        ready: cal.result.ready,
        connected_servers_with_calendar_tools: cal.result.matched_servers.clone(),
        calendar_tool_names: cal.result.matched_tools.clone(),
        provider_hints: cal.result.provider_hints.clone(),
        missing_reason: if cal.result.ready {
            serde_json::Value::Null
        } else {
            serde_json::json!(cal.default_missing_reason)
        },
    };

    EvaluatedMcpReadiness {
        report,
        calendar_readiness: calendar_alias,
        tool_flags,
    }
}

struct CalendarEval {
    result: ReadinessCheckResult,
    default_missing_reason: &'static str,
}

fn evaluate_calendar_check(
    snapshots: &[McpToolSnapshot],
    tool_flags: &mut [(McpToolSnapshot, ToolReadinessFlags)],
) -> CalendarEval {
    let mut matched_servers: BTreeSet<String> = BTreeSet::new();
    let mut matched_tools: BTreeSet<String> = BTreeSet::new();
    let mut has_google_like = false;
    let mut has_apple_like = false;
    let mut has_caldav_like = false;

    for (snap, _flags) in tool_flags.iter_mut() {
        let (g, a, c) = server_provider_hints(&snap.server_name);
        has_google_like |= g;
        has_apple_like |= a;
        has_caldav_like |= c;
    }

    for snap in snapshots {
        if tool_matches_calendar(snap) {
            matched_servers.insert(snap.server_name.clone());
            matched_tools.insert(snap.tool_name.clone());
        }
    }

    for (snap, flags) in tool_flags.iter_mut() {
        if tool_matches_calendar(snap) {
            flags.check_ids.insert(CHECK_ID_CALENDAR.to_string());
        }
    }

    let mut ms: Vec<String> = matched_servers.into_iter().collect();
    ms.sort();
    let mut mt: Vec<String> = matched_tools.into_iter().collect();
    mt.sort();

    let ready = !mt.is_empty();
    let provider_hints = CalendarProviderHints {
        google_like_server_connected: has_google_like,
        apple_like_server_connected: has_apple_like,
        caldav_like_server_connected: has_caldav_like,
    };

    let default_missing = "No connected MCP server exposed calendar/event tools. Configure Google Calendar, Apple/CalDAV, or another calendar MCP integration.";
    let missing_reason = if ready {
        None
    } else {
        Some(default_missing.to_string())
    };

    let remediation = if ready {
        String::new()
    } else if provider_hints.google_like_server_connected
        || provider_hints.apple_like_server_connected
        || provider_hints.caldav_like_server_connected
    {
        "A calendar-related MCP server appears connected, but no calendar/event tools were detected. Verify the server exposes tools/list entries for calendar or event operations.".to_string()
    } else {
        "Add a calendar MCP server (Google Calendar or Apple/CalDAV) in ~/.armaraos/config.toml, then reload integrations or restart the daemon. See docs/mcp-a2a.md (Calendar MCP).".to_string()
    };

    CalendarEval {
        result: ReadinessCheckResult {
            id: CHECK_ID_CALENDAR.to_string(),
            label: Some("Calendar".to_string()),
            ready,
            severity: if ready { "ok" } else { "warn" },
            missing_reason,
            matched_servers: ms.clone(),
            matched_tools: mt.clone(),
            provider_hints,
            remediation,
        },
        default_missing_reason: default_missing,
    }
}

struct RepoIntelEval {
    result: ReadinessCheckResult,
}

fn evaluate_repo_intelligence_check(
    snapshots: &[McpToolSnapshot],
    tool_flags: &mut [(McpToolSnapshot, ToolReadinessFlags)],
) -> RepoIntelEval {
    let rows: Vec<McpToolRow> = snapshots
        .iter()
        .map(|s| McpToolRow {
            server_name: s.server_name.clone(),
            tool_name: s.tool_name.clone(),
            description: s.description.clone(),
        })
        .collect();
    let profile = ainl_repo_intel::classify_inventory(&rows);

    let mut matched_servers: BTreeSet<String> = BTreeSet::new();
    let mut matched_tools: BTreeSet<String> = BTreeSet::new();
    for snap in snapshots {
        if tool_matches_repo_intelligence(snap) {
            matched_servers.insert(snap.server_name.clone());
            matched_tools.insert(snap.tool_name.clone());
        }
    }
    for (snap, flags) in tool_flags.iter_mut() {
        if tool_matches_repo_intelligence(snap) {
            flags
                .check_ids
                .insert(CHECK_ID_REPO_INTELLIGENCE.to_string());
        }
    }

    let mut ms: Vec<String> = matched_servers.into_iter().collect();
    ms.sort();
    let mut mt: Vec<String> = matched_tools.into_iter().collect();
    mt.sort();

    let ready = matches!(profile.state, RepoIntelCapabilityState::Ready);
    let severity = if ready {
        "ok"
    } else {
        match profile.state {
            RepoIntelCapabilityState::Degraded => "warn",
            RepoIntelCapabilityState::Absent => "warn",
            RepoIntelCapabilityState::Ready => "ok",
        }
    };

    let default_missing = "No repo-intelligence MCP tools detected (hybrid query, symbol context, impact/blast radius). Connect a GitNexus-class MCP server or index your repo locally.";
    let missing_reason = if ready {
        None
    } else {
        Some(
            profile
                .note
                .clone()
                .unwrap_or_else(|| default_missing.to_string()),
        )
    };

    let remediation = if ready {
        String::new()
    } else {
        "Add an MCP server that exposes codebase analysis tools (e.g. `npx -y gitnexus@latest mcp`), run `gitnexus analyze` in the repo, then reload MCP integrations. See docs/mcp-a2a.md (Repo intelligence MCP).".to_string()
    };

    RepoIntelEval {
        result: ReadinessCheckResult {
            id: CHECK_ID_REPO_INTELLIGENCE.to_string(),
            label: Some("Repo intelligence".to_string()),
            ready,
            severity,
            missing_reason,
            matched_servers: ms,
            matched_tools: mt,
            provider_hints: CalendarProviderHints {
                google_like_server_connected: false,
                apple_like_server_connected: false,
                caldav_like_server_connected: false,
            },
            remediation,
        },
    }
}

#[inline]
fn tool_matches_repo_intelligence(s: &McpToolSnapshot) -> bool {
    use ainl_contracts::RepoIntelToolClass;
    [
        RepoIntelToolClass::Query,
        RepoIntelToolClass::Context,
        RepoIntelToolClass::Impact,
        RepoIntelToolClass::DetectChanges,
        RepoIntelToolClass::Cypher,
    ]
    .into_iter()
    .any(|c| ainl_repo_intel::tool_class_matches(&s.tool_name, &s.description, c))
}

/// Output of [`evaluate_from_connections`] / [`evaluate_from_snapshots`].
pub struct EvaluatedMcpReadiness {
    pub report: ReadinessReport,
    pub calendar_readiness: CalendarReadinessAlias,
    /// Parallel to connection iteration order in the API (re-built there); here used for tests.
    pub tool_flags: Vec<(McpToolSnapshot, ToolReadinessFlags)>,
}

/// Flags for a single tool row without scanning all connections (API tool list).
#[must_use]
pub fn flags_for_tool(server_name: &str, tool_name: &str, description: &str) -> ToolReadinessFlags {
    let snap = McpToolSnapshot {
        server_name: server_name.to_string(),
        tool_name: tool_name.to_string(),
        description: description.to_string(),
    };
    let mut f = ToolReadinessFlags::default();
    if tool_matches_calendar(&snap) {
        f.check_ids.insert(CHECK_ID_CALENDAR.to_string());
    }
    if tool_matches_repo_intelligence(&snap) {
        f.check_ids.insert(CHECK_ID_REPO_INTELLIGENCE.to_string());
    }
    f
}

#[inline]
fn tool_matches_calendar(s: &McpToolSnapshot) -> bool {
    let n = s.tool_name.to_ascii_lowercase();
    let d = s.description.to_ascii_lowercase();
    n.contains("calendar")
        || n.contains("event")
        || n.contains("schedule")
        || n.contains("caldav")
        || n.contains("ical")
        || d.contains("calendar")
        || d.contains("event")
        || d.contains("schedule")
        || d.contains("caldav")
        || d.contains("ical")
}

#[inline]
pub fn server_provider_hints(name: &str) -> (bool, bool, bool) {
    let n = name.to_ascii_lowercase();
    let google = n.contains("google") || n.contains("gcal") || n.contains("gmail");
    let apple = n.contains("apple") || n.contains("icloud") || n.contains("macos");
    let caldav = n.contains("caldav") || n.contains("ical");
    (google, apple, caldav)
}

/// Bounded appendix for system prompts (deterministic ordering by check id).
#[must_use]
pub fn format_prompt_appendix(report: &ReadinessReport, max_chars: usize) -> String {
    if report.checks.is_empty() {
        return String::new();
    }
    let mut lines: Vec<String> = Vec::new();
    lines.push("## MCP tool readiness (host snapshot)".to_string());
    lines.push(format!(
        "Schema v{} — use MCP tools only when the relevant check is ready; do not assume OS-level calendar APIs.",
        report.version
    ));
    for (id, c) in &report.checks {
        let status = if c.ready { "ready" } else { "not_ready" };
        let mut line = format!("- {id}: {status}");
        if !c.ready {
            if let Some(reason) = &c.missing_reason {
                let short: String = reason.chars().take(160).collect();
                line.push_str(&format!(" — {short}"));
            }
        }
        lines.push(line);
    }
    let mut out = lines.join("\n");
    if out.len() > max_chars {
        out.truncate(max_chars);
        out.push_str("\n…");
    }
    out
}

/// Telemetry hints for [`armara_provider_api::InferRequest::planner_context_hints`], aligned with
/// `tooling/ainl_policy_contract.json` / `ainl-contracts`.
#[must_use]
pub fn planner_context_hints_from_connections(
    connections: &[McpConnection],
) -> armara_provider_api::PlannerContextHints {
    let view = crate::ainl_policy::workspace_policy_view(connections);
    let cf = match view.context_freshness {
        ContextFreshness::Fresh => "fresh",
        ContextFreshness::Stale => "stale",
        ContextFreshness::Unknown => "unknown",
    };
    armara_provider_api::PlannerContextHints {
        repo_intelligence_mcp_ready: Some(view.repo_intelligence_ready),
        context_freshness: Some(cf.to_string()),
        freshness_known_at_plan_time: Some(false),
        impact_considered_before_execute: None,
    }
}

/// Digest for deduplicating graph-memory facts / kernel memory (stable JSON).
#[must_use]
pub fn readiness_digest_json(report: &ReadinessReport) -> serde_json::Value {
    serde_json::json!({
        "v": report.version,
        "checks": report.checks.keys().collect::<Vec<_>>(),
        "ready": report.checks.iter().map(|(k,v)| (k.clone(), serde_json::json!(v.ready))).collect::<serde_json::Map<String, serde_json::Value>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_matches_tool_name() {
        let s = McpToolSnapshot {
            server_name: "foo".into(),
            tool_name: "mcp_foo_list_calendars".into(),
            description: "[MCP:foo] x".into(),
        };
        assert!(tool_matches_calendar(&s));
    }

    #[test]
    fn evaluate_empty_not_ready() {
        let ev = evaluate_from_snapshots(&[]);
        assert!(!ev.report.checks[CHECK_ID_CALENDAR].ready);
        assert!(!ev.calendar_readiness.missing_reason.is_null());
    }

    #[test]
    fn repo_intelligence_ready_with_query_and_impact() {
        let snaps = vec![
            McpToolSnapshot {
                server_name: "gitnexus".into(),
                tool_name: "mcp_gitnexus_query".into(),
                description: "hybrid bm25 search".into(),
            },
            McpToolSnapshot {
                server_name: "gitnexus".into(),
                tool_name: "mcp_gitnexus_impact".into(),
                description: "blast radius".into(),
            },
        ];
        let ev = evaluate_from_snapshots(&snaps);
        let ri = ev
            .report
            .checks
            .get(CHECK_ID_REPO_INTELLIGENCE)
            .expect("ri check");
        assert!(ri.ready);
        assert_eq!(ri.id, CHECK_ID_REPO_INTELLIGENCE);
    }

    #[test]
    fn planner_context_hints_match_workspace_policy() {
        let hints = planner_context_hints_from_connections(&[]);
        let view = crate::ainl_policy::workspace_policy_view(&[]);
        assert_eq!(
            hints.repo_intelligence_mcp_ready,
            Some(view.repo_intelligence_ready)
        );
        assert_eq!(hints.context_freshness.as_deref(), Some("unknown"));
        assert_eq!(hints.freshness_known_at_plan_time, Some(false));
    }
}
