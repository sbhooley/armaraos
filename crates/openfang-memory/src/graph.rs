//! AINL graph-memory substrate - Spike implementation
//!
//! Proof-of-concept: Agent memory as execution graph nodes.
//! Every delegation, tool call, and agent turn becomes a typed graph node.
//! This spike proves the concept before extraction to `ainl-memory` crate.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Core AINL node types - the vocabulary of agent memory
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AinlNodeType {
    /// Episodic memory: what happened during an agent turn
    Episode {
        turn_id: Uuid,
        agent_id: String,
        tool_calls: Vec<String>,
        delegation_to: Option<String>,
        trace_id: Option<String>,
        depth: u32,
    },
    /// Semantic memory: facts learned, with confidence
    Semantic {
        fact: String,
        confidence: f32,
        source_turn: Uuid,
    },
    /// Procedural memory: reusable compiled workflow patterns
    Procedural {
        pattern_name: String,
        compiled_graph: Vec<u8>,
    },
    /// Persona memory: traits learned over time
    Persona {
        trait_name: String,
        strength: f32,
        learned_from: Vec<Uuid>,
    },
}

/// A node in the AINL memory graph
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AinlMemoryNode {
    pub id: Uuid,
    pub node_type: AinlNodeType,
    pub timestamp: i64,
    pub edges: Vec<AinlEdge>,
}

/// Typed edge connecting memory nodes
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AinlEdge {
    pub target_id: Uuid,
    pub label: String,
}

impl AinlMemoryNode {
    /// Create a new episode node for an orchestration delegation
    pub fn new_delegation_episode(
        agent_id: String,
        delegated_to: String,
        trace_id: String,
        depth: u32,
    ) -> Self {
        let turn_id = Uuid::new_v4();
        Self {
            id: Uuid::new_v4(),
            node_type: AinlNodeType::Episode {
                turn_id,
                agent_id,
                tool_calls: vec!["agent_delegate".to_string()],
                delegation_to: Some(delegated_to),
                trace_id: Some(trace_id),
                depth,
            },
            timestamp: chrono::Utc::now().timestamp(),
            edges: Vec::new(),
        }
    }

    /// Create a new episode node for a tool use
    pub fn new_tool_episode(agent_id: String, tool_name: String) -> Self {
        let turn_id = Uuid::new_v4();
        Self {
            id: Uuid::new_v4(),
            node_type: AinlNodeType::Episode {
                turn_id,
                agent_id,
                tool_calls: vec![tool_name],
                delegation_to: None,
                trace_id: None,
                depth: 0,
            },
            timestamp: chrono::Utc::now().timestamp(),
            edges: Vec::new(),
        }
    }

    /// Add an edge to another node
    pub fn add_edge(&mut self, target_id: Uuid, label: impl Into<String>) {
        self.edges.push(AinlEdge {
            target_id,
            label: label.into(),
        });
    }
}

/// Graph memory storage - trait for swappable backends
/// (Note: Send + Sync removed for spike - will use Arc<Mutex<>> in production)
pub trait GraphStore {
    /// Write a node to storage
    fn write_node(&self, node: &AinlMemoryNode) -> Result<(), String>;

    /// Read a node by ID
    fn read_node(&self, id: Uuid) -> Result<Option<AinlMemoryNode>, String>;

    /// Query nodes by type
    fn query_by_type(&self, type_name: &str) -> Result<Vec<AinlMemoryNode>, String>;

    /// Query recent episodes for an agent
    fn query_recent_episodes(
        &self,
        agent_id: &str,
        limit: usize,
    ) -> Result<Vec<AinlMemoryNode>, String>;

    /// Walk the graph from a starting node
    fn walk_edges(&self, from_id: Uuid, label: &str) -> Result<Vec<AinlMemoryNode>, String>;
}

/// SQLite implementation of GraphStore (spike - uses existing connection)
pub struct SqliteGraphStore {
    db: rusqlite::Connection,
}

impl SqliteGraphStore {
    /// Create tables if they don't exist
    pub fn ensure_schema(db: &rusqlite::Connection) -> Result<(), rusqlite::Error> {
        db.execute(
            "CREATE TABLE IF NOT EXISTS ainl_graph_nodes (
                id TEXT PRIMARY KEY,
                node_type TEXT NOT NULL,
                payload TEXT NOT NULL,
                timestamp INTEGER NOT NULL
            )",
            [],
        )?;

        db.execute(
            "CREATE INDEX IF NOT EXISTS idx_ainl_nodes_timestamp
             ON ainl_graph_nodes(timestamp DESC)",
            [],
        )?;

        db.execute(
            "CREATE INDEX IF NOT EXISTS idx_ainl_nodes_type
             ON ainl_graph_nodes(node_type)",
            [],
        )?;

        db.execute(
            "CREATE TABLE IF NOT EXISTS ainl_graph_edges (
                from_id TEXT NOT NULL,
                to_id TEXT NOT NULL,
                label TEXT NOT NULL,
                PRIMARY KEY (from_id, to_id, label)
            )",
            [],
        )?;

        db.execute(
            "CREATE INDEX IF NOT EXISTS idx_ainl_edges_from
             ON ainl_graph_edges(from_id, label)",
            [],
        )?;

        Ok(())
    }

    /// Open/create a graph store at the given path
    pub fn open(path: &std::path::Path) -> Result<Self, String> {
        let db = rusqlite::Connection::open(path).map_err(|e| e.to_string())?;
        Self::ensure_schema(&db).map_err(|e| e.to_string())?;
        Ok(Self { db })
    }
}

