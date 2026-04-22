//! Host integration for the `ainl-improvement-proposals` ledger: DB path, env gating, metrics, and
//! validators in [`crate::improvement_proposals_validators`].

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use ainl_contracts::ProposalEnvelope;
use ainl_contracts::telemetry;
use ainl_improvement_proposals::{
    AdoptResult, ImprovementProposalId, ImprovementProposalListItem, ProposalLedger, ProposalLedgerError,
};
use ainl_memory::{AinlMemoryNode, AinlNodeKind, AinlNodeType, GraphMemory, ProcedureType};

use uuid::Uuid;

use crate::improvement_proposals_validators::{
    default_validate_mode, parse_validate_mode, run_validate, structural_prologue_only,
};

const MAX_AINL_EMBED_IN_ADOPTION_CLUSTER: usize = 50_000;
const MAX_PROC_PATTERN_BYTES: usize = 256 * 1024;
const MAX_TOPIC_CLUSTER_CHARS: usize = 100_000;
const ADOPTION_RECALL_SECONDS: i64 = 60 * 60 * 24 * 365 * 10;

static SUBMIT_OK: AtomicU64 = AtomicU64::new(0);
static SUBMIT_HASH_MISMATCH: AtomicU64 = AtomicU64::new(0);
static SUBMIT_LEDGER_ERR: AtomicU64 = AtomicU64::new(0);
static SUBMIT_WHEN_DISABLED: AtomicU64 = AtomicU64::new(0);
static VALIDATE_ACCEPTED: AtomicU64 = AtomicU64::new(0);
static VALIDATE_REJECTED: AtomicU64 = AtomicU64::new(0);
static VALIDATE_LEDGER_ERR: AtomicU64 = AtomicU64::new(0);
static VALIDATE_WHEN_DISABLED: AtomicU64 = AtomicU64::new(0);
static ADOPT_OK: AtomicU64 = AtomicU64::new(0);
static ADOPT_ERR: AtomicU64 = AtomicU64::new(0);
static ADOPT_WHEN_DISABLED: AtomicU64 = AtomicU64::new(0);
static ADOPT_NOT_ADOPTABLE: AtomicU64 = AtomicU64::new(0);
static ADOPT_GRAPH_WRITE_ERR: AtomicU64 = AtomicU64::new(0);
static ADOPT_IDEMPOTENT: AtomicU64 = AtomicU64::new(0);
static ADOPT_REPAIR: AtomicU64 = AtomicU64::new(0);

/// When **unset** or any non-falsy value, the improvement-proposals HTTP routes and ledger
/// host APIs are **on** (same opt-out as `AINL_TRAJECTORY_ENABLED`: `0`, `false`, `no`, `off`).
#[must_use]
pub fn env_enabled() -> bool {
    match std::env::var("AINL_IMPROVEMENT_PROPOSALS_ENABLED") {
        Ok(s) => {
            let v = s.trim().to_ascii_lowercase();
            !(v == "0" || v == "false" || v == "no" || v == "off")
        }
        Err(_) => true,
    }
}

/// Re-exported for tests / tools that need the same `graph` + size pre-check as the ledger
/// `validate` pipeline.
#[inline]
pub fn default_structural_validate(proposed_ainl_text: &str) -> Result<(), String> {
    structural_prologue_only(proposed_ainl_text)
}

/// Re-exported for the HTTP / tools layer.
pub use crate::improvement_proposals_validators::ValidateMode;

/// SQLite path per agent, alongside other graph-memory governance files.
#[must_use]
pub fn improvement_proposals_db_path(home_dir: &Path, agent_id: &str) -> PathBuf {
    home_dir
        .join("agents")
        .join(agent_id)
        .join(".graph-memory")
        .join("improvement_proposals.db")
}

/// `ainl_memory::GraphMemory` path for `agent_id` (matches dashboard / `openfang-api` `graph_db_path`).
#[must_use]
pub fn graph_memory_db_path(home_dir: &Path, agent_id: &str) -> PathBuf {
    home_dir
        .join("agents")
        .join(agent_id)
        .join("ainl_memory.db")
}

