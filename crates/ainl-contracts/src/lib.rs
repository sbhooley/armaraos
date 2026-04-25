//! Cross-runtime contracts for GitNexus-style repo intelligence and impact-first AINL flows.
//! Crate has **no** `openfang_*` dependencies so it can ship stand-alone or with AI_Native_Lang tooling.

use serde::{Deserialize, Serialize};

pub mod learner;
pub mod procedure;
pub mod vitals;

pub use learner::{FailureKind, ProposalEnvelope, TrajectoryOutcome, TrajectoryStep};
pub use procedure::{
    ExperienceBundle, ExperienceEvent, ProcedureArtifact, ProcedureArtifactFormat,
    ProcedureExecutionPlan, ProcedureExecutionStep, ProcedureLifecycle, ProcedurePatch,
    ProcedureReuseOutcome, ProcedureStep, ProcedureStepKind, ProcedureVerification,
};
pub use vitals::{CognitivePhase, CognitiveVitals, VitalsGate};

/// Telemetry / metrics field names — keep identical across ArmaraOS, AINL MCP, and optional inference-server.
pub mod telemetry {
    /// Label: normalized repo-intel capability state (e.g. `ready`, `degraded`, `absent`).
    pub const CAPABILITY_PROFILE_STATE: &str = "capability_profile_state";
    /// Label: context freshness at decision time.
    pub const FRESHNESS_STATE_AT_DECISION: &str = "freshness_state_at_decision";
    /// Counter/gauge: whether impact was assessed before a risky write.
    pub const IMPACT_CHECKED_BEFORE_WRITE: &str = "impact_checked_before_write";
    /// Trajectory + failure + proposal + compression (learner suite).
    pub const TRAJECTORY_RECORDED: &str = "trajectory_recorded";
    pub const TRAJECTORY_OUTCOME: &str = "trajectory_outcome";
    pub const TRAJECTORY_STEP_DURATION_MS: &str = "trajectory_step_duration_ms";
    pub const FAILURE_RECORDED: &str = "failure_recorded";
    pub const FAILURE_RESOLUTION_HIT: &str = "failure_resolution_hit";
    pub const FAILURE_PREVENTED_COUNT: &str = "failure_prevented_count";
    pub const PROPOSAL_VALIDATED: &str = "proposal_validated";
    pub const PROPOSAL_ADOPTED: &str = "proposal_adopted";
    pub const PROCEDURE_MINTED: &str = "procedure_minted";
    pub const PROCEDURE_REUSED: &str = "procedure_reused";
    pub const PROCEDURE_PATCH_PROPOSED: &str = "procedure_patch_proposed";
    pub const PROCEDURE_PATCH_ADOPTED: &str = "procedure_patch_adopted";
    pub const COMPRESSION_PROFILE_TUNED: &str = "compression_profile_tuned";
    pub const COMPRESSION_CACHE_HIT: &str = "compression_cache_hit";
    pub const PERSONA_AXIS_DELTA: &str = "persona_axis_delta";
    pub const VITALS_GATE_AT_TURN: &str = "vitals_gate_at_turn";
    /// Context-compiler suite (`ainl-context-compiler`, Phase 6 of SELF_LEARNING_INTEGRATION_MAP).
    /// Histogram/counter: a single `compose()` call summary.
    pub const CONTEXT_COMPILER_COMPOSE: &str = "context_compiler_compose";
    /// Counter: tier upgraded mid-session (e.g. heuristic → heuristic_summarization).
    pub const CONTEXT_COMPILER_TIER_UPGRADED: &str = "context_compiler_tier_upgraded";
    /// Counter: summarizer call failed and the orchestrator auto-degraded for that turn.
    pub const CONTEXT_COMPILER_SUMMARIZER_FAILED: &str = "context_compiler_summarizer_failed";
    /// Counter: budget exceeded after best-effort compaction (safety-net truncation applied).
    pub const CONTEXT_COMPILER_BUDGET_EXCEEDED: &str = "context_compiler_budget_exceeded";
    /// Counter: a single segment was emitted into the composed prompt.
    pub const CONTEXT_COMPILER_BLOCK_EMITTED: &str = "context_compiler_block_emitted";
}

/// Context-compiler shared vocabulary (Phase 6 of SELF_LEARNING_INTEGRATION_MAP §15.1).
///
/// Lets other AINL hosts read context-compiler telemetry without taking a hard dependency on the
/// `ainl-context-compiler` crate itself. The strings here intentionally mirror the variant names
/// in `ainl_context_compiler::{SegmentKind, Tier}`.
pub mod context_compiler {
    /// Stable lowercase labels for `SegmentKind` (mirrors the crate enum).
    pub mod segment_kind {
        pub const SYSTEM_PROMPT: &str = "system_prompt";
        pub const OLDER_TURN: &str = "older_turn";
        pub const RECENT_TURN: &str = "recent_turn";
        pub const TOOL_DEFINITIONS: &str = "tool_definitions";
        pub const TOOL_RESULT: &str = "tool_result";
        pub const USER_PROMPT: &str = "user_prompt";
        pub const ANCHORED_SUMMARY_RECALL: &str = "anchored_summary_recall";
        pub const MEMORY_BLOCK: &str = "memory_block";
    }

    /// Stable lowercase labels for `Tier`.
    pub mod tier {
        pub const HEURISTIC: &str = "heuristic";
        pub const HEURISTIC_SUMMARIZATION: &str = "heuristic_summarization";
        pub const HEURISTIC_SUMMARIZATION_EMBEDDING: &str = "heuristic_summarization_embedding";
    }
}

/// Version for JSON serialization of policy contract payloads (bump on breaking enum changes).
pub const CONTRACT_SCHEMA_VERSION: u32 = 1;

