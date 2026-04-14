//! Optional **ainl-runtime** orchestration shim (Cargo feature **`ainl-runtime-engine`**).
//!
//! - Opens a **second** [`ainl_memory::SqliteGraphStore`] handle to the same `ainl_memory.db` as
//!   [`crate::graph_memory_writer::GraphMemoryWriter`] (see [`GraphMemoryWriter::sqlite_database_path_for_agent`]).
//! - Wraps [`ainl_runtime::AinlRuntime`] with **`evolution_writes_enabled(false)`** so OpenFang keeps writing the
//!   evolution persona row; registers [`ainl_runtime::GraphPatchAdapter::with_host`] for patch summary logging.
//! - Exposes [`AinlRuntimeBridge::run_turn`] / [`AinlRuntimeBridge::run_turn_async`] and maps
//!   [`ainl_runtime::TurnOutcome`] into this module’s [`TurnOutcome`] for session text + tracing.
//!
//! **Operator guide:** `docs/ainl-runtime-integration.md`. Default chat execution stays in [`crate::agent_loop`]
//! unless manifest **`ainl_runtime_engine`** or **`AINL_RUNTIME_ENGINE=1`** is set (and graph memory opens).

use std::collections::HashMap;
use std::sync::Arc;

use ainl_memory::SqliteGraphStore;
use ainl_runtime::{
    AinlRuntime, AinlRuntimeError, GraphPatchAdapter, GraphPatchHostDispatch, RuntimeConfig,
    TurnInput, TurnOutcome as AinlTurnOutcome, TurnResult as AinlTurnResult, TurnStatus,
};
use serde_json::Value;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

use crate::graph_memory_writer::GraphMemoryWriter;

/// Per-turn inputs supplied by the host (OpenFang) alongside the user message.
#[derive(Debug, Clone, Default)]
pub struct TurnContext {
    pub tools_invoked: Vec<String>,
    pub trace_event: Option<Value>,
    pub depth: u32,
    pub frame: HashMap<String, Value>,
    pub emit_targets: Vec<uuid::Uuid>,
    /// Carried into the mapped EndTurn-style payload; ainl-runtime episode rows still use `None`
    /// for `delegation_to` today — this field is host metadata only.
    pub delegation_to: Option<String>,
    /// Cognitive vitals from the LLM completion (passed through to `TurnInput` → episode write).
    pub vitals_gate: Option<String>,
    pub vitals_phase: Option<String>,
    pub vitals_trust: Option<f32>,
}

/// OpenFang-facing summary of an **ainl-runtime** turn (mirrors fields we surface on an EndTurn-style event).
#[derive(Debug, Clone)]
pub struct TurnOutcome {
    pub output: String,
    pub tool_calls: Vec<String>,
    pub delegation_to: Option<String>,
    pub cost_estimate: Option<f64>,
}

/// Structured log line for observability: EndTurn-shaped fields after an **ainl-runtime** turn.
pub fn log_mapped_end_turn_fields(agent_name: &str, mapped: &TurnOutcome) {
    info!(
        agent = %agent_name,
        output_len = mapped.output.len(),
        tool_calls = ?mapped.tool_calls,
        delegation_to = ?mapped.delegation_to,
        cost_estimate = ?mapped.cost_estimate,
        "ainl-runtime-engine: EndTurn-shaped summary (no LLM stop_reason in ainl-runtime)"
    );
}

/// Maps **ainl-runtime** output into [`TurnOutcome`] and logs anything we do not forward to the dashboard yet.
pub fn map_ainl_turn_outcome(
    ainl: &AinlTurnOutcome,
    turn_ctx: &TurnContext,
) -> TurnOutcome {
    let r = ainl.result();
    let output = build_output_text(r);
    let mut tool_calls = turn_ctx.tools_invoked.clone();
    for p in &r.patch_dispatch_results {
        if let Some(name) = &p.adapter_name {
            if !tool_calls.iter().any(|t| t == name) {
                tool_calls.push(name.clone());
            }
        }
    }
    if tool_calls.is_empty() {
        tool_calls.push("turn".to_string());
    }

    warn_on_unmapped_ainl_fields(ainl, r);

    // ainl-runtime does not emit token-metered USD; surface a tiny deterministic host estimate so
    // budget / usage_footer paths can attribute non-zero work when LLM usage is zero.
    let step_units = r.steps_executed as f64;
    let cost_estimate = Some((step_units * 1.0e-6).max(1e-9));

    TurnOutcome {
        output,
        tool_calls,
        delegation_to: turn_ctx.delegation_to.clone(),
        cost_estimate,
    }
}

