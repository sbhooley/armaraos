//! **ainl-runtime** v0.3.5-alpha — orchestration layer for the unified AINL graph (memory substrate + extraction).
//!
//! This crate **does not** call LLMs, parse AINL IR, or implement tool adapters. It coordinates
//! [`ainl_memory`], persona axis state via [`ainl_persona::EvolutionEngine`] (shared with
//! [`ainl_graph_extractor::GraphExtractorTask`]), and scheduled graph extraction — with [`TurnHooks`] for host
//! integration (e.g. OpenFang).
//!
//! **Evolution:** [`EvolutionEngine`] lives in **ainl-persona**. [`AinlRuntime::evolution_engine_mut`] and
//! helpers ([`AinlRuntime::apply_evolution_signals`], [`AinlRuntime::persist_evolution_snapshot`], …) drive it
//! without going through the extractor. [`GraphExtractorTask::run_pass`] remains one signal producer (graph
//! extract + recurrence + pattern heuristics), not the only way to evolve persona axes.
//! Scheduled passes attach [`ExtractionReport`] to [`TurnResult`]; populated
//! **`extract_error` / `pattern_error` / `persona_error`** slots become separate [`TurnWarning`] entries
//! tagged with [`TurnPhase::ExtractionPass`], [`TurnPhase::PatternPersistence`], and [`TurnPhase::PersonaEvolution`].
//!
//! For a minimal “record episodes + run extractor” path without the full engine, see [`RuntimeContext`].
//!
//! ## Semantic ranking / [`MemoryContext`]
//!
//! **`compile_memory_context_for(None)`** no longer inherits previous episode text for semantic
//! ranking; pass **`Some(user_message)`** if you want topic-aware [`MemoryContext::relevant_semantic`].
//! [`AinlRuntime::compile_memory_context`] still calls `compile_memory_context_for(None)` (empty
//! message → high-recurrence fallback). [`AinlRuntime::run_turn`] always passes the current turn text.
//!
//! **Async / Tokio:** enable the optional **`async`** crate feature for `AinlRuntime::run_turn_async`.
//! Graph memory is then `Arc<std::sync::Mutex<GraphMemory>>` (not `tokio::sync::Mutex`) so
//! [`AinlRuntime::new`] and [`AinlRuntime::sqlite_store`] can take short locks on any thread; SQLite
//! work for async turns is still offloaded with `tokio::task::spawn_blocking`. See the crate
//! **`README.md`** for rationale; ArmaraOS hub **`docs/ainl-runtime.md`**, patch dispatch
//! **`docs/ainl-runtime-graph-patch.md`**, and optional OpenFang embed **`docs/ainl-runtime-integration.md`**
//! cover host integration and registry crates.io pins.

mod adapters;
mod engine;
mod graph_cell;
mod hooks;
mod runtime;

pub use adapters::{AdapterRegistry, GraphPatchAdapter, GraphPatchHostDispatch, PatchAdapter};

pub use ainl_semantic_tagger::infer_topic_tags;

pub use ainl_graph_extractor::{run_extraction_pass, ExtractionReport, GraphExtractorTask};
pub use ainl_persona::axes::default_axis_map;
pub use ainl_persona::{
    EvolutionEngine, MemoryNodeType, PersonaAxis, PersonaSnapshot, RawSignal, EVOLUTION_TRAIT_NAME,
    INGEST_SCORE_EPSILON,
};

pub use engine::{
    AinlGraphArtifact, AinlRuntimeError, MemoryContext, PatchDispatchContext, PatchDispatchResult,
    PatchSkipReason, TurnInput, TurnOutcome, TurnPhase, TurnResult, TurnStatus, TurnWarning,
    EMIT_TO_EDGE,
};
pub use graph_cell::SqliteStoreRef;
pub use hooks::{NoOpHooks, TurnHooks};
#[cfg(feature = "async")]
pub use hooks::{NoOpAsyncHooks, TurnHooksAsync};
pub use runtime::AinlRuntime;

