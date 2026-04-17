//! AINL Memory - Graph-based agent memory substrate
//!
//! **Graph-as-memory for AI agents. Execution IS the memory.**
//!
//! AINL Memory implements agent memory as an execution graph. Every agent turn,
//! tool call, and delegation becomes a typed graph node. No separate retrieval
//! layer—the graph itself is the memory.
//!
//! # Quick Start
//!
//! ```no_run
//! use ainl_memory::GraphMemory;
//! use std::path::Path;
//!
//! let memory = GraphMemory::new(Path::new("memory.db")).unwrap();
//!
//! // Record an episode
//! memory.write_episode(
//!     vec!["file_read".to_string(), "agent_delegate".to_string()],
//!     Some("agent-B".to_string()),
//!     None,
//! ).unwrap();
//!
//! // Recall recent episodes
//! let recent = memory.recall_recent(100).unwrap();
//! ```
//!
//! # Architecture
//!
//! AINL Memory is designed as infrastructure that any agent framework can adopt:
//! - Zero dependencies on specific agent runtimes
//! - Simple trait-based API via `GraphStore`
//! - Bring your own storage backend
//!
//! ## Graph store: query, export, validation (since 0.1.4-alpha)
//!
//! - **[`SqliteGraphStore`]**: SQLite backend with **`PRAGMA foreign_keys = ON`**, `FOREIGN KEY` constraints
//!   on `ainl_graph_edges`, one-time migration for legacy DBs (see [CHANGELOG.md](../CHANGELOG.md)).
//! - **[`GraphQuery`]**: `store.query(agent_id)` — agent-scoped SQL helpers (episodes, lineage, tags, …).
//! - **Snapshots**: [`AgentGraphSnapshot`], [`SnapshotEdge`], [`SNAPSHOT_SCHEMA_VERSION`];
//!   [`SqliteGraphStore::export_graph`] / [`SqliteGraphStore::import_graph`] (strict vs repair via
//!   `allow_dangling_edges`).
//! - **Validation**: [`GraphValidationReport`], [`DanglingEdgeDetail`]; [`SqliteGraphStore::validate_graph`]
//!   for agent-scoped semantics beyond raw FK enforcement.
//! - **[`GraphMemory`]** forwards the above where hosts should not reach past the facade (see impl block).
//!
//! ## Node Types
//!
//! - **Episode**: What happened during an agent turn (tool calls, delegations)
//! - **Semantic**: Facts learned with confidence scores
//! - **Procedural**: Reusable compiled workflow patterns
//! - **Persona**: Agent traits learned over time
//! - **Runtime state** (`RuntimeStateNode`, `node_type = runtime_state`): Optional persisted session
//!   counters and persona snapshot JSON for **ainl-runtime** (see [`GraphMemory::read_runtime_state`] /
//!   [`GraphMemory::write_runtime_state`]).

pub mod node;
pub mod query;
pub mod snapshot;
pub mod store;

pub use node::{
    AinlEdge, AinlMemoryNode, AinlNodeKind, AinlNodeType, EpisodicNode, MemoryCategory,
    PersonaLayer, PersonaNode, PersonaSource, ProceduralNode, ProcedureType, RuntimeStateNode,
    SemanticNode, Sentiment, StrengthEvent,
};
pub use query::{
    count_by_topic_cluster, find_high_confidence_facts, find_patterns, find_strong_traits,
    recall_by_procedure_type, recall_by_topic_cluster, recall_contradictions,
    recall_delta_by_relevance, recall_episodes_by_conversation, recall_episodes_with_signal,
    recall_flagged_episodes, recall_low_success_procedures, recall_recent, recall_strength_history,
    recall_task_scoped_episodes,
    walk_from, GraphQuery,
};
pub use snapshot::{
    AgentGraphSnapshot, DanglingEdgeDetail, GraphValidationReport, SnapshotEdge,
    SNAPSHOT_SCHEMA_VERSION,
};
pub use store::{GraphStore, GraphValidationError, SnapshotImportError, SqliteGraphStore};

use uuid::Uuid;

/// High-level graph memory API - the main entry point for AINL memory.
///
/// Wraps a GraphStore implementation with a simplified 5-method API.
pub struct GraphMemory {
    store: SqliteGraphStore,
}

impl GraphMemory {
    /// Create a new graph memory at the given database path.
    ///
    /// This will create the database file if it doesn't exist, and
    /// ensure the AINL graph schema is initialized.
    pub fn new(db_path: &std::path::Path) -> Result<Self, String> {
        let store = SqliteGraphStore::open(db_path)?;
        Ok(Self { store })
    }

