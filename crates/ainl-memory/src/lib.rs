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
//! - **Trajectory** (`TrajectoryNode`): execution traces for replay / learning.
//! - **Failure** (`FailureNode`): typed failures (e.g. loop guard) with optional FTS search
//!   ([`GraphMemory::search_failures_for_agent`]).

pub mod anchored_summary;
pub mod node;
pub mod pattern_promotion;
pub mod query;
pub mod snapshot;
pub mod store;
mod trajectory_persist;
pub mod trajectory_table;

pub use anchored_summary::{anchored_summary_id, ANCHORED_SUMMARY_TAG};

pub use trajectory_persist::{
    persist_trajectory_coarse_tools, persist_trajectory_for_episode, trajectory_env_enabled,
};

pub use node::{
    AinlEdge, AinlMemoryNode, AinlNodeKind, AinlNodeType, EpisodicNode, FailureNode,
    MemoryCategory, PersonaLayer, PersonaNode, PersonaSource, ProceduralNode, ProcedureType,
    RuntimeStateNode, SemanticNode, Sentiment, StrengthEvent, TrajectoryNode,
};
pub use query::{
    count_by_topic_cluster, find_high_confidence_facts, find_patterns, find_strong_traits,
    recall_by_procedure_type, recall_by_topic_cluster, recall_contradictions,
    recall_delta_by_relevance, recall_episodes_by_conversation, recall_episodes_with_signal,
    recall_flagged_episodes, recall_low_success_procedures, recall_recent, recall_strength_history,
    recall_task_scoped_episodes, walk_from, GraphQuery,
};
pub use snapshot::{
    AgentGraphSnapshot, DanglingEdgeDetail, GraphValidationReport, SnapshotEdge,
    SNAPSHOT_SCHEMA_VERSION,
};
pub use store::{GraphStore, GraphValidationError, SnapshotImportError, SqliteGraphStore};
pub use trajectory_table::TrajectoryDetailRecord;

use ainl_contracts::{
    ProcedureArtifact, ProcedureLifecycle, ProcedureReuseOutcome, ProcedureStepKind,
    TrajectoryOutcome,
};
use uuid::Uuid;

/// High-level graph memory API - the main entry point for AINL memory.
///
/// Wraps a GraphStore implementation with a simplified 5-method API.
pub struct GraphMemory {
    store: SqliteGraphStore,
}

