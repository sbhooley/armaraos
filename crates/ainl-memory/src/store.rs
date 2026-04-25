//! Graph storage backends for AINL memory.
//!
//! Defines the [`GraphStore`] trait and the SQLite implementation.
//!
//! ## Referential integrity (SQLite)
//!
//! `ainl_graph_edges` uses real `FOREIGN KEY (from_id)` / `FOREIGN KEY (to_id)` references to
//! `ainl_graph_nodes(id)` with `ON DELETE CASCADE`. [`SqliteGraphStore::open`] and
//! [`SqliteGraphStore::from_connection`] run `PRAGMA foreign_keys = ON` on the handle.
//!
//! Databases created before these constraints used a plain edges table; [`SqliteGraphStore::ensure_schema`]
//! runs a one-time `migrate_edges_add_foreign_keys` rebuild. Edge rows whose endpoints
//! are missing from `ainl_graph_nodes` **cannot** be kept under FK rules and are **omitted** from
//! the migrated copy.
//!
//! ## Above the database (still recommended)
//!
//! - **Eager checks**: [`SqliteGraphStore::write_node_with_edges`], [`SqliteGraphStore::insert_graph_edge_checked`]
//!   give clear errors without relying on SQLite error text alone.
//! - **Repair / forensic import**: [`SqliteGraphStore::import_graph`] with `allow_dangling_edges: true`
//!   is the **supported** way to load snapshots that violate referential integrity: FK enforcement is
//!   disabled only for the duration of that import, then turned back on. Follow with
//!   [`SqliteGraphStore::validate_graph`] before resuming normal writes on the same connection.
//! - **Semantic graph checks**: [`SqliteGraphStore::validate_graph`] (agent-scoped edges, dangling
//!   diagnostics, cross-agent boundary counts, etc.) — orthogonal to FK row existence.
//!
//! SQLite tables integrate with existing openfang-memory schema where applicable.

use crate::node::{AinlMemoryNode, AinlNodeType, MemoryCategory, RuntimeStateNode};
use crate::snapshot::{
    AgentGraphSnapshot, DanglingEdgeDetail, GraphValidationReport, SnapshotEdge,
    SNAPSHOT_SCHEMA_VERSION,
};
use crate::trajectory_table::TrajectoryDetailRecord;
use ainl_contracts::{TrajectoryOutcome, TrajectoryStep};
use chrono::Utc;
use rusqlite::OptionalExtension;
use std::collections::HashSet;
use uuid::Uuid;

/// Typed failures for snapshot import.
#[derive(Debug, Clone)]
pub enum SnapshotImportError {
    UnsupportedSchemaVersion { got: String, expected: &'static str },
    Sqlite(String),
}

impl std::fmt::Display for SnapshotImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedSchemaVersion { got, expected } => write!(
                f,
                "unsupported snapshot schema_version '{got}'; expected '{expected}'"
            ),
            Self::Sqlite(e) => write!(f, "{e}"),
        }
    }
}

/// Typed failures for graph validation.
#[derive(Debug, Clone)]
pub enum GraphValidationError {
    Sqlite(String),
}

impl std::fmt::Display for GraphValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sqlite(e) => write!(f, "{e}"),
        }
    }
}

/// Graph memory storage trait - swappable backends
pub trait GraphStore {
    /// Write a node to storage
    fn write_node(&self, node: &AinlMemoryNode) -> Result<(), String>;

    /// Read a node by ID
    fn read_node(&self, id: Uuid) -> Result<Option<AinlMemoryNode>, String>;

    /// Query episodes since a given timestamp
    fn query_episodes_since(
        &self,
        since_timestamp: i64,
        limit: usize,
    ) -> Result<Vec<AinlMemoryNode>, String>;

    /// Find nodes by type
    fn find_by_type(&self, type_name: &str) -> Result<Vec<AinlMemoryNode>, String>;

    /// Walk edges from a node with a given label
    fn walk_edges(&self, from_id: Uuid, label: &str) -> Result<Vec<AinlMemoryNode>, String>;
}

/// SQLite implementation of GraphStore
///
/// Integrates with existing openfang-memory schema by adding two tables:
/// - `ainl_graph_nodes`: stores node payloads
/// - `ainl_graph_edges`: stores graph edges
pub struct SqliteGraphStore {
    conn: rusqlite::Connection,
}

fn enable_foreign_keys(conn: &rusqlite::Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch("PRAGMA foreign_keys = ON;")
}

fn migrate_edge_columns(conn: &rusqlite::Connection) -> Result<(), rusqlite::Error> {
    let mut stmt = conn.prepare("PRAGMA table_info(ainl_graph_edges)")?;
    let cols = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    if !cols.iter().any(|c| c == "weight") {
        conn.execute(
            "ALTER TABLE ainl_graph_edges ADD COLUMN weight REAL NOT NULL DEFAULT 1.0",
            [],
        )?;
    }
    if !cols.iter().any(|c| c == "metadata") {
        conn.execute("ALTER TABLE ainl_graph_edges ADD COLUMN metadata TEXT", [])?;
    }
    Ok(())
}

/// True when `ainl_graph_edges` declares at least one foreign-key reference (new schema).
fn edges_table_has_foreign_keys(conn: &rusqlite::Connection) -> Result<bool, rusqlite::Error> {
    let exists: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='ainl_graph_edges'",
        [],
        |r| r.get(0),
    )?;
    if exists == 0 {
        return Ok(false);
    }
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pragma_foreign_key_list('ainl_graph_edges')",
        [],
        |r| r.get(0),
    )?;
    Ok(n > 0)
}

