//! SQLite ledger for improvement proposals.

use std::path::Path;

use ainl_contracts::ProposalEnvelope;
use rusqlite::OptionalExtension;
use thiserror::Error;
use uuid::Uuid;

use crate::proposed_hash_matches;

/// Row identifier in the ledger.
pub type ImprovementProposalId = Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImprovementProposalRow {
    pub id: ImprovementProposalId,
    pub agent_id: String,
    pub kind: String,
    pub original_hash: String,
    pub proposed_hash: String,
    pub accepted: bool,
    pub validation_error: Option<String>,
    pub created_at: i64,
    /// Set when the accepted proposal is materialized into the agent graph (`ainl_memory.db`).
    pub adopted_at: Option<i64>,
    pub adopted_graph_node_id: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct AdoptResult {
    pub id: ImprovementProposalId,
    pub accepted: bool,
    pub error: Option<String>,
}

/// Data needed to materialize an accepted proposal into the AINL graph.
#[derive(Debug, Clone, PartialEq)]
pub struct AdoptGraphPayload {
    pub row: ImprovementProposalRow,
    pub envelope: ProposalEnvelope,
    pub proposed_ainl_text: String,
}

#[derive(Error, Debug)]
pub enum ProposalLedgerError {
    #[error("hash mismatch: proposed_ainl text does not match ProposalEnvelope.proposed_hash")]
    HashMismatch,
    #[error("rusqlite: {0}")]
    Rusqlite(#[from] rusqlite::Error),
    #[error("json: {0}")]
    SerdeJson(#[from] serde_json::Error),
    #[error("adoption preconditions not met: row missing, not structurally accepted, or already adopted")]
    AdoptState,
}

/// Append-only + update-for-validation proposal store.
pub struct ProposalLedger {
    conn: rusqlite::Connection,
}

impl ProposalLedger {
    pub fn open(path: &Path) -> Result<Self, ProposalLedgerError> {
        let conn = rusqlite::Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS improvement_proposals (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL,
                kind TEXT NOT NULL,
                original_hash TEXT NOT NULL,
                proposed_hash TEXT NOT NULL,
                envelope_json TEXT NOT NULL,
                proposed_ainl_text TEXT NOT NULL,
                accepted INTEGER NOT NULL DEFAULT 0,
                validation_error TEXT,
                created_at INTEGER NOT NULL,
                adopted_at INTEGER,
                adopted_graph_node_id TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_improvement_proposals_agent
                ON improvement_proposals(agent_id, created_at DESC);
            ",
        )?;
        const MIG: &[&str] = &[
            "ALTER TABLE improvement_proposals ADD COLUMN adopted_at INTEGER",
            "ALTER TABLE improvement_proposals ADD COLUMN adopted_graph_node_id TEXT",
        ];
        for &sql in MIG {
            let _ = conn.execute(sql, []);
        }
        Ok(Self { conn })
    }

    /// Inserts a row after verifying the proposed AINL body matches `envelope.proposed_hash`.
    pub fn submit(
        &self,
        agent_id: &str,
        envelope: &ProposalEnvelope,
        proposed_ainl_text: &str,
    ) -> Result<ImprovementProposalId, ProposalLedgerError> {
        if !proposed_hash_matches(envelope, proposed_ainl_text) {
            return Err(ProposalLedgerError::HashMismatch);
        }
        let id = Uuid::new_v4();
        let now = chrono::Utc::now().timestamp();
        let env_json = serde_json::to_string(envelope)?;
        self.conn.execute(
            "INSERT INTO improvement_proposals (
                id, agent_id, kind, original_hash, proposed_hash,
                envelope_json, proposed_ainl_text, accepted, validation_error, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, NULL, ?8)",
            rusqlite::params![
                id.to_string(),
                agent_id,
                envelope.kind,
                envelope.original_hash,
                envelope.proposed_hash,
                env_json,
                proposed_ainl_text,
                now,
            ],
        )?;
        Ok(id)
    }

    /// Runs `validate` on the stored proposed text. On `Ok(())` sets `accepted=1`, else records
    /// the error and leaves `accepted=0` (rejected proposal never flips to accepted in a later
    /// call without a new row — regression tests rely on this).
    pub fn validate_and_record<V, E: std::fmt::Display>(
        &self,
        id: ImprovementProposalId,
        mut validate: V,
    ) -> Result<AdoptResult, ProposalLedgerError>
    where
        V: FnMut(&str) -> std::result::Result<(), E>,
    {
        let text: String = self.conn.query_row(
            "SELECT proposed_ainl_text FROM improvement_proposals WHERE id = ?1",
            [id.to_string()],
            |r| r.get(0),
        )?;
        match validate(&text) {
            Ok(()) => {
                self.conn.execute(
                    "UPDATE improvement_proposals SET accepted = 1, validation_error = NULL WHERE id = ?1",
                    [id.to_string()],
                )?;
                Ok(AdoptResult {
                    id,
                    accepted: true,
                    error: None,
                })
            }
            Err(e) => {
                let err = e.to_string();
                self.conn.execute(
                    "UPDATE improvement_proposals SET accepted = 0, validation_error = ?1 WHERE id = ?2",
                    rusqlite::params![err, id.to_string()],
                )?;
                Ok(AdoptResult {
                    id,
                    accepted: false,
                    error: Some(err),
                })
            }
        }
    }

    pub fn get(
        &self,
        id: ImprovementProposalId,
    ) -> Result<Option<ImprovementProposalRow>, ProposalLedgerError> {
        self.conn
            .query_row(
                "SELECT id, agent_id, kind, original_hash, proposed_hash, accepted, validation_error, created_at, adopted_at, adopted_graph_node_id
                 FROM improvement_proposals WHERE id = ?1",
                [id.to_string()],
                |r| {
                    let id_s: String = r.get(0)?;
                    Ok(ImprovementProposalRow {
                        id: Uuid::parse_str(&id_s)
                            .expect("improvement_proposals.id is a UUID string"),
                        agent_id: r.get(1)?,
                        kind: r.get(2)?,
                        original_hash: r.get(3)?,
                        proposed_hash: r.get(4)?,
                        accepted: r.get::<_, i64>(5)? != 0,
                        validation_error: r.get(6)?,
                        created_at: r.get(7)?,
                        adopted_at: r.get(8)?,
                        adopted_graph_node_id: r.get(9)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    /// After graph materialization, record adoption so a proposal cannot be applied twice.
    pub fn mark_adopted(
        &self,
        id: ImprovementProposalId,
        adopted_graph_node_id: &str,
    ) -> Result<(), ProposalLedgerError> {
        let now = chrono::Utc::now().timestamp();
        let n = self.conn.execute(
            "UPDATE improvement_proposals
             SET adopted_at = ?1, adopted_graph_node_id = ?2
             WHERE id = ?3 AND accepted = 1 AND adopted_at IS NULL",
            rusqlite::params![now, adopted_graph_node_id, id.to_string()],
        )?;
        if n == 0 {
            return Err(ProposalLedgerError::AdoptState);
        }
        Ok(())
    }

    /// Full row + parsed envelope and proposed AINL text (for host adoption to `ainl_memory.db`).
    pub fn get_for_graph_adopt(
        &self,
        id: ImprovementProposalId,
    ) -> Result<Option<AdoptGraphPayload>, ProposalLedgerError> {
        let raw: Option<(
            String, // 0 id
            String, // 1 agent_id
            String, // 2 kind
            String, // 3 original
            String, // 4 proposed
            i64,    // 5 accepted
            Option<String>, // 6 val err
            i64,    // 7 created
            Option<i64>,    // 8 adopted_at
            Option<String>, // 9 adopted graph id
            String, // 10 envelope json
            String, // 11 proposed text
        )> = self
            .conn
            .query_row(
                "SELECT id, agent_id, kind, original_hash, proposed_hash, accepted, validation_error, created_at, adopted_at, adopted_graph_node_id, envelope_json, proposed_ainl_text
                 FROM improvement_proposals WHERE id = ?1",
                [id.to_string()],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get::<_, i64>(5)?,
                        r.get(6)?,
                        r.get(7)?,
                        r.get(8)?,
                        r.get(9)?,
                        r.get(10)?,
                        r.get(11)?,
                    ))
                },
            )
            .optional()?;
        let Some(t) = raw else { return Ok(None) };
        let row = ImprovementProposalRow {
            id: Uuid::parse_str(&t.0).expect("id"),
            agent_id: t.1,
            kind: t.2,
            original_hash: t.3,
            proposed_hash: t.4,
            accepted: t.5 != 0,
            validation_error: t.6,
            created_at: t.7,
            adopted_at: t.8,
            adopted_graph_node_id: t.9,
        };
        let envelope: ProposalEnvelope = serde_json::from_str(&t.10)?;
        Ok(Some(AdoptGraphPayload {
            row,
            envelope,
            proposed_ainl_text: t.11,
        }))
    }

    /// Recent rows for an agent (newest first) — for dashboards; omits `proposed_ainl_text` and full envelope.
    pub fn list_recent(
        &self,
        agent_id: &str,
        limit: usize,
    ) -> Result<Vec<ImprovementProposalListItem>, ProposalLedgerError> {
        let cap = limit.clamp(1, 200);
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, kind, original_hash, proposed_hash, accepted, validation_error, created_at, adopted_at, adopted_graph_node_id
                 FROM improvement_proposals
                 WHERE agent_id = ?1
                 ORDER BY created_at DESC
                 LIMIT ?2",
            )?;
        let rows = stmt
            .query_map(rusqlite::params![agent_id, cap as i64], |r| {
                let id_s: String = r.get(0)?;
                Ok(ImprovementProposalListItem {
                    id: Uuid::parse_str(&id_s).expect("improvement_proposals.id is a UUID string"),
                    kind: r.get(1)?,
                    original_hash: r.get(2)?,
                    proposed_hash: r.get(3)?,
                    accepted: r.get::<_, i64>(4)? != 0,
                    validation_error: r.get(5)?,
                    created_at: r.get(6)?,
                    adopted_at: r.get(7)?,
                    adopted_graph_node_id: r.get(8)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}

/// Summary row for [`ProposalLedger::list_recent`] (no large text columns).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ImprovementProposalListItem {
    pub id: Uuid,
    pub kind: String,
    pub original_hash: String,
    pub proposed_hash: String,
    pub accepted: bool,
    pub validation_error: Option<String>,
    pub created_at: i64,
    pub adopted_at: Option<i64>,
    pub adopted_graph_node_id: Option<String>,
}