fn score_procedure_artifact(
    artifact: &ProcedureArtifact,
    intent: &str,
    available_tools: &[String],
) -> f32 {
    let haystack = format!(
        "{} {} {}",
        artifact.title.to_ascii_lowercase(),
        artifact.intent.to_ascii_lowercase(),
        artifact.summary.to_ascii_lowercase()
    );
    let tokens = intent
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|token| token.len() >= 3)
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>();
    let intent_score = if tokens.is_empty() {
        0.0
    } else {
        tokens
            .iter()
            .filter(|token| haystack.contains(token.as_str()))
            .count() as f32
            / tokens.len() as f32
    };
    let tool_score = if artifact.required_tools.is_empty() {
        0.2
    } else {
        artifact
            .required_tools
            .iter()
            .filter(|tool| available_tools.iter().any(|available| available == *tool))
            .count() as f32
            / artifact.required_tools.len() as f32
    };
    ((intent_score * 0.55) + (tool_score * 0.30) + (artifact.fitness.clamp(0.0, 1.0) * 0.15))
        .clamp(0.0, 1.0)
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
    ///
    /// This path treats the row as **curated** (prompt-eligible) so a single write is visible in
    /// suggested-procedure style recall; use [`Self::write_node`] with a hand-built node if you
    /// need candidate-only semantics.
    pub fn write_procedural(
        &self,
        pattern_name: &str,
        tool_sequence: Vec<String>,
        confidence: f32,
    ) -> Result<Uuid, String> {
        let mut node = AinlMemoryNode::new_procedural_tools(
            pattern_name.to_string(),
            tool_sequence,
            confidence,
        );
        if let AinlNodeType::Procedural { ref mut procedural } = node.node_type {
            procedural.pattern_observation_count = procedural
                .pattern_observation_count
                .max(crate::pattern_promotion::DEFAULT_MIN_OBSERVATIONS);
            let floor = crate::pattern_promotion::DEFAULT_FITNESS_FLOOR;
            if let Some(f) = procedural.fitness {
                procedural.fitness = Some(f.max(floor));
            } else {
                procedural.fitness = Some(floor);
            }
            procedural.prompt_eligible = true;
        }
        let node_id = node.id;
        self.store.write_node(&node)?;
        Ok(node_id)
    }

    /// Store a portable procedure artifact as a procedural graph node.
    ///
    /// The canonical JSON artifact is stored in `compiled_graph` so older graph consumers can
    /// ignore it safely, while new consumers can recall and deserialize validated procedure
    /// artifacts without adding a separate table.
    pub fn write_procedure_artifact(&self, artifact: &ProcedureArtifact) -> Result<Uuid, String> {
        self.write_procedure_artifact_for_agent("", artifact)
    }

    /// Store a portable procedure artifact for a specific agent.
    pub fn write_procedure_artifact_for_agent(
        &self,
        agent_id: &str,
        artifact: &ProcedureArtifact,
    ) -> Result<Uuid, String> {
        let artifact_json = serde_json::to_vec(artifact).map_err(|e| e.to_string())?;
        let tool_sequence = artifact
            .steps
            .iter()
            .filter_map(|step| match &step.kind {
                ProcedureStepKind::ToolCall { tool, .. } => Some(tool.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        let mut node = AinlMemoryNode::new_pattern(artifact.id.clone(), artifact_json);
        node.agent_id = agent_id.to_string();
        if let AinlNodeType::Procedural { ref mut procedural } = node.node_type {
            procedural.tool_sequence = tool_sequence;
            procedural.confidence = Some(artifact.fitness.clamp(0.0, 1.0));
            procedural.fitness = Some(artifact.fitness.clamp(0.0, 1.0));
            procedural.pattern_observation_count = artifact.observation_count;
            procedural.prompt_eligible = matches!(
                artifact.lifecycle,
                ProcedureLifecycle::Validated | ProcedureLifecycle::Promoted
            );
            procedural.label = artifact.id.clone();
            procedural.trigger_conditions = vec![artifact.intent.clone()];
        }
        let node_id = node.id;
        self.store.write_node(&node)?;
        Ok(node_id)
    }

    /// Update an existing procedural node for `artifact.id`, or write a new one if no node exists.
    pub fn upsert_procedure_artifact_for_agent(
        &self,
        agent_id: &str,
        artifact: &ProcedureArtifact,
    ) -> Result<Uuid, String> {
        for mut node in self.store.find_by_type("procedural")? {
            if node.agent_id != agent_id {
                continue;
            }
            let Some(procedural) = node.procedural() else {
                continue;
            };
            let matches_id = procedural.label == artifact.id
                || serde_json::from_slice::<ProcedureArtifact>(&procedural.compiled_graph)
                    .map(|existing| existing.id == artifact.id)
                    .unwrap_or(false);
            if !matches_id {
                continue;
            }
            let artifact_json = serde_json::to_vec(artifact).map_err(|e| e.to_string())?;
            let tool_sequence = artifact
                .steps
                .iter()
                .filter_map(|step| match &step.kind {
                    ProcedureStepKind::ToolCall { tool, .. } => Some(tool.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            if let AinlNodeType::Procedural { ref mut procedural } = node.node_type {
                procedural.compiled_graph = artifact_json;
                procedural.tool_sequence = tool_sequence;
                procedural.confidence = Some(artifact.fitness.clamp(0.0, 1.0));
                procedural.fitness = Some(artifact.fitness.clamp(0.0, 1.0));
                procedural.pattern_observation_count = artifact.observation_count;
                procedural.prompt_eligible = matches!(
                    artifact.lifecycle,
                    ProcedureLifecycle::Validated | ProcedureLifecycle::Promoted
                );
                procedural.label = artifact.id.clone();
                procedural.trigger_conditions = vec![artifact.intent.clone()];
            }
            let node_id = node.id;
            self.store.write_node(&node)?;
            return Ok(node_id);
        }
        self.write_procedure_artifact_for_agent(agent_id, artifact)
    }

    /// Recall portable procedure artifacts previously stored with [`Self::write_procedure_artifact`].
    pub fn recall_procedure_artifacts(&self) -> Result<Vec<ProcedureArtifact>, String> {
        let mut out = Vec::new();
        for node in self.store.find_by_type("procedural")? {
            let Some(procedural) = node.procedural() else {
                continue;
            };
            if !procedural.prompt_eligible || procedural.compiled_graph.is_empty() {
                continue;
            }
            if let Ok(artifact) =
                serde_json::from_slice::<ProcedureArtifact>(&procedural.compiled_graph)
            {
                out.push(artifact);
            }
        }
        Ok(out)
    }

    /// Search validated/promoted procedure artifacts by intent text and required tool overlap.
    pub fn search_procedure_artifacts_for_agent(
        &self,
        agent_id: &str,
        intent: &str,
        available_tools: &[String],
        limit: usize,
    ) -> Result<Vec<ProcedureArtifact>, String> {
        let mut scored = Vec::new();
        for node in self.store.find_by_type("procedural")? {
            if node.agent_id != agent_id {
                continue;
            }
            let Some(procedural) = node.procedural() else {
                continue;
            };
            if !procedural.prompt_eligible || procedural.compiled_graph.is_empty() {
                continue;
            }
            let Ok(artifact) =
                serde_json::from_slice::<ProcedureArtifact>(&procedural.compiled_graph)
            else {
                continue;
            };
            if matches!(artifact.lifecycle, ProcedureLifecycle::Deprecated) {
                continue;
            }
            let score = score_procedure_artifact(&artifact, intent, available_tools);
            if score > 0.0 {
                scored.push((score, artifact));
            }
        }
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        Ok(scored
            .into_iter()
            .take(limit)
            .map(|(_, artifact)| artifact)
            .collect())
    }

    /// Update artifact lifecycle and fitness after an attempted reuse.
    pub fn record_procedure_reuse_outcome_for_agent(
        &self,
        agent_id: &str,
        outcome: &ProcedureReuseOutcome,
    ) -> Result<Uuid, String> {
        let mut artifacts =
            self.search_procedure_artifacts_for_agent(agent_id, "", &[], usize::MAX)?;
        let Some(mut artifact) = artifacts
            .drain(..)
            .find(|artifact| artifact.id == outcome.procedure_id)
        else {
            return Err(format!(
                "procedure artifact not found: {}",
                outcome.procedure_id
            ));
        };
        artifact.observation_count = artifact.observation_count.saturating_add(1);
        let delta = match outcome.outcome {
            TrajectoryOutcome::Success => 0.04,
            TrajectoryOutcome::PartialSuccess => 0.01,
            TrajectoryOutcome::Failure => -0.08,
            TrajectoryOutcome::Aborted => -0.12,
        };
        artifact.fitness = (artifact.fitness + delta).clamp(0.0, 1.0);
        if let Some(failure_id) = outcome.failure_id.as_ref() {
            if !artifact
                .source_failure_ids
                .iter()
                .any(|id| id == failure_id)
            {
                artifact.source_failure_ids.push(failure_id.clone());
            }
        }
        self.upsert_procedure_artifact_for_agent(agent_id, &artifact)
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

    /// Find a recent procedural (tool-sequence) row for this agent whose `tool_sequence` matches
    /// `tool_sequence` (per-element trim). Returns the **newest** match if several exist
    /// (e.g. legacy duplicates before merge).
    pub fn find_procedural_by_tool_sequence(
        &self,
        agent_id: &str,
        tool_sequence: &[String],
    ) -> Result<Option<AinlMemoryNode>, String> {
        let norm: Vec<String> = tool_sequence.iter().map(|s| s.trim().to_string()).collect();
        if norm.is_empty() {
            return Ok(None);
        }
        let nodes = self.recall_by_type(AinlNodeKind::Procedural, 60 * 60 * 24 * 365 * 5)?;
        for n in nodes {
            if n.agent_id != agent_id {
                continue;
            }
            let AinlNodeType::Procedural { ref procedural } = n.node_type else {
                continue;
            };
            if procedural.tool_sequence.len() != norm.len() {
                continue;
            }
            let same = procedural
                .tool_sequence
                .iter()
                .zip(norm.iter())
                .all(|(a, b)| a.trim() == b.trim());
            if same {
                // `recall_by_type` is most-recent first; first hit is the canonical row to update.
                return Ok(Some(n));
            }
        }
        Ok(None)
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

    /// Insert a detailed trajectory row (see [`SqliteGraphStore::insert_trajectory_detail`]).
    pub fn insert_trajectory_detail(&self, row: &TrajectoryDetailRecord) -> Result<(), String> {
        self.store.insert_trajectory_detail(row)
    }

    /// Recent trajectory detail rows for an agent (see [`SqliteGraphStore::list_trajectories_for_agent`]).
    pub fn list_trajectories_for_agent(
        &self,
        agent_id: &str,
        limit: usize,
        since_timestamp: Option<i64>,
    ) -> Result<Vec<TrajectoryDetailRecord>, String> {
        self.store
            .list_trajectories_for_agent(agent_id, limit, since_timestamp)
    }

    /// How many `ainl_trajectories` detail rows would be removed by
    /// [`Self::prune_trajectory_details_before`] (same `before_recorded_at` semantics).
    pub fn count_trajectory_details_before(
        &self,
        agent_id: &str,
        before_recorded_at: i64,
    ) -> Result<usize, String> {
        self.store
            .count_trajectory_details_before(agent_id, before_recorded_at)
    }

    /// Remove persisted trajectory **detail** rows with `recorded_at` **strictly before** `before_recorded_at` (seconds).
    ///
    /// This targets the `ainl_trajectories` table only. Graph `Trajectory` nodes and cross-links are not
    /// deleted here; use exports / graph tooling if you need a full-store consistency pass after pruning.
    pub fn prune_trajectory_details_before(
        &self,
        agent_id: &str,
        before_recorded_at: i64,
    ) -> Result<usize, String> {
        self.store
            .delete_trajectory_details_before(agent_id, before_recorded_at)
    }

    /// Search persisted [`FailureNode`] rows for an agent (FTS5 over `ainl_failures_fts`).
    pub fn search_failures_for_agent(
        &self,
        agent_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<AinlMemoryNode>, String> {
        self.store
            .search_failures_fts_for_agent(agent_id, query, limit)
    }

    /// Full-graph FTS5 search (`ainl_nodes_fts`); see [`SqliteGraphStore::search_all_nodes_fts_for_agent`].
    pub fn search_all_nodes_fts(
        &self,
        agent_id: &str,
        query: &str,
        project_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<AinlMemoryNode>, String> {
        self.store
            .search_all_nodes_fts_for_agent(agent_id, query, project_id, limit)
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

    /// End-to-end: `Failure` graph row + `ainl_failures_fts` sync + `search_failures_for_agent`.
    #[test]
    fn failure_write_and_fts_search_roundtrip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("ainl_failure_fts_smoke.db");
        let memory = GraphMemory::new(&db_path).expect("graph memory");
        let agent_id = "agent-smoke-fts";

        let mut node = AinlMemoryNode::new_loop_guard_failure(
            "block",
            Some("shell_exec"),
            "repeated identical tool invocation blocked by loop guard",
            Some("session-xyz"),
        );
        node.agent_id = agent_id.to_string();
        let nid = node.id;
        memory.write_node(&node).expect("write failure node");

        let hits = memory
            .search_failures_for_agent(agent_id, "loop", 10)
            .expect("search loop");
        assert_eq!(hits.len(), 1, "expected one FTS hit for token 'loop'");
        assert_eq!(hits[0].id, nid);
        assert!(
            matches!(&hits[0].node_type, AinlNodeType::Failure { .. }),
            "expected Failure node type"
        );

        let hits2 = memory
            .search_failures_for_agent(agent_id, "shell_exec", 10)
            .expect("search tool name");
        assert_eq!(hits2.len(), 1);
        assert_eq!(hits2[0].id, nid);

        let empty = memory
            .search_failures_for_agent(agent_id, "   ", 10)
            .expect("whitespace-only query");
        assert!(empty.is_empty());

        let wrong_agent = memory
            .search_failures_for_agent("other-agent", "loop", 10)
            .expect("wrong agent id");
        assert!(wrong_agent.is_empty());
    }

    /// Full-graph `ainl_nodes_fts` — semantic fact is searchable, not only failures.
    #[test]
    fn all_nodes_fts_write_and_search_roundtrip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("ainl_all_nodes_fts.db");
        let memory = GraphMemory::new(&db_path).expect("graph memory");
        let agent_id = "agent-fts-all";
        let mut node =
            AinlMemoryNode::new_fact("unique-fts-violet-cat-42".into(), 0.8, Uuid::new_v4());
        node.agent_id = agent_id.to_string();
        let nid = node.id;
        memory.write_node(&node).expect("write fact");

        let hits = memory
            .search_all_nodes_fts(agent_id, "violet", None, 10)
            .expect("search");
        assert_eq!(hits.len(), 1, "expected one all-nodes FTS hit");
        assert_eq!(hits[0].id, nid);
    }

    #[test]
    fn tool_execution_failure_write_and_fts_search_roundtrip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("ainl_tool_failure_fts.db");
        let memory = GraphMemory::new(&db_path).expect("graph memory");
        let agent_id = "agent-tool-ft";

        let mut node = AinlMemoryNode::new_tool_execution_failure(
            "file_read",
            "ENOENT: no such file or directory",
            Some("sess-tool-1"),
        );
        node.agent_id = agent_id.to_string();
        let nid = node.id;
        memory.write_node(&node).expect("write tool failure node");

        let hits = memory
            .search_failures_for_agent(agent_id, "ENOENT", 10)
            .expect("search ENOENT");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, nid);

        let src_hits = memory
            .search_failures_for_agent(agent_id, "tool_runner", 10)
            .expect("search source");
        assert_eq!(src_hits.len(), 1);
        assert_eq!(src_hits[0].id, nid);
    }

    #[test]
    fn agent_loop_precheck_failure_write_and_fts_search_roundtrip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("ainl_precheck_failure_fts.db");
        let memory = GraphMemory::new(&db_path).expect("graph memory");
        let agent_id = "agent-precheck-ft";

        let mut node = AinlMemoryNode::new_agent_loop_precheck_failure(
            "param_validation",
            "file_write",
            "missing required field: path",
            Some("sess-pv-1"),
        );
        node.agent_id = agent_id.to_string();
        let nid = node.id;
        memory.write_node(&node).expect("write precheck failure");

        let hits = memory
            .search_failures_for_agent(agent_id, "param_validation", 10)
            .expect("search kind");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, nid);

        let hits2 = memory
            .search_failures_for_agent(agent_id, "agent_loop", 10)
            .expect("search agent_loop prefix");
        assert_eq!(hits2.len(), 1);
    }

    #[test]
    fn ainl_runtime_graph_validation_failure_write_and_fts_search_roundtrip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("ainl_graph_validation_failure_fts.db");
        let memory = GraphMemory::new(&db_path).expect("graph memory");
        let agent_id = "agent-graph-val-ft";

        let mut node = AinlMemoryNode::new_ainl_runtime_graph_validation_failure(
            "graph validation failed before turn: dangling edges …",
            Some("sess-gv-1"),
        );
        node.agent_id = agent_id.to_string();
        let nid = node.id;
        memory
            .write_node(&node)
            .expect("write graph validation failure");

        let hits = memory
            .search_failures_for_agent(agent_id, "graph_validation", 10)
            .expect("search source label");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, nid);

        let hits2 = memory
            .search_failures_for_agent(agent_id, "dangling", 10)
            .expect("search message body");
        assert_eq!(hits2.len(), 1);
    }

    #[test]
    fn trajectory_detail_prune_before_drops_only_old_rows() {
        use ainl_contracts::{TrajectoryOutcome, TrajectoryStep};

        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("ainl_traj_prune.db");
        let memory = GraphMemory::new(&db_path).expect("graph memory");
        let agent = "agent-traj-prune";
        let ep_old = memory
            .write_episode(vec![], None, None)
            .expect("episode for old traj");
        let ep_new = memory
            .write_episode(vec![], None, None)
            .expect("episode for new traj");
        let mk_step = |sid: &str| TrajectoryStep {
            step_id: sid.to_string(),
            timestamp_ms: 0,
            adapter: "a".into(),
            operation: "o".into(),
            inputs_preview: None,
            outputs_preview: None,
            duration_ms: 1,
            success: true,
            error: None,
            vitals: None,
            freshness_at_step: None,
            frame_vars: None,
            tool_telemetry: None,
        };
        let r_old = TrajectoryDetailRecord {
            id: Uuid::new_v4(),
            episode_id: ep_old,
            graph_trajectory_node_id: None,
            agent_id: agent.to_string(),
            session_id: "s-old".into(),
            project_id: None,
            recorded_at: 100,
            outcome: TrajectoryOutcome::Success,
            ainl_source_hash: None,
            duration_ms: 1,
            steps: vec![mk_step("1")],
            frame_vars: None,
            fitness_delta: None,
        };
        let r_new = TrajectoryDetailRecord {
            id: Uuid::new_v4(),
            episode_id: ep_new,
            graph_trajectory_node_id: None,
            agent_id: agent.to_string(),
            session_id: "s-new".into(),
            project_id: None,
            recorded_at: 200,
            outcome: TrajectoryOutcome::Success,
            ainl_source_hash: None,
            duration_ms: 1,
            steps: vec![mk_step("2")],
            frame_vars: None,
            fitness_delta: None,
        };
        memory.insert_trajectory_detail(&r_old).expect("insert old");
        memory.insert_trajectory_detail(&r_new).expect("insert new");
        let before = memory
            .list_trajectories_for_agent(agent, 10, None)
            .expect("list");
        assert_eq!(before.len(), 2);
        let removed = memory
            .prune_trajectory_details_before(agent, 200)
            .expect("prune");
        assert_eq!(removed, 1);
        let after = memory
            .list_trajectories_for_agent(agent, 10, None)
            .expect("list after");
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].recorded_at, 200);
    }

    #[test]
    fn stores_and_recalls_validated_procedure_artifact() {
        use ainl_contracts::{
            ProcedureArtifact, ProcedureArtifactFormat, ProcedureLifecycle, ProcedureStep,
            ProcedureStepKind, ProcedureVerification, LEARNER_SCHEMA_VERSION,
        };

        let tmp = tempfile::tempdir().unwrap();
        let memory = GraphMemory::new(&tmp.path().join("memory.db")).unwrap();
        let artifact = ProcedureArtifact {
            schema_version: LEARNER_SCHEMA_VERSION,
            id: "proc:test".into(),
            title: "Test Procedure".into(),
            intent: "test intent".into(),
            summary: "summary".into(),
            required_tools: vec!["file_read".into()],
            required_adapters: vec![],
            inputs: vec![],
            outputs: vec![],
            preconditions: vec![],
            steps: vec![ProcedureStep {
                step_id: "s1".into(),
                title: "Read".into(),
                kind: ProcedureStepKind::ToolCall {
                    tool: "file_read".into(),
                    args_schema: serde_json::json!({"type":"object"}),
                },
                rationale: None,
            }],
            verification: ProcedureVerification::default(),
            known_failures: vec![],
            recovery: vec![],
            source_trajectory_ids: vec![],
            source_failure_ids: vec![],
            fitness: 0.9,
            observation_count: 3,
            lifecycle: ProcedureLifecycle::Validated,
            render_targets: vec![ProcedureArtifactFormat::PromptOnly],
        };
        memory.write_procedure_artifact(&artifact).unwrap();
        let recalled = memory.recall_procedure_artifacts().unwrap();
        assert_eq!(recalled, vec![artifact]);
    }

    #[test]
    fn searches_and_updates_procedure_reuse_fitness() {
        use ainl_contracts::{
            ProcedureArtifact, ProcedureArtifactFormat, ProcedureLifecycle, ProcedureReuseOutcome,
            ProcedureStep, ProcedureStepKind, ProcedureVerification, TrajectoryOutcome,
            LEARNER_SCHEMA_VERSION,
        };

        let tmp = tempfile::tempdir().unwrap();
        let memory = GraphMemory::new(&tmp.path().join("memory.db")).unwrap();
        let artifact = ProcedureArtifact {
            schema_version: LEARNER_SCHEMA_VERSION,
            id: "proc:review".into(),
            title: "Review PR".into(),
            intent: "review pull request".into(),
            summary: "review code changes safely".into(),
            required_tools: vec!["file_read".into(), "shell_exec".into()],
            required_adapters: vec![],
            inputs: vec![],
            outputs: vec![],
            preconditions: vec![],
            steps: vec![ProcedureStep {
                step_id: "s1".into(),
                title: "Read".into(),
                kind: ProcedureStepKind::ToolCall {
                    tool: "file_read".into(),
                    args_schema: serde_json::json!({"type":"object"}),
                },
                rationale: None,
            }],
            verification: ProcedureVerification::default(),
            known_failures: vec![],
            recovery: vec![],
            source_trajectory_ids: vec![],
            source_failure_ids: vec![],
            fitness: 0.6,
            observation_count: 3,
            lifecycle: ProcedureLifecycle::Promoted,
            render_targets: vec![ProcedureArtifactFormat::PromptOnly],
        };
        memory
            .write_procedure_artifact_for_agent("agent-search", &artifact)
            .unwrap();
        let hits = memory
            .search_procedure_artifacts_for_agent(
                "agent-search",
                "please review this pull request",
                &["file_read".into(), "shell_exec".into()],
                5,
            )
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "proc:review");

        memory
            .record_procedure_reuse_outcome_for_agent(
                "agent-search",
                &ProcedureReuseOutcome {
                    procedure_id: "proc:review".into(),
                    outcome: TrajectoryOutcome::Failure,
                    failure_id: Some("failure-x".into()),
                    notes: None,
                },
            )
            .unwrap();
        let updated = memory
            .search_procedure_artifacts_for_agent("agent-search", "review pull request", &[], 5)
            .unwrap();
        assert_eq!(updated[0].observation_count, 4);
        assert!(updated[0].fitness < 0.6);
        assert!(updated[0].source_failure_ids.contains(&"failure-x".into()));
    }
}
