//! Thin adapter: map OpenFang MCP connections into [`ainl_repo_intel`] / policy contracts,
//! plus workspace-level **freshness + impact** summaries from [`ainl_context_freshness`] /
//! [`ainl_impact_policy`] (no OpenFang policy logic inline).

use crate::mcp::McpConnection;
use ainl_context_freshness::{impact_decision_balanced, impact_decision_strict, FreshnessInputs};
use ainl_contracts::{
    ContextFreshness, ImpactDecision, RecommendedNextTools, RepoIntelCapabilityProfile,
    RepoIntelCapabilityState,
};
use ainl_repo_intel::{self, McpToolRow, ServerRepoIntelSummary};
use serde::Serialize;

/// Snapshot for APIs / prompts: canonical enums + recommended chain.
#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceAinlPolicyView {
    /// Without index/git signals from the host, this stays [`ContextFreshness::Unknown`] until extended.
    pub context_freshness: ContextFreshness,
    pub repo_intelligence_ready: bool,
    pub impact_decision_strict: ImpactDecision,
    pub impact_decision_balanced: ImpactDecision,
    pub recommended_next_tools: RecommendedNextTools,
}

/// Evaluate workspace policy using portable crates (GitNexus-class readiness + default freshness).
#[must_use]
pub fn workspace_policy_view(connections: &[McpConnection]) -> WorkspaceAinlPolicyView {
    let workspace = workspace_repo_intel_profile(connections);
    let repo_intelligence_ready = matches!(workspace.state, RepoIntelCapabilityState::Ready);
    let freshness = ainl_context_freshness::evaluate_freshness(&FreshnessInputs::default());
    WorkspaceAinlPolicyView {
        context_freshness: freshness,
        repo_intelligence_ready,
        impact_decision_strict: impact_decision_strict(freshness, repo_intelligence_ready),
        impact_decision_balanced: impact_decision_balanced(freshness, repo_intelligence_ready),
        recommended_next_tools: ainl_impact_policy::golden_chain(),
    }
}

/// Short paragraph for the agent system prompt when policy calls for caution.
#[must_use]
pub fn format_workspace_policy_user_hint(connections: &[McpConnection]) -> String {
    let v = workspace_policy_view(connections);
    match v.impact_decision_strict {
        ImpactDecision::AllowExecute => String::new(),
        ImpactDecision::RequireImpactFirst => {
            if v.repo_intelligence_ready {
                "AINL policy: repo-intelligence MCP looks ready; prefer validate → compile → IR diff or external impact tools before executing `.ainl` when changes are non-trivial.".to_string()
            } else {
                "AINL policy: context freshness is uncertain and no full repo-intelligence MCP profile was detected. Prefer `ainl_validate` / `ainl_compile` and impact review before `ainl_run` on important paths.".to_string()
            }
        }
        ImpactDecision::BlockUntilFresh => {
            "AINL policy: context appears stale relative to available signals; refresh index/repo context or use repo-intelligence tools before executing.".to_string()
        }
    }
}

/// Build per-connection [`RepoIntelCapabilityProfile`] for API consumers.
#[must_use]
pub fn repo_intel_profiles_for_connections(
    connections: &[McpConnection],
) -> Vec<ServerRepoIntelSummary> {
    let rows = mcp_rows(connections);
    ainl_repo_intel::summarize_per_server(&rows)
}

/// Full-workspace aggregate profile (all tools across servers).
#[must_use]
pub fn workspace_repo_intel_profile(connections: &[McpConnection]) -> RepoIntelCapabilityProfile {
    let rows = mcp_rows(connections);
    ainl_repo_intel::classify_inventory(&rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_policy_unknown_without_intel() {
        let v = workspace_policy_view(&[]);
        assert_eq!(v.context_freshness, ContextFreshness::Unknown);
        assert!(!v.repo_intelligence_ready);
        assert!(matches!(
            v.impact_decision_strict,
            ImpactDecision::RequireImpactFirst
        ));
    }
}

fn mcp_rows(connections: &[McpConnection]) -> Vec<McpToolRow> {
    let mut rows = Vec::new();
    for c in connections {
        let server = c.name().to_string();
        for t in c.tools() {
            rows.push(McpToolRow {
                server_name: server.clone(),
                tool_name: t.name.clone(),
                description: t.description.clone(),
            });
        }
    }
    rows
}
