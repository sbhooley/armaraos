//! Loaded graph artifacts and per-turn data shapes for [`crate::AinlRuntime`].

use std::collections::HashMap;
use std::error::Error;
use std::fmt;

use ainl_graph_extractor::ExtractionReport;
use ainl_memory::{
    AgentGraphSnapshot, AinlMemoryNode, AinlNodeType, GraphValidationReport, ProceduralNode,
    SqliteGraphStore,
};
use ainl_persona::PersonaSnapshot;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Edge label for emit routing (matches `ainl_graph_edges.label`).
pub const EMIT_TO_EDGE: &str = "EMIT_TO";

/// Hard failure for [`crate::AinlRuntime::run_turn`] (store open, invalid graph, invalid compile input, etc.).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AinlRuntimeError(pub String);

impl fmt::Display for AinlRuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Error for AinlRuntimeError {}

impl From<String> for AinlRuntimeError {
    fn from(s: String) -> Self {
        Self(s)
    }
}

/// Per-patch inputs for [`crate::PatchAdapter::execute_patch`] (procedural / GraphPatch nodes).
///
/// [`AinlRuntime`] resolves a label-keyed adapter first, then falls back to the reference
/// [`crate::GraphPatchAdapter`] (registered as [`crate::GraphPatchAdapter::NAME`]) when no adapter
/// matches the patch IR label.
#[derive(Debug, Clone, Copy)]
pub struct PatchDispatchContext<'a> {
    pub patch_label: &'a str,
    pub node: &'a AinlMemoryNode,
    pub frame: &'a HashMap<String, serde_json::Value>,
}

impl<'a> PatchDispatchContext<'a> {
    pub fn procedural(&self) -> Option<&'a ProceduralNode> {
        match &self.node.node_type {
            AinlNodeType::Procedural { procedural } => Some(procedural),
            _ => None,
        }
    }
}

/// Result of attempting to dispatch one procedural patch node.
#[derive(Debug, Clone)]
pub struct PatchDispatchResult {
    pub label: String,
    pub patch_version: u32,
    pub fitness_before: f32,
    pub fitness_after: f32,
    pub dispatched: bool,
    pub skip_reason: Option<PatchSkipReason>,
    /// Output from a registered [`crate::PatchAdapter`], if any ran successfully.
    pub adapter_output: Option<serde_json::Value>,
    /// Name of the adapter that was invoked (including on execution failure).
    pub adapter_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatchSkipReason {
    MissingDeclaredRead(String),
    Retired,
    ZeroVersion,
    /// Node was not a procedural patch payload.
    NotProcedural,
    /// Failed to persist fitness update.
    PersistFailed(String),
}

impl fmt::Display for PatchSkipReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PatchSkipReason::MissingDeclaredRead(s) => write!(f, "missing_declared_read:{s}"),
            PatchSkipReason::Retired => write!(f, "retired"),
            PatchSkipReason::ZeroVersion => write!(f, "zero_version"),
            PatchSkipReason::NotProcedural => write!(f, "not_procedural"),
            PatchSkipReason::PersistFailed(s) => write!(f, "persist_failed:{s}"),
        }
    }
}

/// A loaded, validated AINL graph artifact (memory substrate view for one agent).
#[derive(Debug, Clone)]
pub struct AinlGraphArtifact {
    pub agent_id: String,
    pub snapshot: AgentGraphSnapshot,
    pub validation: GraphValidationReport,
}

impl AinlGraphArtifact {
    /// Load agent graph from store. Fails if validation reports dangling edges.
    pub fn load(store: &SqliteGraphStore, agent_id: &str) -> Result<Self, String> {
        let snapshot = store.export_graph(agent_id)?;
        let validation = store.validate_graph(agent_id)?;
        if !validation.is_valid {
            let mut msg = String::from("graph validation failed: dangling edges");
            for d in &validation.dangling_edge_details {
                msg.push_str(&format!(
                    "; {} -> {} [{}]",
                    d.source_id, d.target_id, d.edge_type
                ));
            }
            return Err(msg);
        }
        Ok(Self {
            agent_id: agent_id.to_string(),
            snapshot,
            validation,
        })
    }

    /// Wrap a snapshot without re-validating (tests / transfer). Caller must validate separately if needed.
    pub fn from_snapshot(snapshot: AgentGraphSnapshot) -> Self {
        let agent_id = snapshot.agent_id.clone();
        let node_count = snapshot.nodes.len();
        let edge_count = snapshot.edges.len();
        let validation = GraphValidationReport {
            agent_id: agent_id.clone(),
            node_count,
            edge_count,
            dangling_edges: Vec::new(),
            dangling_edge_details: Vec::new(),
            cross_agent_boundary_edges: 0,
            orphan_nodes: Vec::new(),
            is_valid: true,
        };
        Self {
            agent_id,
            snapshot,
            validation,
        }
    }
}

