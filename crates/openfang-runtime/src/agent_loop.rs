//! Core agent execution loop.
//!
//! The agent loop handles receiving a user message, recalling relevant memories,
//! calling the LLM, executing tool calls, and saving the conversation.
//!
//! ## AINL graph memory (`GraphMemoryWriter` / `ainl_memory.db`)
//!
//! [`run_agent_loop`] and [`run_agent_loop_streaming`] **always** attempt to open
//! [`crate::graph_memory_writer::GraphMemoryWriter`] for `session.agent_id` at loop start.
//! The `graph_memory` binding is `None` only when that open fails (permissions, disk, schema,
//! etc.); there is no alternate code path that skips the writer for “normal” agents.
//! When open succeeds, post-turn persistence (`record_turn`, `record_fact_with_tags`,
//! `record_pattern`, spawned `run_persona_evolution_pass`, and when `AINL_PERSONA_EVOLUTION=1`
//! also [`crate::persona_evolution::PersonaEvolutionHook::evolve_from_turn`]) uses the same handle
//! for both streaming and non-streaming loops.

use crate::auth_cooldown::{CooldownVerdict, ProviderCooldown};
use crate::context_budget::{apply_context_guard, truncate_tool_result_dynamic, ContextBudget};
use crate::context_overflow::{recover_from_overflow, RecoveryStage};
use crate::embedding::EmbeddingDriver;
use crate::kernel_handle::KernelHandle;
use crate::llm_driver::{CompletionRequest, DriverConfig, LlmDriver, LlmError, StreamEvent};
use crate::llm_errors;
use crate::loop_guard::{LoopGuard, LoopGuardConfig, LoopGuardVerdict};
use crate::mcp::McpConnection;
use crate::tool_runner;
use crate::web_search::WebToolsContext;
use openfang_memory::session::Session;
use openfang_memory::MemorySubstrate;
use openfang_skills::registry::SkillRegistry;
use openfang_types::agent::{AgentManifest, FallbackModel};
use openfang_types::config::LlmConfig;
use openfang_types::error::{OpenFangError, OpenFangResult};
use openfang_types::memory::{Memory, MemoryFilter, MemorySource};
use openfang_types::message::{
    ContentBlock, Message, MessageContent, Role, StopReason, TokenUsage,
};
use openfang_types::runtime_limits::EffectiveRuntimeLimits;
use openfang_types::tool::{ToolCall, ToolDefinition};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

/// When a kernel handle is present, graph-memory writes publish `SystemEvent::GraphMemoryWrite` for dashboard SSE.
fn graph_memory_sse_hook(
    kernel: &Option<Arc<dyn KernelHandle>>,
) -> Option<
    Arc<
        dyn Fn(String, String, Option<openfang_types::event::GraphMemoryWriteProvenance>)
            + Send
            + Sync,
    >,
> {
    kernel.as_ref().map(|k| {
        let k = Arc::clone(k);
        Arc::new(
            move |agent_id: String,
                  kind: String,
                  provenance: Option<openfang_types::event::GraphMemoryWriteProvenance>| {
                let k = Arc::clone(&k);
                tokio::spawn(async move {
                    match k
                        .notify_graph_memory_write(&agent_id, &kind, provenance)
                        .await
                    {
                        Ok(()) => {
                            crate::graph_memory_context::record_graph_memory_kernel_notify_ok();
                        }
                        Err(e) => {
                            crate::graph_memory_context::record_graph_memory_kernel_notify_err();
                            warn!(
                                agent_id = %agent_id,
                                kind = %kind,
                                error = %e,
                                "GraphMemoryWrite kernel notify failed (dashboard timeline/SSE may miss this write)"
                            );
                        }
                    }
                });
            },
        ) as Arc<
            dyn Fn(String, String, Option<openfang_types::event::GraphMemoryWriteProvenance>)
                + Send
                + Sync,
        >
    })
}

/// Correlation payload stored on Episode nodes (`trace_event` / `EpisodicNode.trace_event`).
fn graph_memory_turn_trace_json(
    agent_id: &str,
    orchestration_ctx: &Option<openfang_types::orchestration::OrchestrationContext>,
    compression_metrics: Option<&crate::prompt_compressor::CompressionMetrics>,
    adaptive_eco: Option<&serde_json::Value>,
) -> Option<serde_json::Value> {
    let mut m = serde_json::Map::new();
    m.insert(
        "agent_id".to_string(),
        serde_json::Value::String(agent_id.to_string()),
    );
    if let Some(o) = orchestration_ctx {
        if !o.trace_id.is_empty() {
            m.insert(
                "trace_id".to_string(),
                serde_json::Value::String(o.trace_id.clone()),
            );
        }
    }
    if let Some(cm) = compression_metrics {
        m.insert(
            "compression".to_string(),
            serde_json::json!({
                "mode": format!("{:?}", cm.mode).to_ascii_lowercase(),
                "original_tokens_est": cm.original_tokens,
                "compressed_tokens_est": cm.compressed_tokens,
                "tokens_saved_est": cm.tokens_saved,
                "savings_ratio_pct": cm.savings_ratio_pct,
                "semantic_preservation_score": cm.semantic_preservation_score,
                "elapsed_ms": cm.elapsed_ms,
            }),
        );
    }
    if let Some(a) = adaptive_eco {
        m.insert("adaptive_eco".to_string(), a.clone());
    }
    Some(serde_json::Value::Object(m))
}

fn graph_memory_trace_fact_tags(
    orchestration_ctx: &Option<openfang_types::orchestration::OrchestrationContext>,
) -> Vec<String> {
    orchestration_ctx
        .as_ref()
        .filter(|o| !o.trace_id.is_empty())
        .map(|o| vec![format!("trace_id:{}", o.trace_id)])
        .unwrap_or_default()
}

fn graph_memory_pattern_trace_id(
    orchestration_ctx: &Option<openfang_types::orchestration::OrchestrationContext>,
) -> Option<String> {
    orchestration_ctx
        .as_ref()
        .filter(|o| !o.trace_id.is_empty())
        .map(|o| o.trace_id.clone())
}

/// Append MCP readiness snapshot to the system prompt; when the kernel is available, persist a
/// digest in shared memory and emit a tagged semantic fact on change (planner retrieval).
async fn append_mcp_readiness_context(
    system_prompt: &mut String,
    mcp_connections: Option<&tokio::sync::Mutex<Vec<McpConnection>>>,
    kernel: &Option<Arc<dyn KernelHandle>>,
    graph_memory: &Option<crate::graph_memory_writer::GraphMemoryWriter>,
    agent_id: &openfang_types::agent::AgentId,
) {
    let Some(mcp_mtx) = mcp_connections else {
        return;
    };
    let guard = mcp_mtx.lock().await;
    let ev = crate::mcp_readiness::evaluate_from_connections(&guard);
    drop(guard);
    let appendix = crate::mcp_readiness::format_prompt_appendix(&ev.report, 1200);
    if !appendix.is_empty() {
        system_prompt.push_str("\n\n");
        system_prompt.push_str(&appendix);
    }
    let digest = crate::mcp_readiness::readiness_digest_json(&ev.report);
    let digest_str = digest.to_string();
    if let Some(ref k) = kernel {
        let key = format!("mcp_readiness_digest:{}", agent_id.0);
        let prev = k.memory_recall(&key).ok().flatten();
        let changed = prev.map(|v| v.to_string()) != Some(digest_str.clone());
        if changed {
            let _ = k.memory_store(&key, digest);
            if let Some(ref gm) = graph_memory {
                let fact = format!("MCP readiness snapshot: {}", digest_str);
                let mut tags = vec![
                    "readiness".to_string(),
                    "mcp".to_string(),
                    "planner".to_string(),
                ];
                for id in ev.report.checks.keys() {
                    tags.push(format!("readiness:{id}"));
                }
                gm.record_fact_with_tags(fact, 0.95, uuid::Uuid::new_v4(), &tags)
                    .await;
            }
        }
    }
}

#[cfg(feature = "ainl-runtime-engine")]
fn ainl_runtime_engine_switch_active_with_env(
    manifest_enabled: bool,
    ainl_runtime_engine_env: Option<&str>,
    env_disabled: bool,
) -> bool {
    if env_disabled {
        return false;
    }
    manifest_enabled || ainl_runtime_engine_env == Some("1")
}

#[cfg(feature = "ainl-runtime-engine")]
fn ainl_runtime_engine_switch_active(manifest: &AgentManifest) -> bool {
    ainl_runtime_engine_switch_active_with_env(
        manifest.ainl_runtime_engine,
        std::env::var("AINL_RUNTIME_ENGINE").ok().as_deref(),
        crate::ainl_runtime_engine_env_disabled(),
    )
}

#[cfg(feature = "ainl-runtime-engine")]
fn ainl_runtime_bridge_cache(
) -> &'static dashmap::DashMap<String, std::sync::Arc<crate::ainl_runtime_bridge::AinlRuntimeBridge>>
{
    static CACHE: std::sync::OnceLock<
        dashmap::DashMap<String, std::sync::Arc<crate::ainl_runtime_bridge::AinlRuntimeBridge>>,
    > = std::sync::OnceLock::new();
    CACHE.get_or_init(dashmap::DashMap::new)
}

#[cfg(feature = "ainl-runtime-engine")]
fn ainl_runtime_bridge_cache_hits_counter() -> &'static std::sync::atomic::AtomicU64 {
    static C: std::sync::OnceLock<std::sync::atomic::AtomicU64> = std::sync::OnceLock::new();
    C.get_or_init(|| std::sync::atomic::AtomicU64::new(0))
}

#[cfg(feature = "ainl-runtime-engine")]
fn ainl_runtime_bridge_cache_misses_counter() -> &'static std::sync::atomic::AtomicU64 {
    static C: std::sync::OnceLock<std::sync::atomic::AtomicU64> = std::sync::OnceLock::new();
    C.get_or_init(|| std::sync::atomic::AtomicU64::new(0))
}

#[cfg(feature = "ainl-runtime-engine")]
fn ainl_runtime_bridge_construct_failures_counter() -> &'static std::sync::atomic::AtomicU64 {
    static C: std::sync::OnceLock<std::sync::atomic::AtomicU64> = std::sync::OnceLock::new();
    C.get_or_init(|| std::sync::atomic::AtomicU64::new(0))
}

#[cfg(feature = "ainl-runtime-engine")]
fn ainl_runtime_bridge_run_failures_counter() -> &'static std::sync::atomic::AtomicU64 {
    static C: std::sync::OnceLock<std::sync::atomic::AtomicU64> = std::sync::OnceLock::new();
    C.get_or_init(|| std::sync::atomic::AtomicU64::new(0))
}

#[cfg(feature = "ainl-runtime-engine")]
pub(crate) fn ainl_runtime_bridge_cache_metrics_snapshot() -> (u64, u64, u64, u64) {
    use std::sync::atomic::Ordering;
    (
        ainl_runtime_bridge_cache_hits_counter().load(Ordering::Relaxed),
        ainl_runtime_bridge_cache_misses_counter().load(Ordering::Relaxed),
        ainl_runtime_bridge_construct_failures_counter().load(Ordering::Relaxed),
        ainl_runtime_bridge_run_failures_counter().load(Ordering::Relaxed),
    )
}

#[cfg(feature = "ainl-runtime-engine")]
fn ainl_runtime_bridge_cache_key(agent_id: &str, max_delegation_depth: u32) -> String {
    format!("{agent_id}:{max_delegation_depth}")
}

#[cfg(all(feature = "ainl-runtime-engine", test))]
fn ainl_runtime_bridge_cache_clear_for_tests() {
    use std::sync::atomic::Ordering;
    ainl_runtime_bridge_cache().clear();
    ainl_runtime_bridge_cache_hits_counter().store(0, Ordering::SeqCst);
    ainl_runtime_bridge_cache_misses_counter().store(0, Ordering::SeqCst);
    ainl_runtime_bridge_construct_failures_counter().store(0, Ordering::SeqCst);
    ainl_runtime_bridge_run_failures_counter().store(0, Ordering::SeqCst);
}

#[cfg(feature = "ainl-runtime-engine")]
fn get_or_create_ainl_runtime_bridge(
    agent_id: &str,
    gm: &crate::graph_memory_writer::GraphMemoryWriter,
    max_delegation_depth: u32,
) -> Result<std::sync::Arc<crate::ainl_runtime_bridge::AinlRuntimeBridge>, String> {
    let key = ainl_runtime_bridge_cache_key(agent_id, max_delegation_depth);
    if let Some(existing) = ainl_runtime_bridge_cache().get(&key) {
        ainl_runtime_bridge_cache_hits_counter().fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        return Ok(std::sync::Arc::clone(existing.value()));
    }
    ainl_runtime_bridge_cache_misses_counter().fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let gw = std::sync::Arc::new(tokio::sync::Mutex::new(gm.clone()));
    let bridge = std::sync::Arc::new(
        crate::ainl_runtime_bridge::AinlRuntimeBridge::with_delegation_cap(
            gw,
            max_delegation_depth,
        )
        .map_err(|e| e.to_string())?,
    );
    ainl_runtime_bridge_cache().insert(key, std::sync::Arc::clone(&bridge));
    Ok(bridge)
}

/// Runs **ainl-runtime** graph-memory bookkeeping for this turn, then lets the normal OpenFang
/// LLM loop produce the assistant reply. Does not append assistant messages or end the loop early.
#[cfg(feature = "ainl-runtime-engine")]
async fn run_ainl_runtime_engine_prelude(
    manifest: &AgentManifest,
    graph_memory: &Option<crate::graph_memory_writer::GraphMemoryWriter>,
    session_user_message: &str,
    agent_id_str: &str,
    runtime_limits: &EffectiveRuntimeLimits,
    orchestration_ctx: &Option<openfang_types::orchestration::OrchestrationContext>,
) -> Option<crate::ainl_runtime_bridge::AinlBridgeTelemetry> {
    if !ainl_runtime_engine_switch_active(manifest) {
        return None;
    }
    let Some(gm) = graph_memory else {
        warn!(
            agent = %manifest.name,
            "ainl-runtime-engine: graph memory unavailable; continuing with OpenFang loop"
        );
        return None;
    };
    let bridge = match get_or_create_ainl_runtime_bridge(
        agent_id_str,
        gm,
        runtime_limits.max_agent_call_depth,
    ) {
        Ok(b) => b,
        Err(e) => {
            ainl_runtime_bridge_construct_failures_counter()
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            warn!(
                agent = %manifest.name,
                error = %e,
                "ainl-runtime-engine: failed to construct bridge; continuing with OpenFang loop"
            );
            return None;
        }
    };
    let trace = graph_memory_turn_trace_json(agent_id_str, orchestration_ctx, None, None);
    let ctx = crate::ainl_runtime_bridge::TurnContext {
        tools_invoked: vec![],
        trace_event: trace,
        depth: 0,
        frame: HashMap::new(),
        emit_targets: vec![],
        delegation_to: None,
        vitals_gate: None,
        vitals_phase: None,
        vitals_trust: None,
    };
    let mapped = match bridge.run_turn(agent_id_str, session_user_message, ctx) {
        Ok(m) => m,
        Err(e) => {
            ainl_runtime_bridge_run_failures_counter()
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            warn!(
                agent = %manifest.name,
                error = %e,
                "ainl-runtime-engine: run_turn failed; continuing with OpenFang loop"
            );
            return None;
        }
    };
    // `ainl-runtime` writes via a separate SQLite handle, so those mutations do not naturally
    // pass through GraphMemoryWriter's direct write methods. Emit one observed-write signal so
    // the dashboard Graph Memory live timeline stays in sync for runtime-engine turns.
    gm.emit_write_observed("episode", None);
    crate::ainl_runtime_bridge::log_mapped_end_turn_fields(&manifest.name, &mapped);
    Some(mapped.telemetry.clone())
}

/// Expected per-agent SQLite path (must stay aligned with [`crate::graph_memory_writer::GraphMemoryWriter`]).
fn graph_memory_expected_db_path(agent_id: &openfang_types::agent::AgentId) -> std::path::PathBuf {
    openfang_types::config::openfang_home_dir()
        .join("agents")
        .join(agent_id.to_string())
        .join("ainl_memory.db")
}

/// After persona evolution, write a fresh `AgentGraphSnapshot` JSON for Python `ainl_graph_memory`.
///
/// Path resolution matches [`crate::graph_memory_writer::armaraos_graph_memory_export_json_path`]:
/// optional `AINL_GRAPH_MEMORY_ARMARAOS_EXPORT` (**directory**) → `{dir}/{agent_id}_graph_export.json`,
/// else `{openfang_home_dir()}/agents/{agent_id}/ainl_graph_memory_export.json`.
async fn graph_memory_refresh_armaraos_export_json(agent_id: &str) {
    let path = crate::graph_memory_writer::armaraos_graph_memory_export_json_path(agent_id);
    if let Some(parent) = path.parent() {
        if let Err(e) = tokio::fs::create_dir_all(parent).await {
            warn!(
                agent_id = %agent_id,
                path = %parent.display(),
                error = %e,
                "AINL graph memory: failed to create parent dir for ArmaraOS graph export"
            );
            return;
        }
    }
    match crate::graph_memory_writer::GraphMemoryWriter::export_graph_json_for_agent(agent_id) {
        Ok(v) => {
            let body = match serde_json::to_vec_pretty(&v) {
                Ok(b) => b,
                Err(e) => {
                    warn!(
                        agent_id = %agent_id,
                        error = %e,
                        "AINL graph memory: serialize failed for AINL_GRAPH_MEMORY_ARMARAOS_EXPORT refresh"
                    );
                    return;
                }
            };
            if let Err(e) = tokio::fs::write(&path, body).await {
                warn!(
                    agent_id = %agent_id,
                    path = %path.display(),
                    error = %e,
                    "AINL graph memory: failed to write AINL_GRAPH_MEMORY_ARMARAOS_EXPORT snapshot"
                );
            }
        }
        Err(e) => {
            warn!(
                agent_id = %agent_id,
                error = %e,
                "AINL graph memory: export failed while refreshing AINL_GRAPH_MEMORY_ARMARAOS_EXPORT"
            );
        }
    }
}

/// Maximum retries for rate-limited or overloaded API calls.
const MAX_RETRIES: u32 = 3;

/// Base delay for exponential backoff (milliseconds).
const BASE_RETRY_DELAY_MS: u64 = 1000;

/// Timeout for individual tool executions (seconds).
/// Raised from 120s to 300s so that shell_exec commands with a user-specified
/// timeout_seconds up to 300 are not silently capped by the outer tokio wrapper.
/// Long-running jobs (>300s) should use process_start/process_poll instead.
const TOOL_TIMEOUT_SECS: u64 = 300;

/// Timeout for inter-agent tool calls (seconds).
/// Agent delegation (agent_send, agent_spawn) can involve a full agent loop on the
/// target, so these need a significantly longer timeout than regular tools.
const AGENT_TOOL_TIMEOUT_SECS: u64 = 600;

/// Classifies a tool call after the synchronous pre-pass (loop guard + hooks).
/// `Resolved` carries a pre-built error block; `Pending` holds a call ready for
/// parallel async execution.
enum ToolDispatch<'a> {
    Resolved(openfang_types::message::ContentBlock),
    Pending {
        tool_call: &'a openfang_types::tool::ToolCall,
        verdict: crate::loop_guard::LoopGuardVerdict,
    },
}

/// Returns the appropriate timeout duration for a given tool name.
/// Inter-agent calls get a longer timeout since they may trigger full agent loops.
fn tool_timeout_for(tool_name: &str) -> Duration {
    match tool_name {
        // Inter-agent: can trigger a full nested agent loop
        "agent_send" | "agent_spawn" => Duration::from_secs(AGENT_TOOL_TIMEOUT_SECS),
        // Document processing: large files can be slow
        "document_extract" | "spreadsheet_build" => Duration::from_secs(180),
        // Channel messaging: network round-trip to external service
        "channel_send" | "channel_stream" => Duration::from_secs(30),
        // Media generation / external AI APIs: network + model latency
        "image_generate" | "text_to_speech" | "speech_to_text" | "media_describe"
        | "media_transcribe" => Duration::from_secs(300),
        // External A2A: remote agent may need time to process
        "a2a_send" | "a2a_discover" => Duration::from_secs(300),
        // Persistent process tools: process_start is fast (just spawns), poll/write/kill are instant
        "process_start" | "process_poll" | "process_write" | "process_kill" | "process_list" => {
            Duration::from_secs(30)
        }
        // shell_exec: the inner tool respects LLM-specified timeout_seconds (up to 300s).
        // This outer cap must be at least as large as the inner maximum so the agent loop
        // wrapper never kills a command that the model legitimately asked to run longer.
        "shell_exec" => Duration::from_secs(TOOL_TIMEOUT_SECS),
        _ => Duration::from_secs(TOOL_TIMEOUT_SECS),
    }
}

/// Detect when the LLM claims to have performed an action (sent, posted, emailed)
/// without actually calling any tools. Prevents hallucinated completions.
fn phantom_action_detected(text: &str) -> bool {
    let lower = text.to_lowercase();
    let action_verbs = ["sent ", "posted ", "emailed ", "delivered ", "forwarded "];
    let channel_refs = [
        "telegram",
        "whatsapp",
        "slack",
        "discord",
        "email",
        "channel",
        "message sent",
        "successfully sent",
        "has been sent",
    ];
    let has_action = action_verbs.iter().any(|v| lower.contains(v));
    let has_channel = channel_refs.iter().any(|c| lower.contains(c));
    has_action && has_channel
}

/// Detect when the LLM claims to have started/stopped a process without actually calling
/// `process_start` or `process_kill`. Fires even when other tools were used (e.g., process_list
/// was called but the model then claimed the process was started without calling process_start).
fn process_phantom_detected(text: &str, tools_called: &std::collections::HashSet<String>) -> bool {
    let lower = text.to_lowercase();

    // Phantom start: model claims process is now running without having called process_start
    if !tools_called.contains("process_start") {
        let start_claims = [
            "is up in background",
            "started it now",
            "starting it now",
            "starting now",
            "started now",
            "proc_1 is up",
            "process started",
            "bot started",
            "is now running",
            "is now up",
        ];
        if start_claims.iter().any(|p| lower.contains(p)) {
            return true;
        }
    }

    // Phantom kill/stop: model claims to have killed a process without calling process_kill
    if !tools_called.contains("process_kill") && !tools_called.contains("process_stop") {
        let kill_claims = [
            "killed it",
            "stopped the bot",
            "process stopped",
            "process killed",
            "is now stopped",
        ];
        if kill_claims.iter().any(|p| lower.contains(p)) {
            return true;
        }
    }

    false
}

/// User asked to register recurring / kernel work; model implied it did so without `schedule_*` / `cron_*` tools.
fn scheduling_phantom_detected(
    user_message: &str,
    assistant_text: &str,
    tools_called: &std::collections::HashSet<String>,
) -> bool {
    const SCHED: &[&str] = &[
        "schedule_create",
        "schedule_list",
        "schedule_delete",
        "cron_create",
        "cron_list",
        "cron_cancel",
    ];
    if tools_called.iter().any(|n| SCHED.contains(&n.as_str())) {
        return false;
    }
    let u = user_message.to_lowercase();
    let user_intent = (u.contains("schedule") || u.contains("cron"))
        && (u.contains("every ")
            || u.contains("daily")
            || u.contains("weekly")
            || u.contains("recurring")
            || u.contains("attach")
            || u.contains("ainl")
            || u.contains("timer"));
    if !user_intent {
        return false;
    }
    let a = assistant_text.to_lowercase();
    a.contains("scheduled")
        || a.contains("cron job")
        || a.contains("set up a schedule")
        || (a.contains("added to") && (a.contains("scheduler") || a.contains("schedule")))
        || a.contains("will run every")
        || a.contains("runs every")
        || (a.contains("created") && a.contains("job") && a.contains("recur"))
}

/// Returns true when the agent response text indicates an intentional silent completion.
/// Matches `NO_REPLY` (exact) and `[SILENT]` (case-insensitive).
fn is_silent_token(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed == "NO_REPLY" || trimmed.eq_ignore_ascii_case("[silent]")
}

/// Tracks error patterns across agent loop iterations to inject guidance that is targeted,
/// non-redundant, and coordinated with the loop guard.
///
/// Design principles:
/// - First occurrence of a new error → short, specific guidance matched to the error kind.
/// - Second occurrence of the same error pattern → stronger redirect ("stop, try differently").
/// - Third+ identical pattern → silence; loop guard is already steering; adding more text
///   wastes context and confuses the model.
/// - All-loop-guard-block errors → silence; loop guard messages are already self-contained.
/// - Clean round (no errors) → resets all counters so the next error is treated as first.
struct ToolErrorTracker {
    /// Fingerprint of the last error batch (sorted "tool:error_prefix" pairs).
    last_fingerprint: Option<String>,
    /// How many consecutive iterations produced the identical fingerprint.
    same_fingerprint_streak: usize,
}

impl ToolErrorTracker {
    fn new() -> Self {
        Self {
            last_fingerprint: None,
            same_fingerprint_streak: 0,
        }
    }

    /// Call when a tool round produced no errors so counters reset cleanly.
    fn record_success(&mut self) {
        self.last_fingerprint = None;
        self.same_fingerprint_streak = 0;
    }

    /// Compute the guidance text (if any) to inject after this tool round.
    ///
    /// Updates internal state; must be called exactly once per round.
    fn compute_guidance(
        &mut self,
        tool_result_blocks: &[ContentBlock],
        denial_count: usize,
    ) -> Option<String> {
        // Collect errors (excluding approval-denial ones handled separately)
        let all_errors: Vec<(&str, &str)> = tool_result_blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::ToolResult {
                    tool_name,
                    content,
                    is_error: true,
                    ..
                } => Some((tool_name.as_str(), content.as_str())),
                _ => None,
            })
            .collect();

        if all_errors.is_empty() {
            self.record_success();
            return None;
        }

        // Fingerprint: stable sorted "tool:error_prefix" pairs (60 chars of error text)
        let mut fp_parts: Vec<String> = all_errors
            .iter()
            .map(|(name, content)| {
                let prefix: String = content.chars().take(60).collect();
                format!("{name}:{prefix}")
            })
            .collect();
        fp_parts.sort();
        let fingerprint = fp_parts.join("|");

        if self.last_fingerprint.as_deref() == Some(&fingerprint) {
            self.same_fingerprint_streak += 1;
        } else {
            self.last_fingerprint = Some(fingerprint);
            self.same_fingerprint_streak = 1;
        }

        // Loop-guard block messages are self-contained — don't pile on.
        let all_loop_guard_blocks = all_errors.iter().all(|(_, content)| {
            content.starts_with("Blocked:") || content.starts_with("Circuit breaker:")
        });
        if all_loop_guard_blocks {
            return None;
        }

        let non_denial_errors = all_errors.len().saturating_sub(denial_count);
        if non_denial_errors == 0 {
            return None;
        }

        match self.same_fingerprint_streak {
            // First occurrence: targeted, concise guidance matched to the error kind.
            1 => {
                let has_missing_param = all_errors.iter().any(|(_, c)| {
                    c.contains("Missing '") && c.contains("' parameter")
                        || c.contains("Missing required")
                        || c.contains("missing required")
                });
                let failing_tools: Vec<&str> = all_errors
                    .iter()
                    .map(|(n, _)| *n)
                    .collect::<std::collections::HashSet<_>>()
                    .into_iter()
                    .collect();
                let tool_list = failing_tools.join(", ");

                if has_missing_param {
                    Some(format!(
                        "[System: Tool call(s) to [{tool_list}] are missing required parameters \
                         (received empty or incomplete input). Re-read the tool schema and \
                         supply all required fields — do not call tools with empty {{}} inputs.]"
                    ))
                } else {
                    Some(format!(
                        "[System: {non_denial_errors} tool call(s) failed ([{tool_list}]). \
                         Do not invent results from failed calls or take actions downstream of \
                         failed outputs. If the task is not finished, retry with different \
                         parameters, a different tool, or a different approach.]"
                    ))
                }
            }
            // Second identical failure: escalate — the first guidance was not acted on.
            2 => Some(
                "[System: The same tool error has occurred twice in a row. \
                 You are not making progress. Stop using the failing tool(s), \
                 recall your original goal, and switch to a completely different method \
                 to accomplish it.]"
                    .to_string(),
            ),
            // Third+ identical failure: silence — loop guard is blocking/warning and
            // adding more text only wastes context and derails the model further.
            _ => None,
        }
    }
}

/// Strip a provider prefix from a model ID before sending to the API.
///
/// Many models are stored as `provider/org/model` (e.g. `openrouter/google/gemini-2.5-flash`)
/// but the upstream API expects just `org/model` (e.g. `google/gemini-2.5-flash`).
pub fn strip_provider_prefix(model: &str, provider: &str) -> String {
    let slash_prefix = format!("{}/", provider);
    let colon_prefix = format!("{}:", provider);
    if model.starts_with(&slash_prefix) {
        model[slash_prefix.len()..].to_string()
    } else if model.starts_with(&colon_prefix) {
        model[colon_prefix.len()..].to_string()
    } else {
        model.to_string()
    }
}

/// Default context window size (tokens) for token-based trimming.
const DEFAULT_CONTEXT_WINDOW: usize = 200_000;

/// Agent lifecycle phase within the execution loop.
/// Used for UX indicators (typing, reactions) without coupling to channel types.
#[derive(Debug, Clone, PartialEq)]
pub enum LoopPhase {
    /// Agent is calling the LLM.
    Thinking,
    /// Agent is executing a tool.
    ToolUse { tool_name: String },
    /// Agent is streaming tokens.
    Streaming,
    /// Agent finished successfully.
    Done,
    /// Agent encountered an error.
    Error,
}

/// Callback for agent lifecycle phase changes.
/// Implementations should be non-blocking (fire-and-forget) to avoid slowing the loop.
pub type PhaseCallback = Arc<dyn Fn(LoopPhase) + Send + Sync>;

/// Result of an agent loop execution.
#[derive(Debug)]
pub struct AgentLoopResult {
    /// The final text response from the agent.
    pub response: String,
    /// Total token usage across all LLM calls.
    pub total_usage: TokenUsage,
    /// Number of iterations the loop ran.
    pub iterations: u32,
    /// Estimated cost in USD (populated by the kernel after the loop returns).
    pub cost_usd: Option<f64>,
    /// True when the agent intentionally chose not to reply (NO_REPLY token or [[silent]]).
    pub silent: bool,
    /// Reply directives extracted from the agent's response.
    pub directives: openfang_types::message::ReplyDirectives,
    /// Wall time for the full agent loop (LLM + tools), for dashboard latency.
    pub latency_ms: Option<u64>,
    /// When a fallback model or OpenRouter free-tier path was used instead of the primary.
    pub llm_fallback_note: Option<String>,
    /// Input token reduction percentage achieved by the prompt compressor (0 when off/passthrough).
    pub compression_savings_pct: u8,
    /// The compressed version of the user message (only set when savings_pct > 0; for diff UI).
    pub compressed_input: Option<String>,
    /// Optional semantic preservation score for the compressed input.
    pub compression_semantic_score: Option<f32>,
    /// Heuristic 0.0–1.0 confidence for adaptive eco policy this turn (when adaptive metadata exists).
    pub adaptive_confidence: Option<f32>,
    /// Counterfactual compression comparison (applied vs baselines / recommendation).
    pub eco_counterfactual: Option<openfang_types::adaptive_eco::EcoCounterfactualReceipt>,
    /// Effective eco mode after kernel policy (when `adaptive_eco` metadata is present).
    pub adaptive_eco_effective_mode: Option<String>,
    /// Resolver recommendation (may differ in shadow mode).
    pub adaptive_eco_recommended_mode: Option<String>,
    /// Machine-readable policy reasons for this turn.
    pub adaptive_eco_reason_codes: Option<Vec<String>>,
    /// Structured telemetry from ainl-runtime-engine, when that path handled the turn.
    pub ainl_runtime_telemetry: Option<crate::ainl_runtime_bridge::AinlBridgeTelemetry>,
}