    /// Create from an existing SQLite connection (for integration with existing memory pools)
    pub fn from_connection(conn: rusqlite::Connection) -> Result<Self, String> {
        let store = SqliteGraphStore::from_connection(conn)?;
        Ok(Self { store })
    }

    /// Wrap an already-open [`SqliteGraphStore`] (for hosts that manage connections externally).
    pub fn from_sqlite_store(store: SqliteGraphStore) -> Self {
        Self { store }
    }

    /// Write an episode node (what happened during an agent turn).
    ///
    /// # Arguments
    /// * `tool_calls` - List of tools executed during this turn
    /// * `delegation_to` - Agent ID this turn delegated to (if any)
    /// * `trace_event` - Optional orchestration trace event (serialized JSON)
    ///
    /// # Returns
    /// The ID of the created episode node
    pub fn write_episode(
        &self,
        tool_calls: Vec<String>,
        delegation_to: Option<String>,
        trace_event: Option<serde_json::Value>,
    ) -> Result<Uuid, String> {
        let turn_id = Uuid::new_v4();
        let timestamp = chrono::Utc::now().timestamp();

        let node =
            AinlMemoryNode::new_episode(turn_id, timestamp, tool_calls, delegation_to, trace_event);

        let node_id = node.id;
        self.store.write_node(&node)?;
        Ok(node_id)
    }

    /// Write a semantic fact (learned information with confidence).
    ///
    /// # Arguments
    /// * `fact` - The fact in natural language
    /// * `confidence` - Confidence score (0.0-1.0)
    /// * `source_turn_id` - Turn ID that generated this fact
    ///
    /// # Returns
    /// The ID of the created semantic node
    pub fn write_fact(
        &self,
        fact: String,
        confidence: f32,
        source_turn_id: Uuid,
    ) -> Result<Uuid, String> {
        let node = AinlMemoryNode::new_fact(fact, confidence, source_turn_id);
        let node_id = node.id;
        self.store.write_node(&node)?;
        Ok(node_id)
    }

    /// Store a procedural pattern (compiled workflow).
    ///
    /// # Arguments
    /// * `pattern_name` - Name/identifier for the pattern
    /// * `compiled_graph` - Binary representation of the compiled graph
    ///
    /// # Returns
    /// The ID of the created procedural node
    pub fn store_pattern(
        &self,
        pattern_name: String,
        compiled_graph: Vec<u8>,
    ) -> Result<Uuid, String> {
        let node = AinlMemoryNode::new_pattern(pattern_name, compiled_graph);
        let node_id = node.id;
        self.store.write_node(&node)?;
        Ok(node_id)
    }

    /// Store a procedural pattern derived from a live tool sequence (heuristic extraction).
    pub fn write_procedural(
        &self,
        pattern_name: &str,
        tool_sequence: Vec<String>,
        confidence: f32,
    ) -> Result<Uuid, String> {
        let node = AinlMemoryNode::new_procedural_tools(
            pattern_name.to_string(),
            tool_sequence,
            confidence,
        );
        let node_id = node.id;
        self.store.write_node(&node)?;
        Ok(node_id)
    }

    /// Write a graph edge between nodes (e.g. episode timeline `follows`).
    pub fn write_edge(&self, source: Uuid, target: Uuid, rel: &str) -> Result<(), String> {
        self.store.insert_graph_edge(source, target, rel)
    }

    /// Recall recent episodes (within the last N seconds).
    ///
    /// # Arguments
    /// * `seconds_ago` - Only return episodes from the last N seconds
    ///
    /// # Returns
    /// Vector of episode nodes, most recent first
    pub fn recall_recent(&self, seconds_ago: i64) -> Result<Vec<AinlMemoryNode>, String> {
        let since = chrono::Utc::now().timestamp() - seconds_ago;
        self.store.query_episodes_since(since, 100)
    }

    /// Recall nodes of a specific kind written in the last `seconds_ago` seconds.
    pub fn recall_by_type(
        &self,
        kind: AinlNodeKind,
        seconds_ago: i64,
    ) -> Result<Vec<AinlMemoryNode>, String> {
        let since = chrono::Utc::now().timestamp() - seconds_ago;
        self.store
            .query_nodes_by_type_since(kind.as_str(), since, 500)
    }

    /// Write a persona trait node.
    pub fn write_persona(
        &self,
        trait_name: &str,
        strength: f32,
        learned_from: Vec<Uuid>,
    ) -> Result<Uuid, String> {
        let node = AinlMemoryNode::new_persona(trait_name.to_string(), strength, learned_from);
        let node_id = node.id;
        self.store.write_node(&node)?;
        Ok(node_id)
    }

