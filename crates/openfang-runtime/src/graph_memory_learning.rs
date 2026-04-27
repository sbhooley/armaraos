//! Unified **graph-memory learning** policy and ingest spine for trajectories + typed failures.
//!
//! ## Product switch
//! - **`AINL_LEARNING`** (process env): when set to a falsy token (`0`, `false`, `no`, `off`),
//!   disables **both** trajectory batch capture and failure-node persistence for this process,
//!   regardless of `AINL_TRAJECTORY_ENABLED` / `AINL_FAILURE_LEARNING_ENABLED`.
//! - **`manifest.metadata["ainl_learning"]`**: same tokens when **`AINL_LEARNING` is unset** —
//!   per-agent default (e.g. opt an agent out without global env).
//!
//! When the master switch is **not** off, existing per-subsystem envs apply
//! ([`crate::graph_memory_writer::trajectory_env_enabled`],
//! [`crate::graph_memory_writer::failure_learning_env_enabled`]).
//!
//! ## Ingest
//! All failure paths in the agent loop should go through [`LearningRecorder`] so sanitization,
//! policy, and telemetry stay in one place.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

use openfang_memory::session::Session;
use openfang_types::agent::AgentManifest;
use regex::Regex;
use tracing::{debug, warn};
use uuid::Uuid;

use crate::graph_memory_writer::GraphMemoryWriter;

static FAILURE_RECORDED_OK: AtomicU64 = AtomicU64::new(0);
static FAILURE_SKIPPED_POLICY: AtomicU64 = AtomicU64::new(0);
static FAILURE_SKIPPED_NO_WRITER: AtomicU64 = AtomicU64::new(0);
static FAILURE_WRITE_NONE: AtomicU64 = AtomicU64::new(0);

/// Counters for operator / status introspection (best-effort; relaxed ordering).
#[must_use]
pub fn metrics_snapshot() -> serde_json::Value {
    serde_json::json!({
        "failure_recorded_ok": FAILURE_RECORDED_OK.load(Ordering::Relaxed),
        "failure_skipped_policy": FAILURE_SKIPPED_POLICY.load(Ordering::Relaxed),
        "failure_skipped_no_graph_writer": FAILURE_SKIPPED_NO_WRITER.load(Ordering::Relaxed),
        "failure_write_returned_none": FAILURE_WRITE_NONE.load(Ordering::Relaxed),
    })
}

fn record_ok() {
    FAILURE_RECORDED_OK.fetch_add(1, Ordering::Relaxed);
}

fn record_skipped_policy() {
    FAILURE_SKIPPED_POLICY.fetch_add(1, Ordering::Relaxed);
}

fn record_skipped_no_writer() {
    FAILURE_SKIPPED_NO_WRITER.fetch_add(1, Ordering::Relaxed);
}

fn record_write_none() {
    FAILURE_WRITE_NONE.fetch_add(1, Ordering::Relaxed);
}

#[must_use]
fn master_learning_disabled_token(s: &str) -> bool {
    matches!(
        s.trim().to_ascii_lowercase().as_str(),
        "" | "0" | "false" | "no" | "off"
    )
}

/// When `true`, trajectory + failure **learning stack** writes are suppressed (master off).
#[must_use]
pub fn master_learning_stack_disabled(manifest: &AgentManifest) -> bool {
    if let Ok(v) = std::env::var("AINL_LEARNING") {
        let t = v.trim();
        if !t.is_empty() {
            return master_learning_disabled_token(t);
        }
    }
    if let Some(raw) = manifest
        .metadata
        .get("ainl_learning")
        .and_then(|x| x.as_str())
    {
        let t = raw.trim();
        if !t.is_empty() {
            return master_learning_disabled_token(t);
        }
    }
    false
}

