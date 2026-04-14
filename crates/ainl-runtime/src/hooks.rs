//! Integration seam for hosts (e.g. OpenFang) — all methods default to no-ops.

use uuid::Uuid;

use crate::engine::{MemoryContext, TurnOutcome};
use ainl_graph_extractor::ExtractionReport;

#[cfg(feature = "async")]
use crate::engine::{PatchDispatchContext, TurnInput};

/// Hooks for observability and host wiring. Every method has a default empty body.
pub trait TurnHooks: Send + Sync {
    fn on_artifact_loaded(&self, _agent_id: &str, _node_count: usize) {}
    fn on_persona_compiled(&self, _contribution: Option<&str>) {}
    fn on_memory_context_ready(&self, _ctx: &MemoryContext) {}
    fn on_episode_recorded(&self, _episode_id: Uuid) {}
    fn on_patch_dispatched(&self, _label: &str, _fitness: f32) {}
    fn on_extraction_complete(&self, _report: &ExtractionReport) {}
    fn on_emit(&self, _target: &str, _payload: &serde_json::Value) {}
    fn on_turn_complete(&self, _outcome: &TurnOutcome) {}
}

/// Default hook implementation (no side effects).
pub struct NoOpHooks;

impl TurnHooks for NoOpHooks {}

/// Async observability hooks for [`crate::AinlRuntime::run_turn_async`] (Tokio-friendly).
///
/// Graph SQLite I/O for that path runs on `tokio::task::spawn_blocking`; the graph itself stays
/// under `Arc<std::sync::Mutex<_>>` (not `tokio::sync::Mutex`) so the runtime can be constructed and
/// queried from any thread. See the crate root docs and `README.md`.
#[cfg(feature = "async")]
#[async_trait::async_trait]
pub trait TurnHooksAsync: Send + Sync {
    async fn on_turn_start(&self, _input: &TurnInput) {}

    async fn on_patch_dispatched(
        &self,
        _ctx: &PatchDispatchContext<'_>,
    ) -> Result<serde_json::Value, String> {
        Ok(serde_json::Value::Null)
    }

    async fn on_turn_complete(&self, _outcome: &TurnOutcome) {}
}

/// Default async hook implementation (no side effects).
#[cfg(feature = "async")]
pub struct NoOpAsyncHooks;

#[cfg(feature = "async")]
#[async_trait::async_trait]
impl TurnHooksAsync for NoOpAsyncHooks {}