fn build_output_text(r: &AinlTurnResult) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(p) = &r.persona_prompt_contribution {
        if !p.trim().is_empty() {
            parts.push(p.trim().to_string());
        }
    }
    if parts.is_empty() {
        parts.push(format!(
            "[ainl-runtime] turn finished (episode_id={}, status={:?}, steps={})",
            r.episode_id, r.status, r.steps_executed
        ));
    }
    parts.join("\n\n")
}

fn warn_on_unmapped_ainl_fields(ainl: &AinlTurnOutcome, r: &AinlTurnResult) {
    if ainl.is_partial_success() {
        warn!(
            warnings = ?ainl.warnings(),
            "ainl-runtime: partial success — TurnWarning entries have no OpenFang EndTurn equivalent yet"
        );
    }
    if r.extraction_report.is_some() {
        warn!("ainl-runtime: extraction_report is not mapped to OpenFang EndTurn telemetry yet");
    }
    let mc = &r.memory_context;
    if !mc.recent_episodes.is_empty()
        || !mc.relevant_semantic.is_empty()
        || !mc.active_patches.is_empty()
        || mc.persona_snapshot.is_some()
    {
        warn!(
            recent_episodes = mc.recent_episodes.len(),
            relevant_semantic = mc.relevant_semantic.len(),
            active_patches = mc.active_patches.len(),
            has_persona_snapshot = mc.persona_snapshot.is_some(),
            "ainl-runtime: MemoryContext slices are summarized in output text only; no structured EndTurn mapping yet"
        );
    }
    if !r.patch_dispatch_results.is_empty() {
        for p in &r.patch_dispatch_results {
            if p.adapter_output.is_some() {
                warn!(
                    label = %p.label,
                    "ainl-runtime: patch_dispatch_results.adapter_output not mapped to OpenFang tool_calls payload"
                );
            }
        }
    }
    if r.status != TurnStatus::Ok {
        warn!(status = ?r.status, "ainl-runtime: TurnStatus is reflected in output text only");
    }
    debug!(
        "ainl-runtime: token-level LLM cost is unavailable — host maps steps_executed to a micro-USD estimate"
    );
}

struct GraphPatchLogHost {
    agent_id: String,
}

impl GraphPatchHostDispatch for GraphPatchLogHost {
    fn on_patch_dispatch(&self, envelope: Value) -> Result<Value, String> {
        info!(
            agent_id = %self.agent_id,
            patch = %envelope,
            "ainl-runtime GraphPatchAdapter host dispatch (graph writer shared via same DB path)"
        );
        Ok(envelope)
    }
}

/// Thin embedder for [`AinlRuntime`] + the agent’s [`GraphMemoryWriter`] handle.
pub struct AinlRuntimeBridge {
    runtime: Arc<std::sync::Mutex<AinlRuntime>>,
    graph_writer: Arc<Mutex<GraphMemoryWriter>>,
}

#[cfg(test)]
pub(crate) mod test_hooks {
    use std::sync::atomic::{AtomicUsize, Ordering};

    static BRIDGE_NEW_COUNT: AtomicUsize = AtomicUsize::new(0);

    pub fn reset_bridge_new_count() {
        BRIDGE_NEW_COUNT.store(0, Ordering::SeqCst);
    }

    pub fn bridge_new_count() -> usize {
        BRIDGE_NEW_COUNT.load(Ordering::SeqCst)
    }

    pub(crate) fn record_bridge_new() {
        BRIDGE_NEW_COUNT.fetch_add(1, Ordering::SeqCst);
    }
}

impl AinlRuntimeBridge {
    /// Opens **ainl-runtime** on the same SQLite file as `graph_writer` (second connection).
    /// Delegation cap defaults to `8`; prefer [`Self::with_delegation_cap`] from the agent loop so it
    /// matches `[runtime_limits].max_agent_call_depth`.
    pub fn new(graph_writer: Arc<Mutex<GraphMemoryWriter>>) -> Result<Self, String> {
        Self::with_delegation_cap(graph_writer, 8)
    }

    /// Same as [`Self::new`] but aligns **ainl-runtime**’s nested `run_turn` depth guard with the host.
    pub fn with_delegation_cap(
        graph_writer: Arc<Mutex<GraphMemoryWriter>>,
        max_delegation_depth: u32,
    ) -> Result<Self, String> {
        #[cfg(test)]
        test_hooks::record_bridge_new();

        let agent_id = graph_writer
            .try_lock()
            .map_err(|_| {
                "ainl_runtime_bridge: GraphMemoryWriter mutex is held; cannot construct bridge synchronously"
                    .to_string()
            })?
            .agent_id()
            .to_string();
        let path = GraphMemoryWriter::sqlite_database_path_for_agent(&agent_id)?;
        let store = SqliteGraphStore::open(&path).map_err(|e| e.to_string())?;
        let max_delegation_depth = max_delegation_depth.max(1);
        let cfg = RuntimeConfig {
            agent_id: agent_id.clone(),
            max_delegation_depth,
            enable_graph_memory: true,
            ..Default::default()
        };
        let mut runtime = AinlRuntime::new(cfg, store).with_evolution_writes_enabled(false);
        runtime.register_adapter(GraphPatchAdapter::with_host(Arc::new(GraphPatchLogHost {
            agent_id,
        })));

        Ok(Self {
            runtime: Arc::new(std::sync::Mutex::new(runtime)),
            graph_writer,
        })
    }