/// Resolved policy for trajectory batching and failure persistence (after master + subsystem envs).
#[derive(Debug, Clone, Copy)]
pub struct LearningStackPolicy {
    pub master_stack_disabled: bool,
    pub trajectories: bool,
    pub failures: bool,
    /// `true` when the agent loop should auto-submit `pattern_promote` improvement proposals at
    /// recurrence-detection time (`prompt_eligible: false → true`). Inherits master switch and
    /// the per-agent manifest opt-out; defaults on when both upstream gates are on.
    pub auto_submit_pattern_proposals: bool,
}

impl LearningStackPolicy {
    #[must_use]
    pub fn resolve(manifest: &AgentManifest) -> Self {
        let master_stack_disabled = master_learning_stack_disabled(manifest);
        let trajectories =
            !master_stack_disabled && crate::graph_memory_writer::trajectory_env_enabled();
        let failures =
            !master_stack_disabled && crate::graph_memory_writer::failure_learning_env_enabled();
        let auto_submit_pattern_proposals = !master_stack_disabled
            && crate::improvement_proposals_host::env_enabled()
            && crate::improvement_proposals_host::auto_submit_env_enabled()
            && !manifest_auto_submit_opt_out(manifest);
        Self {
            master_stack_disabled,
            trajectories,
            failures,
            auto_submit_pattern_proposals,
        }
    }
}

#[must_use]
fn manifest_auto_submit_opt_out(manifest: &AgentManifest) -> bool {
    if let Some(raw) = manifest
        .metadata
        .get("ainl_auto_submit_pattern_proposals")
        .and_then(|x| x.as_str())
    {
        let t = raw.trim().to_ascii_lowercase();
        return matches!(t.as_str(), "0" | "false" | "no" | "off");
    }
    false
}

/// Best-effort redaction + size cap before persistence / FTS (`failure` nodes).
#[must_use]
pub fn sanitize_failure_message(input: &str) -> String {
    static BEARER: OnceLock<Regex> = OnceLock::new();
    let re = BEARER.get_or_init(|| {
        Regex::new(r"(?i)bearer\s+[A-Za-z0-9._~+/=-]{8,}").expect("static bearer redaction regex")
    });
    let redacted = re.replace_all(input, "Bearer <redacted>");
    let mut s: String = redacted.into_owned();
    const MAX_CHARS: usize = 8000;
    if s.chars().count() > MAX_CHARS {
        s = s.chars().take(MAX_CHARS).collect();
        s.push('…');
    }
    s
}

/// Tokens too short or too common add noise to FTS recall; stripped before
/// [`ainl_memory::GraphMemory::search_failures_for_agent`] (which applies prefix-AND FTS).
#[must_use]
pub fn failure_recall_fts_query(user_message: &str) -> Option<String> {
    const STOPWORDS: &[&str] = &[
        "the", "and", "for", "not", "you", "all", "can", "her", "was", "one", "our", "out", "are",
        "but", "has", "have", "had", "how", "what", "when", "where", "who", "why", "with", "from",
        "your", "this", "that", "into", "than", "then", "them", "they", "their", "there", "these",
        "those", "will", "would", "could", "should", "about", "after", "before", "also", "just",
        "like", "some", "such", "very", "more", "most", "other", "only", "same", "each", "both",
        "been", "being", "here", "help", "please", "want", "need", "make", "sure",
    ];
    // Keep `_` inside identifiers (`shell_exec`, `tool_runner`) — `char::is_alphanumeric` is false for `_`.
    let mut raw: Vec<String> = Vec::new();
    let mut cur = String::new();
    for ch in user_message.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            cur.push(ch.to_ascii_lowercase());
        } else if !cur.is_empty() {
            raw.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        raw.push(cur);
    }
    let mut tokens: Vec<String> = raw
        .into_iter()
        .filter(|t| t.len() >= 3 && !STOPWORDS.contains(&t.as_str()))
        .take(12)
        .collect();
    tokens.sort_unstable();
    tokens.dedup();
    if tokens.is_empty() {
        None
    } else {
        Some(tokens.join(" "))
    }
}

