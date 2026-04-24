//! Improvement proposal **ledger** (Phase 4 — `SELF_LEARNING_INTEGRATION_MAP.md` §8).
//!
//! Persists [`ProposalEnvelope`] + proposed AINL text, verifies `proposed_hash` against
//! `sha256(proposed_ainl_text)`, and runs a caller-supplied strict validator before marking a row
//! **accepted**. Hosts wire `validator` to `mcp_ainl_ainl_validate` / compile / or `ainl-runtime`
//! as appropriate; this crate stays free of `openfang_*` and `ainl-runtime` (no dependency cycles).

mod ledger;

pub use ledger::{
    AdoptGraphPayload, AdoptResult, ImprovementProposalId, ImprovementProposalListItem,
    ImprovementProposalRow, ProposalLedger, ProposalLedgerError,
};

use ainl_contracts::ProposalEnvelope;
use sha2::{Digest, Sha256};

/// Lowercase hex SHA-256 of UTF-8 bytes (for comparing to `ProposalEnvelope::proposed_hash`).
#[must_use]
pub fn sha256_hex_lower(s: &str) -> String {
    let d = Sha256::digest(s.as_bytes());
    hex::encode(d)
}

/// `true` when the proposal text matches the envelope’s `proposed_hash` (both compared lowercase).
#[must_use]
pub fn proposed_hash_matches(envelope: &ProposalEnvelope, proposed_ainl_text: &str) -> bool {
    let h = sha256_hex_lower(proposed_ainl_text);
    h == envelope.proposed_hash.trim().to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ProposalLedger, ProposalLedgerError};
    use ainl_contracts::{
        ContextFreshness, ImpactDecision, ProposalEnvelope, LEARNER_SCHEMA_VERSION,
    };
    use uuid::Uuid;

    fn sample_envelope(ainl: &str) -> ProposalEnvelope {
        let ph = sha256_hex_lower(ainl);
        ProposalEnvelope {
            schema_version: LEARNER_SCHEMA_VERSION,
            original_hash: "a".repeat(64),
            proposed_hash: ph,
            kind: "pattern_promote".into(),
            rationale: "test".into(),
            freshness_at_proposal: ContextFreshness::Fresh,
            impact_decision: ImpactDecision::AllowExecute,
        }
    }

    #[test]
    fn submit_rejects_hash_mismatch() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("prop.db");
        let p = ProposalLedger::open(&db).unwrap();
        let env = sample_envelope("graph\nt1\n");
        let err = p
            .submit("a1", &env, "wrong\n")
            .expect_err("expected mismatch");
        assert!(matches!(err, ProposalLedgerError::HashMismatch));
    }

    #[test]
    fn accept_and_reject_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("prop2.db");
        let p = ProposalLedger::open(&db).unwrap();
        let text = "graph\nt1\n";
        let env = sample_envelope(text);
        let id = p.submit("a1", &env, text).expect("insert");

        let r_ok = p
            .validate_and_record(id, |s: &str| if s == text { Ok(()) } else { Err("bad") })
            .expect("validate");
        assert!(r_ok.accepted, "{r_ok:?}");
        let row = p.get(id).unwrap().expect("row");
        assert!(row.accepted);
        assert!(row.validation_error.is_none());

        let id2 = p
            .submit("a1", &sample_envelope("other\n"), "other\n")
            .expect("insert2");
        let r_no = p
            .validate_and_record(id2, |_s: &str| Err::<(), _>("no"))
            .expect("valid");
        assert!(!r_no.accepted);
        assert_eq!(r_no.error.as_deref(), Some("no"));
        let row2 = p.get(id2).unwrap().expect("row2");
        assert!(!row2.accepted);
        assert!(row2.adopted_at.is_none());
        assert!(p.get(Uuid::new_v4()).unwrap().is_none());
    }

    #[test]
    fn mark_adopted_after_validation() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("prop3.db");
        let p = ProposalLedger::open(&db).unwrap();
        let text = "graph\nt1\n";
        let env = sample_envelope(text);
        let id = p.submit("a1", &env, text).expect("insert");
        p.validate_and_record(id, |s: &str| if s == text { Ok(()) } else { Err("bad") })
            .expect("validate");
        p.mark_adopted(id, "node-1").expect("adopted");
        let r = p.get(id).unwrap().expect("row");
        assert!(r.adopted_at.is_some());
        assert_eq!(r.adopted_graph_node_id.as_deref(), Some("node-1"));
    }
}