/// Counters for `/api/status` and operators (relaxed ordering).
#[must_use]
pub fn metrics_snapshot() -> serde_json::Value {
    let v_ok = VALIDATE_ACCEPTED.load(Ordering::Relaxed);
    let v_rej = VALIDATE_REJECTED.load(Ordering::Relaxed);
    let adopt_ok = ADOPT_OK.load(Ordering::Relaxed);
    serde_json::json!({
        "env_enabled": env_enabled(),
        "submit_ok": SUBMIT_OK.load(Ordering::Relaxed),
        "submit_hash_mismatch": SUBMIT_HASH_MISMATCH.load(Ordering::Relaxed),
        "submit_ledger_error": SUBMIT_LEDGER_ERR.load(Ordering::Relaxed),
        "submit_when_disabled": SUBMIT_WHEN_DISABLED.load(Ordering::Relaxed),
        "validate_accepted": v_ok,
        "validate_rejected": v_rej,
        "adopt_to_graph_ok": adopt_ok,
        "adopt_idempotent": ADOPT_IDEMPOTENT.load(Ordering::Relaxed),
        "adopt_ledger_repair": ADOPT_REPAIR.load(Ordering::Relaxed),
        "adopt_to_graph_error": ADOPT_ERR.load(Ordering::Relaxed),
        "adopt_not_adoptable": ADOPT_NOT_ADOPTABLE.load(Ordering::Relaxed),
        "adopt_graph_write_error": ADOPT_GRAPH_WRITE_ERR.load(Ordering::Relaxed),
        "adopt_when_disabled": ADOPT_WHEN_DISABLED.load(Ordering::Relaxed),
        telemetry::PROPOSAL_VALIDATED: v_ok + v_rej,
        telemetry::PROPOSAL_ADOPTED: adopt_ok,
        "validate_ledger_error": VALIDATE_LEDGER_ERR.load(Ordering::Relaxed),
        "validate_when_disabled": VALIDATE_WHEN_DISABLED.load(Ordering::Relaxed),
    })
}

fn open_ledger(home_dir: &Path, agent_id: &str) -> Result<ProposalLedger, String> {
    let path = improvement_proposals_db_path(home_dir, agent_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create parent dir: {e}"))?;
    }
    ProposalLedger::open(&path).map_err(|e: ProposalLedgerError| e.to_string())
}

/// Submits a proposal (hash must match) when [`env_enabled`] is true.
pub fn submit(
    home_dir: &Path,
    agent_id: &str,
    envelope: &ProposalEnvelope,
    proposed_ainl_text: &str,
) -> Result<ImprovementProposalId, String> {
    if !env_enabled() {
        SUBMIT_WHEN_DISABLED.fetch_add(1, Ordering::Relaxed);
        return Err(
            "improvement proposals are disabled (AINL_IMPROVEMENT_PROPOSALS_ENABLED=0|false|no|off)"
                .to_string(),
        );
    }
    let ledger = open_ledger(home_dir, agent_id).map_err(|e| {
        SUBMIT_LEDGER_ERR.fetch_add(1, Ordering::Relaxed);
        e
    })?;
    match ledger.submit(agent_id, envelope, proposed_ainl_text) {
        Ok(id) => {
            SUBMIT_OK.fetch_add(1, Ordering::Relaxed);
            Ok(id)
        }
        Err(ProposalLedgerError::HashMismatch) => {
            SUBMIT_HASH_MISMATCH.fetch_add(1, Ordering::Relaxed);
            Err(ProposalLedgerError::HashMismatch.to_string())
        }
        Err(e) => {
            SUBMIT_LEDGER_ERR.fetch_add(1, Ordering::Relaxed);
            Err(e.to_string())
        }
    }
}