/// Rebuild `ainl_graph_edges` with `FOREIGN KEY` constraints. Rows whose endpoints are missing
/// from `ainl_graph_nodes` are **dropped** (they cannot be represented under FK rules).
fn migrate_edges_add_foreign_keys(conn: &rusqlite::Connection) -> Result<(), rusqlite::Error> {
    if edges_table_has_foreign_keys(conn)? {
        return Ok(());
    }

    let exists: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='ainl_graph_edges'",
        [],
        |r| r.get(0),
    )?;
    if exists == 0 {
        return Ok(());
    }

    conn.execute("BEGIN IMMEDIATE", [])?;
    let res: Result<(), rusqlite::Error> = (|| {
        conn.execute("DROP INDEX IF EXISTS idx_ainl_edges_from", [])?;
        conn.execute(
            "ALTER TABLE ainl_graph_edges RENAME TO ainl_graph_edges__old",
            [],
        )?;
        conn.execute(
            r#"CREATE TABLE ainl_graph_edges (
                from_id TEXT NOT NULL,
                to_id TEXT NOT NULL,
                label TEXT NOT NULL,
                weight REAL NOT NULL DEFAULT 1.0,
                metadata TEXT,
                PRIMARY KEY (from_id, to_id, label),
                FOREIGN KEY (from_id) REFERENCES ainl_graph_nodes(id) ON DELETE CASCADE,
                FOREIGN KEY (to_id) REFERENCES ainl_graph_nodes(id) ON DELETE CASCADE
            )"#,
            [],
        )?;
        conn.execute(
            r#"INSERT INTO ainl_graph_edges (from_id, to_id, label, weight, metadata)
               SELECT o.from_id, o.to_id, o.label,
                      COALESCE(o.weight, 1.0),
                      o.metadata
               FROM ainl_graph_edges__old o
               WHERE EXISTS (SELECT 1 FROM ainl_graph_nodes n WHERE n.id = o.from_id)
                 AND EXISTS (SELECT 1 FROM ainl_graph_nodes n2 WHERE n2.id = o.to_id)"#,
            [],
        )?;
        conn.execute("DROP TABLE ainl_graph_edges__old", [])?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_ainl_edges_from ON ainl_graph_edges(from_id, label)",
            [],
        )?;
        Ok(())
    })();

    match res {
        Ok(()) => {
            conn.execute("COMMIT", [])?;
        }
        Err(e) => {
            let _ = conn.execute("ROLLBACK", []);
            return Err(e);
        }
    }
    Ok(())
}

fn node_type_name(node: &AinlMemoryNode) -> &'static str {
    match &node.node_type {
        AinlNodeType::Episode { .. } => "episode",
        AinlNodeType::Semantic { .. } => "semantic",
        AinlNodeType::Procedural { .. } => "procedural",
        AinlNodeType::Persona { .. } => "persona",
        AinlNodeType::RuntimeState { .. } => "runtime_state",
        AinlNodeType::Trajectory { .. } => "trajectory",
        AinlNodeType::Failure { .. } => "failure",
    }
}

fn node_timestamp(node: &AinlMemoryNode) -> i64 {
    match &node.node_type {
        AinlNodeType::Episode { episodic } => episodic.timestamp,
        AinlNodeType::RuntimeState { runtime_state } => runtime_state.updated_at,
        AinlNodeType::Trajectory { trajectory } => trajectory.recorded_at,
        AinlNodeType::Failure { failure } => failure.recorded_at,
        _ => chrono::Utc::now().timestamp(),
    }
}

fn failure_fts_body(node: &AinlMemoryNode) -> Option<String> {
    match &node.node_type {
        AinlNodeType::Failure { failure } => Some(format!(
            "{} {} {} {} {}",
            failure.source,
            failure.tool_name.as_deref().unwrap_or(""),
            failure
                .source_namespace
                .as_deref()
                .unwrap_or(""),
            failure.source_tool.as_deref().unwrap_or(""),
            failure.message
        )),
        _ => None,
    }
}

/// Token-prefix AND query for FTS5 `body MATCH` (returns empty → skip search).
fn fts5_prefix_match_query(raw: &str) -> String {
    raw.split_whitespace()
        .filter(|t| !t.is_empty())
        .filter_map(|t| {
            let esc: String = t.chars().filter(|c| !c.is_control() && *c != '"').collect();
            if esc.is_empty() {
                return None;
            }
            Some(format!("\"{esc}*\""))
        })
        .collect::<Vec<_>>()
        .join(" AND ")
}