    fn build_turn_input(
        _agent_id: &str,
        user_message: &str,
        ctx: &TurnContext,
    ) -> TurnInput {
        let mut frame = ctx.frame.clone();
        // Inject cognitive vitals as AINL frame keys so programs can branch on them
        // via `core.GET result "_vitals_gate"` without any syntax changes.
        if let Some(ref gate) = ctx.vitals_gate {
            frame.insert("_vitals_gate".to_string(), Value::String(gate.clone()));
        }
        if let Some(ref phase) = ctx.vitals_phase {
            frame.insert("_vitals_phase".to_string(), Value::String(phase.clone()));
        }
        if let Some(trust) = ctx.vitals_trust {
            frame.insert("_vitals_trust".to_string(), Value::from(trust as f64));
        }
        TurnInput {
            user_message: user_message.to_string(),
            tools_invoked: ctx.tools_invoked.clone(),
            trace_event: ctx.trace_event.clone(),
            depth: ctx.depth,
            frame,
            emit_targets: ctx.emit_targets.clone(),
            vitals_gate: ctx.vitals_gate.clone(),
            vitals_phase: ctx.vitals_phase.clone(),
            vitals_trust: ctx.vitals_trust,
        }
    }

    /// Synchronous turn (blocks current thread on SQLite / graph work).
    pub fn run_turn(
        &self,
        agent_id: &str,
        input: &str,
        context: TurnContext,
    ) -> Result<TurnOutcome, AinlRuntimeError> {
        let turn_in = Self::build_turn_input(agent_id, input, &context);
        let ainl_outcome = self.runtime.lock().unwrap().run_turn(turn_in)?;
        Ok(map_ainl_turn_outcome(&ainl_outcome, &context))
    }

    /// Async turn: runs [`AinlRuntime::run_turn_async`] on a blocking thread (nested `block_on` when
    /// invoked from the Tokio multithread runtime).
    #[allow(clippy::await_holding_lock)] // `AinlRuntime` is behind `std::sync::Mutex`; ainl-runtime offloads SQLite internally.
    pub fn run_turn_async(
        &self,
        agent_id: &str,
        input: &str,
        context: TurnContext,
    ) -> tokio::task::JoinHandle<Result<TurnOutcome, String>> {
        let rt = Arc::clone(&self.runtime);
        let gw = Arc::clone(&self.graph_writer);
        let agent_id = agent_id.to_string();
        let input_s = input.to_string();
        tokio::task::spawn_blocking(move || {
            let _gw = gw;
            let turn_in = Self::build_turn_input(&agent_id, &input_s, &context);
            let handle = tokio::runtime::Handle::try_current()
                .map_err(|e| format!("ainl-runtime-engine: no Tokio handle: {e}"))?;
            let async_out = handle.block_on(async move {
                let mut g = rt.lock().unwrap();
                g.run_turn_async(turn_in).await
            });
            match async_out {
                Ok(o) => Ok(map_ainl_turn_outcome(&o, &context)),
                Err(e) => Err(e.to_string()),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_ainl_runtime_bridge_round_trips_simple_turn() {
        let dir = tempfile::tempdir().expect("tempdir");
        let prev = std::env::var("ARMARAOS_HOME").ok();
        std::env::set_var("ARMARAOS_HOME", dir.path().as_os_str());
        let agent = format!("bridge-test-{}", uuid::Uuid::new_v4());
        let writer = GraphMemoryWriter::open(&agent).expect("open graph memory");
        let bridge = AinlRuntimeBridge::new(Arc::new(Mutex::new(writer))).expect("bridge");
        let out = bridge
            .run_turn(
                &agent,
                "hello from ainl-runtime shim",
                TurnContext::default(),
            )
            .expect("run_turn");
        assert!(!out.output.trim().is_empty(), "output: {:?}", out.output);
        if let Some(p) = prev {
            std::env::set_var("ARMARAOS_HOME", p);
        } else {
            std::env::remove_var("ARMARAOS_HOME");
        }
    }
}