/// Single entry point for graph-memory **failure** learning from the agent loop.
#[derive(Clone)]
pub struct LearningRecorder {
    gm: Option<GraphMemoryWriter>,
    policy: LearningStackPolicy,
    /// When `AINL_MEMORY_PROJECT_SCOPE` is on, copied from the manifest (tags failure nodes, etc.).
    memory_project_id: Option<String>,
}

impl LearningRecorder {
    #[must_use]
    pub fn new(gm: Option<GraphMemoryWriter>, manifest: &AgentManifest) -> Self {
        Self {
            gm,
            policy: LearningStackPolicy::resolve(manifest),
            memory_project_id: crate::memory_project_scope::effective_memory_project_id(manifest),
        }
    }

    #[must_use]
    pub fn policy(&self) -> LearningStackPolicy {
        self.policy
    }

    /// Per-turn trajectory slot buffer (OpenFang host) — master off forces off.
    #[must_use]
    pub fn trajectories_on(&self) -> bool {
        self.policy.trajectories && self.gm.is_some()
    }

    #[must_use]
    pub fn failures_on(&self) -> bool {
        self.policy.failures && self.gm.is_some()
    }

    fn failures_record_gate(&self) -> Result<&GraphMemoryWriter, ()> {
        if !self.policy.failures {
            record_skipped_policy();
            return Err(());
        }
        let Some(ref gm) = self.gm else {
            record_skipped_no_writer();
            return Err(());
        };
        Ok(gm)
    }

    fn mcp_ainl_graph_snapshot_gate(&self) -> Option<&GraphMemoryWriter> {
        if self.policy.master_stack_disabled {
            return None;
        }
        self.gm.as_ref()
    }