/// Check whether a tool call is missing any required parameters.
///
/// Reads the JSON Schema `required` array from the tool definition and returns
/// a rich, self-contained error string that names every missing field with its
/// type and description — so the LLM can self-correct in one step without
/// needing to re-read the schema out-of-band.
///
/// Returns `None` when all required fields are present.
fn missing_required_params_error(
    tool_name: &str,
    input: &serde_json::Value,
    tool_def: &openfang_types::tool::ToolDefinition,
) -> Option<String> {
    let required = tool_def.input_schema.get("required")?.as_array()?;
    if required.is_empty() {
        return None;
    }
    let properties = tool_def.input_schema.get("properties");
    let mut missing: Vec<String> = Vec::new();
    for req in required {
        let field = match req.as_str() {
            Some(f) => f,
            None => continue,
        };
        let absent = input.get(field).is_none() || input[field].is_null();
        if absent {
            let field_desc = properties
                .and_then(|p| p.get(field))
                .map(|f| {
                    let ty = f.get("type").and_then(|t| t.as_str()).unwrap_or("any");
                    let desc = f.get("description").and_then(|d| d.as_str()).unwrap_or("");
                    if desc.is_empty() {
                        format!("{field} ({ty})")
                    } else {
                        format!("{field} ({ty}): {desc}")
                    }
                })
                .unwrap_or_else(|| field.to_string());
            missing.push(field_desc);
        }
    }
    if missing.is_empty() {
        return None;
    }
    Some(format!(
        "Error: '{tool_name}' called without required parameter(s). \
         Received: {input}. \
         Missing required field(s) — {}. \
         Retry with all required fields supplied.",
        missing.join("; ")
    ))
}

/// `manifest.model.system_prompt` should already be the output of [`crate::prompt_builder::build_system_prompt`]
/// (kernel sets [`crate::prompt_builder::KERNEL_EXPANDED_SYSTEM_PROMPT_META_KEY`]). If not, append minimal
/// host anchor sections so AINL / OpenClaw guidance is not missing.
fn loop_time_system_prompt_from_manifest(manifest: &AgentManifest) -> String {
    let mut system_prompt = manifest.model.system_prompt.clone();
    if !manifest
        .metadata
        .get(crate::prompt_builder::KERNEL_EXPANDED_SYSTEM_PROMPT_META_KEY)
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        crate::prompt_builder::ensure_mandatory_host_anchor_sections(&mut system_prompt);
    }
    system_prompt
}