use ainl_memory::{GraphMemory, GraphStore};
use serde::{Deserialize, Serialize};

/// Configuration for [`AinlRuntime`] and [`RuntimeContext`].
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(default)]
pub struct RuntimeConfig {
    /// Owning agent id (required for extraction, graph queries, and [`AinlRuntime`]).
    pub agent_id: String,
    /// Maximum nested [`AinlRuntime::run_turn`] depth (internal counter); see [`TurnInput::depth`].
    pub max_delegation_depth: u32,
    pub enable_graph_memory: bool,
    /// Cap for the minimal BFS graph walk in [`AinlRuntime::run_turn`].
    pub max_steps: u32,
    /// Run [`GraphExtractorTask::run_pass`] every N completed turns (`0` disables scheduled passes).
    pub extraction_interval: u32,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            agent_id: String::new(),
            max_delegation_depth: 8,
            enable_graph_memory: true,
            max_steps: 1000,
            extraction_interval: 10,
        }
    }
}

/// Host context: optional memory plus optional stateful extractor (legacy / lightweight).
pub struct RuntimeContext {
    _config: RuntimeConfig,
    memory: Option<GraphMemory>,
    extractor: Option<GraphExtractorTask>,
}

impl RuntimeContext {
    /// Create a new runtime context with the given memory backend.
    pub fn new(config: RuntimeConfig, memory: Option<GraphMemory>) -> Self {
        Self {
            _config: config,
            memory,
            extractor: None,
        }
    }

    /// Record an agent delegation as an episode node.
    pub fn record_delegation(
        &self,
        delegated_to: String,
        trace_event: Option<serde_json::Value>,
    ) -> Result<uuid::Uuid, String> {
        if let Some(ref memory) = self.memory {
            memory.write_episode(
                vec!["agent_delegate".to_string()],
                Some(delegated_to),
                trace_event,
            )
        } else {
            Err("Memory not initialized".to_string())
        }
    }

    /// Record a tool execution as an episode node.
    pub fn record_tool_execution(
        &self,
        tool_name: String,
        trace_event: Option<serde_json::Value>,
    ) -> Result<uuid::Uuid, String> {
        if let Some(ref memory) = self.memory {
            memory.write_episode(vec![tool_name], None, trace_event)
        } else {
            Err("Memory not initialized".to_string())
        }
    }

    /// Record a turn as an episode with an explicit tool-call list.
    pub fn record_episode(
        &self,
        tool_calls: Vec<String>,
        delegation_to: Option<String>,
        trace_event: Option<serde_json::Value>,
    ) -> Result<uuid::Uuid, String> {
        if let Some(ref memory) = self.memory {
            memory.write_episode(tool_calls, delegation_to, trace_event)
        } else {
            Err("Memory not initialized".to_string())
        }
    }

    /// Get direct access to the underlying store for advanced queries.
    pub fn store(&self) -> Option<&dyn GraphStore> {
        self.memory.as_ref().map(|m| m.store())
    }