    /// Persist `mcp_ainl_ainl_capabilities` / `recommended_next_tools` snapshots to **semantic** graph rows
    /// when the session cache reports new content (hash changed).
    pub async fn record_mcp_ainl_tool_snapshots(
        &self,
        _session: &Session,
        tool_name: &str,
        content: &str,
        apply: crate::mcp_ainl_session::McpAinlApplyResult,
    ) {
        if !apply.new_capabilities_for_graph
            && !apply.new_recommended_next_for_graph
            && !apply.new_wizard_state_for_graph
        {
            return;
        }
        let Some(gm) = self.mcp_ainl_graph_snapshot_gate() else {
            return;
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(content) else {
            return;
        };
        let pid = self.memory_project_id.as_deref();

        if apply.new_capabilities_for_graph && tool_name == "mcp_ainl_ainl_capabilities" {
            if let Some(fact) = crate::mcp_ainl_session::format_capabilities_digest(&v) {
                let h = crate::mcp_ainl_session::content_sha16(content);
                let tags = vec!["mcp:ainl:capabilities".to_string(), format!("v:{h}")];
                gm.record_fact_with_tags(fact, 0.95, Uuid::new_v4(), &tags, pid)
                    .await;
            };
        }

        if apply.new_recommended_next_for_graph && v.get("recommended_next_tools").is_some() {
            if let Some(fact) = crate::mcp_ainl_session::format_recommended_next_tools_echo(&v) {
                let h = crate::mcp_ainl_session::content_sha16(&fact);
                let tags = vec!["mcp:ainl:recommended_next".to_string(), format!("v:{h}")];
                gm.record_fact_with_tags(fact, 0.9, Uuid::new_v4(), &tags, pid)
                    .await;
            }
        }

        if apply.new_wizard_state_for_graph && tool_name == "mcp_ainl_ainl_get_started" {
            if let Some(fact) = crate::mcp_ainl_session::format_wizard_hint(&v) {
                let h = crate::mcp_ainl_session::content_sha16(&fact);
                let tags = vec!["mcp:ainl:wizard_state".to_string(), format!("v:{h}")];
                gm.record_fact_with_tags(fact, 0.9, Uuid::new_v4(), &tags, pid)
                    .await;
            }
        }
    }

    pub async fn record_loop_guard_failure(
        &self,
        session: &Session,
        verdict_label: &str,
        tool_name: &str,
        message: &str,
    ) {
        let Ok(gm) = self.failures_record_gate() else {
            return;
        };
        let sid = session.id.0.to_string();
        let msg = sanitize_failure_message(message);
        let r = gm
            .record_loop_guard_failure(
                verdict_label,
                Some(tool_name),
                msg.as_str(),
                Some(sid.as_str()),
                self.memory_project_id.as_deref(),
            )
            .await;
        if r.is_some() {
            record_ok();
        } else {
            record_write_none();
        }
    }

    pub async fn record_tool_execution_failure(
        &self,
        session: &Session,
        tool_name: &str,
        message: &str,
    ) {
        self.record_tool_execution_failure_with_source(session, tool_name, message, None, None)
            .await;
    }

    /// Like [`Self::record_tool_execution_failure`], with optional MCP `source_namespace` / `source_tool` metadata.
    pub async fn record_tool_execution_failure_with_source(
        &self,
        session: &Session,
        tool_name: &str,
        message: &str,
        source_namespace: Option<&str>,
        source_tool: Option<&str>,
    ) {
        let Ok(gm) = self.failures_record_gate() else {
            return;
        };
        let sid = session.id.0.to_string();
        let msg = sanitize_failure_message(message);
        let r = gm
            .record_tool_execution_failure_with_source(
                tool_name,
                msg.as_str(),
                Some(sid.as_str()),
                self.memory_project_id.as_deref(),
                source_namespace,
                source_tool,
            )
            .await;
        if r.is_some() {
            record_ok();
        } else {
            record_write_none();
        }
    }

    pub async fn record_agent_loop_precheck_failure(
        &self,
        session: &Session,
        kind: &str,
        tool_name: &str,
        message: &str,
    ) {
        let Ok(gm) = self.failures_record_gate() else {
            return;
        };
        let sid = session.id.0.to_string();
        let msg = sanitize_failure_message(message);
        let r = gm
            .record_agent_loop_tool_precheck_failure(
                kind,
                tool_name,
                msg.as_str(),
                Some(sid.as_str()),
                self.memory_project_id.as_deref(),
            )
            .await;
        if r.is_some() {
            record_ok();
        } else {
            record_write_none();
        }
    }

    /// `true` when [`Self::maybe_auto_submit_pattern_promotion`] should attempt a submit for
    /// promotion-event outcomes from this loop.
    #[must_use]
    pub fn auto_submit_pattern_proposals_on(&self) -> bool {
        self.policy.auto_submit_pattern_proposals
    }

    /// Recurrence-detection trigger: when a tool-sequence pattern just crossed the
    /// [`ainl_memory::pattern_promotion::should_promote`] gate
    /// (`outcome.just_promoted == true`), spawn a blocking task that auto-submits a
    /// `pattern_promote` improvement proposal via
    /// [`crate::improvement_proposals_host::auto_submit_pattern_proposal`].
    ///
    /// Best-effort: never blocks the live agent loop and never propagates a submit failure.
    /// All gating (master switch, env opt-out, manifest opt-out, dedup, idempotency) lives
    /// inside the host helper.
    pub fn maybe_auto_submit_pattern_promotion(
        &self,
        home_dir: PathBuf,
        agent_id: String,
        outcome: &crate::graph_memory_writer::PatternUpsertOutcome,
    ) {
        if !outcome.just_promoted {
            return;
        }
        if !self.policy.auto_submit_pattern_proposals {
            debug!(
                agent_id = %agent_id,
                pattern = %outcome.name,
                "auto-submit disabled by policy; skipping pattern_promote proposal"
            );
            return;
        }
        let name = outcome.name.clone();
        let tool_sequence = outcome.tool_sequence.clone();
        let observation_count = outcome.observation_count;
        let fitness = outcome.fitness;
        tokio::task::spawn_blocking(move || {
            let spec = crate::improvement_proposals_host::AutoSubmitPatternProposal {
                name: name.as_str(),
                tool_sequence: tool_sequence.as_slice(),
                observation_count,
                fitness,
                freshness_at_proposal: None,
            };
            match crate::improvement_proposals_host::auto_submit_pattern_proposal(
                home_dir.as_path(),
                agent_id.as_str(),
                &spec,
            ) {
                Ok(Some(id)) => debug!(
                    agent_id = %agent_id,
                    pattern = %name,
                    proposal_id = %id,
                    observations = observation_count,
                    fitness,
                    "auto-submitted pattern_promote improvement proposal"
                ),
                Ok(None) => {
                    debug!(
                        agent_id = %agent_id,
                        pattern = %name,
                        "auto-submit no-op (env disabled or duplicate proposed_hash)"
                    );
                }
                Err(e) => warn!(
                    agent_id = %agent_id,
                    pattern = %name,
                    error = %e,
                    "auto-submit pattern_promote improvement proposal failed"
                ),
            }
            let proc_spec = crate::procedure_learning_host::ProcedureMintFromPattern {
                name: name.as_str(),
                tool_sequence: tool_sequence.as_slice(),
                observation_count,
                fitness,
                freshness_at_proposal: None,
            };
            match crate::procedure_learning_host::auto_submit_procedure_mint_from_pattern(
                home_dir.as_path(),
                agent_id.as_str(),
                &proc_spec,
            ) {
                Ok(Some(id)) => debug!(
                    agent_id = %agent_id,
                    pattern = %name,
                    proposal_id = %id,
                    observations = observation_count,
                    fitness,
                    "auto-submitted procedure_mint improvement proposal"
                ),
                Ok(None) => debug!(
                    agent_id = %agent_id,
                    pattern = %name,
                    "procedure_mint auto-submit no-op"
                ),
                Err(e) => warn!(
                    agent_id = %agent_id,
                    pattern = %name,
                    error = %e,
                    "auto-submit procedure_mint improvement proposal failed"
                ),
            };
        });
    }

    /// Graph validation failure from **`ainl-runtime`** before `run_turn` proceeds (dangling edges, etc.).
    pub async fn record_ainl_runtime_graph_validation_failure(
        &self,
        session: &Session,
        message: &str,
    ) {
        let Ok(gm) = self.failures_record_gate() else {
            return;
        };
        let sid = session.id.0.to_string();
        let msg = sanitize_failure_message(message);
        let r = gm
            .record_ainl_runtime_graph_validation_failure(
                msg.as_str(),
                Some(sid.as_str()),
                self.memory_project_id.as_deref(),
            )
            .await;
        if r.is_some() {
            record_ok();
        } else {
            record_write_none();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn master_off_from_env_overrides_manifest() {
        let _lock = crate::runtime_env_test_lock().blocking_lock();
        let key = "AINL_LEARNING";
        let old = std::env::var(key).ok();
        std::env::set_var(key, "off");
        let mut m = AgentManifest::default();
        m.metadata.insert("ainl_learning".into(), json!("on"));
        assert!(master_learning_stack_disabled(&m));
        match old {
            None => std::env::remove_var(key),
            Some(v) => std::env::set_var(key, v),
        }
    }

    #[test]
    fn master_off_from_manifest_when_env_unset() {
        let _lock = crate::runtime_env_test_lock().blocking_lock();
        let key = "AINL_LEARNING";
        let old = std::env::var(key).ok();
        std::env::remove_var(key);
        let mut m = AgentManifest::default();
        m.metadata.insert("ainl_learning".into(), json!("false"));
        assert!(master_learning_stack_disabled(&m));
        match old {
            None => {}
            Some(v) => std::env::set_var(key, v),
        }
    }

    #[test]
    fn sanitize_strips_bearer() {
        let s = "curl -H 'Authorization: Bearer secretoken1234567890' https://x";
        let o = sanitize_failure_message(s);
        assert!(!o.contains("secretoken"));
        assert!(o.contains("<redacted>") || o.contains("Bearer"));
    }

    #[test]
    fn failure_recall_fts_query_extracts_meaningful_tokens() {
        let q = failure_recall_fts_query("Please debug the shell_exec quantumretirement error")
            .expect("query");
        assert!(q.contains("shell_exec"));
        assert!(q.contains("quantumretirement"));
        assert!(!q.contains("please"));
    }

    #[test]
    fn failure_recall_fts_query_none_when_only_stopwords_or_short() {
        assert!(failure_recall_fts_query("a b the and").is_none());
        assert!(failure_recall_fts_query("ok go").is_none());
    }

    #[test]
    fn auto_submit_policy_default_on_when_master_on() {
        let _lock = crate::runtime_env_test_lock().blocking_lock();
        for k in [
            "AINL_LEARNING",
            "AINL_IMPROVEMENT_PROPOSALS_ENABLED",
            "AINL_AUTO_SUBMIT_PATTERN_PROPOSALS",
        ] {
            std::env::remove_var(k);
        }
        let m = AgentManifest::default();
        let p = LearningStackPolicy::resolve(&m);
        assert!(
            p.auto_submit_pattern_proposals,
            "auto-submit defaults to on when master + env unset"
        );
    }

    #[test]
    fn auto_submit_policy_off_when_master_disabled() {
        let _lock = crate::runtime_env_test_lock().blocking_lock();
        let prev = std::env::var("AINL_LEARNING").ok();
        std::env::set_var("AINL_LEARNING", "off");
        let m = AgentManifest::default();
        let p = LearningStackPolicy::resolve(&m);
        assert!(p.master_stack_disabled);
        assert!(!p.auto_submit_pattern_proposals);
        match prev {
            Some(v) => std::env::set_var("AINL_LEARNING", v),
            None => std::env::remove_var("AINL_LEARNING"),
        }
    }

    #[test]
    fn auto_submit_policy_off_via_env_opt_out() {
        let _lock = crate::runtime_env_test_lock().blocking_lock();
        let prev_master = std::env::var("AINL_LEARNING").ok();
        let prev_auto = std::env::var("AINL_AUTO_SUBMIT_PATTERN_PROPOSALS").ok();
        std::env::remove_var("AINL_LEARNING");
        std::env::set_var("AINL_AUTO_SUBMIT_PATTERN_PROPOSALS", "false");
        let m = AgentManifest::default();
        let p = LearningStackPolicy::resolve(&m);
        assert!(!p.auto_submit_pattern_proposals);
        match prev_master {
            Some(v) => std::env::set_var("AINL_LEARNING", v),
            None => std::env::remove_var("AINL_LEARNING"),
        }
        match prev_auto {
            Some(v) => std::env::set_var("AINL_AUTO_SUBMIT_PATTERN_PROPOSALS", v),
            None => std::env::remove_var("AINL_AUTO_SUBMIT_PATTERN_PROPOSALS"),
        }
    }

    #[test]
    fn auto_submit_policy_off_via_manifest_metadata() {
        let _lock = crate::runtime_env_test_lock().blocking_lock();
        for k in [
            "AINL_LEARNING",
            "AINL_AUTO_SUBMIT_PATTERN_PROPOSALS",
            "AINL_IMPROVEMENT_PROPOSALS_ENABLED",
        ] {
            std::env::remove_var(k);
        }
        let mut m = AgentManifest::default();
        m.metadata
            .insert("ainl_auto_submit_pattern_proposals".into(), json!("off"));
        let p = LearningStackPolicy::resolve(&m);
        assert!(
            !p.auto_submit_pattern_proposals,
            "manifest opt-out wins over env defaults"
        );
    }
}
