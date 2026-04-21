//! Cross-runtime contracts for GitNexus-style repo intelligence and impact-first AINL flows.
//! Crate has **no** `openfang_*` dependencies so it can ship stand-alone or with AI_Native_Lang tooling.

use serde::{Deserialize, Serialize};

/// Telemetry / metrics field names — keep identical across ArmaraOS, AINL MCP, and optional inference-server.
pub mod telemetry {
    /// Label: normalized repo-intel capability state (e.g. `ready`, `degraded`, `absent`).
    pub const CAPABILITY_PROFILE_STATE: &str = "capability_profile_state";
    /// Label: context freshness at decision time.
    pub const FRESHNESS_STATE_AT_DECISION: &str = "freshness_state_at_decision";
    /// Counter/gauge: whether impact was assessed before a risky write.
    pub const IMPACT_CHECKED_BEFORE_WRITE: &str = "impact_checked_before_write";
}

/// Version for JSON serialization of policy contract payloads (bump on breaking enum changes).
pub const CONTRACT_SCHEMA_VERSION: u32 = 1;

/// Class of repo-intelligence MCP tool (GitNexus-class naming).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepoIntelToolClass {
    Query,
    Context,
    Impact,
    DetectChanges,
    /// Optional graph query surface (e.g. Cypher).
    Cypher,
}

/// Aggregate readiness for repo-intelligence MCP capabilities.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepoIntelCapabilityState {
    /// At least impact + (query or context) detected across tools.
    Ready,
    /// Some classes present but not enough for full blast-radius workflow.
    Degraded,
    /// No repo-intelligence tools detected.
    Absent,
}

/// Profile returned by [`ainl_repo_intel`](crate) normalization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoIntelCapabilityProfile {
    pub schema_version: u32,
    pub state: RepoIntelCapabilityState,
    /// Which [`RepoIntelToolClass`] values had at least one matching tool.
    pub classes_present: Vec<RepoIntelToolClass>,
    /// Human-readable note (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Freshness of code/repo context for safe edits (independent of inference-server).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextFreshness {
    /// Index/context is in sync or confidently current.
    Fresh,
    /// Known stale (e.g. index behind HEAD).
    Stale,
    /// Cannot determine — treat conservatively in strict modes.
    Unknown,
}

/// Decision gate for executing versus gathering more context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImpactDecision {
    /// Safe to proceed with compile/run per policy.
    AllowExecute,
    /// Prefer impact/diff/context tools first.
    RequireImpactFirst,
    /// Block run until context is refreshed or user confirms.
    BlockUntilFresh,
}

/// One recommended tool step in the impact-first chain (AINL MCP names or logical ids).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecommendedToolStep {
    pub tool: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Ordered recommendation list (validate → compile → impact/diff → run).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecommendedNextTools {
    pub schema_version: u32,
    pub steps: Vec<RecommendedToolStep>,
}

impl Default for RecommendedNextTools {
    fn default() -> Self {
        Self {
            schema_version: CONTRACT_SCHEMA_VERSION,
            steps: Vec::new(),
        }
    }
}

impl RecommendedNextTools {
    pub fn golden_default_chain() -> Self {
        Self {
            schema_version: CONTRACT_SCHEMA_VERSION,
            steps: vec![
                RecommendedToolStep {
                    tool: "ainl_validate".into(),
                    reason: Some("Strict check after edits".into()),
                },
                RecommendedToolStep {
                    tool: "ainl_compile".into(),
                    reason: Some("IR before diff/impact".into()),
                },
                RecommendedToolStep {
                    tool: "ainl_ir_diff".into(),
                    reason: Some("Blast radius vs prior IR when available".into()),
                },
                RecommendedToolStep {
                    tool: "ainl_run".into(),
                    reason: Some("Execute only after validation + impact awareness".into()),
                },
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn contract_json_roundtrip() {
        let p = RepoIntelCapabilityProfile {
            schema_version: CONTRACT_SCHEMA_VERSION,
            state: RepoIntelCapabilityState::Ready,
            classes_present: vec![RepoIntelToolClass::Impact, RepoIntelToolClass::Query],
            note: None,
        };
        let j = serde_json::to_value(&p).unwrap();
        let back: RepoIntelCapabilityProfile = serde_json::from_value(j).unwrap();
        assert_eq!(p.state, back.state);
    }

    #[test]
    fn golden_fixture_file_matches_recommended_chain() {
        let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        p.push("tests/fixtures/contract_v1.json");
        let raw = std::fs::read_to_string(&p).expect("fixture");
        let v: serde_json::Value = serde_json::from_str(&raw).expect("json");
        let steps = v["RecommendedNextTools"]["steps"].as_array().expect("steps");
        let tools: Vec<String> = steps
            .iter()
            .filter_map(|s| s.get("tool").and_then(|t| t.as_str().map(String::from)))
            .collect();
        assert_eq!(
            tools,
            vec![
                "ainl_validate",
                "ainl_compile",
                "ainl_ir_diff",
                "ainl_run"
            ]
        );
    }
}