/// Run the agent execution loop for a single user message.
///
/// This is the core of OpenFang: it loads session context, recalls memories,
/// runs the LLM in a tool-use loop, and saves the updated session.
#[allow(clippy::too_many_arguments)]
pub async fn run_agent_loop(
    manifest: &AgentManifest,
    user_message: &str,
    session: &mut Session,
    memory: &MemorySubstrate,
    driver: Arc<dyn LlmDriver>,
    available_tools: &[ToolDefinition],
    kernel: Option<Arc<dyn KernelHandle>>,
    skill_registry: Option<&SkillRegistry>,
    mcp_connections: Option<&tokio::sync::Mutex<Vec<McpConnection>>>,
    web_ctx: Option<&WebToolsContext>,
    browser_ctx: Option<&crate::browser::BrowserManager>,
    embedding_driver: Option<&(dyn EmbeddingDriver + Send + Sync)>,
    workspace_root: Option<&Path>,
    // Host AINL library root (~/.armaraos/ainl-library) for file_read / file_list virtual paths.
    ainl_library_root: Option<&Path>,
    on_phase: Option<&PhaseCallback>,
    media_engine: Option<&crate::media_understanding::MediaEngine>,
    tts_engine: Option<&crate::tts::TtsEngine>,
    docker_config: Option<&openfang_types::config::DockerSandboxConfig>,
    hooks: Option<&crate::hooks::HookRegistry>,
    context_window_tokens: Option<usize>,
    process_manager: Option<&crate::process_manager::ProcessManager>,
    user_content_blocks: Option<Vec<ContentBlock>>,
    btw_rx: Option<tokio::sync::mpsc::Receiver<String>>,
    redirect_rx: Option<tokio::sync::mpsc::Receiver<String>>,
    runtime_limits: EffectiveRuntimeLimits,
    orchestration_ctx: Option<openfang_types::orchestration::OrchestrationContext>,
    orchestration_live: Option<&tool_runner::OrchestrationLive>,
) -> OpenFangResult<AgentLoopResult> {
    tool_runner::MAX_AGENT_CALL_DEPTH_LIMIT
        .scope(
            std::cell::Cell::new(runtime_limits.max_agent_call_depth),
            async {
                info!(agent = %manifest.name, "Starting agent loop");

                // Initialize AINL graph memory writer (non-fatal if it fails)
                let graph_memory = match crate::graph_memory_writer::GraphMemoryWriter::open_with_notify(
                    &session.agent_id.to_string(),
                    graph_memory_sse_hook(&kernel),
                ) {
                    Ok(gm) => Some(gm),
                    Err(e) => {
                        let expected_db = graph_memory_expected_db_path(&session.agent_id);
                        warn!(
                            agent_id = %session.agent_id,
                            error = %e,
                            expected_db = %expected_db.display(),
                            "AINL graph memory: writer unavailable — episodes, facts, patterns, persona prompt hook, and evolution will not run for this agent until the DB opens successfully (check path and permissions)"
                        );
                        None
                    }
                };

                if let Some(ref gm) = graph_memory {
                    gm.drain_python_graph_memory_inbox().await;
                }

                let live_llm = kernel
                    .as_ref()
                    .and_then(|k| k.live_llm_config());

    // Extract hand-allowed env vars from manifest metadata (set by kernel for hand settings)
    let hand_allowed_env: Vec<String> = manifest
        .metadata
        .get("hand_allowed_env")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    let memory_policy = crate::graph_memory_context::MemoryContextPolicy::from_manifest_for_agent(
        manifest,
        Some(&session.agent_id.to_string()),
    );
    if memory_policy.temporary_mode {
        crate::graph_memory_context::record_temp_mode_read_suppressed();
        info!(
            agent = %manifest.name,
            "Memory temporary mode enabled: skipping runtime memory recalls and graph-memory prompt injection"
        );
    }

    // Recall relevant memories — prefer vector similarity search when embedding driver is available
    let memories = if !memory_policy.allow_reads() {
        Vec::new()
    } else if let Some(emb) = embedding_driver {
        match emb.embed_one(user_message).await {
            Ok(query_vec) => {
                debug!("Using vector recall (dims={})", query_vec.len());
                memory
                    .recall_with_embedding_async(
                        user_message,
                        5,
                        Some(MemoryFilter {
                            agent_id: Some(session.agent_id),
                            ..Default::default()
                        }),
                        Some(&query_vec),
                    )
                    .await
                    .unwrap_or_default()
            }
            Err(e) => {
                warn!("Embedding recall failed, falling back to text search: {e}");
                memory
                    .recall(
                        user_message,
                        5,
                        Some(MemoryFilter {
                            agent_id: Some(session.agent_id),
                            ..Default::default()
                        }),
                    )
                    .await
                    .unwrap_or_default()
            }
        }
    } else {
        memory
            .recall(
                user_message,
                5,
                Some(MemoryFilter {
                    agent_id: Some(session.agent_id),
                    ..Default::default()
                }),
            )
            .await
            .unwrap_or_default()
    };

    // Fire BeforePromptBuild hook
    let agent_id_str = session.agent_id.0.to_string();
    if let Some(hook_reg) = hooks {
        let ctx = crate::hooks::HookContext {
            agent_name: &manifest.name,
            agent_id: agent_id_str.as_str(),
            event: openfang_types::agent::HookEvent::BeforePromptBuild,
            data: serde_json::json!({
                "system_prompt": &manifest.model.system_prompt,
                "user_message": user_message,
            }),
        };
        let _ = hook_reg.fire(&ctx);
    }

    // Build the system prompt — kernel expands `[model].system_prompt` via prompt_builder; we
    // append recalled memories here since they are resolved at loop time.
    let mut system_prompt = loop_time_system_prompt_from_manifest(manifest);
    if !memories.is_empty() {
        let mem_pairs: Vec<(String, String)> = memories
            .iter()
            .map(|m| (String::new(), m.content.clone()))
            .collect();
        system_prompt.push_str("\n\n");
        system_prompt.push_str(&crate::prompt_builder::build_memory_section(&mem_pairs));
    }

    // Orchestration context: append hierarchical orchestration details to system prompt
    if let Some(ref octx) = orchestration_ctx {
        system_prompt.push_str("\n\n## Orchestration Context\n");
        system_prompt.push_str(&octx.system_prompt_appendix(runtime_limits.max_agent_call_depth));
    }

    // Persona hook: query AINL graph memory for PersonaNodes and prepend
    // to system prompt so the agent's learned traits affect every LLM call.
    if memory_policy.allow_reads() {
        if let Some(ref gm) = graph_memory {
            let prompt_ctx =
                crate::graph_memory_context::build_prompt_memory_context(gm, &memory_policy).await;
            if !prompt_ctx.is_empty() {
                system_prompt.push_str(&prompt_ctx.to_prompt_block());
            }
            if !prompt_ctx.selection_debug.is_empty() {
                debug!(
                    agent_id = %session.agent_id,
                    why_selected = %serde_json::Value::Array(prompt_ctx.selection_debug.clone()),
                    "graph-memory why_selected diagnostics (non-streaming)"
                );
            }
        }
    }
    if memory_policy.allow_reads() {
        if let Some(ref gm) = graph_memory {
        let persona_nodes = gm.recall_persona(60 * 60 * 24 * 90).await; // 90 days
        if !persona_nodes.is_empty() {
            let traits: Vec<String> = persona_nodes
                .iter()
                .filter_map(|n| {
                    if let ainl_memory::AinlNodeType::Persona { persona } = &n.node_type {
                        if persona.strength >= 0.1 {
                            Some(format!(
                                "{} (strength={:.2})",
                                persona.trait_name, persona.strength
                            ))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                })
                .collect();
            if !traits.is_empty() {
                let persona_instruction = format!(
                    "\n\n[Persona traits active: {}]",
                    traits.join(", ")
                );
                system_prompt.push_str(&persona_instruction);
                debug!(
                    agent_id = %session.agent_id,
                    trait_count = traits.len(),
                    "AINL persona hook: injected {} trait(s) into system prompt",
                    traits.len()
                );
            }
        }
    }
    }

    let graph_memory_for_mcp = if memory_policy.allow_writes() {
        graph_memory.clone()
    } else {
        crate::graph_memory_context::record_temp_mode_write_suppressed();
        None
    };
    append_mcp_readiness_context(
        &mut system_prompt,
        mcp_connections,
        &kernel,
        &graph_memory_for_mcp,
        &session.agent_id,
    )
    .await;

    // Ultra Cost-Efficient Mode: compress user message before storing in session / LLM context.
    // Memory recall above intentionally uses the original `user_message` for semantic similarity.
    // Per-agent metadata `efficient_mode` wins over the global config (injected by kernel via
    // `.entry().or_insert_with()` so the manifest value is never overwritten).
    let mut compression_savings_pct: u8 = 0;
    let mode = manifest
        .metadata
        .get("efficient_mode")
        .and_then(|v| v.as_str())
        .map(crate::prompt_compressor::EfficientMode::parse_natural_language)
        .unwrap_or_default();
    let (r, compression_metrics) =
        crate::prompt_compressor::compress_with_metrics(user_message, mode, None);
    if r.tokens_saved() > 0 {
        let ratio_pct = 100u64.saturating_sub(
            (r.compressed_tokens as u64 * 100) / r.original_tokens.max(1) as u64,
        );
        compression_savings_pct = ratio_pct.min(100) as u8;
        // $3/M input is the Claude Sonnet 4.6 list price; actual savings vary by model.
        let est_savings_usd = r.tokens_saved() as f64 / 1_000_000.0 * 3.0;
        info!(
            orig_tok = r.original_tokens,
            compressed_tok = r.compressed_tokens,
            saved_tok = r.tokens_saved(),
            savings_pct = ratio_pct,
            est_savings_usd = format!("{est_savings_usd:.6}"),
            "prompt:compressed"
        );
    }
    let eco_mode_label = match mode {
        crate::prompt_compressor::EfficientMode::Off => "off",
        crate::prompt_compressor::EfficientMode::Balanced => "balanced",
        crate::prompt_compressor::EfficientMode::Aggressive => "aggressive",
    };
    crate::eco_telemetry::record_turn(
        &session.agent_id.0.to_string(),
        eco_mode_label,
        user_message,
        &r.text,
        compression_savings_pct,
    );
    let _compressed_msg = if mode != crate::prompt_compressor::EfficientMode::Off {
        Some(r.text.clone())
    } else {
        None
    };
    // Keep the compressed text for diff UI (returned in AgentLoopResult for dashboard).
    let compressed_input: Option<String> = if compression_savings_pct > 0 {
        _compressed_msg.clone()
    } else {
        None
    };
    let compression_semantic_score = compression_metrics.semantic_preservation_score;
    let adaptive_snap: Option<openfang_types::adaptive_eco::AdaptiveEcoTurnSnapshot> = manifest
        .metadata
        .get("adaptive_eco")
        .and_then(|v| serde_json::from_value(v.clone()).ok());
    let adaptive_confidence = adaptive_snap.as_ref().map(|s| {
        openfang_types::adaptive_eco::compute_adaptive_confidence(s, compression_semantic_score)
    });
    let eco_counterfactual = crate::eco_counterfactual::build_eco_counterfactual_receipt(
        user_message,
        mode,
        &r,
        compression_savings_pct,
        adaptive_snap.as_ref(),
    );
    let adaptive_eco_effective_mode = adaptive_snap.as_ref().map(|s| s.effective_mode.clone());
    let adaptive_eco_recommended_mode = adaptive_snap.as_ref().map(|s| s.recommended_mode.clone());
    let adaptive_eco_reason_codes = adaptive_snap.as_ref().map(|s| s.reason_codes.clone());
    let session_user_message: &str = _compressed_msg.as_deref().unwrap_or(user_message);

    // Add the user message to session history.
    // When content blocks are provided (e.g. text + image from a channel),
    // use multimodal message format so the LLM receives the image for vision.
    if let Some(blocks) = user_content_blocks {
        session.messages.push(Message::user_with_blocks(blocks));
    } else {
        session.messages.push(Message::user(session_user_message));
    }

    // Build the messages for the LLM, filtering system messages
    // System prompt goes into the separate `system` field.
    // NOTE: We build llm_messages BEFORE stripping images so the LLM
    // sees the full image data for the current turn.
    let llm_messages: Vec<Message> = session
        .messages
        .iter()
        .filter(|m| m.role != Role::System)
        .cloned()
        .collect();

    // Strip Image blocks from session to prevent base64 bloat.
    // The LLM already received them via llm_messages above.
    for msg in session.messages.iter_mut() {
        if let MessageContent::Blocks(blocks) = &mut msg.content {
            let had_images = blocks
                .iter()
                .any(|b| matches!(b, ContentBlock::Image { .. }));
            if had_images {
                blocks.retain(|b| !matches!(b, ContentBlock::Image { .. }));
                if blocks.is_empty() {
                    blocks.push(ContentBlock::Text {
                        text: "[Image processed]".to_string(),
                        provider_metadata: None,
                    });
                }
            }
        }
    }

    // Validate and repair session history (drop orphans, merge consecutive)
    let mut messages = crate::session_repair::validate_and_repair(&llm_messages);

    // Inject canonical context as the first user message (not in system prompt)
    // to keep the system prompt stable across turns for provider prompt caching.
    if let Some(cc_msg) = manifest
        .metadata
        .get("canonical_context_msg")
        .and_then(|v| v.as_str())
    {
        if !cc_msg.is_empty() {
            messages.insert(0, Message::user(cc_msg));
        }
    }

    let mut total_usage = TokenUsage::default();
    let final_response;

    // Safety valve: trim excessively long message histories to prevent context overflow.
    // The full compaction system handles sophisticated summarization, but this prevents
    // the catastrophic case where 200+ messages cause instant context overflow.
    if messages.len() > runtime_limits.max_history_messages {
        let trim_count = messages.len() - runtime_limits.max_history_messages;
        warn!(
            agent = %manifest.name,
            total_messages = messages.len(),
            trimming = trim_count,
            "Trimming old messages to prevent context overflow"
        );
        messages.drain(..trim_count);
        // Re-validate after trimming: the drain may have split a ToolUse/ToolResult
        // pair across the cut boundary, leaving orphaned blocks that cause the LLM
        // to return empty responses (input_tokens=0).
        messages = crate::session_repair::validate_and_repair(&messages);
    }

    // Use autonomous config max_iterations if set, else `[runtime_limits]` default.
    let max_iterations = manifest
        .autonomous
        .as_ref()
        .map(|a| a.max_iterations)
        .unwrap_or(runtime_limits.max_iterations);

    // Initialize loop guard — scale circuit breaker for autonomous agents
    let loop_guard_config = {
        let mut cfg = LoopGuardConfig::default();
        if max_iterations > cfg.global_circuit_breaker {
            cfg.global_circuit_breaker = max_iterations * 3;
        }
        cfg
    };
    let mut loop_guard = LoopGuard::new(loop_guard_config);
    let mut consecutive_max_tokens: u32 = 0;
    // Counts consecutive iterations where every tool call was blocked by the loop guard.
    // Used to exit early when the model is clearly stuck calling the same blocked tools.
    let mut consecutive_all_blocked: u32 = 0;
    let mut tool_error_tracker = ToolErrorTracker::new();
    // Tracks the names of all tools called since the last EndTurn response, so process
    // phantom detection can tell whether the model claimed to start/stop a process without
    // actually calling the corresponding tool.
    let mut last_tools_called: std::collections::HashSet<String> = Default::default();

    // Build context budget from model's actual context window (or fallback to default)
    let ctx_window = context_window_tokens.unwrap_or(DEFAULT_CONTEXT_WINDOW);
    let context_budget = ContextBudget::new(ctx_window);
    let mut any_tools_executed = false;
    let loop_t0 = std::time::Instant::now();
    let mut llm_fallback_note: Option<String> = None;

    #[cfg(feature = "ainl-runtime-engine")]
    let ainl_prelude_telemetry: Option<crate::ainl_runtime_bridge::AinlBridgeTelemetry> = {
        let mut ainl_runtime_turn_allowed = true;
        if ainl_runtime_engine_switch_active(manifest) {
            if let Some(ref kh) = kernel {
                if kh.requires_approval("ainl_runtime_engine") {
                    let summary = format!(
                        "{} requests ainl-runtime-engine (message {} chars)",
                        manifest.name,
                        session_user_message.len()
                    );
                    match kh
                        .request_approval(agent_id_str.as_str(), "ainl_runtime_engine", &summary)
                        .await
                    {
                        Ok(true) => {}
                        Ok(false) => {
                            warn!(
                                agent = %manifest.name,
                                "ainl-runtime-engine: human approval denied — continuing with OpenFang loop"
                            );
                            ainl_runtime_turn_allowed = false;
                        }
                        Err(e) => {
                            warn!(
                                agent = %manifest.name,
                                error = %e,
                                "ainl-runtime-engine: approval request failed — continuing with OpenFang loop"
                            );
                            ainl_runtime_turn_allowed = false;
                        }
                    }
                }
            }
        }
        if ainl_runtime_turn_allowed {
            run_ainl_runtime_engine_prelude(
                manifest,
                &graph_memory,
                session_user_message,
                agent_id_str.as_str(),
                &runtime_limits,
                &orchestration_ctx,
            )
            .await
        } else {
            None
        }
    };

    let mut btw_rx = btw_rx;
    let mut redirect_rx = redirect_rx;
    for iteration in 0..max_iterations {
        debug!(iteration, "Agent loop iteration");

        // Drain any /btw context injections the user sent while this loop was running.
        // Each injection is added as a user message so the next LLM call sees it.
        if let Some(ref mut rx) = btw_rx {
            while let Ok(btw_text) = rx.try_recv() {
                info!("Injecting /btw context ({} chars)", btw_text.len());
                messages.push(openfang_types::message::Message::user(format!(
                    "[btw] {btw_text}"
                )));
            }
        }

        // Drain any /redirect override the user sent. Unlike /btw, a redirect prunes
        // recent assistant and tool messages to break the agent's current momentum,
        // then injects a high-priority system message with the new directive.
        if let Some(ref mut rx) = redirect_rx {
            if let Ok(redirect_text) = rx.try_recv() {
                info!(
                    "Applying /redirect override ({} chars)",
                    redirect_text.len()
                );
                // Prune the last ~8 assistant messages from the working context.
                // We walk backwards and remove up to 8 assistant-role messages so the
                // LLM doesn't simply continue its previous plan.
                let mut pruned = 0usize;
                let mut i = messages.len();
                while i > 0 && pruned < 8 {
                    i -= 1;
                    if messages[i].role == Role::Assistant {
                        messages.remove(i);
                        pruned += 1;
                    }
                }
                if pruned > 0 {
                    info!("Pruned {pruned} assistant messages for /redirect");
                }
                // Inject the override as a system message so it carries maximum weight.
                messages.push(Message::system(format!(
                    "[REDIRECT] STOP your current plan immediately. Do not continue previous steps. New directive from user: {redirect_text}"
                )));
            }
        }

        // Context overflow recovery pipeline (replaces emergency_trim_messages)
        let recovery =
            recover_from_overflow(&mut messages, &system_prompt, available_tools, ctx_window);
        if recovery == RecoveryStage::FinalError {
            warn!("Context overflow unrecoverable — suggest /reset or /compact");
        }

        // Re-validate tool_call/tool_result pairing after overflow drains
        // which may have broken assistant→tool ordering invariants.
        if recovery != RecoveryStage::None {
            messages = crate::session_repair::validate_and_repair(&messages);
        }

        // Context guard: compact oversized tool results before LLM call
        apply_context_guard(&mut messages, &context_budget, available_tools);

        // Strip provider prefix: "openrouter/google/gemini-2.5-flash" → "google/gemini-2.5-flash"
        let api_model = strip_provider_prefix(&manifest.model.model, &manifest.model.provider);

        let request = CompletionRequest {
            model: api_model,
            messages: messages.clone(),
            tools: available_tools.to_vec(),
            max_tokens: manifest.model.max_tokens,
            temperature: manifest.model.temperature,
            system: Some(system_prompt.clone()),
            thinking: None,
        };

        // Notify phase: Thinking
        if let Some(cb) = on_phase {
            cb(LoopPhase::Thinking);
        }

        // Stamp last_active before the (potentially long) LLM call so the
        // heartbeat monitor doesn't flag us as unresponsive mid-iteration.
        if let Some(k) = &kernel {
            k.touch_agent(&agent_id_str);
        }

        // Call LLM with retry, error classification, and circuit breaker
        let provider_name = manifest.model.provider.as_str();
        let llm_fb = LlmFallbackContext {
            llm_http: live_llm.as_ref(),
            llm_kernel: kernel.as_ref(),
        };
        let (mut response, fb_note) = call_with_retry(
            &*driver,
            request,
            Some(provider_name),
            None,
            &manifest.fallback_models,
            &llm_fb,
        )
        .await?;
        if fb_note.is_some() {
            llm_fallback_note = fb_note;
        }

        total_usage.input_tokens += response.usage.input_tokens;
        total_usage.output_tokens += response.usage.output_tokens;

        // Recover tool calls output as text by models that don't use the tool_calls API field
        // (e.g. Groq/Llama, DeepSeek emit `<function=name>{json}</function>` in text)
        if matches!(
            response.stop_reason,
            StopReason::EndTurn | StopReason::StopSequence
        ) && response.tool_calls.is_empty()
        {
            let recovered = recover_text_tool_calls(&response.text(), available_tools);
            if !recovered.is_empty() {
                info!(
                    count = recovered.len(),
                    "Recovered text-based tool calls → promoting to ToolUse"
                );
                response.tool_calls = recovered;
                response.stop_reason = StopReason::ToolUse;
                // Build ToolUse content blocks from recovered calls
                let mut new_blocks: Vec<ContentBlock> = Vec::new();
                for tc in &response.tool_calls {
                    new_blocks.push(ContentBlock::ToolUse {
                        id: tc.id.clone(),
                        name: tc.name.clone(),
                        input: tc.input.clone(),
                        provider_metadata: None,
                    });
                }
                response.content = new_blocks;
            }
        }

        match response.stop_reason {
            StopReason::EndTurn | StopReason::StopSequence => {
                // LLM is done — extract text and save
                let text = response.text();

                // Parse reply directives from the response text
                let (cleaned_text, parsed_directives) =
                    crate::reply_directives::parse_directives(&text);
                let text = cleaned_text;

                // NO_REPLY / [SILENT]: agent intentionally chose not to reply.
                // [SILENT] must not be stored literally — it reinforces silence in future turns.
                if is_silent_token(&text) || parsed_directives.silent {
                    debug!(agent = %manifest.name, "Agent chose NO_REPLY/silent — silent completion");
                    session
                        .messages
                        .push(Message::assistant("[no reply needed]".to_string()));
                    memory
                        .save_session_async(session)
                        .await
                        .map_err(|e| OpenFangError::Memory(e.to_string()))?;
                    return Ok(AgentLoopResult {
                        response: String::new(),
                        total_usage,
                        iterations: iteration + 1,
                        cost_usd: None,
                        silent: true,
                        directives: openfang_types::message::ReplyDirectives {
                            reply_to: parsed_directives.reply_to,
                            current_thread: parsed_directives.current_thread,
                            silent: true,
                        },
                        latency_ms: Some(loop_t0.elapsed().as_millis() as u64),
                        llm_fallback_note: llm_fallback_note.clone(),
                        compression_savings_pct,
                        compressed_input: compressed_input.clone(),
                        compression_semantic_score,
                        adaptive_confidence,
                        eco_counterfactual,
                        adaptive_eco_effective_mode: adaptive_eco_effective_mode.clone(),
                        adaptive_eco_recommended_mode: adaptive_eco_recommended_mode.clone(),
                        adaptive_eco_reason_codes: adaptive_eco_reason_codes.clone(),
                        ainl_runtime_telemetry: {
                        #[cfg(feature = "ainl-runtime-engine")]
                        {
                            ainl_prelude_telemetry.clone()
                        }
                        #[cfg(not(feature = "ainl-runtime-engine"))]
                        {
                            None
                        }
                    },
                    });
                }

                // One-shot retry: if the LLM returns empty text with no tool use,
                // try once more before accepting the empty result.
                // Triggers on first call OR when input_tokens=0 (silently failed request).
                if text.trim().is_empty()
                    && response.tool_calls.is_empty()
                    && !response.has_any_content()
                {
                    let is_silent_failure =
                        response.usage.input_tokens == 0 && response.usage.output_tokens == 0;
                    if iteration == 0 || is_silent_failure {
                        warn!(
                            agent = %manifest.name,
                            iteration,
                            input_tokens = response.usage.input_tokens,
                            output_tokens = response.usage.output_tokens,
                            silent_failure = is_silent_failure,
                            "Empty response, retrying once"
                        );
                        // Re-validate messages before retry — the history may have
                        // broken tool_use/tool_result pairs that caused the failure.
                        if is_silent_failure {
                            messages = crate::session_repair::validate_and_repair(&messages);
                        }
                        messages.push(Message::assistant("[no response]".to_string()));
                        messages.push(Message::user("Please provide your response.".to_string()));
                        continue;
                    }
                }

                // Guard against empty response — covers both iteration 0 and post-tool cycles
                let text = if text.trim().is_empty() {
                    warn!(
                        agent = %manifest.name,
                        iteration,
                        input_tokens = total_usage.input_tokens,
                        output_tokens = total_usage.output_tokens,
                        messages_count = messages.len(),
                        "Empty response from LLM — guard activated"
                    );
                    if any_tools_executed {
                        "[Task completed — the agent executed tools but did not produce a text summary.]".to_string()
                    } else {
                        "[The model returned an empty response. This usually means the model is overloaded, the context is too large, or the API key lacks credits. Try again or check /status.]".to_string()
                    }
                } else {
                    text
                };
                // Snapshot tool names for graph memory before phantom detection clears `last_tools_called`.
                let turn_tool_names: Vec<String> = last_tools_called.iter().cloned().collect();
                // Phantom action detection: if the LLM claims it performed a
                // channel action (send, post, email, etc.) but never actually
                // called the corresponding tool, re-prompt once to force real
                // tool usage instead of hallucinated completion.
                let text = if !any_tools_executed
                    && iteration == 0
                    && phantom_action_detected(&text)
                {
                    warn!(agent = %manifest.name, "Phantom action detected — re-prompting for real tool use");
                    messages.push(Message::assistant(text));
                    messages.push(Message::user(
                        "[System: You claimed to perform an action but did not call any tools. \
                         You must use the appropriate tool (e.g., channel_send, web_fetch, file_write) \
                         to actually perform the action. Do not claim completion without executing tools.]"
                    ));
                    continue;
                } else {
                    text
                };
                // Scheduling: user asked for kernel cron / recurring work; model claimed success without schedule_* / cron_*.
                let text = if scheduling_phantom_detected(
                    session_user_message,
                    &text,
                    &last_tools_called,
                ) {
                    warn!(agent = %manifest.name, "Scheduling phantom detected — re-prompting for schedule_create/cron_create");
                    messages.push(Message::assistant(text));
                    messages.push(Message::user(
                        "[System: The user asked to register recurring / scheduled work with the ArmaraOS kernel scheduler. \
                         You must call schedule_create or cron_create (or schedule_list / cron_list first). \
                         Jobs persist under ~/.armaraos/cron_jobs.json. Do not claim crontab, launchd, or memory-only storage alone is sufficient.]",
                    ));
                    last_tools_called.clear();
                    continue;
                } else {
                    text
                };
                // Process phantom detection: the model used some tools (e.g. process_list) but then
                // claimed to have started/stopped a process without calling process_start/process_kill.
                // This fires even when any_tools_executed is true, because the wrong tool was used.
                let text = if process_phantom_detected(&text, &last_tools_called) {
                    warn!(agent = %manifest.name, "Process phantom detected — re-prompting for real process tool use");
                    messages.push(Message::assistant(text));
                    messages.push(Message::user(
                        "[System: You described starting or stopping a process but did not call \
                         process_start or process_kill. You must call the appropriate process \
                         management tool to actually perform the action — do not claim the process \
                         is running/stopped without having called the tool.]",
                    ));
                    last_tools_called.clear();
                    continue;
                } else {
                    last_tools_called.clear();
                    text
                };

                final_response = text.clone();
                session.messages.push(Message::assistant(text));

                // Prune NO_REPLY heartbeat turns to save context budget
                crate::session_repair::prune_heartbeat_turns(&mut session.messages, 10);

                let tools_for_episode =
                    canonicalize_turn_tool_names_for_graph_storage(&turn_tool_names);
                // AINL graph memory: episode + heuristic semantic/procedural extraction (before session persist)
                if memory_policy.allow_writes() {
                    if let Some(ref gm) = graph_memory {
                    let turn_trace =
                        graph_memory_turn_trace_json(
                            agent_id_str.as_str(),
                            &orchestration_ctx,
                            Some(&compression_metrics),
                            manifest.metadata.get("adaptive_eco"),
                        );
                    let trace_tags = graph_memory_trace_fact_tags(&orchestration_ctx);
                    let pattern_trace_id = graph_memory_pattern_trace_id(&orchestration_ctx);
                    let episode_tags = crate::ainl_semantic_tagger_bridge::SemanticTaggerBridge::tag_episode(
                        &tools_for_episode,
                    );
                    let (ep_vitals_gate, ep_vitals_phase, ep_vitals_trust) =
                        response.vitals.as_ref().map(|v| {
                            (Some(v.gate.as_str().to_string()), Some(v.phase.clone()), Some(v.trust))
                        }).unwrap_or((None, None, None));
                    if let Some(episode_id) = gm
                        .record_turn(
                            tools_for_episode.clone(),
                            None,
                            turn_trace,
                            &episode_tags,
                            ep_vitals_gate,
                            ep_vitals_phase,
                            ep_vitals_trust,
                        )
                        .await
                    {
                        let (facts, turn_pattern) =
                            crate::ainl_graph_extractor_bridge::graph_memory_turn_extraction(
                                session_user_message,
                                &final_response,
                                &tools_for_episode,
                                &turn_tool_names,
                                agent_id_str.as_str(),
                            );
                        for fact in facts {
                            let mut fact_tags: Vec<String> = trace_tags.to_vec();
                            fact_tags.extend(
                                crate::ainl_semantic_tagger_bridge::SemanticTaggerBridge::tag_fact(
                                    &fact.text,
                                ),
                            );
                            gm.record_fact_with_tags(
                                fact.text,
                                fact.confidence,
                                episode_id,
                                &fact_tags,
                            )
                            .await;
                        }
                        if let Some(pattern) = turn_pattern {
                            gm.record_pattern(
                                &pattern.name,
                                pattern.tool_sequence,
                                pattern.confidence,
                                pattern_trace_id.clone(),
                            )
                            .await;
                        }
                    } else {
                        warn!(
                            agent_id = %session.agent_id,
                            "AINL graph memory: episode write failed; skipping turn facts/patterns"
                        );
                    }
                }
                } else {
                    crate::graph_memory_context::record_temp_mode_write_suppressed();
                }
                if memory_policy.allow_writes() {
                    if let Some(gm) = graph_memory.clone() {
                    let turn_outcome = crate::persona_evolution::TurnOutcome {
                        tool_calls: tools_for_episode.clone(),
                        delegation_to: None,
                    };
                    // Cooperative barrier before evolution: post-turn graph writes are awaited above,
                    // but `yield_now` lets the runtime finish scheduling work so the spawned reader is
                    // less likely to race the tail of the write path on the same SQLite connection.
                    tokio::task::yield_now().await;
                    tokio::spawn(async move {
                        let agent_id = gm.agent_id().to_string();
                        let _report = gm.run_persona_evolution_pass().await;
                        let _ = gm.run_background_memory_consolidation().await;
                        if let Err(e) = crate::persona_evolution::PersonaEvolutionHook::evolve_from_turn(
                            &gm,
                            &agent_id,
                            &turn_outcome,
                        )
                        .await
                        {
                            warn!(
                                agent_id = %agent_id,
                                error = %e,
                                "AINL persona turn evolution (AINL_PERSONA_EVOLUTION) failed; continuing"
                            );
                        }
                        graph_memory_refresh_armaraos_export_json(&agent_id).await;
                    });
                }
                } else {
                    crate::graph_memory_context::record_temp_mode_write_suppressed();
                }

                // Save session
                memory
                    .save_session_async(session)
                    .await
                    .map_err(|e| OpenFangError::Memory(e.to_string()))?;

                // Remember this interaction (with embedding if available)
                let interaction_text = format!(
                    "User asked: {}\nI responded: {}",
                    user_message, final_response
                );
                if !memory_policy.allow_writes() {
                    crate::graph_memory_context::record_temp_mode_write_suppressed();
                } else if let Some(emb) = embedding_driver {
                    match emb.embed_one(&interaction_text).await {
                        Ok(vec) => {
                            let _ = memory
                                .remember_with_embedding_async(
                                    session.agent_id,
                                    &interaction_text,
                                    MemorySource::Conversation,
                                    "episodic",
                                    HashMap::new(),
                                    Some(&vec),
                                )
                                .await;
                        }
                        Err(e) => {
                            warn!("Embedding for remember failed: {e}");
                            let _ = memory
                                .remember(
                                    session.agent_id,
                                    &interaction_text,
                                    MemorySource::Conversation,
                                    "episodic",
                                    HashMap::new(),
                                )
                                .await;
                        }
                    }
                } else if memory_policy.allow_writes() {
                    let _ = memory
                        .remember(
                            session.agent_id,
                            &interaction_text,
                            MemorySource::Conversation,
                            "episodic",
                            HashMap::new(),
                        )
                        .await;
                }

                // Notify phase: Done
                if let Some(cb) = on_phase {
                    cb(LoopPhase::Done);
                }

                info!(
                    agent = %manifest.name,
                    iterations = iteration + 1,
                    tokens = total_usage.total(),
                    "Agent loop completed"
                );

                // Fire AgentLoopEnd hook
                if let Some(hook_reg) = hooks {
                    let ctx = crate::hooks::HookContext {
                        agent_name: &manifest.name,
                        agent_id: agent_id_str.as_str(),
                        event: openfang_types::agent::HookEvent::AgentLoopEnd,
                        data: serde_json::json!({
                            "iterations": iteration + 1,
                            "response_length": final_response.len(),
                        }),
                    };
                    let _ = hook_reg.fire(&ctx);
                }

                return Ok(AgentLoopResult {
                    response: final_response,
                    total_usage,
                    iterations: iteration + 1,
                    cost_usd: None,
                    silent: false,
                    directives: Default::default(),
                    latency_ms: Some(loop_t0.elapsed().as_millis() as u64),
                    llm_fallback_note: llm_fallback_note.clone(),
                    compression_savings_pct,
                    compressed_input: compressed_input.clone(),
                    compression_semantic_score,
                    adaptive_confidence,
                    eco_counterfactual,
                    adaptive_eco_effective_mode: adaptive_eco_effective_mode.clone(),
                    adaptive_eco_recommended_mode: adaptive_eco_recommended_mode.clone(),
                    adaptive_eco_reason_codes: adaptive_eco_reason_codes.clone(),
                    ainl_runtime_telemetry: {
                        #[cfg(feature = "ainl-runtime-engine")]
                        {
                            ainl_prelude_telemetry.clone()
                        }
                        #[cfg(not(feature = "ainl-runtime-engine"))]
                        {
                            None
                        }
                    },
                });
            }
            StopReason::ToolUse => {
                // Reset MaxTokens continuation counter on tool use
                consecutive_max_tokens = 0;
                any_tools_executed = true;

                // Execute tool calls
                let assistant_blocks = response.content.clone();

                // Add assistant message with tool use blocks
                session.messages.push(Message {
                    role: Role::Assistant,
                    content: MessageContent::Blocks(assistant_blocks.clone()),
                    orchestration_ctx: None,
                });
                messages.push(Message {
                    role: Role::Assistant,
                    content: MessageContent::Blocks(assistant_blocks),
                    orchestration_ctx: None,
                });

                // Build allowed tool names list for capability enforcement
                let allowed_tool_names: Vec<String> =
                    available_tools.iter().map(|t| t.name.clone()).collect();
                let caller_id_str = session.agent_id.to_string();

                // Execute tool calls with loop guard, timeout, and truncation.
                // Pre-pass: apply loop guard and hooks synchronously (they mutate shared state
                // and may circuit-break). Calls that pass become async futures run in parallel.
                let mut tool_result_blocks = Vec::new();

                let deduped = deduplicate_tool_calls(&response);
                let mut dispatches: Vec<ToolDispatch<'_>> = Vec::with_capacity(deduped.len());

                for tool_call in &deduped {
                    let verdict = loop_guard.check(&tool_call.name, &tool_call.input);
                    match &verdict {
                        LoopGuardVerdict::CircuitBreak(msg) => {
                            warn!(tool = %tool_call.name, "Circuit breaker triggered");
                            if let Err(e) = memory.save_session_async(session).await {
                                warn!("Failed to save session on circuit break: {e}");
                            }
                            if let Some(hook_reg) = hooks {
                                let ctx = crate::hooks::HookContext {
                                    agent_name: &manifest.name,
                                    agent_id: agent_id_str.as_str(),
                                    event: openfang_types::agent::HookEvent::AgentLoopEnd,
                                    data: serde_json::json!({
                                        "reason": "circuit_break",
                                        "error": msg.as_str(),
                                    }),
                                };
                                let _ = hook_reg.fire(&ctx);
                            }
                            return Err(OpenFangError::Internal(msg.clone()));
                        }
                        LoopGuardVerdict::Block(msg) => {
                            warn!(tool = %tool_call.name, "Tool call blocked by loop guard");
                            dispatches.push(ToolDispatch::Resolved(ContentBlock::ToolResult {
                                tool_use_id: tool_call.id.clone(),
                                tool_name: tool_call.name.clone(),
                                content: msg.clone(),
                                is_error: true,
                            }));
                            continue;
                        }
                        _ => {}
                    }

                    // Notify phase: ToolUse (first tool sets it; parallel tools fire concurrently but
                    // this callback is cheap and idempotent in practice)
                    if let Some(cb) = on_phase {
                        let sanitized: String = tool_call
                            .name
                            .chars()
                            .filter(|c| !c.is_control())
                            .take(64)
                            .collect();
                        cb(LoopPhase::ToolUse {
                            tool_name: sanitized,
                        });
                    }

                    // Fire BeforeToolCall hook (synchronous gate — must run before dispatch)
                    if let Some(hook_reg) = hooks {
                        let ctx = crate::hooks::HookContext {
                            agent_name: &manifest.name,
                            agent_id: &caller_id_str,
                            event: openfang_types::agent::HookEvent::BeforeToolCall,
                            data: serde_json::json!({
                                "tool_name": &tool_call.name,
                                "input": &tool_call.input,
                            }),
                        };
                        if let Err(reason) = hook_reg.fire(&ctx) {
                            dispatches.push(ToolDispatch::Resolved(ContentBlock::ToolResult {
                                tool_use_id: tool_call.id.clone(),
                                tool_name: tool_call.name.clone(),
                                content: format!(
                                    "Hook blocked tool '{}': {}",
                                    tool_call.name, reason
                                ),
                                is_error: true,
                            }));
                            continue;
                        }
                    }

                    // Pre-execution required-param validation: catch empty or incomplete
                    // tool calls before they reach the tool handler. Returns a rich error
                    // with the full field list so the LLM can self-correct in one step.
                    if let Some(def) = available_tools.iter().find(|d| d.name == tool_call.name) {
                        if let Some(err_msg) =
                            missing_required_params_error(&tool_call.name, &tool_call.input, def)
                        {
                            debug!(tool = %tool_call.name, "Pre-execution param validation failed");
                            dispatches.push(ToolDispatch::Resolved(ContentBlock::ToolResult {
                                tool_use_id: tool_call.id.clone(),
                                tool_name: tool_call.name.clone(),
                                content: err_msg,
                                is_error: true,
                            }));
                            continue;
                        }
                    }

                    // Track the tool name so EndTurn can check for process phantom actions
                    last_tools_called.insert(tool_call.name.clone());
                    dispatches.push(ToolDispatch::Pending { tool_call, verdict });
                }

                // Run all pending (approved) tool calls concurrently, preserving LLM-declared order
                // in the final result list so the next LLM turn sees results in the same order as
                // the tool_use blocks it produced.
                let effective_exec_policy = manifest.exec_policy.as_ref();
                let pending_futures: Vec<_> = dispatches
                    .iter()
                    .filter_map(|d| {
                        if let ToolDispatch::Pending { tool_call, .. } = d {
                            let timeout = tool_timeout_for(&tool_call.name);
                            Some((
                                tool_call,
                                tokio::time::timeout(
                                    timeout,
                                    tool_runner::execute_tool(
                                        &tool_call.id,
                                        &tool_call.name,
                                        &tool_call.input,
                                        kernel.as_ref(),
                                        Some(&allowed_tool_names),
                                        Some(&caller_id_str),
                                        skill_registry,
                                        mcp_connections,
                                        web_ctx,
                                        browser_ctx,
                                        if hand_allowed_env.is_empty() {
                                            None
                                        } else {
                                            Some(&hand_allowed_env)
                                        },
                                        workspace_root,
                                        ainl_library_root,
                                        media_engine,
                                        effective_exec_policy,
                                        tts_engine,
                                        docker_config,
                                        process_manager,
                                        orchestration_live,
                                    ),
                                ),
                            ))
                        } else {
                            None
                        }
                    })
                    .collect();

                // Collect futures while preserving their association with the tool_call metadata
                let (pending_tool_calls, pending_futs): (Vec<&&ToolCall>, Vec<_>) =
                    pending_futures.into_iter().unzip();
                let parallel_results = futures::future::join_all(pending_futs).await;

                // Merge pre-resolved blocks and parallel results back in LLM-declaration order
                let mut pending_iter = pending_tool_calls
                    .into_iter()
                    .zip(parallel_results.into_iter())
                    .peekable();

                for dispatch in &dispatches {
                    match dispatch {
                        ToolDispatch::Resolved(block) => {
                            tool_result_blocks.push(block.clone());
                        }
                        ToolDispatch::Pending { tool_call, verdict } => {
                            let (tc, timeout_result) = pending_iter.next().unwrap();
                            let timeout_secs = tool_timeout_for(&tc.name).as_secs();
                            let result = match timeout_result {
                                Ok(r) => r,
                                Err(_) => {
                                    warn!(tool = %tc.name, "Tool execution timed out after {}s", timeout_secs);
                                    openfang_types::tool::ToolResult {
                                        tool_use_id: tc.id.clone(),
                                        content: format!(
                                            "Tool '{}' timed out after {}s.",
                                            tc.name, timeout_secs
                                        ),
                                        is_error: true,
                                    }
                                }
                            };

                            // Fire AfterToolCall hook
                            if let Some(hook_reg) = hooks {
                                let ctx = crate::hooks::HookContext {
                                    agent_name: &manifest.name,
                                    agent_id: caller_id_str.as_str(),
                                    event: openfang_types::agent::HookEvent::AfterToolCall,
                                    data: serde_json::json!({
                                        "tool_name": &tool_call.name,
                                        "result": &result.content,
                                        "is_error": result.is_error,
                                    }),
                                };
                                let _ = hook_reg.fire(&ctx);
                            }

                            let content =
                                truncate_tool_result_dynamic(&result.content, &context_budget);
                            let final_content =
                                if let LoopGuardVerdict::Warn(ref warn_msg) = verdict {
                                    format!("{content}\n\n[LOOP GUARD] {warn_msg}")
                                } else {
                                    content
                                };

                            tool_result_blocks.push(ContentBlock::ToolResult {
                                tool_use_id: result.tool_use_id,
                                tool_name: tool_call.name.clone(),
                                content: final_content,
                                is_error: result.is_error,
                            });
                        }
                    }
                }

                // All-blocked early exit: if every result in this iteration was a loop-guard
                // block (not just a warning or error), the model is stuck and no guidance
                // has penetrated. After 3 consecutive fully-blocked iterations, exit gracefully
                // rather than burning the remaining iteration budget on futile retries.
                {
                    let all_blocked = !tool_result_blocks.is_empty()
                        && tool_result_blocks.iter().all(|b| {
                            matches!(b, ContentBlock::ToolResult { content, is_error: true, .. }
                            if content.starts_with("Blocked:") || content.starts_with("Circuit breaker:"))
                        });
                    if all_blocked {
                        consecutive_all_blocked += 1;
                    } else {
                        consecutive_all_blocked = 0;
                    }
                    const MAX_CONSECUTIVE_ALL_BLOCKED: u32 = 3;
                    if consecutive_all_blocked >= MAX_CONSECUTIVE_ALL_BLOCKED {
                        warn!(
                            agent = %manifest.name,
                            consecutive_all_blocked,
                            "All tool calls blocked for {} consecutive iterations — exiting early",
                            consecutive_all_blocked
                        );
                        let summary = "I was unable to complete this task: my tool calls were \
                             repeatedly blocked because I appeared to be stuck calling the same \
                             tools in a loop. Please try rephrasing your request, breaking it \
                             into smaller steps, or using a different approach.";
                        session.messages.push(Message::assistant(summary));
                        if let Err(e) = memory.save_session_async(session).await {
                            warn!("Failed to save session on all-blocked exit: {e}");
                        }
                        if let Some(cb) = on_phase {
                            cb(LoopPhase::Done);
                        }
                        return Ok(AgentLoopResult {
                            response: summary.to_string(),
                            total_usage,
                            iterations: iteration + 1,
                            cost_usd: None,
                            silent: false,
                            directives: Default::default(),
                            latency_ms: Some(loop_t0.elapsed().as_millis() as u64),
                            llm_fallback_note: llm_fallback_note.clone(),
                            compression_savings_pct,
                            compressed_input: compressed_input.clone(),
                            compression_semantic_score,
                            adaptive_confidence,
                            eco_counterfactual,
                            adaptive_eco_effective_mode: adaptive_eco_effective_mode.clone(),
                            adaptive_eco_recommended_mode: adaptive_eco_recommended_mode.clone(),
                            adaptive_eco_reason_codes: adaptive_eco_reason_codes.clone(),
                            ainl_runtime_telemetry: {
                        #[cfg(feature = "ainl-runtime-engine")]
                        {
                            ainl_prelude_telemetry.clone()
                        }
                        #[cfg(not(feature = "ainl-runtime-engine"))]
                        {
                            None
                        }
                    },
                        });
                    }
                }

                // Approval denials: always inject — the model must not retry denied tools.
                let denial_count = tool_result_blocks
                    .iter()
                    .filter(|b| {
                        matches!(b, ContentBlock::ToolResult { content, is_error: true, .. }
                        if content.contains("requires human approval and was denied"))
                    })
                    .count();
                if denial_count > 0 {
                    tool_result_blocks.push(ContentBlock::Text {
                        text: format!(
                            "[System: {} tool call(s) were denied by approval policy. \
                             Do NOT retry denied tools. Explain to the user what you \
                             wanted to do and that it requires their approval. \
                             Hint: set auto_approve = true in [approval] section of \
                             config.toml, or start with --yolo flag, to auto-approve \
                             all tool calls.]",
                            denial_count
                        ),
                        provider_metadata: None,
                    });
                }

                // Smart error guidance: targeted on first occurrence, escalating on second,
                // silent on third+ (loop guard handles repeated failures from that point).
                if let Some(guidance) =
                    tool_error_tracker.compute_guidance(&tool_result_blocks, denial_count)
                {
                    tool_result_blocks.push(ContentBlock::Text {
                        text: guidance,
                        provider_metadata: None,
                    });
                }

                // Add tool results as a user message (Anthropic API requirement)
                let tool_results_msg = Message {
                    role: Role::User,
                    content: MessageContent::Blocks(tool_result_blocks.clone()),
                    orchestration_ctx: None,
                };
                session.messages.push(tool_results_msg.clone());
                messages.push(tool_results_msg);

                // Wrap-up injection: if we are within 3 iterations of the limit, tell
                // the LLM to stop calling tools and produce a text summary. This prevents
                // a hard MaxIterationsExceeded error for agents stuck in a tool loop.
                if iteration + 1 + 3 >= max_iterations {
                    warn!(
                        agent = %manifest.name,
                        iteration,
                        max_iterations,
                        "Approaching iteration limit — injecting wrap-up prompt"
                    );
                    messages.push(Message::user(
                        "[System: You are very close to the maximum number of allowed steps. \
                         Stop calling tools now. Write a final text response summarizing \
                         what you have done and any results or next steps for the user.]",
                    ));
                }

                // Interim save after tool execution to prevent data loss on crash
                if let Err(e) = memory.save_session_async(session).await {
                    warn!("Failed to interim-save session: {e}");
                }
            }
            StopReason::MaxTokens => {
                consecutive_max_tokens += 1;
                if consecutive_max_tokens >= runtime_limits.max_continuations {
                    // Return partial response instead of continuing forever
                    let text = response.text();
                    let text = if text.trim().is_empty() {
                        "[Partial response — token limit reached with no text output.]".to_string()
                    } else {
                        text
                    };
                    session.messages.push(Message::assistant(&text));
                    if let Err(e) = memory.save_session_async(session).await {
                        warn!("Failed to save session on max continuations: {e}");
                    }
                    warn!(
                        iteration,
                        consecutive_max_tokens,
                        "Max continuations reached, returning partial response"
                    );
                    // Fire AgentLoopEnd hook
                    if let Some(hook_reg) = hooks {
                        let ctx = crate::hooks::HookContext {
                            agent_name: &manifest.name,
                            agent_id: agent_id_str.as_str(),
                            event: openfang_types::agent::HookEvent::AgentLoopEnd,
                            data: serde_json::json!({
                                "iterations": iteration + 1,
                                "reason": "max_continuations",
                            }),
                        };
                        let _ = hook_reg.fire(&ctx);
                    }
                    return Ok(AgentLoopResult {
                        response: text,
                        total_usage,
                        iterations: iteration + 1,
                        cost_usd: None,
                        silent: false,
                        directives: Default::default(),
                        latency_ms: Some(loop_t0.elapsed().as_millis() as u64),
                        llm_fallback_note: llm_fallback_note.clone(),
                        compression_savings_pct,
                        compressed_input: compressed_input.clone(),
                        compression_semantic_score,
                        adaptive_confidence,
                        eco_counterfactual,
                        adaptive_eco_effective_mode: adaptive_eco_effective_mode.clone(),
                        adaptive_eco_recommended_mode: adaptive_eco_recommended_mode.clone(),
                        adaptive_eco_reason_codes: adaptive_eco_reason_codes.clone(),
                        ainl_runtime_telemetry: {
                        #[cfg(feature = "ainl-runtime-engine")]
                        {
                            ainl_prelude_telemetry.clone()
                        }
                        #[cfg(not(feature = "ainl-runtime-engine"))]
                        {
                            None
                        }
                    },
                    });
                }
                // Model hit token limit — add partial response and continue
                let text = response.text();
                session.messages.push(Message::assistant(&text));
                messages.push(Message::assistant(&text));
                session.messages.push(Message::user("Please continue."));
                messages.push(Message::user("Please continue."));
                warn!(iteration, "Max tokens hit, continuing");
            }
        }
    }

    // Iteration limit reached — degrade gracefully instead of hard-erroring.
    // Return a helpful in-chat message so the user sees it as a reply, not an error banner.
    warn!(
        agent = %manifest.name,
        max_iterations,
        "Agent loop hit max iterations — returning graceful fallback response"
    );

    let fallback = format!(
        "I reached my step limit ({max_iterations} steps) and could not complete the task in one go. \
         If I got stuck in a loop, try `/reset` to clear the session and rephrase your request. \
         If the task genuinely needs more steps, increase `max_iterations` under `[autonomous]` in agent.toml."
    );
    session.messages.push(Message::assistant(&fallback));

    if let Err(e) = memory.save_session_async(session).await {
        warn!("Failed to save session on max iterations: {e}");
    }

    if let Some(hook_reg) = hooks {
        let ctx = crate::hooks::HookContext {
            agent_name: &manifest.name,
            agent_id: agent_id_str.as_str(),
            event: openfang_types::agent::HookEvent::AgentLoopEnd,
            data: serde_json::json!({
                "reason": "max_iterations_exceeded",
                "iterations": max_iterations,
            }),
        };
        let _ = hook_reg.fire(&ctx);
    }

    Ok(AgentLoopResult {
        response: fallback,
        total_usage,
        iterations: max_iterations,
        cost_usd: None,
        silent: false,
        directives: Default::default(),
        latency_ms: Some(loop_t0.elapsed().as_millis() as u64),
        llm_fallback_note,
        compression_savings_pct,
        compressed_input,
        compression_semantic_score,
        adaptive_confidence,
        eco_counterfactual,
        adaptive_eco_effective_mode: adaptive_eco_effective_mode.clone(),
        adaptive_eco_recommended_mode: adaptive_eco_recommended_mode.clone(),
        adaptive_eco_reason_codes: adaptive_eco_reason_codes.clone(),
        ainl_runtime_telemetry: {
                        #[cfg(feature = "ainl-runtime-engine")]
                        {
                            ainl_prelude_telemetry.clone()
                        }
                        #[cfg(not(feature = "ainl-runtime-engine"))]
                        {
                            None
                        }
                    },
    })
            }
        )
        .await
}

/// Live `[llm]` timeouts and optional kernel factory for rare fallback driver paths.
struct LlmFallbackContext<'a> {
    llm_http: Option<&'a LlmConfig>,
    llm_kernel: Option<&'a Arc<dyn KernelHandle>>,
}

/// Attach `[llm]` HTTP timeouts to a one-off driver config (OpenRouter fallbacks, manifest fallbacks).
fn driver_config_with_llm_http(
    mut cfg: DriverConfig,
    llm_http: Option<&LlmConfig>,
) -> DriverConfig {
    let llm = llm_http.cloned().unwrap_or_default();
    if let Ok(client) = crate::drivers::build_llm_http_client(&llm) {
        cfg.http_client = Some(Arc::new(client));
    }
    cfg
}

/// Rare fallback drivers: use the kernel `LlmDriverFactory` when available (metrics + LRU),
/// else `create_driver` with `[llm]` HTTP timeouts from `llm_http` / defaults.
fn resolve_adhoc_driver(
    fb: &LlmFallbackContext<'_>,
    cfg: DriverConfig,
) -> Result<Arc<dyn LlmDriver>, LlmError> {
    if let Some(k) = fb.llm_kernel {
        return k.get_llm_driver(&cfg).map_err(LlmError::Http);
    }
    let cfg = driver_config_with_llm_http(cfg, fb.llm_http);
    crate::drivers::create_driver(&cfg)
}

/// Call an LLM driver with automatic retry on rate-limit and overload errors.
///
/// Uses the `llm_errors` classifier for smart error handling and the
/// `ProviderCooldown` circuit breaker to prevent request storms.
///
/// When the primary model returns a `ModelNotFound` error and `fallback_models`
/// is non-empty, each fallback is tried in order before propagating the error.
async fn call_with_retry(
    driver: &dyn LlmDriver,
    request: CompletionRequest,
    provider: Option<&str>,
    cooldown: Option<&ProviderCooldown>,
    fallback_models: &[FallbackModel],
    llm_fb: &LlmFallbackContext<'_>,
) -> OpenFangResult<(crate::llm_driver::CompletionResponse, Option<String>)> {
    const OR_NOTE: &str = "OpenRouter free-tier model (primary rate limited or overloaded)";
    // Check circuit breaker before calling
    if let (Some(provider), Some(cooldown)) = (provider, cooldown) {
        match cooldown.check(provider) {
            CooldownVerdict::Reject {
                reason,
                retry_after_secs,
            } => {
                return Err(OpenFangError::LlmDriver(format!(
                    "Provider '{provider}' is in cooldown ({reason}). Retry in {retry_after_secs}s."
                )));
            }
            CooldownVerdict::AllowProbe => {
                debug!(provider, "Allowing probe request through circuit breaker");
            }
            CooldownVerdict::Allow => {}
        }
    }

    let mut last_error = None;

    for attempt in 0..=MAX_RETRIES {
        match driver.complete(request.clone()).await {
            Ok(response) => {
                // Record success with circuit breaker
                if let (Some(provider), Some(cooldown)) = (provider, cooldown) {
                    cooldown.record_success(provider);
                }
                return Ok((response, None));
            }
            Err(LlmError::RateLimited { retry_after_ms }) => {
                if attempt == MAX_RETRIES {
                    if let (Some(provider), Some(cooldown)) = (provider, cooldown) {
                        cooldown.record_failure(provider, false);
                    }
                    // Final attempt hit a retryable throttling error — try OpenRouter free-model
                    // fallbacks to keep UX flowing even when the primary provider is rate limited.
                    if let Ok(resp) = try_openrouter_free_fallbacks(request.clone(), llm_fb).await {
                        return Ok((resp, Some(OR_NOTE.to_string())));
                    }
                    return Err(OpenFangError::LlmDriver(format!(
                        "Rate limited after {} retries",
                        MAX_RETRIES
                    )));
                }
                let delay = std::cmp::max(retry_after_ms, BASE_RETRY_DELAY_MS * 2u64.pow(attempt));
                warn!(
                    attempt,
                    delay_ms = delay,
                    "Rate limited, retrying after delay"
                );
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                last_error = Some("Rate limited".to_string());
            }
            Err(LlmError::Overloaded { retry_after_ms }) => {
                if attempt == MAX_RETRIES {
                    if let (Some(provider), Some(cooldown)) = (provider, cooldown) {
                        cooldown.record_failure(provider, false);
                    }
                    if let Ok(resp) = try_openrouter_free_fallbacks(request.clone(), llm_fb).await {
                        return Ok((resp, Some(OR_NOTE.to_string())));
                    }
                    return Err(OpenFangError::LlmDriver(format!(
                        "Model overloaded after {} retries",
                        MAX_RETRIES
                    )));
                }
                let delay = std::cmp::max(retry_after_ms, BASE_RETRY_DELAY_MS * 2u64.pow(attempt));
                warn!(
                    attempt,
                    delay_ms = delay,
                    "Model overloaded, retrying after delay"
                );
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                last_error = Some("Overloaded".to_string());
            }
            Err(e) => {
                // Use classifier for smarter error handling
                let raw_error = e.to_string();
                let status = match &e {
                    LlmError::Api { status, .. } => Some(*status),
                    _ => None,
                };
                let classified = llm_errors::classify_error(&raw_error, status);
                warn!(
                    category = ?classified.category,
                    retryable = classified.is_retryable,
                    raw = %raw_error,
                    "LLM error classified: {}",
                    classified.sanitized_message
                );

                if let (Some(provider), Some(cooldown)) = (provider, cooldown) {
                    cooldown.record_failure(provider, classified.is_billing);
                }

                // --- ModelNotFound fallback chain (issue #845) ---
                // If the primary model was not found and fallback models are
                // configured, try each fallback before giving up.
                if classified.category == llm_errors::LlmErrorCategory::ModelNotFound
                    && !fallback_models.is_empty()
                {
                    warn!(
                        "Primary model not found, trying {} fallback model(s)",
                        fallback_models.len()
                    );
                    for (fb_idx, fb) in fallback_models.iter().enumerate() {
                        let api_key = fb
                            .api_key_env
                            .as_deref()
                            .and_then(|env_name| std::env::var(env_name).ok());
                        let fb_config = DriverConfig {
                            provider: fb.provider.clone(),
                            api_key,
                            base_url: fb.base_url.clone(),
                            skip_permissions: true,
                            ..Default::default()
                        };
                        let fb_driver = match resolve_adhoc_driver(llm_fb, fb_config) {
                            Ok(d) => d,
                            Err(driver_err) => {
                                warn!(
                                    fallback_index = fb_idx,
                                    provider = %fb.provider,
                                    model = %fb.model,
                                    error = %driver_err,
                                    "Failed to create fallback driver, skipping"
                                );
                                continue;
                            }
                        };
                        let mut fb_request = request.clone();
                        fb_request.model = fb.model.clone();
                        warn!(
                            fallback_index = fb_idx,
                            provider = %fb.provider,
                            model = %fb.model,
                            "Trying fallback model"
                        );
                        match fb_driver.complete(fb_request).await {
                            Ok(response) => {
                                info!(
                                    fallback_index = fb_idx,
                                    provider = %fb.provider,
                                    model = %fb.model,
                                    "Fallback model succeeded"
                                );
                                let note = format!(
                                    "Fallback {}/{} (primary model not found)",
                                    fb.provider, fb.model
                                );
                                return Ok((response, Some(note)));
                            }
                            Err(fb_err) => {
                                warn!(
                                    fallback_index = fb_idx,
                                    provider = %fb.provider,
                                    model = %fb.model,
                                    error = %fb_err,
                                    "Fallback model failed"
                                );
                            }
                        }
                    }
                    // All fallbacks exhausted — fall through to return the
                    // original ModelNotFound error below.
                }

                // Include raw error detail so dashboard users can debug
                let user_msg = if classified.category == llm_errors::LlmErrorCategory::Format {
                    format!("{} — raw: {}", classified.sanitized_message, raw_error)
                } else {
                    classified.sanitized_message
                };
                return Err(OpenFangError::LlmDriver(user_msg));
            }
        }
    }

    Err(OpenFangError::LlmDriver(
        last_error.unwrap_or_else(|| "Unknown error".to_string()),
    ))
}

async fn try_openrouter_free_fallbacks(
    request: CompletionRequest,
    llm_fb: &LlmFallbackContext<'_>,
) -> OpenFangResult<crate::llm_driver::CompletionResponse> {
    // Models requested by product default strategy.
    const FB_MODELS: [&str; 2] = [
        "stepfun/step-3.5-flash:free",
        "nvidia/nemotron-3-super-120b-a12b:free",
    ];

    let api_key = std::env::var("OPENROUTER_API_KEY").ok();
    let cfg = DriverConfig {
        provider: "openrouter".to_string(),
        api_key,
        base_url: None,
        skip_permissions: true,
        ..Default::default()
    };
    let driver = resolve_adhoc_driver(llm_fb, cfg).map_err(|e| {
        OpenFangError::LlmDriver(format!("OpenRouter fallback driver init failed: {e}"))
    })?;

    for (idx, model) in FB_MODELS.iter().enumerate() {
        let mut req = request.clone();
        req.model = model.to_string();
        warn!(fallback_index = idx, model = %model, "Trying OpenRouter fallback model");
        match driver.complete(req).await {
            Ok(resp) => {
                info!(fallback_index = idx, model = %model, "OpenRouter fallback succeeded");
                return Ok(resp);
            }
            Err(e) => {
                warn!(fallback_index = idx, model = %model, error = %e, "OpenRouter fallback failed");
            }
        }
    }

    Err(OpenFangError::LlmDriver(
        "OpenRouter fallback models failed".to_string(),
    ))
}

/// Call an LLM driver in streaming mode with automatic retry on rate-limit and overload errors.
///
/// Uses the `llm_errors` classifier and `ProviderCooldown` circuit breaker.
///
/// When the primary model returns a `ModelNotFound` error and `fallback_models`
/// is non-empty, each fallback is tried in order before propagating the error.
async fn stream_with_retry(
    driver: &dyn LlmDriver,
    request: CompletionRequest,
    tx: mpsc::Sender<StreamEvent>,
    provider: Option<&str>,
    cooldown: Option<&ProviderCooldown>,
    fallback_models: &[FallbackModel],
    llm_fb: &LlmFallbackContext<'_>,
) -> OpenFangResult<(crate::llm_driver::CompletionResponse, Option<String>)> {
    const OR_NOTE: &str = "OpenRouter free-tier model (primary rate limited or overloaded)";
    // Check circuit breaker before calling
    if let (Some(provider), Some(cooldown)) = (provider, cooldown) {
        match cooldown.check(provider) {
            CooldownVerdict::Reject {
                reason,
                retry_after_secs,
            } => {
                return Err(OpenFangError::LlmDriver(format!(
                    "Provider '{provider}' is in cooldown ({reason}). Retry in {retry_after_secs}s."
                )));
            }
            CooldownVerdict::AllowProbe => {
                debug!(
                    provider,
                    "Allowing probe request through circuit breaker (stream)"
                );
            }
            CooldownVerdict::Allow => {}
        }
    }

    let mut last_error = None;

    for attempt in 0..=MAX_RETRIES {
        match driver.stream(request.clone(), tx.clone()).await {
            Ok(response) => {
                if let (Some(provider), Some(cooldown)) = (provider, cooldown) {
                    cooldown.record_success(provider);
                }
                return Ok((response, None));
            }
            Err(LlmError::RateLimited { retry_after_ms }) => {
                if attempt == MAX_RETRIES {
                    if let (Some(provider), Some(cooldown)) = (provider, cooldown) {
                        cooldown.record_failure(provider, false);
                    }
                    if let Ok(resp) =
                        try_openrouter_free_fallbacks_stream(request.clone(), tx.clone(), llm_fb)
                            .await
                    {
                        return Ok((resp, Some(OR_NOTE.to_string())));
                    }
                    return Err(OpenFangError::LlmDriver(format!(
                        "Rate limited after {} retries",
                        MAX_RETRIES
                    )));
                }
                let delay = std::cmp::max(retry_after_ms, BASE_RETRY_DELAY_MS * 2u64.pow(attempt));
                warn!(
                    attempt,
                    delay_ms = delay,
                    "Rate limited (stream), retrying after delay"
                );
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                last_error = Some("Rate limited".to_string());
            }
            Err(LlmError::Overloaded { retry_after_ms }) => {
                if attempt == MAX_RETRIES {
                    if let (Some(provider), Some(cooldown)) = (provider, cooldown) {
                        cooldown.record_failure(provider, false);
                    }
                    if let Ok(resp) =
                        try_openrouter_free_fallbacks_stream(request.clone(), tx.clone(), llm_fb)
                            .await
                    {
                        return Ok((resp, Some(OR_NOTE.to_string())));
                    }
                    return Err(OpenFangError::LlmDriver(format!(
                        "Model overloaded after {} retries",
                        MAX_RETRIES
                    )));
                }
                let delay = std::cmp::max(retry_after_ms, BASE_RETRY_DELAY_MS * 2u64.pow(attempt));
                warn!(
                    attempt,
                    delay_ms = delay,
                    "Model overloaded (stream), retrying after delay"
                );
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                last_error = Some("Overloaded".to_string());
            }
            Err(e) => {
                let raw_error = e.to_string();
                let status = match &e {
                    LlmError::Api { status, .. } => Some(*status),
                    _ => None,
                };
                let classified = llm_errors::classify_error(&raw_error, status);
                warn!(
                    category = ?classified.category,
                    retryable = classified.is_retryable,
                    raw = %raw_error,
                    "LLM stream error classified: {}",
                    classified.sanitized_message
                );

                if let (Some(provider), Some(cooldown)) = (provider, cooldown) {
                    cooldown.record_failure(provider, classified.is_billing);
                }

                // --- ModelNotFound fallback chain (issue #845) ---
                if classified.category == llm_errors::LlmErrorCategory::ModelNotFound
                    && !fallback_models.is_empty()
                {
                    warn!(
                        "Primary model not found (stream), trying {} fallback model(s)",
                        fallback_models.len()
                    );
                    for (fb_idx, fb) in fallback_models.iter().enumerate() {
                        let api_key = fb
                            .api_key_env
                            .as_deref()
                            .and_then(|env_name| std::env::var(env_name).ok());
                        let fb_config = DriverConfig {
                            provider: fb.provider.clone(),
                            api_key,
                            base_url: fb.base_url.clone(),
                            skip_permissions: true,
                            ..Default::default()
                        };
                        let fb_driver = match resolve_adhoc_driver(llm_fb, fb_config) {
                            Ok(d) => d,
                            Err(driver_err) => {
                                warn!(
                                    fallback_index = fb_idx,
                                    provider = %fb.provider,
                                    model = %fb.model,
                                    error = %driver_err,
                                    "Failed to create fallback stream driver, skipping"
                                );
                                continue;
                            }
                        };
                        let mut fb_request = request.clone();
                        fb_request.model = fb.model.clone();
                        warn!(
                            fallback_index = fb_idx,
                            provider = %fb.provider,
                            model = %fb.model,
                            "Trying fallback model (stream)"
                        );
                        match fb_driver.stream(fb_request, tx.clone()).await {
                            Ok(response) => {
                                info!(
                                    fallback_index = fb_idx,
                                    provider = %fb.provider,
                                    model = %fb.model,
                                    "Fallback model succeeded (stream)"
                                );
                                let note = format!(
                                    "Fallback {}/{} (primary model not found)",
                                    fb.provider, fb.model
                                );
                                return Ok((response, Some(note)));
                            }
                            Err(fb_err) => {
                                warn!(
                                    fallback_index = fb_idx,
                                    provider = %fb.provider,
                                    model = %fb.model,
                                    error = %fb_err,
                                    "Fallback model failed (stream)"
                                );
                            }
                        }
                    }
                }

                let user_msg = if classified.category == llm_errors::LlmErrorCategory::Format {
                    format!("{} — raw: {}", classified.sanitized_message, raw_error)
                } else {
                    classified.sanitized_message
                };
                return Err(OpenFangError::LlmDriver(user_msg));
            }
        }
    }

    Err(OpenFangError::LlmDriver(
        last_error.unwrap_or_else(|| "Unknown error".to_string()),
    ))
}

async fn try_openrouter_free_fallbacks_stream(
    request: CompletionRequest,
    tx: mpsc::Sender<StreamEvent>,
    llm_fb: &LlmFallbackContext<'_>,
) -> OpenFangResult<crate::llm_driver::CompletionResponse> {
    const FB_MODELS: [&str; 2] = [
        "stepfun/step-3.5-flash:free",
        "nvidia/nemotron-3-super-120b-a12b:free",
    ];

    let api_key = std::env::var("OPENROUTER_API_KEY").ok();
    let cfg = DriverConfig {
        provider: "openrouter".to_string(),
        api_key,
        base_url: None,
        skip_permissions: true,
        ..Default::default()
    };
    let driver = resolve_adhoc_driver(llm_fb, cfg).map_err(|e| {
        OpenFangError::LlmDriver(format!("OpenRouter fallback driver init failed: {e}"))
    })?;

    for (idx, model) in FB_MODELS.iter().enumerate() {
        let mut req = request.clone();
        req.model = model.to_string();
        warn!(
            fallback_index = idx,
            model = %model,
            "Trying OpenRouter fallback model (stream)"
        );
        match driver.stream(req, tx.clone()).await {
            Ok(resp) => {
                info!(
                    fallback_index = idx,
                    model = %model,
                    "OpenRouter fallback succeeded (stream)"
                );
                return Ok(resp);
            }
            Err(e) => {
                warn!(
                    fallback_index = idx,
                    model = %model,
                    error = %e,
                    "OpenRouter fallback failed (stream)"
                );
            }
        }
    }

    Err(OpenFangError::LlmDriver(
        "OpenRouter fallback models failed".to_string(),
    ))
}

/// Run the agent execution loop with streaming support.
///
/// Like `run_agent_loop`, but sends `StreamEvent`s to the provided channel
/// as tokens arrive from the LLM. Tool execution happens between LLM calls
/// and is not streamed.
#[allow(clippy::too_many_arguments)]
pub async fn run_agent_loop_streaming(
    manifest: &AgentManifest,
    user_message: &str,
    session: &mut Session,
    memory: &MemorySubstrate,
    driver: Arc<dyn LlmDriver>,
    available_tools: &[ToolDefinition],
    kernel: Option<Arc<dyn KernelHandle>>,
    stream_tx: mpsc::Sender<StreamEvent>,
    skill_registry: Option<&SkillRegistry>,
    mcp_connections: Option<&tokio::sync::Mutex<Vec<McpConnection>>>,
    web_ctx: Option<&WebToolsContext>,
    browser_ctx: Option<&crate::browser::BrowserManager>,
    embedding_driver: Option<&(dyn EmbeddingDriver + Send + Sync)>,
    workspace_root: Option<&Path>,
    ainl_library_root: Option<&Path>,
    on_phase: Option<&PhaseCallback>,
    media_engine: Option<&crate::media_understanding::MediaEngine>,
    tts_engine: Option<&crate::tts::TtsEngine>,
    docker_config: Option<&openfang_types::config::DockerSandboxConfig>,
    hooks: Option<&crate::hooks::HookRegistry>,
    context_window_tokens: Option<usize>,
    process_manager: Option<&crate::process_manager::ProcessManager>,
    user_content_blocks: Option<Vec<ContentBlock>>,
    btw_rx: Option<tokio::sync::mpsc::Receiver<String>>,
    redirect_rx: Option<tokio::sync::mpsc::Receiver<String>>,
    runtime_limits: EffectiveRuntimeLimits,
    orchestration_ctx: Option<openfang_types::orchestration::OrchestrationContext>,
    orchestration_live: Option<&tool_runner::OrchestrationLive>,
) -> OpenFangResult<AgentLoopResult> {
    tool_runner::MAX_AGENT_CALL_DEPTH_LIMIT
        .scope(
            std::cell::Cell::new(runtime_limits.max_agent_call_depth),
            async {
                info!(agent = %manifest.name, "Starting streaming agent loop");

                // Initialize AINL graph memory writer (non-fatal if it fails)
                let graph_memory = match crate::graph_memory_writer::GraphMemoryWriter::open_with_notify(
                    &session.agent_id.to_string(),
                    graph_memory_sse_hook(&kernel),
                ) {
                    Ok(gm) => Some(gm),
                    Err(e) => {
                        let expected_db = graph_memory_expected_db_path(&session.agent_id);
                        warn!(
                            agent_id = %session.agent_id,
                            error = %e,
                            expected_db = %expected_db.display(),
                            "AINL graph memory: writer unavailable — episodes, facts, patterns, persona prompt hook, and evolution will not run for this agent until the DB opens successfully (check path and permissions)"
                        );
                        None
                    }
                };

                if let Some(ref gm) = graph_memory {
                    gm.drain_python_graph_memory_inbox().await;
                }

                let live_llm = kernel
                    .as_ref()
                    .and_then(|k| k.live_llm_config());

    // Extract hand-allowed env vars from manifest metadata (set by kernel for hand settings)
    let hand_allowed_env: Vec<String> = manifest
        .metadata
        .get("hand_allowed_env")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    let memory_policy = crate::graph_memory_context::MemoryContextPolicy::from_manifest_for_agent(
        manifest,
        Some(&session.agent_id.to_string()),
    );
    if memory_policy.temporary_mode {
        crate::graph_memory_context::record_temp_mode_read_suppressed();
        info!(
            agent = %manifest.name,
            "Memory temporary mode enabled (streaming): skipping runtime memory recalls and graph-memory prompt injection"
        );
    }

    // Recall relevant memories — prefer vector similarity search when embedding driver is available
    let memories = if !memory_policy.allow_reads() {
        Vec::new()
    } else if let Some(emb) = embedding_driver {
        match emb.embed_one(user_message).await {
            Ok(query_vec) => {
                debug!("Using vector recall (streaming, dims={})", query_vec.len());
                memory
                    .recall_with_embedding_async(
                        user_message,
                        5,
                        Some(MemoryFilter {
                            agent_id: Some(session.agent_id),
                            ..Default::default()
                        }),
                        Some(&query_vec),
                    )
                    .await
                    .unwrap_or_default()
            }
            Err(e) => {
                warn!("Embedding recall failed (streaming), falling back to text search: {e}");
                memory
                    .recall(
                        user_message,
                        5,
                        Some(MemoryFilter {
                            agent_id: Some(session.agent_id),
                            ..Default::default()
                        }),
                    )
                    .await
                    .unwrap_or_default()
            }
        }
    } else {
        memory
            .recall(
                user_message,
                5,
                Some(MemoryFilter {
                    agent_id: Some(session.agent_id),
                    ..Default::default()
                }),
            )
            .await
            .unwrap_or_default()
    };

    // Fire BeforePromptBuild hook
    let agent_id_str = session.agent_id.0.to_string();
    if let Some(hook_reg) = hooks {
        let ctx = crate::hooks::HookContext {
            agent_name: &manifest.name,
            agent_id: agent_id_str.as_str(),
            event: openfang_types::agent::HookEvent::BeforePromptBuild,
            data: serde_json::json!({
                "system_prompt": &manifest.model.system_prompt,
                "user_message": user_message,
            }),
        };
        let _ = hook_reg.fire(&ctx);
    }

    // Build the system prompt — kernel expands `[model].system_prompt` via prompt_builder; we
    // append recalled memories here since they are resolved at loop time.
    let mut system_prompt = loop_time_system_prompt_from_manifest(manifest);
    if !memories.is_empty() {
        let mem_pairs: Vec<(String, String)> = memories
            .iter()
            .map(|m| (String::new(), m.content.clone()))
            .collect();
        system_prompt.push_str("\n\n");
        system_prompt.push_str(&crate::prompt_builder::build_memory_section(&mem_pairs));
    }

    // Orchestration context: append hierarchical orchestration details to system prompt
    if let Some(ref octx) = orchestration_ctx {
        system_prompt.push_str("\n\n## Orchestration Context\n");
        system_prompt.push_str(&octx.system_prompt_appendix(runtime_limits.max_agent_call_depth));
    }

    if memory_policy.allow_reads() {
        if let Some(ref gm) = graph_memory {
            let prompt_ctx =
                crate::graph_memory_context::build_prompt_memory_context(gm, &memory_policy).await;
            if !prompt_ctx.is_empty() {
                system_prompt.push_str(&prompt_ctx.to_prompt_block());
            }
            if !prompt_ctx.selection_debug.is_empty() {
                debug!(
                    agent_id = %session.agent_id,
                    why_selected = %serde_json::Value::Array(prompt_ctx.selection_debug.clone()),
                    "graph-memory why_selected diagnostics (streaming)"
                );
            }
        }
    }

    // Persona hook (streaming): same as `run_agent_loop`.
    if memory_policy.allow_reads() {
        if let Some(ref gm) = graph_memory {
        let persona_nodes = gm.recall_persona(60 * 60 * 24 * 90).await;
        if !persona_nodes.is_empty() {
            let traits: Vec<String> = persona_nodes
                .iter()
                .filter_map(|n| {
                    if let ainl_memory::AinlNodeType::Persona { persona } = &n.node_type {
                        if persona.strength >= 0.1 {
                            Some(format!(
                                "{} (strength={:.2})",
                                persona.trait_name, persona.strength
                            ))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                })
                .collect();
            if !traits.is_empty() {
                let persona_instruction = format!(
                    "\n\n[Persona traits active: {}]",
                    traits.join(", ")
                );
                system_prompt.push_str(&persona_instruction);
                debug!(
                    agent_id = %session.agent_id,
                    trait_count = traits.len(),
                    "AINL persona hook: injected {} trait(s) into system prompt",
                    traits.len()
                );
            }
        }
    }
    }

    let graph_memory_for_mcp = if memory_policy.allow_writes() {
        graph_memory.clone()
    } else {
        crate::graph_memory_context::record_temp_mode_write_suppressed();
        None
    };
    append_mcp_readiness_context(
        &mut system_prompt,
        mcp_connections,
        &kernel,
        &graph_memory_for_mcp,
        &session.agent_id,
    )
    .await;

    // Ultra Cost-Efficient Mode: compress user message before LLM context (streaming path).
    // See run_agent_loop for the full explanation; same logic applies here.
    let mut compression_savings_pct: u8 = 0;
    let mode = manifest
        .metadata
        .get("efficient_mode")
        .and_then(|v| v.as_str())
        .map(crate::prompt_compressor::EfficientMode::parse_natural_language)
        .unwrap_or_default();
    let (r, compression_metrics) =
        crate::prompt_compressor::compress_with_metrics(user_message, mode, None);
    if r.tokens_saved() > 0 {
        let ratio_pct = 100u64.saturating_sub(
            (r.compressed_tokens as u64 * 100) / r.original_tokens.max(1) as u64,
        );
        compression_savings_pct = ratio_pct.min(100) as u8;
        info!(
            orig_tok = r.original_tokens,
            compressed_tok = r.compressed_tokens,
            savings_pct = ratio_pct,
            "prompt:compressed (streaming)"
        );
    }
    let eco_mode_label_s = match mode {
        crate::prompt_compressor::EfficientMode::Off => "off",
        crate::prompt_compressor::EfficientMode::Balanced => "balanced",
        crate::prompt_compressor::EfficientMode::Aggressive => "aggressive",
    };
    crate::eco_telemetry::record_turn(
        &session.agent_id.0.to_string(),
        eco_mode_label_s,
        user_message,
        &r.text,
        compression_savings_pct,
    );
    let _compressed_msg_s = if mode != crate::prompt_compressor::EfficientMode::Off {
        Some(r.text.clone())
    } else {
        None
    };
    let compressed_input: Option<String> = if compression_savings_pct > 0 {
        _compressed_msg_s.clone()
    } else {
        None
    };
    let compression_semantic_score = compression_metrics.semantic_preservation_score;
    let adaptive_snap_s: Option<openfang_types::adaptive_eco::AdaptiveEcoTurnSnapshot> = manifest
        .metadata
        .get("adaptive_eco")
        .and_then(|v| serde_json::from_value(v.clone()).ok());
    let adaptive_confidence = adaptive_snap_s.as_ref().map(|s| {
        openfang_types::adaptive_eco::compute_adaptive_confidence(s, compression_semantic_score)
    });
    let eco_counterfactual = crate::eco_counterfactual::build_eco_counterfactual_receipt(
        user_message,
        mode,
        &r,
        compression_savings_pct,
        adaptive_snap_s.as_ref(),
    );
    let adaptive_eco_effective_mode = adaptive_snap_s.as_ref().map(|s| s.effective_mode.clone());
    let adaptive_eco_recommended_mode = adaptive_snap_s.as_ref().map(|s| s.recommended_mode.clone());
    let adaptive_eco_reason_codes = adaptive_snap_s.as_ref().map(|s| s.reason_codes.clone());
    let session_user_message_s: &str = _compressed_msg_s.as_deref().unwrap_or(user_message);

    // Emit compression stats as the first stream event so the client can display
    // the reduction percentage and diff button before the response arrives.
    if compression_savings_pct > 0 || adaptive_snap_s.is_some() {
        let compressed_text_for_event = compressed_input.clone().unwrap_or_default();
        let _ = stream_tx
            .send(StreamEvent::CompressionStats {
                savings_pct: compression_savings_pct,
                compressed_text: compressed_text_for_event,
                semantic_score: compression_semantic_score,
                adaptive_confidence,
                counterfactual: eco_counterfactual.clone(),
                adaptive_eco_effective_mode: adaptive_eco_effective_mode.clone(),
                adaptive_eco_recommended_mode: adaptive_eco_recommended_mode.clone(),
                adaptive_eco_reason_codes: adaptive_eco_reason_codes.clone(),
            })
            .await;
    }

    // Add the user message to session history.
    // When content blocks are provided (e.g. text + image from a channel),
    // use multimodal message format so the LLM receives the image for vision.
    if let Some(blocks) = user_content_blocks {
        session.messages.push(Message::user_with_blocks(blocks));
    } else {
        session.messages.push(Message::user(session_user_message_s));
    }

    let llm_messages: Vec<Message> = session
        .messages
        .iter()
        .filter(|m| m.role != Role::System)
        .cloned()
        .collect();

    // Strip Image blocks from session to prevent base64 bloat.
    // The LLM already received them via llm_messages above.
    for msg in session.messages.iter_mut() {
        if let MessageContent::Blocks(blocks) = &mut msg.content {
            let had_images = blocks
                .iter()
                .any(|b| matches!(b, ContentBlock::Image { .. }));
            if had_images {
                blocks.retain(|b| !matches!(b, ContentBlock::Image { .. }));
                if blocks.is_empty() {
                    blocks.push(ContentBlock::Text {
                        text: "[Image processed]".to_string(),
                        provider_metadata: None,
                    });
                }
            }
        }
    }

    // Validate and repair session history (drop orphans, merge consecutive)
    let mut messages = crate::session_repair::validate_and_repair(&llm_messages);

    // Inject canonical context as the first user message (not in system prompt)
    // to keep the system prompt stable across turns for provider prompt caching.
    if let Some(cc_msg) = manifest
        .metadata
        .get("canonical_context_msg")
        .and_then(|v| v.as_str())
    {
        if !cc_msg.is_empty() {
            messages.insert(0, Message::user(cc_msg));
        }
    }

    let mut total_usage = TokenUsage::default();
    let final_response;

    // Safety valve: trim excessively long message histories to prevent context overflow.
    if messages.len() > runtime_limits.max_history_messages {
        let trim_count = messages.len() - runtime_limits.max_history_messages;
        warn!(
            agent = %manifest.name,
            total_messages = messages.len(),
            trimming = trim_count,
            "Trimming old messages to prevent context overflow (streaming)"
        );
        messages.drain(..trim_count);
        // Re-validate after trimming: the drain may have split a ToolUse/ToolResult
        // pair across the cut boundary, leaving orphaned blocks that cause the LLM
        // to return empty responses (input_tokens=0).
        messages = crate::session_repair::validate_and_repair(&messages);
    }

    // Use autonomous config max_iterations if set, else `[runtime_limits]` default.
    let max_iterations = manifest
        .autonomous
        .as_ref()
        .map(|a| a.max_iterations)
        .unwrap_or(runtime_limits.max_iterations);

    // Initialize loop guard — scale circuit breaker for autonomous agents
    let loop_guard_config = {
        let mut cfg = LoopGuardConfig::default();
        if max_iterations > cfg.global_circuit_breaker {
            cfg.global_circuit_breaker = max_iterations * 3;
        }
        cfg
    };
    let mut loop_guard = LoopGuard::new(loop_guard_config);
    let mut consecutive_max_tokens: u32 = 0;
    // Counts consecutive iterations where every tool call was blocked by the loop guard.
    // Used to exit early when the model is clearly stuck calling the same blocked tools.
    let mut consecutive_all_blocked: u32 = 0;
    let mut tool_error_tracker = ToolErrorTracker::new();
    // Tracks the names of all tools called since the last EndTurn response, so process
    // phantom detection can tell whether the model claimed to start/stop a process without
    // actually calling the corresponding tool.
    let mut last_tools_called: std::collections::HashSet<String> = Default::default();

    // Build context budget from model's actual context window (or fallback to default)
    let ctx_window = context_window_tokens.unwrap_or(DEFAULT_CONTEXT_WINDOW);
    let context_budget = ContextBudget::new(ctx_window);
    let mut any_tools_executed = false;
    let loop_t0 = std::time::Instant::now();
    let mut llm_fallback_note: Option<String> = None;

    #[cfg(feature = "ainl-runtime-engine")]
    let ainl_prelude_telemetry: Option<crate::ainl_runtime_bridge::AinlBridgeTelemetry> = {
        let mut ainl_runtime_turn_allowed = true;
        if ainl_runtime_engine_switch_active(manifest) {
            if let Some(ref kh) = kernel {
                if kh.requires_approval("ainl_runtime_engine") {
                    let summary = format!(
                        "{} requests ainl-runtime-engine (message {} chars)",
                        manifest.name,
                        session_user_message_s.len()
                    );
                    match kh
                        .request_approval(agent_id_str.as_str(), "ainl_runtime_engine", &summary)
                        .await
                    {
                        Ok(true) => {}
                        Ok(false) => {
                            warn!(
                                agent = %manifest.name,
                                "ainl-runtime-engine: human approval denied — continuing with OpenFang loop (streaming)"
                            );
                            ainl_runtime_turn_allowed = false;
                        }
                        Err(e) => {
                            warn!(
                                agent = %manifest.name,
                                error = %e,
                                "ainl-runtime-engine: approval request failed — continuing with OpenFang loop (streaming)"
                            );
                            ainl_runtime_turn_allowed = false;
                        }
                    }
                }
            }
        }
        if ainl_runtime_turn_allowed {
            run_ainl_runtime_engine_prelude(
                manifest,
                &graph_memory,
                session_user_message_s,
                agent_id_str.as_str(),
                &runtime_limits,
                &orchestration_ctx,
            )
            .await
        } else {
            None
        }
    };

    #[cfg(feature = "ainl-runtime-engine")]
    if let Some(ref t) = ainl_prelude_telemetry {
        let payload = serde_json::json!({
            "turn_status": format!("{:?}", t.turn_status),
            "partial_success": t.partial_success,
            "warning_count": t.warning_count,
            "has_extraction_report": t.has_extraction_report,
            "memory_context_recent_episodes": t.memory_context_recent_episodes,
            "memory_context_relevant_semantic": t.memory_context_relevant_semantic,
            "memory_context_active_patches": t.memory_context_active_patches,
            "memory_context_has_persona_snapshot": t.memory_context_has_persona_snapshot,
            "patch_dispatch_count": t.patch_dispatch_count,
            "patch_dispatch_adapter_output_count": t.patch_dispatch_adapter_output_count,
            "steps_executed": t.steps_executed,
        });
        let _ = stream_tx
            .send(StreamEvent::AinlRuntimeTelemetry { payload })
            .await;
    }

    let mut btw_rx = btw_rx;
    let mut redirect_rx = redirect_rx;
    for iteration in 0..max_iterations {
        debug!(iteration, "Streaming agent loop iteration");

        // Drain any /btw context injections the user sent while this loop was running.
        if let Some(ref mut rx) = btw_rx {
            while let Ok(btw_text) = rx.try_recv() {
                info!("Injecting /btw context ({} chars)", btw_text.len());
                messages.push(openfang_types::message::Message::user(format!(
                    "[btw] {btw_text}"
                )));
            }
        }

        // Drain any /redirect override the user sent. Unlike /btw, a redirect prunes
        // recent assistant and tool messages to break the agent's current momentum,
        // then injects a high-priority system message with the new directive.
        if let Some(ref mut rx) = redirect_rx {
            if let Ok(redirect_text) = rx.try_recv() {
                info!(
                    "Applying /redirect override ({} chars)",
                    redirect_text.len()
                );
                // Prune the last ~8 assistant messages from the working context.
                let mut pruned = 0usize;
                let mut i = messages.len();
                while i > 0 && pruned < 8 {
                    i -= 1;
                    if messages[i].role == Role::Assistant {
                        messages.remove(i);
                        pruned += 1;
                    }
                }
                if pruned > 0 {
                    info!("Pruned {pruned} assistant messages for /redirect");
                }
                // Inject the override as a system message so it carries maximum weight.
                messages.push(Message::system(format!(
                    "[REDIRECT] STOP your current plan immediately. Do not continue previous steps. New directive from user: {redirect_text}"
                )));
            }
        }

        // Context overflow recovery pipeline (replaces emergency_trim_messages)
        let recovery =
            recover_from_overflow(&mut messages, &system_prompt, available_tools, ctx_window);
        match &recovery {
            RecoveryStage::None => {}
            RecoveryStage::FinalError => {
                if stream_tx.send(StreamEvent::PhaseChange {
                    phase: "context_warning".to_string(),
                    detail: Some("Context overflow unrecoverable. Use /reset or /compact.".to_string()),
                }).await.is_err() {
                    warn!("Stream consumer disconnected while sending context overflow warning");
                }
            }
            _ => {
                if stream_tx.send(StreamEvent::PhaseChange {
                    phase: "context_warning".to_string(),
                    detail: Some("Older messages trimmed to stay within context limits. Use /compact for smarter summarization.".to_string()),
                }).await.is_err() {
                    warn!("Stream consumer disconnected while sending context trim warning");
                }
            }
        }

        // Re-validate tool_call/tool_result pairing after overflow drains
        // which may have broken assistant→tool ordering invariants.
        // (Matches the non-streaming loop; fixes Qwen3.5-plus "tool_calls must
        // be followed by tool messages" errors after context overflow recovery.)
        if recovery != RecoveryStage::None {
            messages = crate::session_repair::validate_and_repair(&messages);
        }

        // Context guard: compact oversized tool results before LLM call
        apply_context_guard(&mut messages, &context_budget, available_tools);

        // Strip provider prefix: "openrouter/google/gemini-2.5-flash" → "google/gemini-2.5-flash"
        let api_model = strip_provider_prefix(&manifest.model.model, &manifest.model.provider);

        let request = CompletionRequest {
            model: api_model,
            messages: messages.clone(),
            tools: available_tools.to_vec(),
            max_tokens: manifest.model.max_tokens,
            temperature: manifest.model.temperature,
            system: Some(system_prompt.clone()),
            thinking: None,
        };

        // Notify phase: on first iteration emit Streaming; on subsequent
        // iterations (after tool execution) emit Thinking so the UI shows
        // "Thinking..." instead of overwriting streamed text with "streaming".
        // Also emit an iteration-progress PhaseChange so the client knows
        // we are on step N and still actively working.
        if let Some(cb) = on_phase {
            if iteration == 0 {
                cb(LoopPhase::Streaming);
            } else {
                cb(LoopPhase::Thinking);
            }
        }
        if iteration > 0 {
            let _ = stream_tx
                .send(StreamEvent::PhaseChange {
                    phase: "iteration".to_string(),
                    detail: Some(format!("Step {} — thinking…", iteration + 1)),
                })
                .await;
        }

        // Stream LLM call with retry, error classification, and circuit breaker
        let provider_name = manifest.model.provider.as_str();
        let llm_fb = LlmFallbackContext {
            llm_http: live_llm.as_ref(),
            llm_kernel: kernel.as_ref(),
        };
        let (mut response, fb_note) = stream_with_retry(
            &*driver,
            request,
            stream_tx.clone(),
            Some(provider_name),
            None,
            &manifest.fallback_models,
            &llm_fb,
        )
        .await?;
        if fb_note.is_some() {
            llm_fallback_note = fb_note;
        }

        total_usage.input_tokens += response.usage.input_tokens;
        total_usage.output_tokens += response.usage.output_tokens;

        // Recover tool calls output as text (streaming path)
        if matches!(
            response.stop_reason,
            StopReason::EndTurn | StopReason::StopSequence
        ) && response.tool_calls.is_empty()
        {
            let recovered = recover_text_tool_calls(&response.text(), available_tools);
            if !recovered.is_empty() {
                info!(
                    count = recovered.len(),
                    "Recovered text-based tool calls (streaming) → promoting to ToolUse"
                );
                response.tool_calls = recovered;
                response.stop_reason = StopReason::ToolUse;
                let mut new_blocks: Vec<ContentBlock> = Vec::new();
                for tc in &response.tool_calls {
                    new_blocks.push(ContentBlock::ToolUse {
                        id: tc.id.clone(),
                        name: tc.name.clone(),
                        input: tc.input.clone(),
                        provider_metadata: None,
                    });
                }
                response.content = new_blocks;
            }
        }

        match response.stop_reason {
            StopReason::EndTurn | StopReason::StopSequence => {
                let text = response.text();

                // Parse reply directives from the streaming response text
                let (cleaned_text_s, parsed_directives_s) =
                    crate::reply_directives::parse_directives(&text);
                let text = cleaned_text_s;

                // NO_REPLY / [SILENT]: agent intentionally chose not to reply.
                // [SILENT] must not be stored literally — it reinforces silence in future turns.
                if is_silent_token(&text) || parsed_directives_s.silent {
                    debug!(agent = %manifest.name, "Agent chose NO_REPLY/silent (streaming) — silent completion");
                    session
                        .messages
                        .push(Message::assistant("[no reply needed]".to_string()));
                    memory
                        .save_session_async(session)
                        .await
                        .map_err(|e| OpenFangError::Memory(e.to_string()))?;
                    return Ok(AgentLoopResult {
                        response: String::new(),
                        total_usage,
                        iterations: iteration + 1,
                        cost_usd: None,
                        silent: true,
                        directives: openfang_types::message::ReplyDirectives {
                            reply_to: parsed_directives_s.reply_to,
                            current_thread: parsed_directives_s.current_thread,
                            silent: true,
                        },
                        latency_ms: Some(loop_t0.elapsed().as_millis() as u64),
                        llm_fallback_note: llm_fallback_note.clone(),
                        compression_savings_pct,
                        compressed_input: compressed_input.clone(),
                        compression_semantic_score,
                        adaptive_confidence,
                        eco_counterfactual,
                        adaptive_eco_effective_mode: adaptive_eco_effective_mode.clone(),
                        adaptive_eco_recommended_mode: adaptive_eco_recommended_mode.clone(),
                        adaptive_eco_reason_codes: adaptive_eco_reason_codes.clone(),
                        ainl_runtime_telemetry: {
                        #[cfg(feature = "ainl-runtime-engine")]
                        {
                            ainl_prelude_telemetry.clone()
                        }
                        #[cfg(not(feature = "ainl-runtime-engine"))]
                        {
                            None
                        }
                    },
                    });
                }

                // One-shot retry: if the LLM returns empty text with no tool use,
                // try once more before accepting the empty result.
                // Triggers on first call OR when input_tokens=0 (silently failed request).
                if text.trim().is_empty()
                    && response.tool_calls.is_empty()
                    && !response.has_any_content()
                {
                    let is_silent_failure =
                        response.usage.input_tokens == 0 && response.usage.output_tokens == 0;
                    if iteration == 0 || is_silent_failure {
                        warn!(
                            agent = %manifest.name,
                            iteration,
                            input_tokens = response.usage.input_tokens,
                            output_tokens = response.usage.output_tokens,
                            silent_failure = is_silent_failure,
                            "Empty response (streaming), retrying once"
                        );
                        // Re-validate messages before retry — the history may have
                        // broken tool_use/tool_result pairs that caused the failure.
                        if is_silent_failure {
                            messages = crate::session_repair::validate_and_repair(&messages);
                        }
                        messages.push(Message::assistant("[no response]".to_string()));
                        messages.push(Message::user("Please provide your response.".to_string()));
                        continue;
                    }
                }

                // Guard against empty response — covers both iteration 0 and post-tool cycles
                let text = if text.trim().is_empty() {
                    warn!(
                        agent = %manifest.name,
                        iteration,
                        input_tokens = total_usage.input_tokens,
                        output_tokens = total_usage.output_tokens,
                        messages_count = messages.len(),
                        "Empty response from LLM (streaming) — guard activated"
                    );
                    if any_tools_executed {
                        "[Task completed — the agent executed tools but did not produce a text summary.]".to_string()
                    } else {
                        "[The model returned an empty response. This usually means the model is overloaded, the context is too large, or the API key lacks credits. Try again or check /status.]".to_string()
                    }
                } else {
                    text
                };
                let turn_tool_names: Vec<String> = last_tools_called.iter().cloned().collect();
                // Phantom action detection (streaming): channel actions without tools.
                let text = if !any_tools_executed
                    && iteration == 0
                    && phantom_action_detected(&text)
                {
                    warn!(agent = %manifest.name, "Phantom action detected (streaming) — re-prompting for real tool use");
                    messages.push(Message::assistant(text));
                    messages.push(Message::user(
                        "[System: You claimed to perform an action but did not call any tools. \
                         You must use the appropriate tool (e.g., channel_send, web_fetch, file_write) \
                         to actually perform the action. Do not claim completion without executing tools.]"
                    ));
                    continue;
                } else {
                    text
                };
                let text = if scheduling_phantom_detected(
                    session_user_message_s,
                    &text,
                    &last_tools_called,
                ) {
                    warn!(agent = %manifest.name, "Scheduling phantom detected (streaming) — re-prompting");
                    messages.push(Message::assistant(text));
                    messages.push(Message::user(
                        "[System: The user asked to register recurring / scheduled work with the ArmaraOS kernel scheduler. \
                         You must call schedule_create or cron_create (or schedule_list / cron_list first). \
                         Jobs persist under ~/.armaraos/cron_jobs.json. Do not claim crontab, launchd, or memory-only storage alone is sufficient.]",
                    ));
                    last_tools_called.clear();
                    continue;
                } else {
                    text
                };
                // Process phantom detection (streaming): model claimed to start/stop a process
                // without calling process_start or process_kill.
                let text = if process_phantom_detected(&text, &last_tools_called) {
                    warn!(agent = %manifest.name, "Process phantom detected (streaming) — re-prompting");
                    messages.push(Message::assistant(text));
                    messages.push(Message::user(
                        "[System: You described starting or stopping a process but did not call \
                         process_start or process_kill. You must call the appropriate process \
                         management tool to actually perform the action — do not claim the process \
                         is running/stopped without having called the tool.]",
                    ));
                    last_tools_called.clear();
                    continue;
                } else {
                    last_tools_called.clear();
                    text
                };
                final_response = text.clone();
                session.messages.push(Message::assistant(text));

                // Prune NO_REPLY heartbeat turns to save context budget
                crate::session_repair::prune_heartbeat_turns(&mut session.messages, 10);

                let tools_for_episode =
                    canonicalize_turn_tool_names_for_graph_storage(&turn_tool_names);
                // AINL graph memory: episode + heuristic semantic/procedural extraction (before session persist)
                if memory_policy.allow_writes() {
                    if let Some(ref gm) = graph_memory {
                    let turn_trace =
                        graph_memory_turn_trace_json(
                            agent_id_str.as_str(),
                            &orchestration_ctx,
                            Some(&compression_metrics),
                            manifest.metadata.get("adaptive_eco"),
                        );
                    let trace_tags = graph_memory_trace_fact_tags(&orchestration_ctx);
                    let pattern_trace_id = graph_memory_pattern_trace_id(&orchestration_ctx);
                    let episode_tags = crate::ainl_semantic_tagger_bridge::SemanticTaggerBridge::tag_episode(
                        &tools_for_episode,
                    );
                    let stream_vitals = crate::vitals_classifier::classify_from_text(
                        &final_response,
                        turn_tool_names.len(),
                    );
                    let (sv_gate, sv_phase, sv_trust) = stream_vitals
                        .map(|v| (Some(v.gate.as_str().to_string()), Some(v.phase.clone()), Some(v.trust)))
                        .unwrap_or((None, None, None));
                    if let Some(episode_id) = gm
                        .record_turn(
                            tools_for_episode.clone(),
                            None,
                            turn_trace,
                            &episode_tags,
                            sv_gate,
                            sv_phase,
                            sv_trust,
                        )
                        .await
                    {
                        let (facts, turn_pattern) =
                            crate::ainl_graph_extractor_bridge::graph_memory_turn_extraction(
                                session_user_message_s,
                                &final_response,
                                &tools_for_episode,
                                &turn_tool_names,
                                agent_id_str.as_str(),
                            );
                        for fact in facts {
                            let mut fact_tags: Vec<String> = trace_tags.to_vec();
                            fact_tags.extend(
                                crate::ainl_semantic_tagger_bridge::SemanticTaggerBridge::tag_fact(
                                    &fact.text,
                                ),
                            );
                            gm.record_fact_with_tags(
                                fact.text,
                                fact.confidence,
                                episode_id,
                                &fact_tags,
                            )
                            .await;
                        }
                        if let Some(pattern) = turn_pattern {
                            gm.record_pattern(
                                &pattern.name,
                                pattern.tool_sequence,
                                pattern.confidence,
                                pattern_trace_id.clone(),
                            )
                            .await;
                        }
                    } else {
                        warn!(
                            agent_id = %session.agent_id,
                            "AINL graph memory: episode write failed; skipping turn facts/patterns (streaming)"
                        );
                    }
                }
                } else {
                    crate::graph_memory_context::record_temp_mode_write_suppressed();
                }
                if memory_policy.allow_writes() {
                    if let Some(gm) = graph_memory.clone() {
                    let turn_outcome = crate::persona_evolution::TurnOutcome {
                        tool_calls: tools_for_episode.clone(),
                        delegation_to: None,
                    };
                    // Same cooperative barrier as the non-streaming path (see above).
                    tokio::task::yield_now().await;
                    tokio::spawn(async move {
                        let agent_id = gm.agent_id().to_string();
                        let _report = gm.run_persona_evolution_pass().await;
                        let _ = gm.run_background_memory_consolidation().await;
                        if let Err(e) = crate::persona_evolution::PersonaEvolutionHook::evolve_from_turn(
                            &gm,
                            &agent_id,
                            &turn_outcome,
                        )
                        .await
                        {
                            warn!(
                                agent_id = %agent_id,
                                error = %e,
                                "AINL persona turn evolution (AINL_PERSONA_EVOLUTION) failed; continuing (streaming)"
                            );
                        }
                        graph_memory_refresh_armaraos_export_json(&agent_id).await;
                    });
                }
                } else {
                    crate::graph_memory_context::record_temp_mode_write_suppressed();
                }

                memory
                    .save_session_async(session)
                    .await
                    .map_err(|e| OpenFangError::Memory(e.to_string()))?;

                // Remember this interaction (with embedding if available)
                let interaction_text = format!(
                    "User asked: {}\nI responded: {}",
                    user_message, final_response
                );
                if !memory_policy.allow_writes() {
                    crate::graph_memory_context::record_temp_mode_write_suppressed();
                } else if let Some(emb) = embedding_driver {
                    match emb.embed_one(&interaction_text).await {
                        Ok(vec) => {
                            let _ = memory
                                .remember_with_embedding_async(
                                    session.agent_id,
                                    &interaction_text,
                                    MemorySource::Conversation,
                                    "episodic",
                                    HashMap::new(),
                                    Some(&vec),
                                )
                                .await;
                        }
                        Err(e) => {
                            warn!("Embedding for remember failed (streaming): {e}");
                            let _ = memory
                                .remember(
                                    session.agent_id,
                                    &interaction_text,
                                    MemorySource::Conversation,
                                    "episodic",
                                    HashMap::new(),
                                )
                                .await;
                        }
                    }
                } else if memory_policy.allow_writes() {
                    let _ = memory
                        .remember(
                            session.agent_id,
                            &interaction_text,
                            MemorySource::Conversation,
                            "episodic",
                            HashMap::new(),
                        )
                        .await;
                }

                // Notify phase: Done
                if let Some(cb) = on_phase {
                    cb(LoopPhase::Done);
                }

                info!(
                    agent = %manifest.name,
                    iterations = iteration + 1,
                    tokens = total_usage.total(),
                    "Streaming agent loop completed"
                );

                // Fire AgentLoopEnd hook
                if let Some(hook_reg) = hooks {
                    let ctx = crate::hooks::HookContext {
                        agent_name: &manifest.name,
                        agent_id: agent_id_str.as_str(),
                        event: openfang_types::agent::HookEvent::AgentLoopEnd,
                        data: serde_json::json!({
                            "iterations": iteration + 1,
                            "response_length": final_response.len(),
                        }),
                    };
                    let _ = hook_reg.fire(&ctx);
                }

                return Ok(AgentLoopResult {
                    response: final_response,
                    total_usage,
                    iterations: iteration + 1,
                    cost_usd: None,
                    silent: false,
                    directives: Default::default(),
                    latency_ms: Some(loop_t0.elapsed().as_millis() as u64),
                    llm_fallback_note: llm_fallback_note.clone(),
                    compression_savings_pct,
                    compressed_input: compressed_input.clone(),
                    compression_semantic_score,
                    adaptive_confidence,
                    eco_counterfactual,
                    adaptive_eco_effective_mode: adaptive_eco_effective_mode.clone(),
                    adaptive_eco_recommended_mode: adaptive_eco_recommended_mode.clone(),
                    adaptive_eco_reason_codes: adaptive_eco_reason_codes.clone(),
                    ainl_runtime_telemetry: {
                        #[cfg(feature = "ainl-runtime-engine")]
                        {
                            ainl_prelude_telemetry.clone()
                        }
                        #[cfg(not(feature = "ainl-runtime-engine"))]
                        {
                            None
                        }
                    },
                });
            }
            StopReason::ToolUse => {
                // Reset MaxTokens continuation counter on tool use
                consecutive_max_tokens = 0;
                any_tools_executed = true;

                let assistant_blocks = response.content.clone();

                session.messages.push(Message {
                    role: Role::Assistant,
                    content: MessageContent::Blocks(assistant_blocks.clone()),
                    orchestration_ctx: None,
                });
                messages.push(Message {
                    role: Role::Assistant,
                    content: MessageContent::Blocks(assistant_blocks),
                    orchestration_ctx: None,
                });

                let allowed_tool_names: Vec<String> =
                    available_tools.iter().map(|t| t.name.clone()).collect();
                let caller_id_str = session.agent_id.to_string();

                // Execute tool calls with loop guard, timeout, and truncation (streaming path).
                // Same parallel dispatch strategy as the non-streaming path: pre-pass for guard
                // and hooks, then join_all for approved calls.
                let mut tool_result_blocks = Vec::new();

                let deduped_s = deduplicate_tool_calls(&response);
                let mut dispatches_s: Vec<ToolDispatch<'_>> = Vec::with_capacity(deduped_s.len());

                for tool_call in &deduped_s {
                    let verdict = loop_guard.check(&tool_call.name, &tool_call.input);
                    match &verdict {
                        LoopGuardVerdict::CircuitBreak(msg) => {
                            warn!(tool = %tool_call.name, "Circuit breaker triggered (streaming)");
                            if let Err(e) = memory.save_session_async(session).await {
                                warn!("Failed to save session on circuit break: {e}");
                            }
                            if let Some(hook_reg) = hooks {
                                let ctx = crate::hooks::HookContext {
                                    agent_name: &manifest.name,
                                    agent_id: agent_id_str.as_str(),
                                    event: openfang_types::agent::HookEvent::AgentLoopEnd,
                                    data: serde_json::json!({
                                        "reason": "circuit_break",
                                        "error": msg.as_str(),
                                    }),
                                };
                                let _ = hook_reg.fire(&ctx);
                            }
                            return Err(OpenFangError::Internal(msg.clone()));
                        }
                        LoopGuardVerdict::Block(msg) => {
                            warn!(tool = %tool_call.name, "Tool call blocked by loop guard (streaming)");
                            dispatches_s.push(ToolDispatch::Resolved(ContentBlock::ToolResult {
                                tool_use_id: tool_call.id.clone(),
                                tool_name: tool_call.name.clone(),
                                content: msg.clone(),
                                is_error: true,
                            }));
                            continue;
                        }
                        _ => {}
                    }

                    debug!(tool = %tool_call.name, id = %tool_call.id, "Executing tool (streaming)");

                    if let Some(cb) = on_phase {
                        let sanitized: String = tool_call
                            .name
                            .chars()
                            .filter(|c| !c.is_control())
                            .take(64)
                            .collect();
                        cb(LoopPhase::ToolUse {
                            tool_name: sanitized,
                        });
                    }

                    if let Some(hook_reg) = hooks {
                        let ctx = crate::hooks::HookContext {
                            agent_name: &manifest.name,
                            agent_id: &caller_id_str,
                            event: openfang_types::agent::HookEvent::BeforeToolCall,
                            data: serde_json::json!({
                                "tool_name": &tool_call.name,
                                "input": &tool_call.input,
                            }),
                        };
                        if let Err(reason) = hook_reg.fire(&ctx) {
                            dispatches_s.push(ToolDispatch::Resolved(ContentBlock::ToolResult {
                                tool_use_id: tool_call.id.clone(),
                                tool_name: tool_call.name.clone(),
                                content: format!(
                                    "Hook blocked tool '{}': {}",
                                    tool_call.name, reason
                                ),
                                is_error: true,
                            }));
                            continue;
                        }
                    }

                    // Pre-execution required-param validation (streaming path).
                    if let Some(def) = available_tools.iter().find(|d| d.name == tool_call.name) {
                        if let Some(err_msg) =
                            missing_required_params_error(&tool_call.name, &tool_call.input, def)
                        {
                            debug!(tool = %tool_call.name, "Pre-execution param validation failed (streaming)");
                            dispatches_s.push(ToolDispatch::Resolved(ContentBlock::ToolResult {
                                tool_use_id: tool_call.id.clone(),
                                tool_name: tool_call.name.clone(),
                                content: err_msg,
                                is_error: true,
                            }));
                            continue;
                        }
                    }

                    // Track the tool name so EndTurn can check for process phantom actions
                    last_tools_called.insert(tool_call.name.clone());
                    dispatches_s.push(ToolDispatch::Pending { tool_call, verdict });
                }

                let effective_exec_policy = manifest.exec_policy.as_ref();
                let pending_futures_s: Vec<_> = dispatches_s
                    .iter()
                    .filter_map(|d| {
                        if let ToolDispatch::Pending { tool_call, .. } = d {
                            let timeout = tool_timeout_for(&tool_call.name);
                            Some((
                                tool_call,
                                tokio::time::timeout(
                                    timeout,
                                    tool_runner::execute_tool(
                                        &tool_call.id,
                                        &tool_call.name,
                                        &tool_call.input,
                                        kernel.as_ref(),
                                        Some(&allowed_tool_names),
                                        Some(&caller_id_str),
                                        skill_registry,
                                        mcp_connections,
                                        web_ctx,
                                        browser_ctx,
                                        if hand_allowed_env.is_empty() {
                                            None
                                        } else {
                                            Some(&hand_allowed_env)
                                        },
                                        workspace_root,
                                        ainl_library_root,
                                        media_engine,
                                        effective_exec_policy,
                                        tts_engine,
                                        docker_config,
                                        process_manager,
                                        orchestration_live,
                                    ),
                                ),
                            ))
                        } else {
                            None
                        }
                    })
                    .collect();

                let (pending_tcs_s, pending_futs_s): (Vec<&&ToolCall>, Vec<_>) =
                    pending_futures_s.into_iter().unzip();
                let parallel_results_s = if pending_futs_s.is_empty() {
                    Vec::new()
                } else {
                    let detail_base = pending_tcs_s
                        .iter()
                        .map(|tc| tc.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    let detail_base = if detail_base.is_empty() {
                        "tools".to_string()
                    } else {
                        detail_base
                    };
                    let mut interval = tokio::time::interval(Duration::from_secs(4));
                    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                    interval.tick().await;
                    let join_fut = futures::future::join_all(pending_futs_s);
                    tokio::pin!(join_fut);
                    loop {
                        tokio::select! {
                            biased;
                            res = &mut join_fut => break res,
                            _ = interval.tick() => {
                                let _ = stream_tx
                                    .send(StreamEvent::PhaseChange {
                                        phase: "tool_use".to_string(),
                                        detail: Some(format!("{detail_base} — still running…")),
                                    })
                                    .await;
                            }
                        }
                    }
                };

                let mut pending_iter_s =
                    <Vec<&&ToolCall> as IntoIterator>::into_iter(pending_tcs_s)
                        .zip(parallel_results_s.into_iter())
                        .peekable();

                for dispatch in &dispatches_s {
                    match dispatch {
                        ToolDispatch::Resolved(block) => {
                            tool_result_blocks.push(block.clone());
                        }
                        ToolDispatch::Pending { tool_call, verdict } => {
                            let (tc, timeout_result) = pending_iter_s.next().unwrap();
                            let timeout_secs = tool_timeout_for(&tc.name).as_secs();
                            let result = match timeout_result {
                                Ok(r) => r,
                                Err(_) => {
                                    warn!(tool = %tc.name, "Tool execution timed out after {}s (streaming)", timeout_secs);
                                    openfang_types::tool::ToolResult {
                                        tool_use_id: tc.id.clone(),
                                        content: format!(
                                            "Tool '{}' timed out after {}s.",
                                            tc.name, timeout_secs
                                        ),
                                        is_error: true,
                                    }
                                }
                            };

                            if let Some(hook_reg) = hooks {
                                let ctx = crate::hooks::HookContext {
                                    agent_name: &manifest.name,
                                    agent_id: caller_id_str.as_str(),
                                    event: openfang_types::agent::HookEvent::AfterToolCall,
                                    data: serde_json::json!({
                                        "tool_name": &tool_call.name,
                                        "result": &result.content,
                                        "is_error": result.is_error,
                                    }),
                                };
                                let _ = hook_reg.fire(&ctx);
                            }

                            let content =
                                truncate_tool_result_dynamic(&result.content, &context_budget);
                            let final_content =
                                if let LoopGuardVerdict::Warn(ref warn_msg) = verdict {
                                    format!("{content}\n\n[LOOP GUARD] {warn_msg}")
                                } else {
                                    content
                                };

                            let preview: String = final_content.chars().take(300).collect();
                            if stream_tx
                                .send(StreamEvent::ToolExecutionResult {
                                    id: tool_call.id.clone(),
                                    name: tool_call.name.clone(),
                                    result_preview: preview,
                                    is_error: result.is_error,
                                })
                                .await
                                .is_err()
                            {
                                warn!(agent = %manifest.name, "Stream consumer disconnected — continuing tool loop but will not stream further");
                            }

                            tool_result_blocks.push(ContentBlock::ToolResult {
                                tool_use_id: result.tool_use_id,
                                tool_name: tool_call.name.clone(),
                                content: final_content,
                                is_error: result.is_error,
                            });
                        }
                    }
                }

                // All-blocked early exit (streaming path) — mirrors the non-streaming path.
                {
                    let all_blocked = !tool_result_blocks.is_empty()
                        && tool_result_blocks.iter().all(|b| {
                            matches!(b, ContentBlock::ToolResult { content, is_error: true, .. }
                            if content.starts_with("Blocked:") || content.starts_with("Circuit breaker:"))
                        });
                    if all_blocked {
                        consecutive_all_blocked += 1;
                    } else {
                        consecutive_all_blocked = 0;
                    }
                    const MAX_CONSECUTIVE_ALL_BLOCKED_S: u32 = 3;
                    if consecutive_all_blocked >= MAX_CONSECUTIVE_ALL_BLOCKED_S {
                        warn!(
                            agent = %manifest.name,
                            consecutive_all_blocked,
                            "All tool calls blocked for {} consecutive iterations — exiting early (streaming)",
                            consecutive_all_blocked
                        );
                        let summary = "I was unable to complete this task: my tool calls were \
                             repeatedly blocked because I appeared to be stuck calling the same \
                             tools in a loop. Please try rephrasing your request, breaking it \
                             into smaller steps, or using a different approach.";
                        session.messages.push(Message::assistant(summary));
                        if let Err(e) = memory.save_session_async(session).await {
                            warn!("Failed to save session on all-blocked exit (streaming): {e}");
                        }
                        if let Some(cb) = on_phase {
                            cb(LoopPhase::Done);
                        }
                        let _ = stream_tx
                            .send(StreamEvent::TextDelta {
                                text: summary.to_string(),
                            })
                            .await;
                        return Ok(AgentLoopResult {
                            response: summary.to_string(),
                            total_usage,
                            iterations: iteration + 1,
                            cost_usd: None,
                            silent: false,
                            directives: Default::default(),
                            latency_ms: Some(loop_t0.elapsed().as_millis() as u64),
                            llm_fallback_note: llm_fallback_note.clone(),
                            compression_savings_pct,
                            compressed_input: compressed_input.clone(),
                            compression_semantic_score,
                            adaptive_confidence,
                            eco_counterfactual,
                            adaptive_eco_effective_mode: adaptive_eco_effective_mode.clone(),
                            adaptive_eco_recommended_mode: adaptive_eco_recommended_mode.clone(),
                            adaptive_eco_reason_codes: adaptive_eco_reason_codes.clone(),
                            ainl_runtime_telemetry: {
                        #[cfg(feature = "ainl-runtime-engine")]
                        {
                            ainl_prelude_telemetry.clone()
                        }
                        #[cfg(not(feature = "ainl-runtime-engine"))]
                        {
                            None
                        }
                    },
                        });
                    }
                }

                // Approval denials: always inject — the model must not retry denied tools.
                let denial_count = tool_result_blocks
                    .iter()
                    .filter(|b| {
                        matches!(b, ContentBlock::ToolResult { content, is_error: true, .. }
                        if content.contains("requires human approval and was denied"))
                    })
                    .count();
                if denial_count > 0 {
                    tool_result_blocks.push(ContentBlock::Text {
                        text: format!(
                            "[System: {} tool call(s) were denied by approval policy. \
                             Do NOT retry denied tools. Explain to the user what you \
                             wanted to do and that it requires their approval. \
                             Hint: set auto_approve = true in [approval] section of \
                             config.toml, or start with --yolo flag, to auto-approve \
                             all tool calls.]",
                            denial_count
                        ),
                        provider_metadata: None,
                    });
                }

                // Smart error guidance: targeted on first occurrence, escalating on second,
                // silent on third+ (loop guard handles repeated failures from that point).
                if let Some(guidance) =
                    tool_error_tracker.compute_guidance(&tool_result_blocks, denial_count)
                {
                    tool_result_blocks.push(ContentBlock::Text {
                        text: guidance,
                        provider_metadata: None,
                    });
                }

                let tool_results_msg = Message {
                    role: Role::User,
                    content: MessageContent::Blocks(tool_result_blocks.clone()),
                    orchestration_ctx: None,
                };
                session.messages.push(tool_results_msg.clone());
                messages.push(tool_results_msg);

                // Wrap-up injection: if we are within 3 iterations of the limit, tell
                // the LLM to stop calling tools and produce a text summary.
                if iteration + 1 + 3 >= max_iterations {
                    warn!(
                        agent = %manifest.name,
                        iteration,
                        max_iterations,
                        "Approaching iteration limit (streaming) — injecting wrap-up prompt"
                    );
                    messages.push(Message::user(
                        "[System: You are very close to the maximum number of allowed steps. \
                         Stop calling tools now. Write a final text response summarizing \
                         what you have done and any results or next steps for the user.]",
                    ));
                }

                if let Err(e) = memory.save_session_async(session).await {
                    warn!("Failed to interim-save session: {e}");
                }
            }
            StopReason::MaxTokens => {
                consecutive_max_tokens += 1;
                if consecutive_max_tokens >= runtime_limits.max_continuations {
                    let text = response.text();
                    let text = if text.trim().is_empty() {
                        "[Partial response — token limit reached with no text output.]".to_string()
                    } else {
                        text
                    };
                    session.messages.push(Message::assistant(&text));
                    if let Err(e) = memory.save_session_async(session).await {
                        warn!("Failed to save session on max continuations: {e}");
                    }
                    warn!(
                        iteration,
                        consecutive_max_tokens,
                        "Max continuations reached (streaming), returning partial response"
                    );
                    // Fire AgentLoopEnd hook
                    if let Some(hook_reg) = hooks {
                        let ctx = crate::hooks::HookContext {
                            agent_name: &manifest.name,
                            agent_id: agent_id_str.as_str(),
                            event: openfang_types::agent::HookEvent::AgentLoopEnd,
                            data: serde_json::json!({
                                "iterations": iteration + 1,
                                "reason": "max_continuations",
                            }),
                        };
                        let _ = hook_reg.fire(&ctx);
                    }
                    return Ok(AgentLoopResult {
                        response: text,
                        total_usage,
                        iterations: iteration + 1,
                        cost_usd: None,
                        silent: false,
                        directives: Default::default(),
                        latency_ms: Some(loop_t0.elapsed().as_millis() as u64),
                        llm_fallback_note: llm_fallback_note.clone(),
                        compression_savings_pct,
                        compressed_input: compressed_input.clone(),
                        compression_semantic_score,
                        adaptive_confidence,
                        eco_counterfactual,
                        adaptive_eco_effective_mode: adaptive_eco_effective_mode.clone(),
                        adaptive_eco_recommended_mode: adaptive_eco_recommended_mode.clone(),
                        adaptive_eco_reason_codes: adaptive_eco_reason_codes.clone(),
                        ainl_runtime_telemetry: {
                        #[cfg(feature = "ainl-runtime-engine")]
                        {
                            ainl_prelude_telemetry.clone()
                        }
                        #[cfg(not(feature = "ainl-runtime-engine"))]
                        {
                            None
                        }
                    },
                    });
                }
                let text = response.text();
                session.messages.push(Message::assistant(&text));
                messages.push(Message::assistant(&text));
                session.messages.push(Message::user("Please continue."));
                messages.push(Message::user("Please continue."));
                warn!(iteration, "Max tokens hit (streaming), continuing");
            }
        }
    }

    // Iteration limit reached — degrade gracefully instead of hard-erroring (streaming path).
    warn!(
        agent = %manifest.name,
        max_iterations,
        "Streaming agent loop hit max iterations — returning graceful fallback response"
    );

    let fallback = format!(
        "I reached my step limit ({max_iterations} steps) and could not complete the task in one go. \
         If I got stuck in a loop, try `/reset` to clear the session and rephrase your request. \
         If the task genuinely needs more steps, increase `max_iterations` under `[autonomous]` in agent.toml."
    );
    session.messages.push(Message::assistant(&fallback));

    // Stream the fallback message so the UI displays it in the chat bubble
    let _ = stream_tx
        .send(StreamEvent::TextDelta {
            text: fallback.clone(),
        })
        .await;

    if let Err(e) = memory.save_session_async(session).await {
        warn!("Failed to save session on max iterations: {e}");
    }

    if let Some(hook_reg) = hooks {
        let ctx = crate::hooks::HookContext {
            agent_name: &manifest.name,
            agent_id: agent_id_str.as_str(),
            event: openfang_types::agent::HookEvent::AgentLoopEnd,
            data: serde_json::json!({
                "reason": "max_iterations_exceeded",
                "iterations": max_iterations,
            }),
        };
        let _ = hook_reg.fire(&ctx);
    }

    Ok(AgentLoopResult {
        response: fallback,
        total_usage,
        iterations: max_iterations,
        cost_usd: None,
        silent: false,
        directives: Default::default(),
        latency_ms: Some(loop_t0.elapsed().as_millis() as u64),
        llm_fallback_note,
        compression_savings_pct,
        compressed_input,
        compression_semantic_score,
        adaptive_confidence,
        eco_counterfactual,
        adaptive_eco_effective_mode: adaptive_eco_effective_mode.clone(),
        adaptive_eco_recommended_mode: adaptive_eco_recommended_mode.clone(),
        adaptive_eco_reason_codes: adaptive_eco_reason_codes.clone(),
        ainl_runtime_telemetry: {
                        #[cfg(feature = "ainl-runtime-engine")]
                        {
                            ainl_prelude_telemetry.clone()
                        }
                        #[cfg(not(feature = "ainl-runtime-engine"))]
                        {
                            None
                        }
                    },
    })
            }
        )
        .await
}

/// Recover tool calls that LLMs output as plain text instead of the proper
/// `tool_calls` API field. Covers Groq/Llama, DeepSeek, Qwen, and Ollama models.
///
/// Supported patterns:
/// 1. `<function=tool_name>{"key":"value"}</function>`
/// 2. `<function>tool_name{"key":"value"}</function>`
/// 3. `<tool>tool_name{"key":"value"}</tool>`
/// 4. Markdown code blocks containing `tool_name {"key":"value"}`
/// 5. Backtick-wrapped `tool_name {"key":"value"}`
/// 6. `[TOOL_CALL]...[/TOOL_CALL]` blocks (JSON or arrow syntax) — issue #354
/// 7. `<tool_call>{"name":"tool","arguments":{...}}</tool_call>` — Qwen3, issue #332
/// 8. Bare JSON `{"name":"tool","arguments":{...}}` objects (last resort, only if no tags found)
/// 9. `<function name="tool" parameters="{...}" />` — XML attribute style (Groq/Llama)
/// 10. `<|plugin|>...<|endofblock|>` — Qwen/ChatGLM thinking-model format
/// 11. `Action: tool\nAction Input: {"key":"value"}` — ReAct-style (LM Studio, GPT-OSS)
/// 12. `tool_name\n{"key":"value"}` — bare name + JSON on next line (Llama 4 Scout)
/// 13. `<tool_use>{"name":"tool","arguments":{...}}</tool_use>` — Llama 3.1+ variant
/// 14. `<function=tool><parameter=name>value</parameter></function>` — nested XML parameter style
///
/// Validates tool names against available tools and returns synthetic `ToolCall` entries.
fn recover_text_tool_calls(text: &str, available_tools: &[ToolDefinition]) -> Vec<ToolCall> {
    let mut calls = Vec::new();
    let tool_names: Vec<&str> = available_tools.iter().map(|t| t.name.as_str()).collect();

    // Pattern 1: <function=TOOL_NAME>JSON_BODY</function>
    let mut search_from = 0;
    while let Some(start) = text[search_from..].find("<function=") {
        let abs_start = search_from + start;
        let after_prefix = abs_start + "<function=".len();

        // Extract tool name (ends at '>')
        let Some(name_end) = text[after_prefix..].find('>') else {
            search_from = after_prefix;
            continue;
        };
        let tool_name = &text[after_prefix..after_prefix + name_end];
        let json_start = after_prefix + name_end + 1;

        // Find closing </function>
        let Some(close_offset) = text[json_start..].find("</function>") else {
            search_from = json_start;
            continue;
        };
        let json_body = text[json_start..json_start + close_offset].trim();
        search_from = json_start + close_offset + "</function>".len();

        // Validate: tool name must be in available_tools
        if !tool_names.contains(&tool_name) {
            warn!(
                tool = tool_name,
                "Text-based tool call for unknown tool — skipping"
            );
            continue;
        }

        // Parse JSON input, or fall back to nested XML parameter blocks.
        let input: serde_json::Value = match serde_json::from_str(json_body) {
            Ok(v) => v,
            Err(json_err) => match parse_xml_parameter_blocks(json_body) {
                Some(v) => v,
                None => {
                    warn!(tool = tool_name, error = %json_err, "Failed to parse text-based tool call payload — skipping");
                    continue;
                }
            },
        };

        info!(
            tool = tool_name,
            "Recovered text-based tool call → synthetic ToolUse"
        );
        calls.push(ToolCall {
            id: format!("recovered_{}", uuid::Uuid::new_v4()),
            name: tool_name.to_string(),
            input,
        });
    }

    // Pattern 2: <function>TOOL_NAME{JSON_BODY}</function>
    // (Groq/Llama variant — tool name immediately followed by JSON object)
    search_from = 0;
    while let Some(start) = text[search_from..].find("<function>") {
        let abs_start = search_from + start;
        let after_tag = abs_start + "<function>".len();

        // Find closing </function>
        let Some(close_offset) = text[after_tag..].find("</function>") else {
            search_from = after_tag;
            continue;
        };
        let inner = &text[after_tag..after_tag + close_offset];
        search_from = after_tag + close_offset + "</function>".len();

        // The inner content is "tool_name{json}" — find the first '{' to split
        let Some(brace_pos) = inner.find('{') else {
            continue;
        };
        let tool_name = inner[..brace_pos].trim();
        let json_body = inner[brace_pos..].trim();

        if tool_name.is_empty() {
            continue;
        }

        // Validate: tool name must be in available_tools
        if !tool_names.contains(&tool_name) {
            warn!(
                tool = tool_name,
                "Text-based tool call (variant 2) for unknown tool — skipping"
            );
            continue;
        }

        // Parse JSON input
        let input: serde_json::Value = match serde_json::from_str(json_body) {
            Ok(v) => v,
            Err(e) => {
                warn!(tool = tool_name, error = %e, "Failed to parse text-based tool call JSON (variant 2) — skipping");
                continue;
            }
        };

        // Avoid duplicates if pattern 1 already captured this call
        if calls
            .iter()
            .any(|c| c.name == tool_name && c.input == input)
        {
            continue;
        }

        info!(
            tool = tool_name,
            "Recovered text-based tool call (variant 2) → synthetic ToolUse"
        );
        calls.push(ToolCall {
            id: format!("recovered_{}", uuid::Uuid::new_v4()),
            name: tool_name.to_string(),
            input,
        });
    }

    // Pattern 3: <tool>TOOL_NAME{JSON}</tool>  (Qwen / DeepSeek variant)
    search_from = 0;
    while let Some(start) = text[search_from..].find("<tool>") {
        let abs_start = search_from + start;
        let after_tag = abs_start + "<tool>".len();

        let Some(close_offset) = text[after_tag..].find("</tool>") else {
            search_from = after_tag;
            continue;
        };
        let inner = &text[after_tag..after_tag + close_offset];
        search_from = after_tag + close_offset + "</tool>".len();

        let Some(brace_pos) = inner.find('{') else {
            continue;
        };
        let tool_name = inner[..brace_pos].trim();
        let json_body = inner[brace_pos..].trim();

        if tool_name.is_empty() || !tool_names.contains(&tool_name) {
            continue;
        }

        let input: serde_json::Value = match serde_json::from_str(json_body) {
            Ok(v) => v,
            Err(_) => continue,
        };

        if calls
            .iter()
            .any(|c| c.name == tool_name && c.input == input)
        {
            continue;
        }

        info!(
            tool = tool_name,
            "Recovered text-based tool call (<tool> variant) → synthetic ToolUse"
        );
        calls.push(ToolCall {
            id: format!("recovered_{}", uuid::Uuid::new_v4()),
            name: tool_name.to_string(),
            input,
        });
    }

    // Pattern 4: Markdown code blocks containing tool_name {JSON}
    // Matches: ```\nexec {"command":"ls"}\n``` or ```bash\nexec {"command":"ls"}\n```
    {
        let mut in_block = false;
        let mut block_content = String::new();
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("```") {
                if in_block {
                    // End of block — try to extract tool call from content
                    let content = block_content.trim();
                    if let Some(brace_pos) = content.find('{') {
                        let potential_tool = content[..brace_pos].trim();
                        if tool_names.contains(&potential_tool) {
                            if let Ok(input) = serde_json::from_str::<serde_json::Value>(
                                content[brace_pos..].trim(),
                            ) {
                                if !calls
                                    .iter()
                                    .any(|c| c.name == potential_tool && c.input == input)
                                {
                                    info!(
                                        tool = potential_tool,
                                        "Recovered tool call from markdown code block"
                                    );
                                    calls.push(ToolCall {
                                        id: format!("recovered_{}", uuid::Uuid::new_v4()),
                                        name: potential_tool.to_string(),
                                        input,
                                    });
                                }
                            }
                        }
                    }
                    block_content.clear();
                    in_block = false;
                } else {
                    in_block = true;
                    block_content.clear();
                }
            } else if in_block {
                if !block_content.is_empty() {
                    block_content.push('\n');
                }
                block_content.push_str(trimmed);
            }
        }
    }

    // Pattern 5: Backtick-wrapped tool call: `tool_name {"key":"value"}`
    {
        let parts: Vec<&str> = text.split('`').collect();
        // Every odd-indexed element is inside backticks
        for chunk in parts.iter().skip(1).step_by(2) {
            let trimmed = chunk.trim();
            if let Some(brace_pos) = trimmed.find('{') {
                let potential_tool = trimmed[..brace_pos].trim();
                if !potential_tool.is_empty()
                    && !potential_tool.contains(' ')
                    && tool_names.contains(&potential_tool)
                {
                    if let Ok(input) =
                        serde_json::from_str::<serde_json::Value>(trimmed[brace_pos..].trim())
                    {
                        if !calls
                            .iter()
                            .any(|c| c.name == potential_tool && c.input == input)
                        {
                            info!(
                                tool = potential_tool,
                                "Recovered tool call from backtick-wrapped text"
                            );
                            calls.push(ToolCall {
                                id: format!("recovered_{}", uuid::Uuid::new_v4()),
                                name: potential_tool.to_string(),
                                input,
                            });
                        }
                    }
                }
            }
        }
    }

    // Pattern 6: [TOOL_CALL]...[/TOOL_CALL] blocks (Ollama models like Qwen, issue #354)
    // Handles both JSON args and custom `{tool => "name", args => {--key "value"}}` syntax.
    search_from = 0;
    while let Some(start) = text[search_from..].find("[TOOL_CALL]") {
        let abs_start = search_from + start;
        let after_tag = abs_start + "[TOOL_CALL]".len();

        let Some(close_offset) = text[after_tag..].find("[/TOOL_CALL]") else {
            search_from = after_tag;
            continue;
        };
        let inner = text[after_tag..after_tag + close_offset].trim();
        search_from = after_tag + close_offset + "[/TOOL_CALL]".len();

        // Try standard JSON first: {"name":"tool","arguments":{...}}
        if let Some((tool_name, input)) = parse_json_tool_call_object(inner, &tool_names) {
            if !calls
                .iter()
                .any(|c| c.name == tool_name && c.input == input)
            {
                info!(
                    tool = tool_name.as_str(),
                    "Recovered tool call from [TOOL_CALL] block (JSON)"
                );
                calls.push(ToolCall {
                    id: format!("recovered_{}", uuid::Uuid::new_v4()),
                    name: tool_name,
                    input,
                });
            }
            continue;
        }

        // Custom arrow syntax: {tool => "name", args => {--key "value"}}
        if let Some((tool_name, input)) = parse_arrow_syntax_tool_call(inner, &tool_names) {
            if !calls
                .iter()
                .any(|c| c.name == tool_name && c.input == input)
            {
                info!(
                    tool = tool_name.as_str(),
                    "Recovered tool call from [TOOL_CALL] block (arrow syntax)"
                );
                calls.push(ToolCall {
                    id: format!("recovered_{}", uuid::Uuid::new_v4()),
                    name: tool_name,
                    input,
                });
            }
        }
    }

    // Pattern 7: <tool_call>JSON</tool_call> (Qwen3 models on Ollama, issue #332)
    search_from = 0;
    while let Some(start) = text[search_from..].find("<tool_call>") {
        let abs_start = search_from + start;
        let after_tag = abs_start + "<tool_call>".len();

        let Some(close_offset) = text[after_tag..].find("</tool_call>") else {
            search_from = after_tag;
            continue;
        };
        let inner = text[after_tag..after_tag + close_offset].trim();
        search_from = after_tag + close_offset + "</tool_call>".len();

        if let Some((tool_name, input)) = parse_json_tool_call_object(inner, &tool_names) {
            if !calls
                .iter()
                .any(|c| c.name == tool_name && c.input == input)
            {
                info!(
                    tool = tool_name.as_str(),
                    "Recovered tool call from <tool_call> block"
                );
                calls.push(ToolCall {
                    id: format!("recovered_{}", uuid::Uuid::new_v4()),
                    name: tool_name,
                    input,
                });
            }
        }
    }

    // Pattern 9: <function name="tool" parameters="{...}" /> — XML attribute style
    // Groq/Llama sometimes emit self-closing XML with name/parameters attributes.
    // The parameters value is HTML-entity-escaped JSON (&quot; etc.).
    {
        use regex_lite::Regex;
        // Match both self-closing <function ... /> and <function ...></function>
        let re =
            Regex::new(r#"<function\s+name="([^"]+)"\s+parameters="([^"]*)"[^/]*/?>"#).unwrap();
        for caps in re.captures_iter(text) {
            let tool_name = caps.get(1).unwrap().as_str();
            let raw_params = caps.get(2).unwrap().as_str();

            if !tool_names.contains(&tool_name) {
                warn!(
                    tool = tool_name,
                    "XML-attribute tool call for unknown tool — skipping"
                );
                continue;
            }

            // Unescape HTML entities (&quot; &amp; &lt; &gt; &apos;)
            let unescaped = raw_params
                .replace("&quot;", "\"")
                .replace("&amp;", "&")
                .replace("&lt;", "<")
                .replace("&gt;", ">")
                .replace("&apos;", "'");

            let input: serde_json::Value = match serde_json::from_str(&unescaped) {
                Ok(v) => v,
                Err(e) => {
                    warn!(tool = tool_name, error = %e, "Failed to parse XML-attribute tool call params — skipping");
                    continue;
                }
            };

            if calls
                .iter()
                .any(|c| c.name == tool_name && c.input == input)
            {
                continue;
            }

            info!(
                tool = tool_name,
                "Recovered XML-attribute tool call → synthetic ToolUse"
            );
            calls.push(ToolCall {
                id: format!("recovered_{}", uuid::Uuid::new_v4()),
                name: tool_name.to_string(),
                input,
            });
        }
    }

    // Pattern 10: <|plugin|>...<|endofblock|> (Qwen/ChatGLM thinking-model format)
    search_from = 0;
    while let Some(start) = text[search_from..].find("<|plugin|>") {
        let abs_start = search_from + start;
        let after_tag = abs_start + "<|plugin|>".len();

        let close_tag = "<|endofblock|>";
        let Some(close_offset) = text[after_tag..].find(close_tag) else {
            search_from = after_tag;
            continue;
        };
        let inner = text[after_tag..after_tag + close_offset].trim();
        search_from = after_tag + close_offset + close_tag.len();

        if let Some((tool_name, input)) = parse_json_tool_call_object(inner, &tool_names) {
            if !calls
                .iter()
                .any(|c| c.name == tool_name && c.input == input)
            {
                info!(
                    tool = tool_name.as_str(),
                    "Recovered tool call from <|plugin|> block"
                );
                calls.push(ToolCall {
                    id: format!("recovered_{}", uuid::Uuid::new_v4()),
                    name: tool_name,
                    input,
                });
            }
        }
    }

    // Pattern 11: Action: tool_name\nAction Input: {JSON} (ReAct-style, LM Studio / GPT-OSS)
    {
        let lines: Vec<&str> = text.lines().collect();
        let mut i = 0;
        while i < lines.len() {
            let line = lines[i].trim();
            if let Some(tool_part) = line
                .strip_prefix("Action:")
                .or_else(|| line.strip_prefix("action:"))
            {
                let tool_name = tool_part.trim();
                if tool_names.contains(&tool_name) {
                    // Look for "Action Input:" on the next line(s)
                    if i + 1 < lines.len() {
                        let next = lines[i + 1].trim();
                        if let Some(json_part) = next
                            .strip_prefix("Action Input:")
                            .or_else(|| next.strip_prefix("action input:"))
                            .or_else(|| next.strip_prefix("action_input:"))
                        {
                            let json_str = json_part.trim();
                            if let Ok(input) = serde_json::from_str::<serde_json::Value>(json_str) {
                                if !calls
                                    .iter()
                                    .any(|c| c.name == tool_name && c.input == input)
                                {
                                    info!(
                                        tool = tool_name,
                                        "Recovered tool call from Action/Action Input pattern"
                                    );
                                    calls.push(ToolCall {
                                        id: format!("recovered_{}", uuid::Uuid::new_v4()),
                                        name: tool_name.to_string(),
                                        input,
                                    });
                                }
                            }
                            i += 2;
                            continue;
                        }
                    }
                }
            }
            i += 1;
        }
    }

    // Pattern 12: tool_name\n{"key":"value"} — bare name + JSON on next line (Llama 4 Scout)
    {
        let lines: Vec<&str> = text.lines().collect();
        for i in 0..lines.len().saturating_sub(1) {
            let name_line = lines[i].trim();
            // Tool name must be a single word matching a known tool
            if name_line.contains(' ') || name_line.contains('{') || name_line.is_empty() {
                continue;
            }
            if !tool_names.contains(&name_line) {
                continue;
            }
            // Next line must be valid JSON
            let json_line = lines[i + 1].trim();
            if !json_line.starts_with('{') {
                continue;
            }
            if let Ok(input) = serde_json::from_str::<serde_json::Value>(json_line) {
                if !calls
                    .iter()
                    .any(|c| c.name == name_line && c.input == input)
                {
                    info!(
                        tool = name_line,
                        "Recovered tool call from name+JSON line pair"
                    );
                    calls.push(ToolCall {
                        id: format!("recovered_{}", uuid::Uuid::new_v4()),
                        name: name_line.to_string(),
                        input,
                    });
                }
            }
        }
    }

    // Pattern 13: <tool_use>JSON</tool_use> (Llama 3.1+ variant)
    search_from = 0;
    while let Some(start) = text[search_from..].find("<tool_use>") {
        let abs_start = search_from + start;
        let after_tag = abs_start + "<tool_use>".len();

        let Some(close_offset) = text[after_tag..].find("</tool_use>") else {
            search_from = after_tag;
            continue;
        };
        let inner = text[after_tag..after_tag + close_offset].trim();
        search_from = after_tag + close_offset + "</tool_use>".len();

        if let Some((tool_name, input)) = parse_json_tool_call_object(inner, &tool_names) {
            if !calls
                .iter()
                .any(|c| c.name == tool_name && c.input == input)
            {
                info!(
                    tool = tool_name.as_str(),
                    "Recovered tool call from <tool_use> block"
                );
                calls.push(ToolCall {
                    id: format!("recovered_{}", uuid::Uuid::new_v4()),
                    name: tool_name,
                    input,
                });
            }
        }
    }

    // Pattern 8: Bare JSON tool call objects in text (common Ollama fallback)
    // Matches: {"name":"tool_name","arguments":{"key":"value"}} not already inside tags
    // Only try this if no calls were found by tag-based patterns, to avoid false positives.
    if calls.is_empty() {
        // Scan for JSON objects that look like tool calls
        let mut scan_from = 0;
        while let Some(brace_start) = text[scan_from..].find('{') {
            let abs_brace = scan_from + brace_start;
            // Try to parse a JSON object starting here
            if let Some((tool_name, input)) =
                try_parse_bare_json_tool_call(&text[abs_brace..], &tool_names)
            {
                if !calls
                    .iter()
                    .any(|c| c.name == tool_name && c.input == input)
                {
                    info!(
                        tool = tool_name.as_str(),
                        "Recovered tool call from bare JSON object in text"
                    );
                    calls.push(ToolCall {
                        id: format!("recovered_{}", uuid::Uuid::new_v4()),
                        name: tool_name,
                        input,
                    });
                }
            }
            scan_from = abs_brace + 1;
        }
    }

    calls
}

/// Parse a JSON object that represents a tool call.
/// Supports formats:
/// - `{"name":"tool","arguments":{"key":"value"}}`
/// - `{"name":"tool","parameters":{"key":"value"}}`
/// - `{"function":"tool","arguments":{"key":"value"}}`
/// - `{"tool":"tool_name","args":{"key":"value"}}`
fn parse_json_tool_call_object(
    text: &str,
    tool_names: &[&str],
) -> Option<(String, serde_json::Value)> {
    let obj: serde_json::Value = serde_json::from_str(text).ok()?;
    let obj = obj.as_object()?;

    // Extract tool name from various field names
    let name = obj
        .get("name")
        .or_else(|| obj.get("function"))
        .or_else(|| obj.get("tool"))
        .and_then(|v| v.as_str())?;

    if !tool_names.contains(&name) {
        return None;
    }

    // Extract arguments from various field names
    let args = obj
        .get("arguments")
        .or_else(|| obj.get("parameters"))
        .or_else(|| obj.get("args"))
        .or_else(|| obj.get("input"))
        .cloned()
        .unwrap_or(serde_json::json!({}));

    // If arguments is a string (some models stringify it), try to parse it
    let args = if let Some(s) = args.as_str() {
        serde_json::from_str(s).unwrap_or(serde_json::json!({}))
    } else {
        args
    };

    Some((name.to_string(), args))
}

fn unescape_xml_entities(text: &str) -> String {
    text.replace("&quot;", "\"")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&apos;", "'")
}

fn parse_xml_parameter_blocks(text: &str) -> Option<serde_json::Value> {
    use regex_lite::Regex;

    let re = Regex::new(r#"(?s)<parameter=([A-Za-z0-9_.:-]+)>\s*(.*?)\s*</parameter>"#).unwrap();
    let mut params = serde_json::Map::new();

    for caps in re.captures_iter(text) {
        let Some(name) = caps.get(1).map(|m| m.as_str().trim()) else {
            continue;
        };
        if name.is_empty() {
            continue;
        }

        let raw_value = caps.get(2).map(|m| m.as_str()).unwrap_or_default();
        let value_text = unescape_xml_entities(raw_value).trim().to_string();
        let value =
            serde_json::from_str(&value_text).unwrap_or(serde_json::Value::String(value_text));
        params.insert(name.to_string(), value);
    }

    if params.is_empty() {
        None
    } else {
        Some(serde_json::Value::Object(params))
    }
}

/// Parse the custom arrow syntax used by some Ollama models:
/// `{tool => "name", args => {--key "value"}}` or `{tool => "name", args => {"key":"value"}}`
fn parse_arrow_syntax_tool_call(
    text: &str,
    tool_names: &[&str],
) -> Option<(String, serde_json::Value)> {
    // Extract tool name: look for `tool => "name"` or `tool=>"name"`
    let tool_marker_pos = text.find("tool")?;
    let after_tool = &text[tool_marker_pos + 4..];
    // Skip whitespace and `=>`
    let after_arrow = after_tool.trim_start();
    let after_arrow = after_arrow.strip_prefix("=>")?;
    let after_arrow = after_arrow.trim_start();

    // Extract quoted tool name
    let tool_name = if let Some(stripped) = after_arrow.strip_prefix('"') {
        let end_quote = stripped.find('"')?;
        &stripped[..end_quote]
    } else {
        // Unquoted: take until comma, whitespace, or '}'
        let end = after_arrow
            .find(|c: char| c == ',' || c == '}' || c.is_whitespace())
            .unwrap_or(after_arrow.len());
        &after_arrow[..end]
    };

    if tool_name.is_empty() || !tool_names.contains(&tool_name) {
        return None;
    }

    // Extract args: look for `args => {` or `args=>{`
    let args_value = if let Some(args_pos) = text.find("args") {
        let after_args = &text[args_pos + 4..];
        let after_args = after_args.trim_start();
        let after_args = after_args.strip_prefix("=>")?;
        let after_args = after_args.trim_start();

        if after_args.starts_with('{') {
            // Try standard JSON parse first
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(after_args) {
                v
            } else {
                // Parse `--key "value"` / `--key value` style args
                parse_dash_dash_args(after_args)
            }
        } else {
            serde_json::json!({})
        }
    } else {
        serde_json::json!({})
    };

    Some((tool_name.to_string(), args_value))
}

/// Parse `{--key "value", --flag}` or `{--command "ls -F /"}` style arguments
/// into a JSON object.
fn parse_dash_dash_args(text: &str) -> serde_json::Value {
    let mut map = serde_json::Map::new();

    // Strip outer braces — find matching close brace
    let inner = if text.starts_with('{') {
        let mut depth = 0;
        let mut end = text.len();
        for (i, c) in text.char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = i;
                        break;
                    }
                }
                _ => {}
            }
        }
        text[1..end].trim()
    } else {
        text.trim()
    };

    // Parse --key "value" or --key value pairs
    let mut remaining = inner;
    while let Some(dash_pos) = remaining.find("--") {
        remaining = &remaining[dash_pos + 2..];

        // Extract key: runs until whitespace, '=', '"', or end
        let key_end = remaining
            .find(|c: char| c.is_whitespace() || c == '=' || c == '"')
            .unwrap_or(remaining.len());
        let key = &remaining[..key_end];
        if key.is_empty() {
            continue;
        }
        remaining = &remaining[key_end..];
        remaining = remaining.trim_start();

        // Skip optional '='
        if remaining.starts_with('=') {
            remaining = remaining[1..].trim_start();
        }

        // Extract value
        if remaining.starts_with('"') {
            // Quoted value — find closing quote
            if let Some(end_quote) = remaining[1..].find('"') {
                let value = &remaining[1..1 + end_quote];
                map.insert(
                    key.to_string(),
                    serde_json::Value::String(value.to_string()),
                );
                remaining = &remaining[2 + end_quote..];
            } else {
                // Unclosed quote — take rest
                let value = &remaining[1..];
                map.insert(
                    key.to_string(),
                    serde_json::Value::String(value.to_string()),
                );
                break;
            }
        } else {
            // Unquoted value — take until next --, comma, }, or end
            let val_end = remaining
                .find([',', '}'])
                .or_else(|| remaining.find("--"))
                .unwrap_or(remaining.len());
            let value = remaining[..val_end].trim();
            if !value.is_empty() {
                map.insert(
                    key.to_string(),
                    serde_json::Value::String(value.to_string()),
                );
            } else {
                // Flag with no value — set to true
                map.insert(key.to_string(), serde_json::Value::Bool(true));
            }
            remaining = &remaining[val_end..];
        }

        // Skip comma separator
        remaining = remaining.trim_start();
        if remaining.starts_with(',') {
            remaining = remaining[1..].trim_start();
        }
    }

    serde_json::Value::Object(map)
}

/// Try to parse a bare JSON object as a tool call.
/// The JSON must have a "name"/"function"/"tool" field matching a known tool.
fn try_parse_bare_json_tool_call(
    text: &str,
    tool_names: &[&str],
) -> Option<(String, serde_json::Value)> {
    // Find the end of this JSON object by counting braces
    let mut depth = 0;
    let mut end = 0;
    for (i, c) in text.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = i + 1;
                    break;
                }
            }
            _ => {}
        }
    }
    if end == 0 {
        return None;
    }

    parse_json_tool_call_object(&text[..end], tool_names)
}