fn sync_failure_fts_insert(
    conn: &rusqlite::Connection,
    node_id: &str,
    body: &str,
) -> Result<(), String> {
    conn.execute(
        "DELETE FROM ainl_failures_fts WHERE node_id = ?1",
        [node_id],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO ainl_failures_fts(node_id, body) VALUES (?1, ?2)",
        rusqlite::params![node_id, body],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Full JSON payload (all node kinds) for `ainl_nodes_fts` — generic graph search.
fn graph_node_fts_body_from_payload_json(payload: &str) -> String {
    if payload.chars().count() > 400_000 {
        payload.chars().take(400_000).collect()
    } else {
        payload.to_string()
    }
}

fn sync_all_nodes_fts_insert(
    conn: &rusqlite::Connection,
    node_id: &str,
    agent_id: &str,
    project_id: Option<&str>,
    body: &str,
) -> Result<(), String> {
    let proj = project_id.map(str::trim).filter(|s| !s.is_empty());
    conn.execute("DELETE FROM ainl_nodes_fts WHERE node_id = ?1", [node_id])
        .map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO ainl_nodes_fts(node_id, agent_id, project_id, body) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![node_id, agent_id, proj, body],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn persist_edge(
    conn: &rusqlite::Connection,
    from_id: Uuid,
    to_id: Uuid,
    label: &str,
    weight: f32,
    metadata: Option<&str>,
) -> Result<(), String> {
    conn.execute(
        "INSERT OR REPLACE INTO ainl_graph_edges (from_id, to_id, label, weight, metadata)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![
            from_id.to_string(),
            to_id.to_string(),
            label,
            weight,
            metadata
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// All `ainl_graph_edges` rows whose endpoints are both present in `id_set`, as [`SnapshotEdge`] values.
fn collect_snapshot_edges_for_id_set(
    conn: &rusqlite::Connection,
    id_set: &HashSet<String>,
) -> Result<Vec<SnapshotEdge>, String> {
    let mut edge_stmt = conn
        .prepare("SELECT from_id, to_id, label, weight, metadata FROM ainl_graph_edges")
        .map_err(|e| e.to_string())?;
    let edge_rows = edge_stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, f64>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    let mut edges = Vec::new();
    for (from_id, to_id, label, weight, meta) in edge_rows {
        if !id_set.contains(&from_id) || !id_set.contains(&to_id) {
            continue;
        }
        let source_id = Uuid::parse_str(&from_id).map_err(|e| e.to_string())?;
        let target_id = Uuid::parse_str(&to_id).map_err(|e| e.to_string())?;
        let metadata = match meta {
            Some(s) if !s.is_empty() => Some(serde_json::from_str(&s).map_err(|e| e.to_string())?),
            _ => None,
        };
        edges.push(SnapshotEdge {
            source_id,
            target_id,
            edge_type: label,
            weight: weight as f32,
            metadata,
        });
    }
    Ok(edges)
}

fn persist_node(conn: &rusqlite::Connection, node: &AinlMemoryNode) -> Result<(), String> {
    let payload = serde_json::to_string(node).map_err(|e| e.to_string())?;
    let type_name = node_type_name(node);
    let timestamp = node_timestamp(node);
    let proj = node
        .project_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    conn.execute(
        "INSERT OR REPLACE INTO ainl_graph_nodes (id, node_type, payload, timestamp, project_id)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![node.id.to_string(), type_name, payload, timestamp, proj,],
    )
    .map_err(|e| e.to_string())?;

    for edge in &node.edges {
        persist_edge(
            conn,
            node.id,
            edge.target_id,
            &edge.label,
            1.0,
            None::<&str>,
        )?;
    }

    let body_all = graph_node_fts_body_from_payload_json(&payload);
    if !node.agent_id.trim().is_empty() {
        let _ = sync_all_nodes_fts_insert(
            conn,
            &node.id.to_string(),
            node.agent_id.as_str(),
            proj,
            &body_all,
        );
    }

    if let Some(body) = failure_fts_body(node) {
        // Best-effort: graph row is authoritative; FTS is auxiliary for search.
        let _ = sync_failure_fts_insert(conn, &node.id.to_string(), &body);
    }

    Ok(())
}

fn try_insert_node_ignore(
    conn: &rusqlite::Connection,
    node: &AinlMemoryNode,
) -> Result<(), String> {
    let payload = serde_json::to_string(node).map_err(|e| e.to_string())?;
    let type_name = node_type_name(node);
    let timestamp = node_timestamp(node);
    let proj = node
        .project_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let n = conn
        .execute(
            "INSERT OR IGNORE INTO ainl_graph_nodes (id, node_type, payload, timestamp, project_id)
         VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![node.id.to_string(), type_name, payload, timestamp, proj,],
        )
        .map_err(|e| e.to_string())?;
    if n > 0 {
        if !node.agent_id.trim().is_empty() {
            let body_all = graph_node_fts_body_from_payload_json(&payload);
            let _ = sync_all_nodes_fts_insert(
                conn,
                &node.id.to_string(),
                node.agent_id.as_str(),
                proj,
                &body_all,
            );
        }
        if let Some(body) = failure_fts_body(node) {
            let _ = sync_failure_fts_insert(conn, &node.id.to_string(), &body);
        }
    }
    Ok(())
}

fn try_insert_edge_ignore(conn: &rusqlite::Connection, edge: &SnapshotEdge) -> Result<(), String> {
    let meta = match &edge.metadata {
        Some(v) => Some(serde_json::to_string(v).map_err(|e| e.to_string())?),
        None => None,
    };
    conn.execute(
        "INSERT OR IGNORE INTO ainl_graph_edges (from_id, to_id, label, weight, metadata)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![
            edge.source_id.to_string(),
            edge.target_id.to_string(),
            edge.edge_type,
            edge.weight,
            meta.as_deref(),
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn migrate_failures_fts_v1(conn: &rusqlite::Connection) -> Result<(), rusqlite::Error> {
    conn.execute(
        "CREATE VIRTUAL TABLE IF NOT EXISTS ainl_failures_fts USING fts5(
            node_id UNINDEXED,
            body,
            tokenize = 'unicode61 remove_diacritics 1'
        )",
        [],
    )?;
    Ok(())
}

fn migrate_ainl_graph_nodes_add_project_id(
    conn: &rusqlite::Connection,
) -> Result<(), rusqlite::Error> {
    let mut stmt = conn.prepare("PRAGMA table_info(ainl_graph_nodes)")?;
    let cols = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    if !cols.iter().any(|c| c == "project_id") {
        conn.execute(
            "ALTER TABLE ainl_graph_nodes ADD COLUMN project_id TEXT",
            [],
        )?;
    }
    Ok(())
}

fn migrate_ainl_nodes_fts_v1(conn: &rusqlite::Connection) -> Result<(), rusqlite::Error> {
    conn.execute(
        "CREATE VIRTUAL TABLE IF NOT EXISTS ainl_nodes_fts USING fts5(
            node_id UNINDEXED,
            agent_id UNINDEXED,
            project_id UNINDEXED,
            body,
            tokenize = 'unicode61 remove_diacritics 1'
        )",
        [],
    )?;
    Ok(())
}

/// Remove FTS shadow rows when the primary graph row is deleted (`DELETE` from `ainl_graph_nodes`).
fn install_ainl_graph_node_delete_fts_triggers(
    conn: &rusqlite::Connection,
) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "DROP TRIGGER IF EXISTS ainl_graph_nodes_after_delete_fts;
         CREATE TRIGGER ainl_graph_nodes_after_delete_fts
         AFTER DELETE ON ainl_graph_nodes
         FOR EACH ROW
         BEGIN
           DELETE FROM ainl_nodes_fts WHERE node_id = OLD.id;
           DELETE FROM ainl_failures_fts WHERE node_id = OLD.id;
         END;",
    )?;
    Ok(())
}

/// One-time: populate `ainl_nodes_fts` from existing `ainl_graph_nodes` (legacy DBs).
fn backfill_ainl_nodes_fts_if_empty(conn: &rusqlite::Connection) -> Result<(), String> {
    let fts_n: i64 = conn
        .query_row("SELECT COUNT(*) FROM ainl_nodes_fts", [], |r| r.get(0))
        .unwrap_or(0);
    if fts_n > 0 {
        return Ok(());
    }
    let mut stmt = conn
        .prepare(
            "SELECT id, payload, project_id FROM ainl_graph_nodes
             WHERE TRIM(COALESCE(json_extract(payload, '$.agent_id'), '')) != ''",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    for (id, payload, col_proj) in rows {
        let v: serde_json::Value = match serde_json::from_str(&payload) {
            Ok(x) => x,
            Err(_) => continue,
        };
        let ag = v
            .get("agent_id")
            .and_then(|x| x.as_str())
            .map(str::trim)
            .unwrap_or("");
        if ag.is_empty() {
            continue;
        }
        let json_proj = v
            .get("project_id")
            .and_then(|x| x.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let proj = col_proj
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .or(json_proj);
        let body = graph_node_fts_body_from_payload_json(&payload);
        if sync_all_nodes_fts_insert(conn, &id, ag, proj, &body).is_err() {
            // Best-effort backfill: continue with remaining rows.
        }
    }
    Ok(())
}

fn migrate_trajectories_v1(conn: &rusqlite::Connection) -> Result<(), rusqlite::Error> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS ainl_trajectories (
            id TEXT PRIMARY KEY,
            episode_id TEXT NOT NULL,
            graph_trajectory_node_id TEXT,
            agent_id TEXT NOT NULL,
            session_id TEXT NOT NULL,
            project_id TEXT,
            recorded_at INTEGER NOT NULL,
            outcome_json TEXT NOT NULL,
            ainl_source_hash TEXT,
            duration_ms INTEGER NOT NULL DEFAULT 0,
            steps_json TEXT NOT NULL,
            FOREIGN KEY (episode_id) REFERENCES ainl_graph_nodes(id) ON DELETE CASCADE
        )",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_ainl_traj_agent_time
         ON ainl_trajectories(agent_id, recorded_at DESC)",
        [],
    )?;
    Ok(())
}

fn migrate_ainl_trajectories_add_depth_v1(
    conn: &rusqlite::Connection,
) -> Result<(), rusqlite::Error> {
    let mut stmt = conn.prepare("PRAGMA table_info(ainl_trajectories)")?;
    let cols: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    if !cols.iter().any(|c| c == "frame_vars_json") {
        conn.execute(
            "ALTER TABLE ainl_trajectories ADD COLUMN frame_vars_json TEXT",
            [],
        )?;
    }
    if !cols.iter().any(|c| c == "fitness_delta") {
        conn.execute(
            "ALTER TABLE ainl_trajectories ADD COLUMN fitness_delta REAL",
            [],
        )?;
    }
    Ok(())
}

impl SqliteGraphStore {
    /// Ensure the AINL graph schema exists in the database
    pub fn ensure_schema(conn: &rusqlite::Connection) -> Result<(), rusqlite::Error> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS ainl_graph_nodes (
                id TEXT PRIMARY KEY,
                node_type TEXT NOT NULL,
                payload TEXT NOT NULL,
                timestamp INTEGER NOT NULL
            )",
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_ainl_nodes_timestamp
             ON ainl_graph_nodes(timestamp DESC)",
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_ainl_nodes_type
             ON ainl_graph_nodes(node_type)",
            [],
        )?;

        migrate_ainl_graph_nodes_add_project_id(conn)?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_ainl_nodes_project_type_time
             ON ainl_graph_nodes(project_id, node_type, timestamp)",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS ainl_graph_edges (
                from_id TEXT NOT NULL,
                to_id TEXT NOT NULL,
                label TEXT NOT NULL,
                weight REAL NOT NULL DEFAULT 1.0,
                metadata TEXT,
                PRIMARY KEY (from_id, to_id, label),
                FOREIGN KEY (from_id) REFERENCES ainl_graph_nodes(id) ON DELETE CASCADE,
                FOREIGN KEY (to_id) REFERENCES ainl_graph_nodes(id) ON DELETE CASCADE
            )",
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_ainl_edges_from
             ON ainl_graph_edges(from_id, label)",
            [],
        )?;

        migrate_edge_columns(conn)?;
        migrate_edges_add_foreign_keys(conn)?;
        migrate_trajectories_v1(conn)?;
        migrate_ainl_trajectories_add_depth_v1(conn)?;
        migrate_failures_fts_v1(conn)?;
        migrate_ainl_nodes_fts_v1(conn)?;
        if backfill_ainl_nodes_fts_if_empty(conn).is_err() {
            // Non-fatal: new DBs may have empty graph; legacy rows can be re-synced on next write.
        }
        let _ = install_ainl_graph_node_delete_fts_triggers(conn);
        Ok(())
    }

    /// Open/create a graph store at the given path
    pub fn open(path: &std::path::Path) -> Result<Self, String> {
        let conn = rusqlite::Connection::open(path).map_err(|e| e.to_string())?;
        enable_foreign_keys(&conn).map_err(|e| e.to_string())?;
        Self::ensure_schema(&conn).map_err(|e| e.to_string())?;
        Ok(Self { conn })
    }

    /// Create from an existing connection (for integration with openfang-memory pool)
    pub fn from_connection(conn: rusqlite::Connection) -> Result<Self, String> {
        enable_foreign_keys(&conn).map_err(|e| e.to_string())?;
        Self::ensure_schema(&conn).map_err(|e| e.to_string())?;
        Ok(Self { conn })
    }

    /// Low-level access for query builders in this crate.
    pub(crate) fn conn(&self) -> &rusqlite::Connection {
        &self.conn
    }

    /// Insert a directed edge between two node IDs (separate from per-node edge payloads).
    pub fn insert_graph_edge(&self, from_id: Uuid, to_id: Uuid, label: &str) -> Result<(), String> {
        persist_edge(&self.conn, from_id, to_id, label, 1.0, None)
    }

    /// Like [`Self::insert_graph_edge`], but verifies both endpoints exist first (clear errors for strict runtime wiring).
    pub fn insert_graph_edge_checked(
        &self,
        from_id: Uuid,
        to_id: Uuid,
        label: &str,
    ) -> Result<(), String> {
        if !self.node_row_exists(&from_id.to_string())? {
            return Err(format!(
                "insert_graph_edge_checked: missing source node row {}",
                from_id
            ));
        }
        if !self.node_row_exists(&to_id.to_string())? {
            return Err(format!(
                "insert_graph_edge_checked: missing target node row {}",
                to_id
            ));
        }
        self.insert_graph_edge(from_id, to_id, label)
    }

    /// Same as [`Self::insert_graph_edge`], with optional edge weight and JSON metadata.
    pub fn insert_graph_edge_with_meta(
        &self,
        from_id: Uuid,
        to_id: Uuid,
        label: &str,
        weight: f32,
        metadata: Option<&serde_json::Value>,
    ) -> Result<(), String> {
        let meta = metadata
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| e.to_string())?;
        persist_edge(&self.conn, from_id, to_id, label, weight, meta.as_deref())
    }

    /// Nodes of a given `node_type` with `timestamp >= since_timestamp`, most recent first.
    pub fn query_nodes_by_type_since(
        &self,
        node_type: &str,
        since_timestamp: i64,
        limit: usize,
    ) -> Result<Vec<AinlMemoryNode>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT payload FROM ainl_graph_nodes
                 WHERE node_type = ?1 AND timestamp >= ?2
                 ORDER BY timestamp DESC
                 LIMIT ?3",
            )
            .map_err(|e| e.to_string())?;

        let rows = stmt
            .query_map(
                rusqlite::params![node_type, since_timestamp, limit as i64],
                |row| {
                    let payload: String = row.get(0)?;
                    Ok(payload)
                },
            )
            .map_err(|e| e.to_string())?;

        let mut nodes = Vec::new();
        for row in rows {
            let payload = row.map_err(|e| e.to_string())?;
            let node: AinlMemoryNode = serde_json::from_str(&payload).map_err(|e| e.to_string())?;
            nodes.push(node);
        }

        Ok(nodes)
    }

    /// Read the most recent persisted [`RuntimeStateNode`] for `agent_id`, if any.
    ///
    /// Rows are stored with `node_type = 'runtime_state'` and JSON `$.node_type.runtime_state.agent_id` matching the agent.
    pub fn read_runtime_state(&self, agent_id: &str) -> Result<Option<RuntimeStateNode>, String> {
        if agent_id.is_empty() {
            return Ok(None);
        }
        let mut stmt = self
            .conn
            .prepare(
                "SELECT payload FROM ainl_graph_nodes
                 WHERE node_type = 'runtime_state'
                   AND json_extract(payload, '$.node_type.runtime_state.agent_id') = ?1
                 ORDER BY timestamp DESC
                 LIMIT 1",
            )
            .map_err(|e| e.to_string())?;

        let payload_opt: Option<String> = stmt
            .query_row([agent_id], |row| row.get(0))
            .optional()
            .map_err(|e| e.to_string())?;

        let Some(payload) = payload_opt else {
            return Ok(None);
        };

        let node: AinlMemoryNode = serde_json::from_str(&payload).map_err(|e| e.to_string())?;
        match node.node_type {
            AinlNodeType::RuntimeState { runtime_state } => Ok(Some(runtime_state)),
            _ => Err("runtime_state row had unexpected node_type payload".to_string()),
        }
    }

    /// Upsert one [`RuntimeStateNode`] row per agent (stable id via [`Uuid::new_v5`]).
    pub fn write_runtime_state(&self, state: &RuntimeStateNode) -> Result<(), String> {
        let id = Uuid::new_v5(&Uuid::NAMESPACE_OID, state.agent_id.as_bytes());
        let node = AinlMemoryNode {
            id,
            memory_category: MemoryCategory::RuntimeState,
            importance_score: 0.5,
            agent_id: state.agent_id.clone(),
            project_id: None,
            node_type: AinlNodeType::RuntimeState {
                runtime_state: state.clone(),
            },
            edges: Vec::new(),
        };
        self.write_node(&node)
    }

    /// Write a node and its embedded edges in one transaction; fails if any edge target is missing.
    pub fn write_node_with_edges(&mut self, node: &AinlMemoryNode) -> Result<(), String> {
        let tx = self.conn.transaction().map_err(|e| e.to_string())?;
        for edge in &node.edges {
            let exists: Option<i32> = tx
                .query_row(
                    "SELECT 1 FROM ainl_graph_nodes WHERE id = ?1",
                    [edge.target_id.to_string()],
                    |_| Ok(1),
                )
                .optional()
                .map_err(|e| e.to_string())?;
            if exists.is_none() {
                return Err(format!(
                    "write_node_with_edges: missing target node {}",
                    edge.target_id
                ));
            }
        }
        persist_node(&tx, node)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Validate structural integrity for one agent's induced subgraph.
    pub fn validate_graph(&self, agent_id: &str) -> Result<GraphValidationReport, String> {
        self.validate_graph_checked(agent_id)
            .map_err(|e| e.to_string())
    }

    /// Typed validation variant for callers that need structured error handling.
    pub fn validate_graph_checked(
        &self,
        agent_id: &str,
    ) -> Result<GraphValidationReport, GraphValidationError> {
        use std::collections::HashSet;

        let agent_nodes = self
            .agent_node_ids(agent_id)
            .map_err(GraphValidationError::Sqlite)?;
        let node_count = agent_nodes.len();

        let mut stmt = self
            .conn
            .prepare("SELECT from_id, to_id, label FROM ainl_graph_edges")
            .map_err(|e| GraphValidationError::Sqlite(e.to_string()))?;
        let all_edges: Vec<(String, String, String)> = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| GraphValidationError::Sqlite(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| GraphValidationError::Sqlite(e.to_string()))?;

        let mut edge_pairs = Vec::new();
        for (from_id, to_id, label) in all_edges {
            let touches_agent = agent_nodes.contains(&from_id) || agent_nodes.contains(&to_id);
            if touches_agent {
                edge_pairs.push((from_id, to_id, label));
            }
        }

        let edge_count = edge_pairs.len();
        let mut dangling_edges = Vec::new();
        let mut dangling_edge_details = Vec::new();
        let mut cross_agent_boundary_edges = 0usize;

        for (from_id, to_id, label) in &edge_pairs {
            let from_ok = self
                .node_row_exists(from_id)
                .map_err(GraphValidationError::Sqlite)?;
            let to_ok = self
                .node_row_exists(to_id)
                .map_err(GraphValidationError::Sqlite)?;
            if !from_ok || !to_ok {
                dangling_edges.push((from_id.clone(), to_id.clone()));
                dangling_edge_details.push(DanglingEdgeDetail {
                    source_id: from_id.clone(),
                    target_id: to_id.clone(),
                    edge_type: label.clone(),
                });
                continue;
            }
            let fa = agent_nodes.contains(from_id);
            let ta = agent_nodes.contains(to_id);
            if fa ^ ta {
                cross_agent_boundary_edges += 1;
            }
        }

        let mut touched: HashSet<String> =
            HashSet::with_capacity(edge_pairs.len().saturating_mul(2));
        for (a, b, _) in &edge_pairs {
            if agent_nodes.contains(a) {
                touched.insert(a.clone());
            }
            if agent_nodes.contains(b) {
                touched.insert(b.clone());
            }
        }

        let mut orphan_nodes = Vec::new();
        for id in &agent_nodes {
            if !touched.contains(id) {
                orphan_nodes.push(id.clone());
            }
        }

        let is_valid = dangling_edges.is_empty();
        Ok(GraphValidationReport {
            agent_id: agent_id.to_string(),
            node_count,
            edge_count,
            dangling_edges,
            dangling_edge_details,
            cross_agent_boundary_edges,
            orphan_nodes,
            is_valid,
        })
    }

    fn node_row_exists(&self, id: &str) -> Result<bool, String> {
        let v: Option<i32> = self
            .conn
            .query_row("SELECT 1 FROM ainl_graph_nodes WHERE id = ?1", [id], |_| {
                Ok(1)
            })
            .optional()
            .map_err(|e| e.to_string())?;
        Ok(v.is_some())
    }

    fn agent_node_ids(&self, agent_id: &str) -> Result<HashSet<String>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id FROM ainl_graph_nodes
                 WHERE COALESCE(json_extract(payload, '$.agent_id'), '') = ?1",
            )
            .map_err(|e| e.to_string())?;
        let ids = stmt
            .query_map([agent_id], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<HashSet<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(ids)
    }

    /// Directed edges where **both** endpoints are nodes owned by `agent_id` (aligned with [`Self::export_graph`] edge set).
    pub fn agent_subgraph_edges(&self, agent_id: &str) -> Result<Vec<SnapshotEdge>, String> {
        let id_set = self.agent_node_ids(agent_id)?;
        collect_snapshot_edges_for_id_set(&self.conn, &id_set)
    }

    /// Export all nodes and interconnecting edges for `agent_id`.
    pub fn export_graph(&self, agent_id: &str) -> Result<AgentGraphSnapshot, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT payload FROM ainl_graph_nodes
                 WHERE COALESCE(json_extract(payload, '$.agent_id'), '') = ?1",
            )
            .map_err(|e| e.to_string())?;
        let nodes: Vec<AinlMemoryNode> = stmt
            .query_map([agent_id], |row| {
                let payload: String = row.get(0)?;
                Ok(payload)
            })
            .map_err(|e| e.to_string())?
            .map(|r| {
                let payload = r.map_err(|e| e.to_string())?;
                serde_json::from_str(&payload).map_err(|e| e.to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;

        let id_set: std::collections::HashSet<String> =
            nodes.iter().map(|n| n.id.to_string()).collect();

        let edges = collect_snapshot_edges_for_id_set(&self.conn, &id_set)?;

        Ok(AgentGraphSnapshot {
            agent_id: agent_id.to_string(),
            exported_at: Utc::now(),
            schema_version: std::borrow::Cow::Borrowed(SNAPSHOT_SCHEMA_VERSION),
            nodes,
            edges,
        })
    }

    /// Import a snapshot in one transaction (`INSERT OR IGNORE` per row).
    ///
    /// * `allow_dangling_edges == false` (**default / production**): `PRAGMA foreign_keys` stays
    ///   enabled; every edge must reference existing node rows after inserts (same invariants as
    ///   [`Self::write_node_with_edges`]).
    /// * `allow_dangling_edges == true` (**repair / forensic**): FK checks are disabled only for
    ///   this import so partially invalid snapshots can be loaded; run [`Self::validate_graph`]
    ///   afterward and repair before returning to normal writes.
    pub fn import_graph(
        &mut self,
        snapshot: &AgentGraphSnapshot,
        allow_dangling_edges: bool,
    ) -> Result<(), String> {
        self.import_graph_checked(snapshot, allow_dangling_edges)
            .map_err(|e| e.to_string())
    }

    /// Typed import variant for callers that want structured error handling.
    pub fn import_graph_checked(
        &mut self,
        snapshot: &AgentGraphSnapshot,
        allow_dangling_edges: bool,
    ) -> Result<(), SnapshotImportError> {
        if snapshot.schema_version.as_ref() != SNAPSHOT_SCHEMA_VERSION {
            return Err(SnapshotImportError::UnsupportedSchemaVersion {
                got: snapshot.schema_version.to_string(),
                expected: SNAPSHOT_SCHEMA_VERSION,
            });
        }

        if allow_dangling_edges {
            self.conn
                .execute_batch("PRAGMA foreign_keys = OFF;")
                .map_err(|e| SnapshotImportError::Sqlite(e.to_string()))?;
        }

        let result: Result<(), SnapshotImportError> = (|| {
            let tx = self
                .conn
                .transaction()
                .map_err(|e| SnapshotImportError::Sqlite(e.to_string()))?;
            for node in &snapshot.nodes {
                try_insert_node_ignore(&tx, node).map_err(SnapshotImportError::Sqlite)?;
            }
            for edge in &snapshot.edges {
                try_insert_edge_ignore(&tx, edge).map_err(SnapshotImportError::Sqlite)?;
            }
            tx.commit()
                .map_err(|e| SnapshotImportError::Sqlite(e.to_string()))?;
            Ok(())
        })();

        if allow_dangling_edges {
            self.conn
                .execute_batch("PRAGMA foreign_keys = ON;")
                .map_err(|e| SnapshotImportError::Sqlite(e.to_string()))?;
        }

        result
    }

    /// Large-step trajectory row (sibling table); episode row must exist first.
    pub fn insert_trajectory_detail(&self, row: &TrajectoryDetailRecord) -> Result<(), String> {
        let steps_json = serde_json::to_string(&row.steps).map_err(|e| e.to_string())?;
        let outcome_json = serde_json::to_string(&row.outcome).map_err(|e| e.to_string())?;
        let frame_s = match &row.frame_vars {
            None => None,
            Some(v) => Some(serde_json::to_string(v).map_err(|e| e.to_string())?),
        };
        self.conn
            .execute(
                "INSERT OR REPLACE INTO ainl_trajectories (
                    id, episode_id, graph_trajectory_node_id, agent_id, session_id, project_id,
                    recorded_at, outcome_json, ainl_source_hash, duration_ms, steps_json,
                    frame_vars_json, fitness_delta
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                rusqlite::params![
                    row.id.to_string(),
                    row.episode_id.to_string(),
                    row.graph_trajectory_node_id.map(|u| u.to_string()),
                    row.agent_id,
                    row.session_id,
                    row.project_id,
                    row.recorded_at,
                    outcome_json,
                    row.ainl_source_hash,
                    row.duration_ms as i64,
                    steps_json,
                    frame_s,
                    row.fitness_delta,
                ],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Recent trajectory detail rows for an agent (newest first).
    pub fn list_trajectories_for_agent(
        &self,
        agent_id: &str,
        limit: usize,
        since_timestamp: Option<i64>,
    ) -> Result<Vec<TrajectoryDetailRecord>, String> {
        let cap = limit.clamp(1, 500) as i64;
        let sql = if since_timestamp.is_some() {
            "SELECT id, episode_id, graph_trajectory_node_id, agent_id, session_id, project_id,
                    recorded_at, outcome_json, ainl_source_hash, duration_ms, steps_json,
                    frame_vars_json, fitness_delta
             FROM ainl_trajectories
             WHERE agent_id = ?1 AND recorded_at >= ?2
             ORDER BY recorded_at DESC
             LIMIT ?3"
        } else {
            "SELECT id, episode_id, graph_trajectory_node_id, agent_id, session_id, project_id,
                    recorded_at, outcome_json, ainl_source_hash, duration_ms, steps_json,
                    frame_vars_json, fitness_delta
             FROM ainl_trajectories
             WHERE agent_id = ?1
             ORDER BY recorded_at DESC
             LIMIT ?2"
        };

        let mut stmt = self.conn.prepare(sql).map_err(|e| e.to_string())?;
        let rows = if let Some(since) = since_timestamp {
            stmt.query_map(rusqlite::params![agent_id, since, cap], map_trajectory_row)
        } else {
            stmt.query_map(rusqlite::params![agent_id, cap], map_trajectory_row)
        }
        .map_err(|e| e.to_string())?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| e.to_string())?);
        }
        Ok(out)
    }

    /// Count `ainl_trajectories` rows for `agent_id` with `recorded_at` **strictly before** `before_unix` (seconds).
    pub fn count_trajectory_details_before(
        &self,
        agent_id: &str,
        before_unix: i64,
    ) -> Result<usize, String> {
        if agent_id.trim().is_empty() {
            return Err("agent_id is empty".into());
        }
        let n: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM ainl_trajectories WHERE agent_id = ?1 AND recorded_at < ?2",
                rusqlite::params![agent_id, before_unix],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        Ok(n as usize)
    }

    /// Delete `ainl_trajectories` detail rows for `agent_id` with `recorded_at` **strictly before**
    /// `before_unix` (seconds). Returns the number of rows removed.
    ///
    /// This does **not** delete `Trajectory` nodes from `ainl_graph_nodes` or any edges; use graph
    /// export / repair paths if you need a fully consistent graph after bulk pruning.
    pub fn delete_trajectory_details_before(
        &self,
        agent_id: &str,
        before_unix: i64,
    ) -> Result<usize, String> {
        if agent_id.trim().is_empty() {
            return Err("agent_id is empty".into());
        }
        let n = self
            .conn
            .execute(
                "DELETE FROM ainl_trajectories WHERE agent_id = ?1 AND recorded_at < ?2",
                rusqlite::params![agent_id, before_unix],
            )
            .map_err(|e| e.to_string())?;
        Ok(n)
    }

    /// Full-text search over persisted failure nodes for one agent (`node_type = failure`).
    ///
    /// Returns matching [`AinlMemoryNode`] rows (newest first). Invalid FTS syntax yields an empty list.
    pub fn search_failures_fts_for_agent(
        &self,
        agent_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<AinlMemoryNode>, String> {
        let fts_q = fts5_prefix_match_query(query);
        if fts_q.is_empty() || agent_id.trim().is_empty() {
            return Ok(Vec::new());
        }
        let cap = limit.clamp(1, 200) as i64;
        let mut stmt = self
            .conn
            .prepare(
                "SELECT n.payload
                 FROM ainl_failures_fts AS f
                 INNER JOIN ainl_graph_nodes AS n ON n.id = f.node_id
                 WHERE n.node_type = 'failure'
                   AND json_extract(n.payload, '$.agent_id') = ?1
                   AND f.body MATCH ?2
                 ORDER BY n.timestamp DESC
                 LIMIT ?3",
            )
            .map_err(|e| e.to_string())?;

        let rows = stmt.query_map(rusqlite::params![agent_id, fts_q, cap], |row| {
            let payload: String = row.get(0)?;
            Ok(payload)
        });

        let mut out = Vec::new();
        let rows = match rows {
            Ok(r) => r,
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("fts5") || msg.to_ascii_lowercase().contains("syntax") {
                    return Ok(Vec::new());
                }
                return Err(msg);
            }
        };
        for row in rows {
            match row {
                Ok(payload) => {
                    if let Ok(node) = serde_json::from_str::<AinlMemoryNode>(&payload) {
                        out.push(node);
                    }
                }
                Err(e) => {
                    let msg = e.to_string();
                    if msg.contains("fts5") || msg.to_ascii_lowercase().contains("syntax") {
                        return Ok(Vec::new());
                    }
                    return Err(msg);
                }
            }
        }
        Ok(out)
    }

    /// Full-text search over all graph node JSON payloads (see `ainl_nodes_fts`), scoped by agent.
    ///
    /// `project_id`: when `Some` (non-empty), return matches whose stored project is empty/NULL
    /// **or** equal to that id (so legacy unscoped rows remain visible in a project workspace).
    /// When `None`, do not filter by project.
    pub fn search_all_nodes_fts_for_agent(
        &self,
        agent_id: &str,
        query: &str,
        project_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<AinlMemoryNode>, String> {
        let fts_q = fts5_prefix_match_query(query);
        if fts_q.is_empty() || agent_id.trim().is_empty() {
            return Ok(Vec::new());
        }
        let cap = limit.clamp(1, 200) as i64;
        let project_filter = project_id.map(str::trim).filter(|s| !s.is_empty());
        let mut out = Vec::new();
        if let Some(p) = project_filter {
            let mut stmt = self
                .conn
                .prepare(
                    "SELECT n.payload
                 FROM ainl_nodes_fts AS f
                 INNER JOIN ainl_graph_nodes AS n ON n.id = f.node_id
                 WHERE f.agent_id = ?1
                   AND (COALESCE(f.project_id, '') = '' OR f.project_id = ?3)
                   AND f.body MATCH ?2
                 ORDER BY n.timestamp DESC
                 LIMIT ?4",
                )
                .map_err(|e| e.to_string())?;
            let mut rows = stmt
                .query(rusqlite::params![agent_id, fts_q, p, cap])
                .map_err(|e| e.to_string())?;
            while let Some(row) = rows.next().map_err(|e| e.to_string())? {
                let payload: String = row.get(0).map_err(|e| e.to_string())?;
                if let Ok(node) = serde_json::from_str::<AinlMemoryNode>(&payload) {
                    out.push(node);
                }
            }
        } else {
            let mut stmt = self
                .conn
                .prepare(
                    "SELECT n.payload
                 FROM ainl_nodes_fts AS f
                 INNER JOIN ainl_graph_nodes AS n ON n.id = f.node_id
                 WHERE f.agent_id = ?1
                   AND f.body MATCH ?2
                 ORDER BY n.timestamp DESC
                 LIMIT ?3",
                )
                .map_err(|e| e.to_string())?;
            let mut rows = stmt
                .query(rusqlite::params![agent_id, fts_q, cap])
                .map_err(|e| e.to_string())?;
            while let Some(row) = rows.next().map_err(|e| e.to_string())? {
                let payload: String = row.get(0).map_err(|e| e.to_string())?;
                if let Ok(node) = serde_json::from_str::<AinlMemoryNode>(&payload) {
                    out.push(node);
                }
            }
        }
        Ok(out)
    }
}

fn map_trajectory_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TrajectoryDetailRecord> {
    let id_s: String = row.get(0)?;
    let episode_s: String = row.get(1)?;
    let graph_traj: Option<String> = row.get(2)?;
    let agent_id: String = row.get(3)?;
    let session_id: String = row.get(4)?;
    let project_id: Option<String> = row.get(5)?;
    let recorded_at: i64 = row.get(6)?;
    let outcome_json: String = row.get(7)?;
    let hash: Option<String> = row.get(8)?;
    let duration_ms: i64 = row.get(9)?;
    let steps_json: String = row.get(10)?;
    let frame_vars_json: Option<String> = row.get(11)?;
    let fitness_sql: Option<f64> = row.get(12)?;
    let id = Uuid::parse_str(&id_s).map_err(|_| {
        rusqlite::Error::InvalidColumnType(0, "id".into(), rusqlite::types::Type::Text)
    })?;
    let episode_id = Uuid::parse_str(&episode_s).map_err(|_| {
        rusqlite::Error::InvalidColumnType(1, "episode_id".into(), rusqlite::types::Type::Text)
    })?;
    let graph_trajectory_node_id = graph_traj
        .filter(|s| !s.is_empty())
        .map(|s| Uuid::parse_str(&s))
        .transpose()
        .map_err(|_| {
            rusqlite::Error::InvalidColumnType(
                2,
                "graph_trajectory_node_id".into(),
                rusqlite::types::Type::Text,
            )
        })?;
    let outcome: TrajectoryOutcome = serde_json::from_str(&outcome_json).map_err(|_| {
        rusqlite::Error::InvalidColumnType(7, "outcome_json".into(), rusqlite::types::Type::Text)
    })?;
    let steps: Vec<TrajectoryStep> = serde_json::from_str(&steps_json).map_err(|_| {
        rusqlite::Error::InvalidColumnType(10, "steps_json".into(), rusqlite::types::Type::Text)
    })?;
    let frame_vars = frame_vars_json
        .filter(|s| !s.trim().is_empty())
        .and_then(|s| serde_json::from_str(&s).ok());
    let fitness_delta = fitness_sql.map(|f| f as f32);
    Ok(TrajectoryDetailRecord {
        id,
        episode_id,
        graph_trajectory_node_id,
        agent_id,
        session_id,
        project_id,
        recorded_at,
        outcome,
        ainl_source_hash: hash,
        duration_ms: duration_ms.max(0) as u64,
        steps,
        frame_vars,
        fitness_delta,
    })
}

impl GraphStore for SqliteGraphStore {
    /// Persists the full node JSON under `id` via `INSERT OR REPLACE` (upsert).
    /// Backfill pattern: `read_node` → patch fields (e.g. episodic signals) → `write_node`, preserving loaded `edges`.
    fn write_node(&self, node: &AinlMemoryNode) -> Result<(), String> {
        persist_node(&self.conn, node)
    }

    fn read_node(&self, id: Uuid) -> Result<Option<AinlMemoryNode>, String> {
        let payload: Option<String> = self
            .conn
            .query_row(
                "SELECT payload FROM ainl_graph_nodes WHERE id = ?1",
                [id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|e: rusqlite::Error| e.to_string())?;

        match payload {
            Some(p) => {
                let node: AinlMemoryNode = serde_json::from_str(&p).map_err(|e| e.to_string())?;
                Ok(Some(node))
            }
            None => Ok(None),
        }
    }

    fn query_episodes_since(
        &self,
        since_timestamp: i64,
        limit: usize,
    ) -> Result<Vec<AinlMemoryNode>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT payload FROM ainl_graph_nodes
                 WHERE node_type = 'episode' AND timestamp >= ?1
                 ORDER BY timestamp DESC
                 LIMIT ?2",
            )
            .map_err(|e| e.to_string())?;

        let rows = stmt
            .query_map([since_timestamp, limit as i64], |row| {
                let payload: String = row.get(0)?;
                Ok(payload)
            })
            .map_err(|e| e.to_string())?;

        let mut nodes = Vec::new();
        for row in rows {
            let payload = row.map_err(|e| e.to_string())?;
            let node: AinlMemoryNode = serde_json::from_str(&payload).map_err(|e| e.to_string())?;
            nodes.push(node);
        }

        Ok(nodes)
    }

    fn find_by_type(&self, type_name: &str) -> Result<Vec<AinlMemoryNode>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT payload FROM ainl_graph_nodes
                 WHERE node_type = ?1
                 ORDER BY timestamp DESC",
            )
            .map_err(|e| e.to_string())?;

        let rows = stmt
            .query_map([type_name], |row| {
                let payload: String = row.get(0)?;
                Ok(payload)
            })
            .map_err(|e| e.to_string())?;

        let mut nodes = Vec::new();
        for row in rows {
            let payload = row.map_err(|e| e.to_string())?;
            let node: AinlMemoryNode = serde_json::from_str(&payload).map_err(|e| e.to_string())?;
            nodes.push(node);
        }

        Ok(nodes)
    }

    fn walk_edges(&self, from_id: Uuid, label: &str) -> Result<Vec<AinlMemoryNode>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT to_id FROM ainl_graph_edges
                 WHERE from_id = ?1 AND label = ?2",
            )
            .map_err(|e| e.to_string())?;

        let target_ids: Vec<String> = stmt
            .query_map([from_id.to_string(), label.to_string()], |row| row.get(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;

        let mut nodes = Vec::new();
        for target_id in target_ids {
            let id = Uuid::parse_str(&target_id).map_err(|e| e.to_string())?;
            if let Some(node) = self.read_node(id)? {
                nodes.push(node);
            }
        }

        Ok(nodes)
    }
}
