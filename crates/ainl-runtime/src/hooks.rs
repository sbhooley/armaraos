//! Integration seam for hosts (e.g. OpenFang) — all methods default to no-ops.

use uuid::Uuid;

use crate::engine::{MemoryContext, TurnOutcome};
use ainl_graph_extractor::ExtractionReport;

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