impl GraphStore for SqliteGraphStore {
    fn write_node(&self, node: &AinlMemoryNode) -> Result<(), String> {
        let payload = serde_json::to_string(node).map_err(|e| e.to_string())?;
        let type_name = match &node.node_type {
            AinlNodeType::Episode { .. } => "episode",
            AinlNodeType::Semantic { .. } => "semantic",
            AinlNodeType::Procedural { .. } => "procedural",
            AinlNodeType::Persona { .. } => "persona",
        };

        self.db
            .execute(
                "INSERT OR REPLACE INTO ainl_graph_nodes (id, node_type, payload, timestamp)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![
                    node.id.to_string(),
                    type_name,
                    payload,
                    node.timestamp,
                ],
            )
            .map_err(|e| e.to_string())?;

        // Write edges
        for edge in &node.edges {
            self.db
                .execute(
                    "INSERT OR REPLACE INTO ainl_graph_edges (from_id, to_id, label)
                     VALUES (?1, ?2, ?3)",
                    rusqlite::params![
                        node.id.to_string(),
                        edge.target_id.to_string(),
                        edge.label,
                    ],
                )
                .map_err(|e| e.to_string())?;
        }

        Ok(())
    }

    fn read_node(&self, id: Uuid) -> Result<Option<AinlMemoryNode>, String> {
        use rusqlite::OptionalExtension;

        let payload: Option<String> = self
            .db
            .query_row(
                "SELECT payload FROM ainl_graph_nodes WHERE id = ?1",
                [id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|e: rusqlite::Error| e.to_string())?;

        match payload {
            Some(p) => {
                let node: AinlMemoryNode =
                    serde_json::from_str(&p).map_err(|e| e.to_string())?;
                Ok(Some(node))
            }
            None => Ok(None),
        }
    }

    fn query_by_type(&self, type_name: &str) -> Result<Vec<AinlMemoryNode>, String> {
        let mut stmt = self
            .db
            .prepare("SELECT payload FROM ainl_graph_nodes WHERE node_type = ?1 ORDER BY timestamp DESC")
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
            let node: AinlMemoryNode =
                serde_json::from_str(&payload).map_err(|e| e.to_string())?;
            nodes.push(node);
        }

        Ok(nodes)
    }

    fn query_recent_episodes(
        &self,
        agent_id: &str,
        limit: usize,
    ) -> Result<Vec<AinlMemoryNode>, String> {
        let mut stmt = self
            .db
            .prepare(
                "SELECT payload FROM ainl_graph_nodes
                 WHERE node_type = 'episode'
                 ORDER BY timestamp DESC
                 LIMIT ?1",
            )
            .map_err(|e| e.to_string())?;

        let rows = stmt
            .query_map([limit], |row| {
                let payload: String = row.get(0)?;
                Ok(payload)
            })
            .map_err(|e| e.to_string())?;

        let mut nodes = Vec::new();
        for row in rows {
            let payload = row.map_err(|e| e.to_string())?;
            let node: AinlMemoryNode =
                serde_json::from_str(&payload).map_err(|e| e.to_string())?;

            // Filter by agent_id in the Episode variant
            if let AinlNodeType::Episode {
                agent_id: node_agent,
                ..
            } = &node.node_type
            {
                if node_agent == agent_id {
                    nodes.push(node);
                }
            }
        }

        Ok(nodes)
    }

    fn walk_edges(&self, from_id: Uuid, label: &str) -> Result<Vec<AinlMemoryNode>, String> {
        let mut stmt = self
            .db
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_delegation_episode() {
        let node = AinlMemoryNode::new_delegation_episode(
            "agent-123".to_string(),
            "agent-456".to_string(),
            "trace-xyz".to_string(),
            1,
        );

        assert!(matches!(node.node_type, AinlNodeType::Episode { .. }));
        if let AinlNodeType::Episode {
            delegation_to,
            depth,
            ..
        } = node.node_type
        {
            assert_eq!(delegation_to, Some("agent-456".to_string()));
            assert_eq!(depth, 1);
        }
    }

    #[test]
    fn test_sqlite_store_write_read() {
        let temp_dir = std::env::temp_dir();
        let db_path = temp_dir.join("ainl_test.db");
        let _ = std::fs::remove_file(&db_path); // Clean up from previous run

        let store = SqliteGraphStore::open(&db_path).expect("Failed to open store");

        let node = AinlMemoryNode::new_delegation_episode(
            "agent-123".to_string(),
            "agent-456".to_string(),
            "trace-xyz".to_string(),
            1,
        );

        store.write_node(&node).expect("Failed to write node");

        let retrieved = store
            .read_node(node.id)
            .expect("Failed to read node")
            .expect("Node not found");

        assert_eq!(retrieved.id, node.id);
        assert_eq!(retrieved.timestamp, node.timestamp);
    }

    #[test]
    fn test_query_recent_episodes() {
        let temp_dir = std::env::temp_dir();
        let db_path = temp_dir.join("ainl_test_query.db");
        let _ = std::fs::remove_file(&db_path);

        let store = SqliteGraphStore::open(&db_path).expect("Failed to open store");

        // Write 3 episodes for the same agent
        for i in 0..3 {
            let node = AinlMemoryNode::new_delegation_episode(
                "agent-123".to_string(),
                format!("agent-{}", i),
                format!("trace-{}", i),
                i as u32,
            );
            store.write_node(&node).expect("Failed to write node");
            std::thread::sleep(std::time::Duration::from_millis(10)); // Ensure different timestamps
        }

        let episodes = store
            .query_recent_episodes("agent-123", 10)
            .expect("Failed to query");

        assert_eq!(episodes.len(), 3);
    }
}