    /// Run `ainl-graph-extractor` on the backing SQLite store.
    pub fn run_graph_extraction_pass(&mut self) -> Result<ExtractionReport, String> {
        if self._config.agent_id.is_empty() {
            return Err("RuntimeConfig.agent_id is required for graph extraction".to_string());
        }
        let memory = self
            .memory
            .as_ref()
            .ok_or_else(|| "Graph memory is required for graph extraction".to_string())?;

        self.extractor
            .get_or_insert_with(|| GraphExtractorTask::new(&self._config.agent_id));
        let store = memory.sqlite_store();
        let report = self
            .extractor
            .as_mut()
            .expect("get_or_insert_with always leaves Some")
            .run_pass(store);

        if report.has_errors() {
            tracing::warn!(
                agent_id = %report.agent_id,
                extract_error = ?report.extract_error,
                pattern_error = ?report.pattern_error,
                persona_error = ?report.persona_error,
                "ainl-graph-extractor pass completed with phase errors"
            );
        } else {
            tracing::info!(
                agent_id = %report.agent_id,
                signals_extracted = report.signals_extracted,
                signals_applied = report.signals_applied,
                semantic_nodes_updated = report.semantic_nodes_updated,
                "ainl-graph-extractor pass completed"
            );
        }
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TurnStatus;
    use ainl_memory::{AinlMemoryNode, SqliteGraphStore};
    use std::path::PathBuf;
    use uuid::Uuid;

    #[test]
    fn ainl_runtime_error_helpers() {
        let e = AinlRuntimeError::DelegationDepthExceeded { depth: 2, max: 8 };
        assert!(e.is_delegation_depth_exceeded());
        assert_eq!(e.delegation_depth_exceeded(), Some((2, 8)));
        assert!(e.message_str().is_none());

        let m = AinlRuntimeError::Message("graph validation failed".into());
        assert!(!m.is_delegation_depth_exceeded());
        assert!(m.delegation_depth_exceeded().is_none());
        assert_eq!(m.message_str(), Some("graph validation failed"));

        let from_str: AinlRuntimeError = "via from".to_string().into();
        assert_eq!(from_str.message_str(), Some("via from"));
    }

    #[test]
    fn test_runtime_config_default() {
        let config = RuntimeConfig::default();
        assert_eq!(config.max_delegation_depth, 8);
        assert!(config.enable_graph_memory);
        assert!(config.agent_id.is_empty());
        assert_eq!(config.max_steps, 1000);
        assert_eq!(config.extraction_interval, 10);
    }

    #[test]
    fn extraction_pass_requires_agent_id() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("t.db");
        let mem = GraphMemory::new(&db).unwrap();
        let mut ctx = RuntimeContext::new(RuntimeConfig::default(), Some(mem));
        let err = ctx.run_graph_extraction_pass().unwrap_err();
        assert!(err.contains("agent_id"));
    }

    #[test]
    fn extraction_pass_runs_with_memory_and_agent() {
        let dir = tempfile::tempdir().unwrap();
        let db: PathBuf = dir.path().join("t.db");
        let mem = GraphMemory::new(&db).unwrap();
        let cfg = RuntimeConfig {
            agent_id: "agent-test".into(),
            ..RuntimeConfig::default()
        };
        let mut ctx = RuntimeContext::new(cfg, Some(mem));
        ctx.record_tool_execution("noop".into(), None).unwrap();
        let report = ctx.run_graph_extraction_pass().expect("extraction");
        assert_eq!(report.agent_id, "agent-test");
    }

    #[test]
    fn ainl_runtime_run_turn_smoke() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("rt.db");
        let _ = std::fs::remove_file(&db);
        let store = SqliteGraphStore::open(&db).unwrap();
        let ag = "rt-agent";
        let mut ep = AinlMemoryNode::new_episode(Uuid::new_v4(), 3_000_000_000, vec![], None, None);
        ep.agent_id = ag.into();
        store.write_node(&ep).unwrap();

        let cfg = RuntimeConfig {
            agent_id: ag.into(),
            extraction_interval: 1,
            max_steps: 50,
            ..RuntimeConfig::default()
        };
        let mut rt = AinlRuntime::new(cfg, store);
        let art = rt.load_artifact().expect("load");
        assert!(art.validation.is_valid);

        let out = rt
            .run_turn(TurnInput {
                user_message: "hello".into(),
                tools_invoked: vec!["noop".into()],
                trace_event: None,
                depth: 0,
                ..Default::default()
            })
            .expect("turn");
        assert!(out.is_complete());
        assert_eq!(out.turn_status(), TurnStatus::Ok);
        assert_ne!(out.result().episode_id, Uuid::nil());
        assert!(out.result().extraction_report.is_some());
    }
}