/// Runs [`run_validate`] with the given `mode` (or `default_validate_mode` when `None`) and
/// records the result in the ledger.
pub fn validate_proposal(
    home_dir: &Path,
    agent_id: &str,
    id: ImprovementProposalId,
    mode: Option<ValidateMode>,
) -> Result<AdoptResult, String> {
    if !env_enabled() {
        VALIDATE_WHEN_DISABLED.fetch_add(1, Ordering::Relaxed);
        return Err(
            "improvement proposals are disabled (AINL_IMPROVEMENT_PROPOSALS_ENABLED=0|false|no|off)"
                .to_string(),
        );
    }
    let mode = mode.unwrap_or_else(default_validate_mode);
    let ledger = open_ledger(home_dir, agent_id).map_err(|e| {
        VALIDATE_LEDGER_ERR.fetch_add(1, Ordering::Relaxed);
        e
    })?;
    let res = ledger
        .validate_and_record(id, |text: &str| run_validate(mode, text))
        .map_err(|e: ProposalLedgerError| {
            VALIDATE_LEDGER_ERR.fetch_add(1, Ordering::Relaxed);
            e.to_string()
        });
    if let Ok(ref r) = res {
        if r.accepted {
            VALIDATE_ACCEPTED.fetch_add(1, Ordering::Relaxed);
        } else {
            VALIDATE_REJECTED.fetch_add(1, Ordering::Relaxed);
        }
    }
    res
}

/// Back-compat: uses [`default_validate_mode`] (env `AINL_IMPROVEMENT_PROPOSALS_DEFAULT_VALIDATE_MODE`).
pub fn validate_with_default(
    home_dir: &Path,
    agent_id: &str,
    id: ImprovementProposalId,
) -> Result<AdoptResult, String> {
    validate_proposal(home_dir, agent_id, id, None)
}

/// Re-export parse for the HTTP layer.
pub fn parse_mode(s: &str) -> Option<ValidateMode> {
    parse_validate_mode(s)
}

/// Recent rows from the per-agent proposal ledger.
pub fn list_proposals(
    home_dir: &Path,
    agent_id: &str,
    limit: usize,
) -> Result<Vec<ImprovementProposalListItem>, String> {
    if !env_enabled() {
        return Err(
            "improvement proposals are disabled (AINL_IMPROVEMENT_PROPOSALS_ENABLED=0|false|no|off)"
                .to_string(),
        );
    }
    let ledger = open_ledger(home_dir, agent_id)?;
    ledger
        .list_recent(agent_id, limit)
        .map_err(|e: ProposalLedgerError| e.to_string())
}

/// New `semantic` or (for `pattern_promote*`) `procedural` node in `ainl_memory.db` after a successful
/// `adopt_validated_proposal` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdoptToGraphResult {
    pub graph_node_id: Uuid,
    /// Echoes [`ProposalEnvelope::kind`].
    pub proposal_kind: String,
    /// `true` when the graph node already existed (idempotent) or the ledger was repaired from
    /// an existing graph node after a previous partial failure.
    pub idempotent: bool,
}

fn is_pattern_promote_kind(k: &str) -> bool {
    let t = k.trim();
    t.eq_ignore_ascii_case("pattern_promote")
        || t.eq_ignore_ascii_case("pattern-promote")
        || t.to_ascii_lowercase() == "pattern promote"
}