/// Schema version for [`ProposalEnvelope`] and other learner wire types.
pub const LEARNER_SCHEMA_VERSION: u32 = 1;

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
    fn cognitive_vitals_json_roundtrip() {
        let v = CognitiveVitals {
            gate: VitalsGate::Pass,
            phase: "reasoning:0.71".into(),
            trust: 0.82,
            mean_logprob: -0.4,
            entropy: 0.12,
            sample_tokens: 12,
        };
        let j = serde_json::to_value(&v).unwrap();
        let back: CognitiveVitals = serde_json::from_value(j).unwrap();
        assert_eq!(v, back);
    }

    #[test]
    fn trajectory_step_json_roundtrip() {
        let s = TrajectoryStep {
            step_id: "s1".into(),
            timestamp_ms: 1,
            adapter: "http".into(),
            operation: "GET".into(),
            inputs_preview: None,
            outputs_preview: None,
            duration_ms: 3,
            success: true,
            error: None,
            vitals: None,
            freshness_at_step: Some(ContextFreshness::Fresh),
            frame_vars: None,
            tool_telemetry: None,
        };
        let j = serde_json::to_value(&s).unwrap();
        let back: TrajectoryStep = serde_json::from_value(j).unwrap();
        assert_eq!(s, back);
    }

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
        let steps = v["RecommendedNextTools"]["steps"]
            .as_array()
            .expect("steps");
        let tools: Vec<String> = steps
            .iter()
            .filter_map(|s| s.get("tool").and_then(|t| t.as_str().map(String::from)))
            .collect();
        assert_eq!(
            tools,
            vec!["ainl_validate", "ainl_compile", "ainl_ir_diff", "ainl_run"]
        );
    }

    #[test]
    fn cognitive_phase_json_roundtrip() {
        let p = CognitivePhase::Retrieval;
        let j = serde_json::to_value(p).unwrap();
        let back: CognitivePhase = serde_json::from_value(j).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn trajectory_outcome_json_roundtrip() {
        for o in [
            TrajectoryOutcome::Success,
            TrajectoryOutcome::PartialSuccess,
            TrajectoryOutcome::Failure,
            TrajectoryOutcome::Aborted,
        ] {
            let j = serde_json::to_value(o).unwrap();
            let back: TrajectoryOutcome = serde_json::from_value(j).unwrap();
            assert_eq!(o, back);
        }
    }

    #[test]
    fn failure_kind_json_roundtrip_variants() {
        let cases = vec![
            FailureKind::AdapterTypo {
                offered: "httP".into(),
                suggestion: Some("http".into()),
            },
            FailureKind::ValidatorReject {
                rule: "no_raw_shell".into(),
            },
            FailureKind::AdapterTimeout {
                adapter: "web".into(),
                ms: 5000,
            },
            FailureKind::ToolError {
                tool: "file_read".into(),
                message: "ENOENT".into(),
            },
            FailureKind::LoopGuardFire {
                tool: "noop".into(),
                repeat_count: 3,
            },
            FailureKind::Other {
                message: "misc".into(),
            },
        ];
        for fk in cases {
            let j = serde_json::to_value(&fk).unwrap();
            let back: FailureKind = serde_json::from_value(j).unwrap();
            assert_eq!(fk, back);
        }
    }

    #[test]
    fn proposal_envelope_json_roundtrip() {
        let pe = ProposalEnvelope {
            schema_version: LEARNER_SCHEMA_VERSION,
            original_hash: "abc".into(),
            proposed_hash: "def".into(),
            kind: "promote_pattern".into(),
            rationale: "recurrence".into(),
            freshness_at_proposal: ContextFreshness::Stale,
            impact_decision: ImpactDecision::RequireImpactFirst,
        };
        let j = serde_json::to_value(&pe).unwrap();
        let back: ProposalEnvelope = serde_json::from_value(j).unwrap();
        assert_eq!(pe, back);
    }

    #[test]
    fn trajectory_step_with_nested_vitals_roundtrip() {
        let v = CognitiveVitals {
            gate: VitalsGate::Warn,
            phase: "reasoning:0.5".into(),
            trust: 0.5,
            mean_logprob: -0.2,
            entropy: 0.1,
            sample_tokens: 8,
        };
        let s = TrajectoryStep {
            step_id: "s2".into(),
            timestamp_ms: 2,
            adapter: "builtin".into(),
            operation: "list".into(),
            inputs_preview: Some("a".into()),
            outputs_preview: Some("b".into()),
            duration_ms: 9,
            success: false,
            error: Some("boom".into()),
            vitals: Some(v.clone()),
            freshness_at_step: Some(ContextFreshness::Unknown),
            frame_vars: None,
            tool_telemetry: None,
        };
        let j = serde_json::to_value(&s).unwrap();
        let back: TrajectoryStep = serde_json::from_value(j).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn telemetry_learner_keys_are_unique_and_non_empty() {
        use telemetry::*;
        let keys = [
            TRAJECTORY_RECORDED,
            TRAJECTORY_OUTCOME,
            TRAJECTORY_STEP_DURATION_MS,
            FAILURE_RECORDED,
            FAILURE_RESOLUTION_HIT,
            FAILURE_PREVENTED_COUNT,
            PROPOSAL_VALIDATED,
            PROPOSAL_ADOPTED,
            COMPRESSION_PROFILE_TUNED,
            COMPRESSION_CACHE_HIT,
            PERSONA_AXIS_DELTA,
            VITALS_GATE_AT_TURN,
        ];
        for k in keys {
            assert!(!k.is_empty());
        }
        for i in 0..keys.len() {
            for j in (i + 1)..keys.len() {
                assert_ne!(keys[i], keys[j], "duplicate telemetry key");
            }
        }
    }
}