/// Deduplicate tool calls from the response.
/// Returns a reference to the deduplicated tool calls.
pub fn deduplicate_tool_calls(response: &crate::llm_driver::CompletionResponse) -> Vec<&ToolCall> {
    let mut hash_set = std::collections::HashSet::new();
    let mut deduplicated = Vec::new();
    for tool_call in &response.tool_calls {
        let hash = LoopGuard::compute_hash(&tool_call.name, &tool_call.input);
        if hash_set.insert(hash) {
            deduplicated.push(tool_call);
        }
    }
    deduplicated
}

/// Normalize per-turn tool names via [`ainl_graph_extractor::tag_tool_names`] for stable graph
/// episode storage: plain lowercase slugs (for example `bash`), deduped in first-seen order.
/// Does not use namespaced debug strings such as `tool:bash` — only each tag's `value` string.
fn canonicalize_turn_tool_names_for_graph_storage(raw: &[String]) -> Vec<String> {
    #[cfg(feature = "ainl-extractor")]
    {
        use ainl_graph_extractor::{tag_tool_names, TagNamespace};
        let tags = tag_tool_names(raw);
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for t in tags {
            if t.namespace != TagNamespace::Tool {
                continue;
            }
            if seen.insert(t.value.clone()) {
                out.push(t.value);
            }
        }
        out
    }
    #[cfg(not(feature = "ainl-extractor"))]
    {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for s in raw {
            let n = s.trim().to_ascii_lowercase();
            if n.is_empty() {
                continue;
            }
            if seen.insert(n.clone()) {
                out.push(n);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm_driver::{CompletionResponse, LlmError};
    use async_trait::async_trait;
    use openfang_types::runtime_limits::EffectiveRuntimeLimits;
    use openfang_types::tool::ToolCall;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn runtime_env_lock() -> &'static tokio::sync::Mutex<()> {
        crate::runtime_env_test_lock()
    }

    #[test]
    #[cfg(feature = "ainl-runtime-engine")]
    fn test_ainl_runtime_engine_switch_matrix_manifest_and_env() {
        assert!(ainl_runtime_engine_switch_active_with_env(
            true, None, false
        ));
        assert!(ainl_runtime_engine_switch_active_with_env(
            false,
            Some("1"),
            false
        ));
        assert!(!ainl_runtime_engine_switch_active_with_env(
            false,
            Some("true"),
            false
        ));
        assert!(!ainl_runtime_engine_switch_active_with_env(
            false, None, false
        ));
    }

    #[test]
    #[cfg(feature = "ainl-runtime-engine")]
    fn test_ainl_runtime_engine_switch_kill_switch_wins() {
        assert!(!ainl_runtime_engine_switch_active_with_env(
            true,
            Some("1"),
            true
        ));
        assert!(!ainl_runtime_engine_switch_active_with_env(
            false,
            Some("1"),
            true
        ));
    }

    #[test]
    #[cfg(feature = "ainl-extractor")]
    fn canonicalize_turn_tool_names_shell_aliases_to_single_bash() {
        let raw = vec!["bash".into(), "Bash".into(), "shell".into(), "sh".into()];
        assert_eq!(
            canonicalize_turn_tool_names_for_graph_storage(&raw),
            vec!["bash".to_string()]
        );
    }

    #[test]
    #[cfg(feature = "ainl-extractor")]
    fn canonicalize_turn_tool_names_python_and_python3_same_key() {
        assert_eq!(
            canonicalize_turn_tool_names_for_graph_storage(&["python".into(), "python3".into()]),
            vec!["python_repl".to_string()]
        );
        assert_eq!(
            canonicalize_turn_tool_names_for_graph_storage(&["python3".into(), "python".into()]),
            vec!["python_repl".to_string()]
        );
    }

    #[test]
    #[cfg(feature = "ainl-extractor")]
    fn canonicalize_turn_tool_names_distinct_tools_preserved() {
        let raw = vec!["file_read".into(), "search_web".into()];
        assert_eq!(
            canonicalize_turn_tool_names_for_graph_storage(&raw),
            vec!["file_read".to_string(), "search_web".to_string()]
        );
    }

    #[test]
    fn canonicalize_turn_tool_names_empty_ok() {
        assert!(canonicalize_turn_tool_names_for_graph_storage(&[]).is_empty());
        assert!(
            canonicalize_turn_tool_names_for_graph_storage(&["".into(), "  ".into()]).is_empty()
        );
    }

    #[test]
    fn test_max_iterations_default_limits() {
        assert_eq!(EffectiveRuntimeLimits::legacy_defaults().max_iterations, 80);
    }

    #[test]
    fn test_retry_constants() {
        assert_eq!(MAX_RETRIES, 3);
        assert_eq!(BASE_RETRY_DELAY_MS, 1000);
    }

    #[test]
    fn test_dynamic_truncate_short_unchanged() {
        use crate::context_budget::{truncate_tool_result_dynamic, ContextBudget};
        let budget = ContextBudget::new(200_000);
        let short = "Hello, world!";
        assert_eq!(truncate_tool_result_dynamic(short, &budget), short);
    }

    #[test]
    fn test_dynamic_truncate_over_limit() {
        use crate::context_budget::{truncate_tool_result_dynamic, ContextBudget};
        let budget = ContextBudget::new(200_000);
        let long = "x".repeat(budget.per_result_cap() + 10_000);
        let result = truncate_tool_result_dynamic(&long, &budget);
        assert!(result.len() <= budget.per_result_cap() + 200);
        assert!(result.contains("[TRUNCATED:"));
    }

    #[test]
    fn test_dynamic_truncate_newline_boundary() {
        use crate::context_budget::{truncate_tool_result_dynamic, ContextBudget};
        // Small budget to force truncation
        let budget = ContextBudget::new(1_000);
        let content = (0..200)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let result = truncate_tool_result_dynamic(&content, &budget);
        // Should break at a newline, not mid-line
        let before_marker = result.split("[TRUNCATED:").next().unwrap();
        let trimmed = before_marker.trim_end();
        assert!(!trimmed.is_empty());
    }

    #[test]
    fn test_max_continuations_default_limits() {
        assert_eq!(
            EffectiveRuntimeLimits::legacy_defaults().max_continuations,
            5
        );
    }

    #[test]
    fn test_tool_timeout_constant() {
        assert_eq!(TOOL_TIMEOUT_SECS, 300);
        assert_eq!(AGENT_TOOL_TIMEOUT_SECS, 600);
    }

    #[test]
    fn test_tool_timeout_for_agent_tools() {
        assert_eq!(tool_timeout_for("agent_send"), Duration::from_secs(600));
        assert_eq!(tool_timeout_for("agent_spawn"), Duration::from_secs(600));
        assert_eq!(
            tool_timeout_for("document_extract"),
            Duration::from_secs(180)
        );
        assert_eq!(
            tool_timeout_for("spreadsheet_build"),
            Duration::from_secs(180)
        );
        // Media generation / external AI APIs
        assert_eq!(tool_timeout_for("image_generate"), Duration::from_secs(300));
        assert_eq!(tool_timeout_for("text_to_speech"), Duration::from_secs(300));
        assert_eq!(tool_timeout_for("media_describe"), Duration::from_secs(300));
        assert_eq!(tool_timeout_for("a2a_send"), Duration::from_secs(300));
        // Persistent process tools
        assert_eq!(tool_timeout_for("process_start"), Duration::from_secs(30));
        assert_eq!(tool_timeout_for("process_poll"), Duration::from_secs(30));
        // Standard tools use TOOL_TIMEOUT_SECS (300s)
        assert_eq!(tool_timeout_for("file_read"), Duration::from_secs(300));
        assert_eq!(tool_timeout_for("shell_exec"), Duration::from_secs(300));
    }

    #[test]
    fn test_max_history_messages_default_limits() {
        assert_eq!(
            EffectiveRuntimeLimits::legacy_defaults().max_history_messages,
            60
        );
    }

    #[test]
    fn scheduling_phantom_triggers_without_schedule_tools() {
        let tools = std::collections::HashSet::new();
        assert!(scheduling_phantom_detected(
            "Please schedule my ainl to run daily at 9am",
            "Done — I've scheduled it and it will run every morning.",
            &tools,
        ));
    }

    #[test]
    fn scheduling_phantom_suppressed_when_schedule_tool_ran() {
        let mut tools = std::collections::HashSet::new();
        tools.insert("schedule_create".to_string());
        assert!(!scheduling_phantom_detected(
            "Please schedule my ainl to run daily",
            "All set.",
            &tools,
        ));
    }

    #[test]
    fn scheduling_phantom_not_triggered_without_user_intent() {
        let tools = std::collections::HashSet::new();
        assert!(!scheduling_phantom_detected(
            "What is cron?",
            "Cron is a Unix job scheduler.",
            &tools,
        ));
    }

    // --- Integration tests for empty response guards ---

    fn test_manifest() -> AgentManifest {
        let mut m = AgentManifest {
            name: "test-agent".to_string(),
            model: openfang_types::agent::ModelConfig {
                system_prompt: "You are a test agent.".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        // Default agent-loop tests to the legacy LLM path; opt in explicitly when needed.
        m.ainl_runtime_engine = false;
        m.metadata.insert(
            crate::prompt_builder::KERNEL_EXPANDED_SYSTEM_PROMPT_META_KEY.to_string(),
            serde_json::Value::Bool(true),
        );
        m
    }

    #[cfg(feature = "ainl-runtime-engine")]
    #[tokio::test]
    async fn test_agent_loop_uses_openfang_by_default() {
        let _guard = runtime_env_lock().lock().await;
        use std::sync::atomic::AtomicBool;
        use std::sync::Arc;

        crate::ainl_runtime_bridge::test_hooks::reset_bridge_new_count();
        static DRIVER_USED: AtomicBool = AtomicBool::new(false);

        #[derive(Clone)]
        struct CountingDriver;

        #[async_trait]
        impl LlmDriver for CountingDriver {
            async fn complete(
                &self,
                _request: CompletionRequest,
            ) -> Result<CompletionResponse, LlmError> {
                DRIVER_USED.store(true, Ordering::SeqCst);
                Ok(CompletionResponse {
                    content: vec![ContentBlock::Text {
                        text: "ok".to_string(),
                        provider_metadata: None,
                    }],
                    stop_reason: StopReason::EndTurn,
                    tool_calls: vec![],
                    usage: TokenUsage::default(),
                    vitals: None,
                })
            }
        }

        let memory = openfang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
        let agent_id = openfang_types::agent::AgentId::new();
        let mut session = openfang_memory::session::Session {
            id: openfang_types::agent::SessionId::new(),
            agent_id,
            messages: Vec::new(),
            context_window_tokens: 0,
            label: None,
        };
        let mut manifest = test_manifest();
        manifest.ainl_runtime_engine = false;
        std::env::remove_var("AINL_RUNTIME_ENGINE");
        let driver: Arc<dyn LlmDriver> = Arc::new(CountingDriver);

        run_agent_loop(
            &manifest,
            "ping",
            &mut session,
            &memory,
            driver,
            &[],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            EffectiveRuntimeLimits::legacy_defaults(),
            None,
            None,
        )
        .await
        .expect("loop");

        assert_eq!(
            crate::ainl_runtime_bridge::test_hooks::bridge_new_count(),
            0
        );
        assert!(DRIVER_USED.load(Ordering::SeqCst));
    }

    #[cfg(feature = "ainl-runtime-engine")]
    #[tokio::test]
    async fn test_ainl_runtime_bridge_cache_reuses_instance() {
        let _guard = runtime_env_lock().lock().await;
        crate::ainl_runtime_bridge::test_hooks::reset_bridge_new_count();
        ainl_runtime_bridge_cache_clear_for_tests();

        let dir = tempfile::tempdir().expect("tempdir");
        let prev = std::env::var("ARMARAOS_HOME").ok();
        std::env::set_var("ARMARAOS_HOME", dir.path().as_os_str());

        let agent = format!("bridge-cache-{}", uuid::Uuid::new_v4());
        let writer = crate::graph_memory_writer::GraphMemoryWriter::open(&agent).expect("writer");
        let depth = 7u32;
        let b1 = get_or_create_ainl_runtime_bridge(&agent, &writer, depth).expect("bridge 1");
        let b2 = get_or_create_ainl_runtime_bridge(&agent, &writer, depth).expect("bridge 2");

        assert!(std::sync::Arc::ptr_eq(&b1, &b2));
        assert_eq!(
            crate::ainl_runtime_bridge::test_hooks::bridge_new_count(),
            1
        );
        let (hits, misses, construct_failures, run_failures) =
            ainl_runtime_bridge_cache_metrics_snapshot();
        assert_eq!(hits, 1);
        assert_eq!(misses, 1);
        assert_eq!(construct_failures, 0);
        assert_eq!(run_failures, 0);

        if let Some(p) = prev {
            std::env::set_var("ARMARAOS_HOME", p);
        } else {
            std::env::remove_var("ARMARAOS_HOME");
        }
    }

    /// Mock driver that simulates: first call returns ToolUse with no text,
    /// second call returns EndTurn with empty text. This reproduces the bug
    /// where the LLM ends with no text after a tool-use cycle.
    struct EmptyAfterToolUseDriver {
        call_count: AtomicU32,
    }

    impl EmptyAfterToolUseDriver {
        fn new() -> Self {
            Self {
                call_count: AtomicU32::new(0),
            }
        }
    }

    #[async_trait]
    impl LlmDriver for EmptyAfterToolUseDriver {
        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            let call = self.call_count.fetch_add(1, Ordering::Relaxed);
            if call == 0 {
                // First call: LLM wants to use a tool (with no text block)
                Ok(CompletionResponse {
                    content: vec![ContentBlock::ToolUse {
                        id: "tool_1".to_string(),
                        name: "fake_tool".to_string(),
                        input: serde_json::json!({"query": "test"}),
                        provider_metadata: None,
                    }],
                    stop_reason: StopReason::ToolUse,
                    tool_calls: vec![ToolCall {
                        id: "tool_1".to_string(),
                        name: "fake_tool".to_string(),
                        input: serde_json::json!({"query": "test"}),
                    }],
                    usage: TokenUsage {
                        input_tokens: 10,
                        output_tokens: 5,
                        ..Default::default()
                    },
                    vitals: None,
                })
            } else {
                // Second call: LLM returns EndTurn with EMPTY text (the bug)
                Ok(CompletionResponse {
                    content: vec![],
                    stop_reason: StopReason::EndTurn,
                    tool_calls: vec![],
                    usage: TokenUsage {
                        input_tokens: 10,
                        output_tokens: 0,
                        ..Default::default()
                    },
                    vitals: None,
                })
            }
        }
    }

    /// Mock driver that returns empty text with MaxTokens stop reason,
    /// repeated MAX_CONTINUATIONS times to trigger the max continuations path.
    struct EmptyMaxTokensDriver;

    #[async_trait]
    impl LlmDriver for EmptyMaxTokensDriver {
        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            Ok(CompletionResponse {
                content: vec![],
                stop_reason: StopReason::MaxTokens,
                tool_calls: vec![],
                usage: TokenUsage {
                    input_tokens: 10,
                    output_tokens: 0,
                    ..Default::default()
                },
                vitals: None,
            })
        }
    }

    /// Mock driver that returns normal text (sanity check).
    struct NormalDriver;

    #[async_trait]
    impl LlmDriver for NormalDriver {
        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            Ok(CompletionResponse {
                content: vec![ContentBlock::Text {
                    text: "Hello from the agent!".to_string(),
                    provider_metadata: None,
                }],
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
                usage: TokenUsage {
                    input_tokens: 10,
                    output_tokens: 8,
                    ..Default::default()
                },
                vitals: None,
            })
        }
    }

    #[tokio::test]
    async fn test_empty_response_after_tool_use_returns_fallback() {
        let memory = openfang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
        let agent_id = openfang_types::agent::AgentId::new();
        let mut session = openfang_memory::session::Session {
            id: openfang_types::agent::SessionId::new(),
            agent_id,
            messages: Vec::new(),
            context_window_tokens: 0,
            label: None,
        };
        let manifest = test_manifest();
        let driver: Arc<dyn LlmDriver> = Arc::new(EmptyAfterToolUseDriver::new());

        let result = run_agent_loop(
            &manifest,
            "Do something with tools",
            &mut session,
            &memory,
            driver,
            &[], // no tools registered — the tool call will fail, which is fine
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None, // ainl_library_root
            None, // on_phase
            None, // media_engine
            None, // tts_engine
            None, // docker_config
            None, // hooks
            None, // context_window_tokens
            None, // process_manager
            None, // user_content_blocks
            None, // btw_rx
            None, // redirect_rx
            EffectiveRuntimeLimits::legacy_defaults(),
            None, // orchestration_ctx
            None, // orchestration_live
        )
        .await
        .expect("Loop should complete without error");

        // The response MUST NOT be empty — it should contain our fallback text
        assert!(
            !result.response.trim().is_empty(),
            "Response should not be empty after tool use, got: {:?}",
            result.response
        );
        assert!(
            result.response.contains("Task completed"),
            "Expected fallback message, got: {:?}",
            result.response
        );
    }

    #[tokio::test]
    async fn test_tool_error_injects_error_guidance() {
        let memory = openfang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
        let agent_id = openfang_types::agent::AgentId::new();
        let mut session = openfang_memory::session::Session {
            id: openfang_types::agent::SessionId::new(),
            agent_id,
            messages: Vec::new(),
            context_window_tokens: 0,
            label: None,
        };
        let manifest = test_manifest();
        let driver: Arc<dyn LlmDriver> = Arc::new(EmptyAfterToolUseDriver::new());

        run_agent_loop(
            &manifest,
            "Do something with tools",
            &mut session,
            &memory,
            driver,
            &[], // no tools registered — the tool call will fail, which is fine
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None, // ainl_library_root
            None, // on_phase
            None, // media_engine
            None, // tts_engine
            None, // docker_config
            None, // hooks
            None, // context_window_tokens
            None, // process_manager
            None, // user_content_blocks
            None, // btw_rx
            None, // redirect_rx
            EffectiveRuntimeLimits::legacy_defaults(),
            None, // orchestration_ctx
            None, // orchestration_live
        )
        .await
        .expect("Loop should complete without error");

        // On the first tool error, the tracker should inject a [System: ...] guidance block.
        let guidance_seen = session.messages.iter().any(|msg| {
            match &msg.content {
            MessageContent::Blocks(blocks) => blocks.iter().any(|block| {
                matches!(block, ContentBlock::Text { text, .. } if text.starts_with("[System:"))
            }),
            _ => false,
        }
        });

        assert!(
            guidance_seen,
            "Expected [System: ...] error guidance in session messages after first failed tool call"
        );
    }

    #[tokio::test]
    async fn test_empty_response_max_tokens_returns_fallback() {
        let memory = openfang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
        let agent_id = openfang_types::agent::AgentId::new();
        let mut session = openfang_memory::session::Session {
            id: openfang_types::agent::SessionId::new(),
            agent_id,
            messages: Vec::new(),
            context_window_tokens: 0,
            label: None,
        };
        let manifest = test_manifest();
        let driver: Arc<dyn LlmDriver> = Arc::new(EmptyMaxTokensDriver);

        let result = run_agent_loop(
            &manifest,
            "Tell me something long",
            &mut session,
            &memory,
            driver,
            &[],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None, // ainl_library_root
            None, // on_phase
            None, // media_engine
            None, // tts_engine
            None, // docker_config
            None, // hooks
            None, // context_window_tokens
            None, // process_manager
            None, // user_content_blocks
            None, // btw_rx
            None, // redirect_rx
            EffectiveRuntimeLimits::legacy_defaults(),
            None, // orchestration_ctx
            None, // orchestration_live
        )
        .await
        .expect("Loop should complete without error");

        // Should hit MAX_CONTINUATIONS and return fallback instead of empty
        assert!(
            !result.response.trim().is_empty(),
            "Response should not be empty on max tokens, got: {:?}",
            result.response
        );
        assert!(
            result.response.contains("token limit"),
            "Expected max-tokens fallback message, got: {:?}",
            result.response
        );
    }

    #[tokio::test]
    async fn test_normal_response_not_replaced_by_fallback() {
        let memory = openfang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
        let agent_id = openfang_types::agent::AgentId::new();
        let mut session = openfang_memory::session::Session {
            id: openfang_types::agent::SessionId::new(),
            agent_id,
            messages: Vec::new(),
            context_window_tokens: 0,
            label: None,
        };
        let manifest = test_manifest();
        let driver: Arc<dyn LlmDriver> = Arc::new(NormalDriver);

        let result = run_agent_loop(
            &manifest,
            "Say hello",
            &mut session,
            &memory,
            driver,
            &[],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None, // ainl_library_root
            None, // on_phase
            None, // media_engine
            None, // tts_engine
            None, // docker_config
            None, // hooks
            None, // context_window_tokens
            None, // process_manager
            None, // user_content_blocks
            None, // btw_rx
            None, // redirect_rx
            EffectiveRuntimeLimits::legacy_defaults(),
            None, // orchestration_ctx
            None, // orchestration_live
        )
        .await
        .expect("Loop should complete without error");

        // Normal response should pass through unchanged
        assert_eq!(result.response, "Hello from the agent!");
    }

    #[tokio::test]
    async fn test_streaming_empty_response_after_tool_use_returns_fallback() {
        let memory = openfang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
        let agent_id = openfang_types::agent::AgentId::new();
        let mut session = openfang_memory::session::Session {
            id: openfang_types::agent::SessionId::new(),
            agent_id,
            messages: Vec::new(),
            context_window_tokens: 0,
            label: None,
        };
        let manifest = test_manifest();
        let driver: Arc<dyn LlmDriver> = Arc::new(EmptyAfterToolUseDriver::new());
        let (tx, _rx) = mpsc::channel(64);

        let result = run_agent_loop_streaming(
            &manifest,
            "Do something with tools",
            &mut session,
            &memory,
            driver,
            &[],
            None,
            tx,
            None,
            None,
            None,
            None,
            None,
            None,
            None, // ainl_library_root
            None, // on_phase
            None, // media_engine
            None, // tts_engine
            None, // docker_config
            None, // hooks
            None, // context_window_tokens
            None, // process_manager
            None, // user_content_blocks
            None, // btw_rx
            None, // redirect_rx
            EffectiveRuntimeLimits::legacy_defaults(),
            None, // orchestration_ctx
            None, // orchestration_live
        )
        .await
        .expect("Streaming loop should complete without error");

        assert!(
            !result.response.trim().is_empty(),
            "Streaming response should not be empty after tool use, got: {:?}",
            result.response
        );
        assert!(
            result.response.contains("Task completed"),
            "Expected fallback message in streaming, got: {:?}",
            result.response
        );
    }

    /// Mock driver that returns empty text on first call (EndTurn), then normal text on second.
    /// This tests the one-shot retry logic for iteration 0 empty responses.
    struct EmptyThenNormalDriver {
        call_count: AtomicU32,
    }

    impl EmptyThenNormalDriver {
        fn new() -> Self {
            Self {
                call_count: AtomicU32::new(0),
            }
        }
    }

    #[async_trait]
    impl LlmDriver for EmptyThenNormalDriver {
        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            let call = self.call_count.fetch_add(1, Ordering::Relaxed);
            if call == 0 {
                // First call: empty EndTurn (triggers retry)
                Ok(CompletionResponse {
                    content: vec![],
                    stop_reason: StopReason::EndTurn,
                    tool_calls: vec![],
                    usage: TokenUsage {
                        input_tokens: 10,
                        output_tokens: 0,
                        ..Default::default()
                    },
                    vitals: None,
                })
            } else {
                // Second call (retry): normal response
                Ok(CompletionResponse {
                    content: vec![ContentBlock::Text {
                        text: "Recovered after retry!".to_string(),
                        provider_metadata: None,
                    }],
                    stop_reason: StopReason::EndTurn,
                    tool_calls: vec![],
                    usage: TokenUsage {
                        input_tokens: 15,
                        output_tokens: 8,
                        ..Default::default()
                    },
                    vitals: None,
                })
            }
        }
    }

    /// Mock driver that always returns empty EndTurn (no recovery on retry).
    /// Tests that the fallback message appears when retry also fails.
    struct AlwaysEmptyDriver;

    #[async_trait]
    impl LlmDriver for AlwaysEmptyDriver {
        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            Ok(CompletionResponse {
                content: vec![],
                stop_reason: StopReason::EndTurn,
                tool_calls: vec![],
                usage: TokenUsage {
                    input_tokens: 10,
                    output_tokens: 0,
                    ..Default::default()
                },
                vitals: None,
            })
        }
    }

    #[tokio::test]
    async fn test_empty_first_response_retries_and_recovers() {
        let memory = openfang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
        let agent_id = openfang_types::agent::AgentId::new();
        let mut session = openfang_memory::session::Session {
            id: openfang_types::agent::SessionId::new(),
            agent_id,
            messages: Vec::new(),
            context_window_tokens: 0,
            label: None,
        };
        let manifest = test_manifest();
        let driver: Arc<dyn LlmDriver> = Arc::new(EmptyThenNormalDriver::new());

        let result = run_agent_loop(
            &manifest,
            "Hello",
            &mut session,
            &memory,
            driver,
            &[],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None, // ainl_library_root
            None,
            None,
            None,
            None,
            None,
            None, // context_window_tokens
            None, // process_manager
            None, // user_content_blocks
            None, // btw_rx
            None, // redirect_rx
            EffectiveRuntimeLimits::legacy_defaults(),
            None, // orchestration_ctx
            None, // orchestration_live
        )
        .await
        .expect("Loop should recover via retry");

        assert_eq!(result.response, "Recovered after retry!");
        assert_eq!(
            result.iterations, 2,
            "Should have taken 2 iterations (retry)"
        );
    }

    #[tokio::test]
    async fn test_empty_first_response_fallback_when_retry_also_empty() {
        let memory = openfang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
        let agent_id = openfang_types::agent::AgentId::new();
        let mut session = openfang_memory::session::Session {
            id: openfang_types::agent::SessionId::new(),
            agent_id,
            messages: Vec::new(),
            context_window_tokens: 0,
            label: None,
        };
        let manifest = test_manifest();
        let driver: Arc<dyn LlmDriver> = Arc::new(AlwaysEmptyDriver);

        let result = run_agent_loop(
            &manifest,
            "Hello",
            &mut session,
            &memory,
            driver,
            &[],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None, // ainl_library_root
            None,
            None,
            None,
            None,
            None,
            None, // context_window_tokens
            None, // process_manager
            None, // user_content_blocks
            None, // btw_rx
            None, // redirect_rx
            EffectiveRuntimeLimits::legacy_defaults(),
            None, // orchestration_ctx
            None, // orchestration_live
        )
        .await
        .expect("Loop should complete with fallback");

        // No tools were executed, so should get the empty response message
        assert!(
            result.response.contains("empty response"),
            "Expected empty response fallback (no tools executed), got: {:?}",
            result.response
        );
    }

    #[tokio::test]
    async fn test_streaming_empty_response_max_tokens_returns_fallback() {
        let memory = openfang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
        let agent_id = openfang_types::agent::AgentId::new();
        let mut session = openfang_memory::session::Session {
            id: openfang_types::agent::SessionId::new(),
            agent_id,
            messages: Vec::new(),
            context_window_tokens: 0,
            label: None,
        };
        let manifest = test_manifest();
        let driver: Arc<dyn LlmDriver> = Arc::new(EmptyMaxTokensDriver);
        let (tx, _rx) = mpsc::channel(64);

        let result = run_agent_loop_streaming(
            &manifest,
            "Tell me something long",
            &mut session,
            &memory,
            driver,
            &[],
            None,
            tx,
            None,
            None,
            None,
            None,
            None,
            None,
            None, // ainl_library_root
            None, // on_phase
            None, // media_engine
            None, // tts_engine
            None, // docker_config
            None, // hooks
            None, // context_window_tokens
            None, // process_manager
            None, // user_content_blocks
            None, // btw_rx
            None, // redirect_rx
            EffectiveRuntimeLimits::legacy_defaults(),
            None, // orchestration_ctx
            None, // orchestration_live
        )
        .await
        .expect("Streaming loop should complete without error");

        assert!(
            !result.response.trim().is_empty(),
            "Streaming response should not be empty on max tokens, got: {:?}",
            result.response
        );
        assert!(
            result.response.contains("token limit"),
            "Expected max-tokens fallback in streaming, got: {:?}",
            result.response
        );
    }

    #[test]
    fn test_recover_text_tool_calls_basic() {
        let tools = vec![ToolDefinition {
            name: "web_search".into(),
            description: "Search the web".into(),
            input_schema: serde_json::json!({}),
        }];
        let text =
            r#"Let me search for that. <function=web_search>{"query":"rust async"}</function>"#;
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "web_search");
        assert_eq!(calls[0].input["query"], "rust async");
        assert!(calls[0].id.starts_with("recovered_"));
    }

    #[test]
    fn test_recover_text_tool_calls_xml_parameters() {
        let tools = vec![ToolDefinition {
            name: "shell_exec".into(),
            description: "Execute".into(),
            input_schema: serde_json::json!({}),
        }];
        let text = r#"<function=shell_exec><parameter=command>python3 "/tmp/run.py" --flag value</parameter></function>"#;
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "shell_exec");
        assert_eq!(
            calls[0].input["command"],
            r#"python3 "/tmp/run.py" --flag value"#
        );
    }

    #[test]
    fn test_recover_text_tool_calls_xml_parameters_with_wrapper() {
        let tools = vec![ToolDefinition {
            name: "shell_exec".into(),
            description: "Execute".into(),
            input_schema: serde_json::json!({}),
        }];
        let text = r#"<tool_call>
<function=shell_exec>
<parameter=command>python3 "/tmp/poll.py" --job-id "abc123"</parameter>
</function>
</tool_call>"#;
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "shell_exec");
        assert_eq!(
            calls[0].input["command"],
            r#"python3 "/tmp/poll.py" --job-id "abc123""#
        );
    }

    #[test]
    fn test_recover_text_tool_calls_unknown_tool() {
        let tools = vec![ToolDefinition {
            name: "web_search".into(),
            description: "Search the web".into(),
            input_schema: serde_json::json!({}),
        }];
        let text = r#"<function=hack_system>{"cmd":"rm -rf /"}</function>"#;
        let calls = recover_text_tool_calls(text, &tools);
        assert!(calls.is_empty(), "Unknown tools should be rejected");
    }

    #[test]
    fn test_recover_text_tool_calls_invalid_json() {
        let tools = vec![ToolDefinition {
            name: "web_search".into(),
            description: "Search the web".into(),
            input_schema: serde_json::json!({}),
        }];
        let text = r#"<function=web_search>not valid json</function>"#;
        let calls = recover_text_tool_calls(text, &tools);
        assert!(calls.is_empty(), "Invalid JSON should be skipped");
    }

    #[test]
    fn test_recover_text_tool_calls_multiple() {
        let tools = vec![
            ToolDefinition {
                name: "web_search".into(),
                description: "Search".into(),
                input_schema: serde_json::json!({}),
            },
            ToolDefinition {
                name: "read_file".into(),
                description: "Read a file".into(),
                input_schema: serde_json::json!({}),
            },
        ];
        let text = r#"<function=web_search>{"query":"hello"}</function> then <function=read_file>{"path":"a.txt"}</function>"#;
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "web_search");
        assert_eq!(calls[1].name, "read_file");
    }

    #[test]
    fn test_recover_text_tool_calls_no_pattern() {
        let tools = vec![ToolDefinition {
            name: "web_search".into(),
            description: "Search".into(),
            input_schema: serde_json::json!({}),
        }];
        let text = "Just a normal response with no tool calls.";
        let calls = recover_text_tool_calls(text, &tools);
        assert!(calls.is_empty());
    }

    #[test]
    fn test_recover_text_tool_calls_empty_tools() {
        let text = r#"<function=web_search>{"query":"hello"}</function>"#;
        let calls = recover_text_tool_calls(text, &[]);
        assert!(calls.is_empty(), "No tools = no recovery");
    }

    // --- Deep edge-case tests for text-to-tool recovery ---

    #[test]
    fn test_recover_text_tool_calls_nested_json() {
        let tools = vec![ToolDefinition {
            name: "web_search".into(),
            description: "Search".into(),
            input_schema: serde_json::json!({}),
        }];
        let text = r#"<function=web_search>{"query":"rust","filters":{"lang":"en","year":2024}}</function>"#;
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].input["filters"]["lang"], "en");
    }

    #[test]
    fn test_recover_text_tool_calls_with_surrounding_text() {
        let tools = vec![ToolDefinition {
            name: "web_search".into(),
            description: "Search".into(),
            input_schema: serde_json::json!({}),
        }];
        let text = "Sure, let me search that for you.\n\n<function=web_search>{\"query\":\"rust async programming\"}</function>\n\nI'll get back to you with results.";
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].input["query"], "rust async programming");
    }

    #[test]
    fn test_recover_text_tool_calls_whitespace_in_json() {
        let tools = vec![ToolDefinition {
            name: "web_search".into(),
            description: "Search".into(),
            input_schema: serde_json::json!({}),
        }];
        // Some models emit pretty-printed JSON
        let text = "<function=web_search>\n  {\"query\": \"hello world\"}\n</function>";
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].input["query"], "hello world");
    }

    #[test]
    fn test_recover_text_tool_calls_unclosed_tag() {
        let tools = vec![ToolDefinition {
            name: "web_search".into(),
            description: "Search".into(),
            input_schema: serde_json::json!({}),
        }];
        // Missing </function> — should gracefully skip
        let text = r#"<function=web_search>{"query":"test"}"#;
        let calls = recover_text_tool_calls(text, &tools);
        assert!(calls.is_empty(), "Unclosed tag should be skipped");
    }

    #[test]
    fn test_recover_text_tool_calls_missing_closing_bracket() {
        let tools = vec![ToolDefinition {
            name: "web_search".into(),
            description: "Search".into(),
            input_schema: serde_json::json!({}),
        }];
        // Missing > after tool name
        let text = r#"<function=web_search{"query":"test"}</function>"#;
        let calls = recover_text_tool_calls(text, &tools);
        // The parser finds > inside JSON, will likely produce invalid tool name
        // or invalid JSON — either way, should not panic
        // (just verifying no panic / no bad behavior)
        let _ = calls;
    }

    #[test]
    fn test_recover_text_tool_calls_empty_json_object() {
        let tools = vec![ToolDefinition {
            name: "list_files".into(),
            description: "List".into(),
            input_schema: serde_json::json!({}),
        }];
        let text = r#"<function=list_files>{}</function>"#;
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "list_files");
        assert_eq!(calls[0].input, serde_json::json!({}));
    }

    #[test]
    fn test_recover_text_tool_calls_mixed_valid_invalid() {
        let tools = vec![
            ToolDefinition {
                name: "web_search".into(),
                description: "Search".into(),
                input_schema: serde_json::json!({}),
            },
            ToolDefinition {
                name: "read_file".into(),
                description: "Read".into(),
                input_schema: serde_json::json!({}),
            },
        ];
        // First: valid, second: unknown tool, third: valid
        let text = r#"<function=web_search>{"q":"a"}</function> <function=unknown>{"x":1}</function> <function=read_file>{"path":"b"}</function>"#;
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 2, "Should recover 2 valid, skip 1 unknown");
        assert_eq!(calls[0].name, "web_search");
        assert_eq!(calls[1].name, "read_file");
    }

    // --- Variant 2 pattern tests: <function>NAME{JSON}</function> ---

    #[test]
    fn test_recover_variant2_basic() {
        let tools = vec![ToolDefinition {
            name: "web_fetch".into(),
            description: "Fetch".into(),
            input_schema: serde_json::json!({}),
        }];
        let text = r#"<function>web_fetch{"url":"https://example.com"}</function>"#;
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "web_fetch");
        assert_eq!(calls[0].input["url"], "https://example.com");
    }

    #[test]
    fn test_recover_variant2_unknown_tool() {
        let tools = vec![ToolDefinition {
            name: "web_search".into(),
            description: "Search".into(),
            input_schema: serde_json::json!({}),
        }];
        let text = r#"<function>unknown_tool{"q":"test"}</function>"#;
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 0);
    }

    #[test]
    fn test_recover_variant2_with_surrounding_text() {
        let tools = vec![ToolDefinition {
            name: "web_search".into(),
            description: "Search".into(),
            input_schema: serde_json::json!({}),
        }];
        let text = r#"Let me search for that. <function>web_search{"query":"rust lang"}</function> I'll find the answer."#;
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "web_search");
    }

    #[test]
    fn test_recover_both_variants_mixed() {
        let tools = vec![
            ToolDefinition {
                name: "web_search".into(),
                description: "Search".into(),
                input_schema: serde_json::json!({}),
            },
            ToolDefinition {
                name: "web_fetch".into(),
                description: "Fetch".into(),
                input_schema: serde_json::json!({}),
            },
        ];
        // Mix of variant 1 and variant 2
        let text = r#"<function=web_search>{"q":"a"}</function> <function>web_fetch{"url":"https://x.com"}</function>"#;
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "web_search");
        assert_eq!(calls[1].name, "web_fetch");
    }

    #[test]
    fn test_recover_tool_tag_variant() {
        let tools = vec![ToolDefinition {
            name: "exec".into(),
            description: "Execute".into(),
            input_schema: serde_json::json!({}),
        }];
        let text = r#"I'll run that for you. <tool>exec{"command":"ls -la"}</tool>"#;
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "exec");
        assert_eq!(calls[0].input["command"], "ls -la");
    }

    #[test]
    fn test_recover_markdown_code_block() {
        let tools = vec![ToolDefinition {
            name: "exec".into(),
            description: "Execute".into(),
            input_schema: serde_json::json!({}),
        }];
        let text = "I'll execute that command:\n```\nexec {\"command\": \"ls -la\"}\n```";
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "exec");
        assert_eq!(calls[0].input["command"], "ls -la");
    }

    #[test]
    fn test_recover_markdown_code_block_with_lang() {
        let tools = vec![ToolDefinition {
            name: "web_search".into(),
            description: "Search".into(),
            input_schema: serde_json::json!({}),
        }];
        let text = "```json\nweb_search {\"query\": \"rust\"}\n```";
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "web_search");
    }

    #[test]
    fn test_recover_backtick_wrapped() {
        let tools = vec![ToolDefinition {
            name: "exec".into(),
            description: "Execute".into(),
            input_schema: serde_json::json!({}),
        }];
        let text = r#"Let me run `exec {"command":"pwd"}` for you."#;
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "exec");
        assert_eq!(calls[0].input["command"], "pwd");
    }

    #[test]
    fn test_recover_backtick_ignores_unknown_tool() {
        let tools = vec![ToolDefinition {
            name: "exec".into(),
            description: "Execute".into(),
            input_schema: serde_json::json!({}),
        }];
        let text = r#"Try `unknown_tool {"key":"val"}` instead."#;
        let calls = recover_text_tool_calls(text, &tools);
        assert!(calls.is_empty());
    }

    #[test]
    fn test_recover_no_duplicates_across_patterns() {
        let tools = vec![ToolDefinition {
            name: "exec".into(),
            description: "Execute".into(),
            input_schema: serde_json::json!({}),
        }];
        // Same call in both function tag and tool tag — should only appear once
        let text =
            r#"<function=exec>{"command":"ls"}</function> <tool>exec{"command":"ls"}</tool>"#;
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
    }

    // --- Pattern 6: [TOOL_CALL]...[/TOOL_CALL] tests (issue #354) ---

    #[test]
    fn test_recover_tool_call_block_json() {
        let tools = vec![ToolDefinition {
            name: "shell_exec".into(),
            description: "Execute shell command".into(),
            input_schema: serde_json::json!({}),
        }];
        let text = "[TOOL_CALL]\n{\"name\": \"shell_exec\", \"arguments\": {\"command\": \"ls -la\"}}\n[/TOOL_CALL]";
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "shell_exec");
        assert_eq!(calls[0].input["command"], "ls -la");
    }

    #[test]
    fn test_recover_tool_call_block_arrow_syntax() {
        let tools = vec![ToolDefinition {
            name: "shell_exec".into(),
            description: "Execute shell command".into(),
            input_schema: serde_json::json!({}),
        }];
        // Exact format from issue #354
        let text = "[TOOL_CALL]\n{tool => \"shell_exec\", args => {\n--command \"ls -F /\"\n}}\n[/TOOL_CALL]";
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "shell_exec");
        assert_eq!(calls[0].input["command"], "ls -F /");
    }

    #[test]
    fn test_recover_tool_call_block_unknown_tool() {
        let tools = vec![ToolDefinition {
            name: "shell_exec".into(),
            description: "Execute".into(),
            input_schema: serde_json::json!({}),
        }];
        let text = "[TOOL_CALL]\n{\"name\": \"hack_system\", \"arguments\": {\"cmd\": \"rm -rf /\"}}\n[/TOOL_CALL]";
        let calls = recover_text_tool_calls(text, &tools);
        assert!(calls.is_empty());
    }

    #[test]
    fn test_recover_tool_call_block_multiple() {
        let tools = vec![
            ToolDefinition {
                name: "shell_exec".into(),
                description: "Execute".into(),
                input_schema: serde_json::json!({}),
            },
            ToolDefinition {
                name: "file_read".into(),
                description: "Read".into(),
                input_schema: serde_json::json!({}),
            },
        ];
        let text = "[TOOL_CALL]\n{\"name\": \"shell_exec\", \"arguments\": {\"command\": \"ls\"}}\n[/TOOL_CALL]\nSome text.\n[TOOL_CALL]\n{\"name\": \"file_read\", \"arguments\": {\"path\": \"/tmp/test.txt\"}}\n[/TOOL_CALL]";
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "shell_exec");
        assert_eq!(calls[1].name, "file_read");
    }

    #[test]
    fn test_recover_tool_call_block_unclosed() {
        let tools = vec![ToolDefinition {
            name: "shell_exec".into(),
            description: "Execute".into(),
            input_schema: serde_json::json!({}),
        }];
        // Unclosed [TOOL_CALL] — pattern 6 skips it, but pattern 8 (bare JSON)
        // still finds the valid JSON tool call object.
        let text = "[TOOL_CALL]\n{\"name\": \"shell_exec\", \"arguments\": {\"command\": \"ls\"}}";
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1, "Bare JSON fallback should recover this");
        assert_eq!(calls[0].name, "shell_exec");
    }

    // --- Pattern 7: <tool_call>JSON</tool_call> tests (Qwen3, issue #332) ---

    #[test]
    fn test_recover_tool_call_xml_basic() {
        let tools = vec![ToolDefinition {
            name: "shell_exec".into(),
            description: "Execute".into(),
            input_schema: serde_json::json!({}),
        }];
        let text = "<tool_call>\n{\"name\": \"shell_exec\", \"arguments\": {\"command\": \"ls -la\"}}\n</tool_call>";
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "shell_exec");
        assert_eq!(calls[0].input["command"], "ls -la");
    }

    #[test]
    fn test_recover_tool_call_xml_with_surrounding_text() {
        let tools = vec![ToolDefinition {
            name: "web_search".into(),
            description: "Search".into(),
            input_schema: serde_json::json!({}),
        }];
        let text = "I'll search for that.\n\n<tool_call>\n{\"name\": \"web_search\", \"arguments\": {\"query\": \"rust async\"}}\n</tool_call>\n\nLet me get results.";
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "web_search");
        assert_eq!(calls[0].input["query"], "rust async");
    }

    #[test]
    fn test_recover_tool_call_xml_function_field() {
        let tools = vec![ToolDefinition {
            name: "file_read".into(),
            description: "Read".into(),
            input_schema: serde_json::json!({}),
        }];
        let text = "<tool_call>{\"function\": \"file_read\", \"arguments\": {\"path\": \"/etc/hosts\"}}</tool_call>";
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "file_read");
    }

    #[test]
    fn test_recover_tool_call_xml_parameters_field() {
        let tools = vec![ToolDefinition {
            name: "web_fetch".into(),
            description: "Fetch".into(),
            input_schema: serde_json::json!({}),
        }];
        let text = "<tool_call>{\"name\": \"web_fetch\", \"parameters\": {\"url\": \"https://example.com\"}}</tool_call>";
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "web_fetch");
        assert_eq!(calls[0].input["url"], "https://example.com");
    }

    #[test]
    fn test_recover_tool_call_xml_stringified_args() {
        let tools = vec![ToolDefinition {
            name: "shell_exec".into(),
            description: "Execute".into(),
            input_schema: serde_json::json!({}),
        }];
        let text = "<tool_call>{\"name\": \"shell_exec\", \"arguments\": \"{\\\"command\\\": \\\"pwd\\\"}\"}</tool_call>";
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "shell_exec");
        assert_eq!(calls[0].input["command"], "pwd");
    }

    #[test]
    fn test_recover_tool_call_xml_unknown_tool() {
        let tools = vec![ToolDefinition {
            name: "shell_exec".into(),
            description: "Execute".into(),
            input_schema: serde_json::json!({}),
        }];
        let text = "<tool_call>{\"name\": \"hack_system\", \"arguments\": {\"cmd\": \"rm -rf /\"}}</tool_call>";
        let calls = recover_text_tool_calls(text, &tools);
        assert!(calls.is_empty());
    }

    #[test]
    fn test_recover_tool_call_xml_multiple() {
        let tools = vec![
            ToolDefinition {
                name: "shell_exec".into(),
                description: "Execute".into(),
                input_schema: serde_json::json!({}),
            },
            ToolDefinition {
                name: "web_search".into(),
                description: "Search".into(),
                input_schema: serde_json::json!({}),
            },
        ];
        let text = "<tool_call>{\"name\": \"shell_exec\", \"arguments\": {\"command\": \"ls\"}}</tool_call>\n<tool_call>{\"name\": \"web_search\", \"arguments\": {\"query\": \"rust\"}}</tool_call>";
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "shell_exec");
        assert_eq!(calls[1].name, "web_search");
    }

    // --- Pattern 8: Bare JSON tool call object tests ---

    #[test]
    fn test_recover_bare_json_tool_call() {
        let tools = vec![ToolDefinition {
            name: "shell_exec".into(),
            description: "Execute".into(),
            input_schema: serde_json::json!({}),
        }];
        let text =
            "I'll run that: {\"name\": \"shell_exec\", \"arguments\": {\"command\": \"ls -la\"}}";
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "shell_exec");
        assert_eq!(calls[0].input["command"], "ls -la");
    }

    #[test]
    fn test_recover_bare_json_no_false_positive() {
        let tools = vec![ToolDefinition {
            name: "shell_exec".into(),
            description: "Execute".into(),
            input_schema: serde_json::json!({}),
        }];
        let text = "The config looks like {\"debug\": true, \"level\": \"info\"}";
        let calls = recover_text_tool_calls(text, &tools);
        assert!(calls.is_empty());
    }

    #[test]
    fn test_recover_bare_json_skipped_when_tags_found() {
        let tools = vec![ToolDefinition {
            name: "shell_exec".into(),
            description: "Execute".into(),
            input_schema: serde_json::json!({}),
        }];
        let text = "<function=shell_exec>{\"command\":\"ls\"}</function> {\"name\": \"shell_exec\", \"arguments\": {\"command\": \"pwd\"}}";
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].input["command"], "ls");
    }

    // --- Pattern 9: XML-attribute style <function name="..." parameters="..." /> ---

    #[test]
    fn test_recover_xml_attribute_basic() {
        let tools = vec![ToolDefinition {
            name: "web_search".into(),
            description: "Search".into(),
            input_schema: serde_json::json!({}),
        }];
        let text = r#"<function name="web_search" parameters="{&quot;query&quot;: &quot;best crypto 2024&quot;}" />"#;
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "web_search");
        assert_eq!(calls[0].input["query"], "best crypto 2024");
    }

    #[test]
    fn test_recover_xml_attribute_unknown_tool() {
        let tools = vec![ToolDefinition {
            name: "web_search".into(),
            description: "Search".into(),
            input_schema: serde_json::json!({}),
        }];
        let text = r#"<function name="unknown_tool" parameters="{&quot;x&quot;: 1}" />"#;
        let calls = recover_text_tool_calls(text, &tools);
        assert!(calls.is_empty());
    }

    #[test]
    fn test_recover_xml_attribute_non_selfclosing() {
        let tools = vec![ToolDefinition {
            name: "shell_exec".into(),
            description: "Execute".into(),
            input_schema: serde_json::json!({}),
        }];
        let text = r#"<function name="shell_exec" parameters="{&quot;command&quot;: &quot;ls&quot;}"></function>"#;
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "shell_exec");
    }

    // --- Pattern 10: <|plugin|>...<|endofblock|> tests ---

    #[test]
    fn test_recover_plugin_block() {
        let tools = vec![ToolDefinition {
            name: "web_search".into(),
            description: "Search".into(),
            input_schema: serde_json::json!({}),
        }];
        let text = "<|plugin|>\n{\"name\": \"web_search\", \"arguments\": {\"query\": \"rust\"}}\n<|endofblock|>";
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "web_search");
        assert_eq!(calls[0].input["query"], "rust");
    }

    #[test]
    fn test_recover_plugin_block_unknown_tool() {
        let tools = vec![ToolDefinition {
            name: "web_search".into(),
            description: "Search".into(),
            input_schema: serde_json::json!({}),
        }];
        let text =
            "<|plugin|>\n{\"name\": \"hack\", \"arguments\": {\"cmd\": \"rm\"}}\n<|endofblock|>";
        let calls = recover_text_tool_calls(text, &tools);
        assert!(calls.is_empty());
    }

    // --- Pattern 11: Action/Action Input tests ---

    #[test]
    fn test_recover_action_input() {
        let tools = vec![ToolDefinition {
            name: "web_search".into(),
            description: "Search".into(),
            input_schema: serde_json::json!({}),
        }];
        let text = "Action: web_search\nAction Input: {\"query\": \"rust programming\"}";
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "web_search");
        assert_eq!(calls[0].input["query"], "rust programming");
    }

    #[test]
    fn test_recover_action_input_unknown_tool() {
        let tools = vec![ToolDefinition {
            name: "web_search".into(),
            description: "Search".into(),
            input_schema: serde_json::json!({}),
        }];
        let text = "Action: unknown_tool\nAction Input: {\"key\": \"value\"}";
        let calls = recover_text_tool_calls(text, &tools);
        assert!(calls.is_empty());
    }

    // --- Pattern 12: name + JSON on next line tests ---

    #[test]
    fn test_recover_name_json_nextline() {
        let tools = vec![ToolDefinition {
            name: "shell_exec".into(),
            description: "Execute".into(),
            input_schema: serde_json::json!({}),
        }];
        let text = "shell_exec\n{\"command\": \"ls -la\"}";
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "shell_exec");
        assert_eq!(calls[0].input["command"], "ls -la");
    }

    #[test]
    fn test_recover_name_json_nextline_unknown() {
        let tools = vec![ToolDefinition {
            name: "shell_exec".into(),
            description: "Execute".into(),
            input_schema: serde_json::json!({}),
        }];
        let text = "unknown_tool\n{\"command\": \"ls\"}";
        let calls = recover_text_tool_calls(text, &tools);
        assert!(calls.is_empty());
    }

    // --- Pattern 13: <tool_use> tests ---

    #[test]
    fn test_recover_tool_use_block() {
        let tools = vec![ToolDefinition {
            name: "web_search".into(),
            description: "Search".into(),
            input_schema: serde_json::json!({}),
        }];
        let text =
            "<tool_use>{\"name\": \"web_search\", \"arguments\": {\"query\": \"test\"}}</tool_use>";
        let calls = recover_text_tool_calls(text, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "web_search");
    }

    #[test]
    fn test_recover_tool_use_block_unknown() {
        let tools = vec![ToolDefinition {
            name: "web_search".into(),
            description: "Search".into(),
            input_schema: serde_json::json!({}),
        }];
        let text = "<tool_use>{\"name\": \"hack\", \"arguments\": {\"cmd\": \"rm\"}}</tool_use>";
        let calls = recover_text_tool_calls(text, &tools);
        assert!(calls.is_empty());
    }

    // --- Helper function tests ---

    #[test]
    fn test_parse_dash_dash_args_basic() {
        let result = parse_dash_dash_args("{--command \"ls -F /\"}");
        assert_eq!(result["command"], "ls -F /");
    }

    #[test]
    fn test_parse_dash_dash_args_multiple() {
        let result = parse_dash_dash_args("{--file \"test.txt\", --verbose}");
        assert_eq!(result["file"], "test.txt");
        assert_eq!(result["verbose"], true);
    }

    #[test]
    fn test_parse_dash_dash_args_unquoted_value() {
        let result = parse_dash_dash_args("{--count 5}");
        assert_eq!(result["count"], "5");
    }

    #[test]
    fn test_parse_json_tool_call_object_standard() {
        let tool_names = vec!["shell_exec"];
        let result = parse_json_tool_call_object(
            "{\"name\": \"shell_exec\", \"arguments\": {\"command\": \"ls\"}}",
            &tool_names,
        );
        assert!(result.is_some());
        let (name, args) = result.unwrap();
        assert_eq!(name, "shell_exec");
        assert_eq!(args["command"], "ls");
    }

    #[test]
    fn test_parse_json_tool_call_object_function_field() {
        let tool_names = vec!["web_fetch"];
        let result = parse_json_tool_call_object(
            "{\"function\": \"web_fetch\", \"parameters\": {\"url\": \"https://x.com\"}}",
            &tool_names,
        );
        assert!(result.is_some());
        let (name, args) = result.unwrap();
        assert_eq!(name, "web_fetch");
        assert_eq!(args["url"], "https://x.com");
    }

    #[test]
    fn test_parse_json_tool_call_object_unknown_tool() {
        let tool_names = vec!["shell_exec"];
        let result =
            parse_json_tool_call_object("{\"name\": \"unknown\", \"arguments\": {}}", &tool_names);
        assert!(result.is_none());
    }

    // --- End-to-end integration test: text-as-tool-call recovery through agent loop ---

    /// Mock driver that simulates a Groq/Llama model outputting tool calls as text.
    /// Call 1: Returns text with `<function=web_search>...</function>` (EndTurn, no tool_calls)
    /// Call 2: Returns a normal text response (after tool result is provided)
    struct TextToolCallDriver {
        call_count: AtomicU32,
    }

    impl TextToolCallDriver {
        fn new() -> Self {
            Self {
                call_count: AtomicU32::new(0),
            }
        }
    }

    /// Mock driver that emits nested XML parameter-style tool calls as plain text.
    struct NestedXmlTextToolCallDriver {
        call_count: AtomicU32,
    }

    impl NestedXmlTextToolCallDriver {
        fn new() -> Self {
            Self {
                call_count: AtomicU32::new(0),
            }
        }
    }

    #[async_trait]
    impl LlmDriver for NestedXmlTextToolCallDriver {
        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            let call = self.call_count.fetch_add(1, Ordering::Relaxed);
            if call == 0 {
                Ok(CompletionResponse {
                    content: vec![ContentBlock::Text {
                        text: "<tool_call><function=web_search><parameter=query>rust async</parameter></function></tool_call>".to_string(),
                        provider_metadata: None,
                    }],
                    stop_reason: StopReason::EndTurn,
                    tool_calls: vec![],
                    usage: TokenUsage {
                        input_tokens: 18,
                        output_tokens: 10,
                        ..Default::default()
                    },
                                vitals: None,
})
            } else {
                Ok(CompletionResponse {
                    content: vec![ContentBlock::Text {
                        text: "Recovered nested XML tool call successfully.".to_string(),
                        provider_metadata: None,
                    }],
                    stop_reason: StopReason::EndTurn,
                    tool_calls: vec![],
                    usage: TokenUsage {
                        input_tokens: 24,
                        output_tokens: 8,
                        ..Default::default()
                    },
                    vitals: None,
                })
            }
        }
    }

    #[async_trait]
    impl LlmDriver for TextToolCallDriver {
        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            let call = self.call_count.fetch_add(1, Ordering::Relaxed);
            if call == 0 {
                // Simulate Groq/Llama: tool call as text, not in tool_calls field
                Ok(CompletionResponse {
                    content: vec![ContentBlock::Text {
                        text: r#"Let me search for that. <function=web_search>{"query":"rust async"}</function>"#.to_string(),
                        provider_metadata: None,
                    }],
                    stop_reason: StopReason::EndTurn,
                    tool_calls: vec![], // BUG: no tool_calls!
                    usage: TokenUsage {
                        input_tokens: 20,
                        output_tokens: 15,
                        ..Default::default()
                    },
                                vitals: None,
})
            } else {
                // After tool result, return normal response
                Ok(CompletionResponse {
                    content: vec![ContentBlock::Text {
                        text: "Based on the search results, Rust async is great!".to_string(),
                        provider_metadata: None,
                    }],
                    stop_reason: StopReason::EndTurn,
                    tool_calls: vec![],
                    usage: TokenUsage {
                        input_tokens: 30,
                        output_tokens: 12,
                        ..Default::default()
                    },
                    vitals: None,
                })
            }
        }
    }

    #[tokio::test]
    async fn test_text_tool_call_recovery_e2e() {
        // This is THE critical test: a model outputs a tool call as text,
        // the recovery code detects it, promotes it to ToolUse, executes the tool,
        // and the agent loop continues to produce a final response.
        let memory = openfang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
        let agent_id = openfang_types::agent::AgentId::new();
        let mut session = openfang_memory::session::Session {
            id: openfang_types::agent::SessionId::new(),
            agent_id,
            messages: Vec::new(),
            context_window_tokens: 0,
            label: None,
        };
        let manifest = test_manifest();
        let driver: Arc<dyn LlmDriver> = Arc::new(TextToolCallDriver::new());

        // Provide web_search as an available tool so recovery can match it
        let tools = vec![ToolDefinition {
            name: "web_search".into(),
            description: "Search the web".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string"}
                }
            }),
        }];

        let result = run_agent_loop(
            &manifest,
            "Search for rust async programming",
            &mut session,
            &memory,
            driver,
            &tools,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None, // ainl_library_root
            None, // on_phase
            None, // media_engine
            None, // tts_engine
            None, // docker_config
            None, // hooks
            None, // context_window_tokens
            None, // process_manager
            None, // user_content_blocks
            None, // btw_rx
            None, // redirect_rx
            EffectiveRuntimeLimits::legacy_defaults(),
            None, // orchestration_ctx
            None, // orchestration_live
        )
        .await
        .expect("Agent loop should complete");

        // The response should contain the second call's output, NOT the raw function tag
        assert!(
            !result.response.contains("<function="),
            "Response should not contain raw function tags, got: {:?}",
            result.response
        );
        assert!(
            result.iterations >= 2,
            "Should have at least 2 iterations (tool call + final response), got: {}",
            result.iterations
        );
        // Verify the final text response came through
        assert!(
            result.response.contains("search results") || result.response.contains("Rust async"),
            "Expected final response text, got: {:?}",
            result.response
        );
    }

    #[tokio::test]
    async fn test_nested_xml_text_tool_call_recovery_e2e() {
        let memory = openfang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
        let agent_id = openfang_types::agent::AgentId::new();
        let mut session = openfang_memory::session::Session {
            id: openfang_types::agent::SessionId::new(),
            agent_id,
            messages: Vec::new(),
            context_window_tokens: 0,
            label: None,
        };
        let manifest = test_manifest();
        let driver: Arc<dyn LlmDriver> = Arc::new(NestedXmlTextToolCallDriver::new());

        let tools = vec![ToolDefinition {
            name: "web_search".into(),
            description: "Search the web".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string"}
                }
            }),
        }];

        let result = run_agent_loop(
            &manifest,
            "Search for rust async programming",
            &mut session,
            &memory,
            driver,
            &tools,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None, // ainl_library_root
            None, // on_phase
            None, // media_engine
            None, // tts_engine
            None, // docker_config
            None, // hooks
            None, // context_window_tokens
            None, // process_manager
            None, // user_content_blocks
            None, // btw_rx
            None, // redirect_rx
            EffectiveRuntimeLimits::legacy_defaults(),
            None, // orchestration_ctx
            None, // orchestration_live
        )
        .await
        .expect("Agent loop should recover nested XML tool calls");

        assert!(
            !result.response.contains("<tool_call>"),
            "Response should not contain raw tool_call tags, got: {:?}",
            result.response
        );
        assert!(
            !result.response.contains("<function="),
            "Response should not contain raw function tags, got: {:?}",
            result.response
        );
        assert!(
            result
                .response
                .contains("Recovered nested XML tool call successfully."),
            "Expected final response text, got: {:?}",
            result.response
        );
        assert!(
            result.iterations >= 2,
            "Should have at least 2 iterations (tool call + final response), got: {}",
            result.iterations
        );
    }

    /// Mock driver that returns NO text-based tool calls — just normal text.
    /// Verifies recovery does NOT interfere with normal flow.
    #[tokio::test]
    async fn test_normal_flow_unaffected_by_recovery() {
        let memory = openfang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
        let agent_id = openfang_types::agent::AgentId::new();
        let mut session = openfang_memory::session::Session {
            id: openfang_types::agent::SessionId::new(),
            agent_id,
            messages: Vec::new(),
            context_window_tokens: 0,
            label: None,
        };
        let manifest = test_manifest();
        let driver: Arc<dyn LlmDriver> = Arc::new(NormalDriver);

        let tools = vec![ToolDefinition {
            name: "web_search".into(),
            description: "Search the web".into(),
            input_schema: serde_json::json!({}),
        }];

        let result = run_agent_loop(
            &manifest,
            "Say hello",
            &mut session,
            &memory,
            driver,
            &tools, // tools available but not used
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None, // ainl_library_root
            None, // on_phase
            None, // media_engine
            None, // tts_engine
            None, // docker_config
            None, // hooks
            None, // context_window_tokens
            None, // process_manager
            None, // user_content_blocks
            None, // btw_rx
            None, // redirect_rx
            EffectiveRuntimeLimits::legacy_defaults(),
            None, // orchestration_ctx
            None, // orchestration_live
        )
        .await
        .expect("Normal loop should complete");

        assert_eq!(result.response, "Hello from the agent!");
        assert_eq!(
            result.iterations, 1,
            "Normal response should complete in 1 iteration"
        );
    }

    // --- Streaming path: text-as-tool-call recovery ---

    #[tokio::test]
    async fn test_text_tool_call_recovery_streaming_e2e() {
        let memory = openfang_memory::MemorySubstrate::open_in_memory(0.01).unwrap();
        let agent_id = openfang_types::agent::AgentId::new();
        let mut session = openfang_memory::session::Session {
            id: openfang_types::agent::SessionId::new(),
            agent_id,
            messages: Vec::new(),
            context_window_tokens: 0,
            label: None,
        };
        let manifest = test_manifest();
        let driver: Arc<dyn LlmDriver> = Arc::new(TextToolCallDriver::new());

        let tools = vec![ToolDefinition {
            name: "web_search".into(),
            description: "Search the web".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string"}
                }
            }),
        }];

        let (tx, mut rx) = mpsc::channel(64);

        let result = run_agent_loop_streaming(
            &manifest,
            "Search for rust async programming",
            &mut session,
            &memory,
            driver,
            &tools,
            None,
            tx,
            None,
            None,
            None,
            None,
            None,
            None,
            None, // ainl_library_root
            None, // on_phase
            None, // media_engine
            None, // tts_engine
            None, // docker_config
            None, // hooks
            None, // context_window_tokens
            None, // process_manager
            None, // user_content_blocks
            None, // btw_rx
            None, // redirect_rx
            EffectiveRuntimeLimits::legacy_defaults(),
            None, // orchestration_ctx
            None, // orchestration_live
        )
        .await
        .expect("Streaming loop should complete");

        // Same assertions as non-streaming
        assert!(
            !result.response.contains("<function="),
            "Streaming: response should not contain raw function tags, got: {:?}",
            result.response
        );
        assert!(
            result.iterations >= 2,
            "Streaming: should have at least 2 iterations, got: {}",
            result.iterations
        );

        // Drain the stream channel to verify events were sent
        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }
        assert!(!events.is_empty(), "Should have received stream events");
    }

    #[test]
    fn test_silent_detection_uppercase() {
        assert!(is_silent_token("[SILENT]"));
    }

    #[test]
    fn test_silent_detection_lowercase() {
        assert!(is_silent_token("[silent]"));
    }

    #[test]
    fn test_silent_detection_mixed_case() {
        assert!(is_silent_token("[Silent]"));
    }

    #[test]
    fn test_silent_detection_with_whitespace() {
        assert!(is_silent_token("  [SILENT]  "));
    }

    #[test]
    fn test_silent_detection_no_reply() {
        assert!(is_silent_token("NO_REPLY"));
    }

    #[test]
    fn test_silent_detection_rejects_normal_text() {
        assert!(!is_silent_token("Hello, how can I help?"));
        assert!(!is_silent_token("SILENT"));
        assert!(!is_silent_token(""));
    }
}
