//! Serializable graph snapshots and validation reports.

use crate::node::AinlMemoryNode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use uuid::Uuid;

/// Schema version string embedded in [`AgentGraphSnapshot`] for forward compatibility.
pub const SNAPSHOT_SCHEMA_VERSION: &str = "1.0";

fn default_snapshot_schema_cow() -> Cow<'static, str> {
    Cow::Borrowed(SNAPSHOT_SCHEMA_VERSION)
}

/// Full export of one agent's subgraph (nodes + interconnecting edges).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentGraphSnapshot {
    pub agent_id: String,
    pub exported_at: DateTime<Utc>,
    #[serde(default = "default_snapshot_schema_cow")]
    pub schema_version: Cow<'static, str>,
    pub nodes: Vec<AinlMemoryNode>,
    pub edges: Vec<SnapshotEdge>,
}

/// One directed edge in a snapshot (maps to `ainl_graph_edges` rows).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotEdge {
    pub source_id: Uuid,
    pub target_id: Uuid,
    pub edge_type: String,
    #[serde(default = "default_edge_weight")]
    pub weight: f32,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

fn default_edge_weight() -> f32 {
    1.0
}

/// One edge row that references a missing `ainl_graph_nodes` endpoint (see [`GraphValidationReport`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DanglingEdgeDetail {
    pub source_id: String,
    pub target_id: String,
    pub edge_type: String,
}

/// Result of [`crate::SqliteGraphStore::validate_graph`].
#[derive(Debug, Clone)]
pub struct GraphValidationReport {
    pub agent_id: String,
    pub node_count: usize,
    pub edge_count: usize,
    /// `(source_id, target_id)` edge endpoint pairs that reference a missing node row.
    pub dangling_edges: Vec<(String, String)>,
    /// Same dangling rows as [`Self::dangling_edges`], including edge label for diagnostics.
    pub dangling_edge_details: Vec<DanglingEdgeDetail>,
    /// Edges that touch this agent’s node set on exactly one side while both node rows exist
    /// (often a shared/global neighbor or another agent’s node — informational, not invalid).
    pub cross_agent_boundary_edges: usize,
    /// Node ids (for this agent) that do not appear in any edge as source or target.
    pub orphan_nodes: Vec<String>,
    pub is_valid: bool,
}