fn find_existing_proposal_node(
    gm: &GraphMemory,
    agent_id: &str,
    proposal_id: Uuid,
) -> Option<Uuid> {
    let want_tag = format!("proposal:{proposal_id}");
    for kind in [AinlNodeKind::Semantic, AinlNodeKind::Procedural] {
        if let Ok(nodes) = gm.recall_by_type(kind, ADOPTION_RECALL_SECONDS) {
            for n in nodes {
                if n.agent_id != agent_id {
                    continue;
                }
                match &n.node_type {
                    AinlNodeType::Semantic { semantic } => {
                        if semantic.tags.iter().any(|t| t == &want_tag) {
                            return Some(n.id);
                        }
                    }
                    AinlNodeType::Procedural { procedural } => {
                        if procedural.trace_id.as_deref() == Some(&proposal_id.to_string()) {
                            return Some(n.id);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    None
}

/// Writes a graph node, then calls [`ProposalLedger::mark_adopted`]. Handles idempotent replay and
/// ledger–graph repair when a prior run wrote the node but the ledger was not updated.
pub fn adopt_validated_proposal(
    home_dir: &Path,
    agent_id: &str,
    proposal_id: ImprovementProposalId,
) -> Result<AdoptToGraphResult, String> {
    if !env_enabled() {
        ADOPT_WHEN_DISABLED.fetch_add(1, Ordering::Relaxed);
        return Err(
            "improvement proposals are disabled (AINL_IMPROVEMENT_PROPOSALS_ENABLED=0|false|no|off)"
                .to_string(),
        );
    }
    let ledger = open_ledger(home_dir, agent_id).map_err(|e| {
        ADOPT_ERR.fetch_add(1, Ordering::Relaxed);
        e
    })?;
    let payload = match ledger
        .get_for_graph_adopt(proposal_id)
        .map_err(|e: ProposalLedgerError| {
            ADOPT_ERR.fetch_add(1, Ordering::Relaxed);
            e.to_string()
        })? {
        Some(p) => p,
        None => {
            ADOPT_NOT_ADOPTABLE.fetch_add(1, Ordering::Relaxed);
            return Err("proposal not found in ledger".to_string());
        }
    };
    if payload.row.agent_id != agent_id {
        ADOPT_NOT_ADOPTABLE.fetch_add(1, Ordering::Relaxed);
        return Err("proposal agent_id does not match request".to_string());
    }
    if !payload.row.accepted {
        ADOPT_NOT_ADOPTABLE.fetch_add(1, Ordering::Relaxed);
        return Err("proposal is not structurally accepted; run validate first".to_string());
    }
    let gm_path = graph_memory_db_path(home_dir, agent_id);
    if let Some(p) = gm_path.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    let gm = GraphMemory::new(&gm_path)
        .map_err(|e| {
            ADOPT_GRAPH_WRITE_ERR.fetch_add(1, Ordering::Relaxed);
            e.to_string()
        })?;

    // Idempotent: already adopted in ledger and node still present
    if payload.row.adopted_at.is_some() {
        if let Some(ref s) = payload.row.adopted_graph_node_id {
            if let Ok(nid) = Uuid::parse_str(s) {
                if let Ok(opt) = gm.store().read_node(nid) {
                    if let Some(n) = opt {
                        if n.agent_id == agent_id {
                            ADOPT_IDEMPOTENT.fetch_add(1, Ordering::Relaxed);
                            return Ok(AdoptToGraphResult {
                                graph_node_id: nid,
                                proposal_kind: payload.envelope.kind,
                                idempotent: true,
                            });
                        }
                    }
                }
            }
        }
        return Err(
            "proposal is marked adopted in the ledger but the graph node is missing; restore from snapshot or clear adoption manually"
                .to_string(),
        );
    }

    // Repair: graph node exists (e.g. `mark_adopted` failed after `write_node`) but ledger is not
    if let Some(nid) = find_existing_proposal_node(&gm, agent_id, proposal_id) {
        ledger
            .mark_adopted(proposal_id, &nid.to_string())
            .map_err(|e: ProposalLedgerError| {
                ADOPT_ERR.fetch_add(1, Ordering::Relaxed);
                e.to_string()
            })?;
        ADOPT_REPAIR.fetch_add(1, Ordering::Relaxed);
        ADOPT_OK.fetch_add(1, Ordering::Relaxed);
        return Ok(AdoptToGraphResult {
            graph_node_id: nid,
            proposal_kind: payload.envelope.kind,
            idempotent: true,
        });
    }

    let proposal_kind = payload.envelope.kind.clone();
    let pattern = is_pattern_promote_kind(&proposal_kind);
    let node: AinlMemoryNode = if pattern {
        let mut pbytes = payload.proposed_ainl_text.as_bytes().to_vec();
        if pbytes.len() > MAX_PROC_PATTERN_BYTES {
            pbytes.truncate(MAX_PROC_PATTERN_BYTES);
        }
        let mut n = AinlMemoryNode::new_pattern(
            format!("improvement_proposal/{}", &proposal_id),
            pbytes,
        );
        n.agent_id = agent_id.to_string();
        if let AinlNodeType::Procedural { ref mut procedural } = n.node_type {
            procedural.trace_id = Some(proposal_id.to_string());
            procedural.label = "improvement_proposal".to_string();
            procedural.prompt_eligible = false;
            procedural.procedure_type = ProcedureType::BehavioralRule;
            procedural.fitness = Some(0.78);
        } else {
            return Err("internal: expected procedural node for pattern_adopt".to_string());
        }
        n
    } else {
        let fact = format!(
            "Adopted AINL improvement proposal {} ({})",
            proposal_kind, proposal_id
        );
        let source_turn = Uuid::new_v4();
        let mut n = AinlMemoryNode::new_fact(fact, 0.95, source_turn);
        n.agent_id = agent_id.to_string();
        if let AinlNodeType::Semantic { ref mut semantic } = n.node_type {
            semantic
                .tags
                .push("scope:agent_private".to_string());
            semantic.tags.push("improvement_proposal_adopted".to_string());
            semantic.tags.push(format!("proposal:{proposal_id}"));
            let rsum = payload
                .envelope
                .rationale
                .trim()
                .chars()
                .take(200)
                .collect::<String>();
            let a_trim = if payload.proposed_ainl_text.len() > MAX_AINL_EMBED_IN_ADOPTION_CLUSTER {
                payload.proposed_ainl_text[..MAX_AINL_EMBED_IN_ADOPTION_CLUSTER].to_string()
            } else {
                payload.proposed_ainl_text.clone()
            };
            let j = serde_json::json!({
                "schema": "improvement_proposal_adopted_v1",
                "proposal_id": proposal_id.to_string(),
                "envelope_kind": &payload.envelope.kind,
                "rationale": &payload.envelope.rationale,
                "rationale_head": rsum,
                "proposed_ainl_text": a_trim,
                "ainl_embed_truncated": payload.proposed_ainl_text.len() > MAX_AINL_EMBED_IN_ADOPTION_CLUSTER,
            });
            let mut s = j.to_string();
            if s.len() > MAX_TOPIC_CLUSTER_CHARS {
                s = s.chars().take(MAX_TOPIC_CLUSTER_CHARS).collect();
            }
            semantic.topic_cluster = Some(s);
        } else {
            return Err("internal: expected semantic node for adoption".to_string());
        }
        n
    };

    gm.write_node(&node)
        .map_err(|e| {
            ADOPT_GRAPH_WRITE_ERR.fetch_add(1, Ordering::Relaxed);
            e.to_string()
        })?;
    let nid = node.id;
    ledger
        .mark_adopted(proposal_id, &nid.to_string())
        .map_err(|e: ProposalLedgerError| {
            ADOPT_ERR.fetch_add(1, Ordering::Relaxed);
            format!("graph write succeeded but ledger could not be marked adopted (re-fetch proposals and adopt again to repair): {e}")
        })?;
    ADOPT_OK.fetch_add(1, Ordering::Relaxed);
    Ok(AdoptToGraphResult {
        graph_node_id: nid,
        proposal_kind,
        idempotent: false,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, OnceLock};

    use super::*;

    static AINL_IMPROVEMENT_PROPOSALS_ENV_TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    #[test]
    fn env_enabled_unset_is_on_and_opt_out_off() {
        const K: &str = "AINL_IMPROVEMENT_PROPOSALS_ENABLED";
        let _g = AINL_IMPROVEMENT_PROPOSALS_ENV_TEST_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("lock");
        let prev = std::env::var(K).ok();
        let restore = || {
            match &prev {
                Some(s) => std::env::set_var(K, s),
                None => std::env::remove_var(K),
            }
        };
        std::env::remove_var(K);
        assert!(env_enabled());
        for off in ["0", "false", "no", "off"] {
            std::env::set_var(K, off);
            assert!(!env_enabled(), "expected opt-out: {off}");
        }
        for on in ["1", "true", "yes", "on", ""] {
            std::env::set_var(K, on);
            assert!(env_enabled(), "expected on: {on:?}");
        }
        restore();
    }

    #[test]
    fn structural_accepts_graph_header() {
        assert!(default_structural_validate("graph\n# x\n").is_ok());
    }

    #[test]
    fn structural_rejects_without_graph() {
        assert!(default_structural_validate("# no graph line\n").is_err());
    }
}