    /// Get direct access to the underlying store for advanced queries
    pub fn store(&self) -> &dyn GraphStore {
        &self.store
    }

    /// SQLite backing store (for components such as `ainl-graph-extractor` that require concrete SQL access).
    pub fn sqlite_store(&self) -> &SqliteGraphStore {
        &self.store
    }

    /// [`SqliteGraphStore::validate_graph`] for the same backing database (checkpoint / boot gate).
    pub fn validate_graph(&self, agent_id: &str) -> Result<GraphValidationReport, String> {
        self.store.validate_graph(agent_id)
    }

    /// [`SqliteGraphStore::export_graph`].
    pub fn export_graph(&self, agent_id: &str) -> Result<AgentGraphSnapshot, String> {
        self.store.export_graph(agent_id)
    }

    /// [`SqliteGraphStore::import_graph`] — use `allow_dangling_edges: false` for normal loads; `true` only for repair.
    pub fn import_graph(
        &mut self,
        snapshot: &AgentGraphSnapshot,
        allow_dangling_edges: bool,
    ) -> Result<(), String> {
        self.store.import_graph(snapshot, allow_dangling_edges)
    }

    /// [`SqliteGraphStore::agent_subgraph_edges`].
    pub fn agent_subgraph_edges(&self, agent_id: &str) -> Result<Vec<SnapshotEdge>, String> {
        self.store.agent_subgraph_edges(agent_id)
    }

    /// [`SqliteGraphStore::write_node_with_edges`] (transactional; fails if embedded edge targets are missing).
    pub fn write_node_with_edges(&mut self, node: &AinlMemoryNode) -> Result<(), String> {
        self.store.write_node_with_edges(node)
    }

    /// [`SqliteGraphStore::insert_graph_edge_checked`].
    pub fn insert_graph_edge_checked(
        &self,
        from_id: Uuid,
        to_id: Uuid,
        label: &str,
    ) -> Result<(), String> {
        self.store.insert_graph_edge_checked(from_id, to_id, label)
    }

    /// Read persisted [`RuntimeStateNode`] for `agent_id` (most recent row).
    pub fn read_runtime_state(&self, agent_id: &str) -> Result<Option<RuntimeStateNode>, String> {
        self.store.read_runtime_state(agent_id)
    }

    /// Upsert persisted [`RuntimeStateNode`] for the given agent (stable node id per `agent_id`).
    pub fn write_runtime_state(&self, state: &RuntimeStateNode) -> Result<(), String> {
        self.store.write_runtime_state(state)
    }

    /// Write a fully constructed node (additive API for callers that set extended metadata).
    pub fn write_node(&self, node: &AinlMemoryNode) -> Result<(), String> {
        self.store.write_node(node)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_graph_memory_api() {
        let temp_dir = std::env::temp_dir();
        let db_path = temp_dir.join("ainl_lib_test.db");
        let _ = std::fs::remove_file(&db_path);

        let memory = GraphMemory::new(&db_path).expect("Failed to create memory");

        // Write an episode
        let episode_id = memory
            .write_episode(
                vec!["file_read".to_string(), "agent_delegate".to_string()],
                Some("agent-B".to_string()),
                None,
            )
            .expect("Failed to write episode");

        assert_ne!(episode_id, Uuid::nil());

        // Write a fact
        let fact_id = memory
            .write_fact(
                "User prefers concise responses".to_string(),
                0.85,
                episode_id,
            )
            .expect("Failed to write fact");

        assert_ne!(fact_id, Uuid::nil());

        // Recall recent episodes
        let recent = memory.recall_recent(60).expect("Failed to recall");
        assert_eq!(recent.len(), 1);

        // Verify the episode content
        if let AinlNodeType::Episode { episodic } = &recent[0].node_type {
            assert_eq!(episodic.delegation_to, Some("agent-B".to_string()));
            assert_eq!(episodic.tool_calls.len(), 2);
        } else {
            panic!("Wrong node type");
        }
    }

    #[test]
    fn test_store_pattern() {
        let temp_dir = std::env::temp_dir();
        let db_path = temp_dir.join("ainl_lib_test_pattern.db");
        let _ = std::fs::remove_file(&db_path);

        let memory = GraphMemory::new(&db_path).expect("Failed to create memory");

        let pattern_id = memory
            .store_pattern("research_workflow".to_string(), vec![1, 2, 3, 4])
            .expect("Failed to store pattern");

        assert_ne!(pattern_id, Uuid::nil());

        // Query it back
        let patterns = find_patterns(memory.store(), "research").expect("Query failed");
        assert_eq!(patterns.len(), 1);
    }
}
