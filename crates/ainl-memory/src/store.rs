//! Graph storage backends for AINL memory.
//!
//! Defines the GraphStore trait and SQLite implementation.
//! SQLite tables integrate with existing openfang-memory schema.

use crate::node::{AinlMemoryNode, AinlNodeType};
use rusqlite::OptionalExtension;
use uuid::Uuid;

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

        conn.execute(
            "CREATE TABLE IF NOT EXISTS ainl_graph_edges (
                from_id TEXT NOT NULL,
                to_id TEXT NOT NULL,
                label TEXT NOT NULL,
                PRIMARY KEY (from_id, to_id, label)
            )",
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_ainl_edges_from
             ON ainl_graph_edges(from_id, label)",
            [],
        )?;

        Ok(())
    }

    /// Open/create a graph store at the given path
    pub fn open(path: &std::path::Path) -> Result<Self, String> {
        let conn = rusqlite::Connection::open(path).map_err(|e| e.to_string())?;
        Self::ensure_schema(&conn).map_err(|e| e.to_string())?;
        Ok(Self { conn })
    }

    /// Create from an existing connection (for integration with openfang-memory pool)
    pub fn from_connection(conn: rusqlite::Connection) -> Result<Self, String> {
        Self::ensure_schema(&conn).map_err(|e| e.to_string())?;
        Ok(Self { conn })
    }

    /// Insert a directed edge between two node IDs (separate from per-node edge payloads).
    pub fn insert_graph_edge(&self, from_id: Uuid, to_id: Uuid, label: &str) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO ainl_graph_edges (from_id, to_id, label)
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![from_id.to_string(), to_id.to_string(), label],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
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
}

impl GraphStore for SqliteGraphStore {
    /// Persists the full node JSON under `id` via `INSERT OR REPLACE` (upsert).
    /// Backfill pattern: `read_node` → patch fields (e.g. episodic signals) → `write_node`, preserving loaded `edges`.
    fn write_node(&self, node: &AinlMemoryNode) -> Result<(), String> {
        let payload = serde_json::to_string(node).map_err(|e| e.to_string())?;
        let type_name = match &node.node_type {
            AinlNodeType::Episode { .. } => "episode",
            AinlNodeType::Semantic { .. } => "semantic",
            AinlNodeType::Procedural { .. } => "procedural",
            AinlNodeType::Persona { .. } => "persona",
        };

        // Extract timestamp from the node
        let timestamp = match &node.node_type {
            AinlNodeType::Episode { episodic } => episodic.timestamp,
            _ => chrono::Utc::now().timestamp(),
        };

        self.conn
            .execute(
                "INSERT OR REPLACE INTO ainl_graph_nodes (id, node_type, payload, timestamp)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![node.id.to_string(), type_name, payload, timestamp,],
            )
            .map_err(|e| e.to_string())?;

        // Write edges
        for edge in &node.edges {
            self.conn
                .execute(
                    "INSERT OR REPLACE INTO ainl_graph_edges (from_id, to_id, label)
                     VALUES (?1, ?2, ?3)",
                    rusqlite::params![node.id.to_string(), edge.target_id.to_string(), edge.label,],
                )
                .map_err(|e| e.to_string())?;
        }

        Ok(())
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