/// Input for a single agent turn (host fills; runtime does not call LLMs).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TurnInput {
    pub user_message: String,
    pub tools_invoked: Vec<String>,
    pub trace_event: Option<serde_json::Value>,
    /// Caller-supplied depth hint. Not used for enforcement — internal `delegation_depth` is authoritative.
    pub depth: u32,
    /// Frame variables required by procedural `declared_reads` during patch dispatch.
    pub frame: HashMap<String, serde_json::Value>,
    /// After the episode row is written, `EMIT_TO` edges are inserted from `episode_id` to each target
    /// (additive; default empty). Hosts/tests use this to wire emit routing in the same turn.
    pub emit_targets: Vec<Uuid>,
}

/// Compiled memory context for a turn (prompt-side assembly in the host).
#[derive(Debug, Clone)]
pub struct MemoryContext {
    pub recent_episodes: Vec<AinlMemoryNode>,
    pub relevant_semantic: Vec<AinlMemoryNode>,
    pub active_patches: Vec<AinlMemoryNode>,
    pub persona_snapshot: Option<PersonaSnapshot>,
    pub compiled_at: DateTime<Utc>,
}

impl Default for MemoryContext {
    fn default() -> Self {
        Self {
            recent_episodes: Vec::new(),
            relevant_semantic: Vec::new(),
            active_patches: Vec::new(),
            persona_snapshot: None,
            compiled_at: Utc::now(),
        }
    }
}

/// Non-fatal bookkeeping phase inside [`AinlRuntime::run_turn`] (SQLite / export / persona persistence).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TurnPhase {
    EpisodeWrite,
    FitnessWriteBack,
    ExtractionPass,
    PatternPersistence,
    PersonaEvolution,
    ExportRefresh,
}

/// One non-fatal failure recorded during a turn (the turn still returns a usable [`TurnResult`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnWarning {
    pub phase: TurnPhase,
    pub error: String,
}

/// Soft outcome for depth / step caps / disabled graph (not store write failures — those become [`TurnWarning`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnStatus {
    Ok,
    DepthLimitExceeded,
    StepLimitExceeded {
        steps_executed: u32,
    },
    GraphMemoryDisabled,
}

/// Payload from a finished turn (memory context, episode id, patch dispatch, etc.).
#[derive(Debug, Clone)]
pub struct TurnResult {
    pub episode_id: Uuid,
    pub persona_prompt_contribution: Option<String>,
    pub memory_context: MemoryContext,
    pub extraction_report: Option<ExtractionReport>,
    pub steps_executed: u32,
    pub patch_dispatch_results: Vec<PatchDispatchResult>,
    pub status: TurnStatus,
}

impl Default for TurnResult {
    fn default() -> Self {
        Self {
            episode_id: Uuid::nil(),
            persona_prompt_contribution: None,
            memory_context: MemoryContext::default(),
            extraction_report: None,
            steps_executed: 0,
            patch_dispatch_results: Vec::new(),
            status: TurnStatus::Ok,
        }
    }
}

/// Full success vs partial success after non-fatal write failures.
#[derive(Debug, Clone)]
pub enum TurnOutcome {
    /// All bookkeeping writes succeeded.
    Complete(TurnResult),
    /// Turn completed but one or more non-fatal writes failed; [`TurnResult`] is still valid.
    PartialSuccess {
        result: TurnResult,
        warnings: Vec<TurnWarning>,
    },
}

impl TurnOutcome {
    pub fn result(&self) -> &TurnResult {
        match self {
            TurnOutcome::Complete(r) | TurnOutcome::PartialSuccess { result: r, .. } => r,
        }
    }

    pub fn warnings(&self) -> &[TurnWarning] {
        match self {
            TurnOutcome::Complete(_) => &[],
            TurnOutcome::PartialSuccess { warnings, .. } => warnings.as_slice(),
        }
    }

    pub fn into_result(self) -> TurnResult {
        match self {
            TurnOutcome::Complete(r) | TurnOutcome::PartialSuccess { result: r, .. } => r,
        }
    }

    pub fn is_complete(&self) -> bool {
        matches!(self, TurnOutcome::Complete(_))
    }

    pub fn is_partial_success(&self) -> bool {
        matches!(self, TurnOutcome::PartialSuccess { .. })
    }

    pub fn turn_status(&self) -> TurnStatus {
        self.result().status
    }
}
