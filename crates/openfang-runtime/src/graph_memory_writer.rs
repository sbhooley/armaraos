//! Bridge between openfang-runtime agent turns and ainl-memory graph store.
//!
//! Every agent turn that completes gets an **Episode** node. **Semantic** facts
//! and optional **procedural** patterns for that turn are written after the episode
//! (see **`agent_loop`** + **`ainl_graph_extractor_bridge`**). Every A2A delegation
//! gets an Episode with **`delegation_to`** set.
//!
//! This is the wire that makes ainl-memory non-dead-code in the binary and
//! fulfills the architectural claim: execution IS the memory.
//!
//! **Export:** [`GraphMemoryWriter::export_graph_json`] and [`GraphMemoryWriter::export_graph_json_for_agent`]
//! call into **ainl-memory**’s graph export (same JSON shape as `AgentGraphSnapshot`). CLI:
//! `openfang memory graph-export <agent> --output path.json`. Python `ainl_graph_memory` can seed reads
//! via [`armaraos_graph_memory_export_json_path`] / `AINL_GRAPH_MEMORY_ARMARAOS_EXPORT` (see **ainativelang**
//! `docs/adapters/AINL_GRAPH_MEMORY.md`).

use ainl_contracts::{TrajectoryOutcome, TrajectoryStep};
#[cfg(feature = "ainl-extractor")]
use ainl_graph_extractor::GraphExtractorTask;
use ainl_memory::pattern_promotion;
use ainl_memory::{AinlMemoryNode, AinlNodeType, GraphMemory};
#[cfg(all(feature = "ainl-extractor", feature = "ainl-persona-evolution"))]
use ainl_persona::PersonaAxis;
use openfang_types::agent::AgentManifest;
use openfang_types::event::GraphMemoryWriteProvenance;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::{Mutex as StdMutex, OnceLock};
use tokio::sync::Mutex;
use tracing::{debug, warn};
use uuid::Uuid;

/// Hook invoked after graph writes: `(agent_id, kind, provenance)`.
pub(crate) type GraphMemoryWriteNotifyFn =
    Arc<dyn Fn(String, String, Option<GraphMemoryWriteProvenance>) + Send + Sync>;

/// Result of [`GraphMemoryWriter::run_persona_evolution_pass`].
#[cfg(feature = "ainl-extractor")]
pub type PersonaEvolutionExtractionReport = ainl_graph_extractor::ExtractionReport;

#[cfg(not(feature = "ainl-extractor"))]
#[derive(Debug, Clone)]
pub struct PersonaEvolutionExtractionReport {
    pub agent_id: String,
}

#[cfg(not(feature = "ainl-extractor"))]
impl PersonaEvolutionExtractionReport {
    pub fn has_errors(&self) -> bool {
        false
    }
}

/// Horizon for “any persona row exists” checks (per-agent DB; long window ≈ all history).
pub(crate) const PERSONA_PRIOR_LOOKBACK_SECS: i64 = 60 * 60 * 24 * 365 * 100;
const CONSOLIDATION_MIN_INTERVAL_SECS: i64 = 60;

/// When **unset** or any non-falsy value: record [`TrajectoryNode`] rows after each successful
/// [`GraphMemoryWriter::record_turn`] (opt-out mirrors `AINL_EXTRACTOR_ENABLED`).
/// Falsy: `0`, `false`, `no`, `off` (case-insensitive).
#[must_use]
pub fn trajectory_env_enabled() -> bool {
    ainl_memory::trajectory_env_enabled()
}

/// When **unset** or any non-falsy value: record [`ainl_memory::FailureNode`] rows for loop-guard outcomes.
/// Falsy: `0`, `false`, `no`, `off` (case-insensitive).
#[must_use]
pub fn failure_learning_env_enabled() -> bool {
    match std::env::var("AINL_FAILURE_LEARNING_ENABLED") {
        Ok(v) => {
            let t = v.trim().to_ascii_lowercase();
            !matches!(t.as_str(), "" | "0" | "false" | "no" | "off")
        }
        Err(_) => true,
    }
}

/// Resolve optional AINL source fingerprint for trajectory rows: process env first, then manifest
/// metadata string keys (`ainl_source_hash`, `ainl_bundle_sha256`).
#[must_use]
pub fn ainl_source_hash_for_trajectory_persist(manifest: &AgentManifest) -> Option<String> {
    for key in ["AINL_SOURCE_HASH", "AINL_BUNDLE_SHA256"] {
        if let Ok(v) = std::env::var(key) {
            let t = v.trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
    }
    for meta_key in ["ainl_source_hash", "ainl_bundle_sha256"] {
        let Some(raw) = manifest.metadata.get(meta_key) else {
            continue;
        };
        if let Some(s) = raw.as_str() {
            let t = s.trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
    }
    None
}

fn trajectory_steps_from_tools(tools: &[String]) -> Vec<TrajectoryStep> {
    let base_ms = chrono::Utc::now().timestamp_millis();
    tools
        .iter()
        .enumerate()
        .map(|(i, name)| TrajectoryStep {
            step_id: format!("step_{i}"),
            timestamp_ms: base_ms + i as i64,
            adapter: "builtin".into(),
            operation: name.clone(),
            inputs_preview: None,
            outputs_preview: None,
            duration_ms: 0,
            success: true,
            error: None,
            vitals: None,
            freshness_at_step: None,
            frame_vars: None,
            tool_telemetry: None,
        })
        .collect()
}

/// Optional turn-level **frame** snapshot for trajectory learning (JSON).
///
/// Hosts may extend this over time; typical keys: `vitals_trust`, `compression_semantic_score`.
#[must_use]
pub fn trajectory_turn_frame_vars(
    vitals_trust: Option<f32>,
    compression_semantic: Option<f32>,
) -> Option<serde_json::Value> {
    if vitals_trust.is_none() && compression_semantic.is_none() {
        return None;
    }
    let mut m = serde_json::Map::new();
    if let Some(t) = vitals_trust {
        m.insert("vitals_trust".to_string(), serde_json::json!(t));
    }
    if let Some(s) = compression_semantic {
        m.insert(
            "compression_semantic_score".to_string(),
            serde_json::json!(s),
        );
    }
    Some(serde_json::Value::Object(m))
}

fn consolidation_tracker() -> &'static StdMutex<HashMap<String, i64>> {
    static TRACKER: OnceLock<StdMutex<HashMap<String, i64>>> = OnceLock::new();
    TRACKER.get_or_init(|| StdMutex::new(HashMap::new()))
}

/// JSON snapshot path for the Python `ainl_graph_memory` bridge (auto-refresh after persona evolution).
///
/// If `AINL_GRAPH_MEMORY_ARMARAOS_EXPORT` is set, it names a **directory**; the file is
/// `{dir}/{agent_id}_graph_export.json`. If unset, uses the agent data directory next to `ainl_memory.db`:
/// `{openfang_home_dir()}/agents/{agent_id}/ainl_graph_memory_export.json`.
pub fn armaraos_graph_memory_export_json_path(agent_id: &str) -> PathBuf {
    let trimmed = std::env::var("AINL_GRAPH_MEMORY_ARMARAOS_EXPORT")
        .unwrap_or_default()
        .trim()
        .to_string();
    if !trimmed.is_empty() {
        PathBuf::from(trimmed).join(format!("{agent_id}_graph_export.json"))
    } else {
        openfang_types::config::openfang_home_dir()
            .join("agents")
            .join(agent_id)
            .join("ainl_graph_memory_export.json")
    }
}

/// Thread-safe wrapper around GraphMemory for use in the async agent loop.
#[derive(Clone)]
pub struct GraphMemoryWriter {
    pub(crate) inner: Arc<Mutex<GraphMemory>>,
    pub(crate) agent_id: String,
    pub(crate) on_write: Option<GraphMemoryWriteNotifyFn>,
}

impl GraphMemoryWriter {
    /// Agent id this writer is bound to (same as graph rows `agent_id`).
    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    /// Open or create the AINL graph memory DB for this agent.
    /// Path: ~/.armaraos/agents/<agent_id>/ainl_memory.db
    pub fn open(agent_id: &str) -> Result<Self, String> {
        Self::open_with_notify(agent_id, None)
    }

    /// Same as [`Self::open`], with an optional hook invoked after each successful write.
    /// Arguments to the hook: `(agent_id, kind, provenance)` where `kind` is e.g. `episode`,
    /// `delegation`, `fact`, `procedural`, `persona`.
    pub fn open_with_notify(
        agent_id: &str,
        on_write: Option<GraphMemoryWriteNotifyFn>,
    ) -> Result<Self, String> {
        let path = Self::db_path(agent_id)?;
        std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| format!("create dir: {e}"))?;
        let memory = GraphMemory::new(&path).map_err(|e| format!("open graph memory: {e}"))?;
        Ok(Self {
            inner: Arc::new(Mutex::new(memory)),
            agent_id: agent_id.to_string(),
            on_write,
        })
    }

    /// Test-only: bind a caller-supplied [`GraphMemory`] (e.g. temp-file SQLite) instead of `~/.armaraos/...`.
    #[cfg(test)]
    pub(crate) fn from_memory_for_tests(
        memory: GraphMemory,
        agent_id: impl Into<String>,
        on_write: Option<GraphMemoryWriteNotifyFn>,
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(memory)),
            agent_id: agent_id.into(),
            on_write,
        }
    }

    fn db_path(agent_id: &str) -> Result<PathBuf, String> {
        // Must match kernel agent dirs (`KernelConfig::home_dir` / `openfang_home_dir`) and
        // `GET /api/graph-memory` — not `dirs::home_dir().join(".armaraos")` alone (breaks `ARMARAOS_HOME`).
        Ok(openfang_types::config::openfang_home_dir()
            .join("agents")
            .join(agent_id)
            .join("ainl_memory.db"))
    }

    /// Canonical `ainl_memory.db` path for an agent (same as [`Self::open`]). Used to open a
    /// secondary SQLite handle (e.g. **ainl-runtime**) alongside this writer.
    pub fn sqlite_database_path_for_agent(agent_id: &str) -> Result<PathBuf, String> {
        Self::db_path(agent_id)
    }

    fn fire_write_hook(&self, kind: &str, provenance: Option<GraphMemoryWriteProvenance>) {
        if let Some(f) = &self.on_write {
            f(self.agent_id.clone(), kind.to_string(), provenance);
        }
    }

    /// Emit a graph-memory write notification without performing a direct write in this wrapper.
    ///
    /// This is used by secondary writers (for example `ainl-runtime` running through a separate
    /// SQLite handle) so dashboard live timelines still receive `GraphMemoryWrite` events for the
    /// same agent and can refresh promptly.
    pub fn emit_write_observed(&self, kind: &str, provenance: Option<GraphMemoryWriteProvenance>) {
        self.fire_write_hook(kind, provenance);
    }

    /// Import `ainl_graph_memory_inbox.json` (Python `AinlMemorySyncWriter`) into this agent's
    /// SQLite graph, then reset the inbox to an empty envelope.
    pub async fn drain_python_graph_memory_inbox(&self) {
        match crate::ainl_inbox_reader::drain_inbox(self).await {
            Ok(imported) if imported > 0 => {
                self.fire_write_hook(
                    "fact",
                    Some(GraphMemoryWriteProvenance {
                        node_ids: vec![],
                        node_kind: None,
                        reason: Some("inbox_import".to_string()),
                        summary: Some(format!(
                            "Imported {imported} node(s) from Python graph-memory inbox"
                        )),
                        trace_id: None,
                        tool_name: None,
                    }),
                );
            }
            Ok(_) => {}
            Err(e) => {
                warn!(
                    agent_id = %self.agent_id,
                    error = %e,
                    "AINL graph memory inbox drain failed"
                );
            }
        }
    }

    /// Record a completed agent turn as an EpisodeNode.
    ///
    /// On success, returns the new episode **node** id (same id space as
    /// [`Self::record_fact`] `source_turn_id` in existing call sites).
    #[allow(clippy::too_many_arguments)]
    pub async fn record_turn(
        &self,
        tool_calls: Vec<String>,
        delegation_to: Option<String>,
        trace_json: Option<serde_json::Value>,
        episode_tags: &[String],
        vitals_gate: Option<String>,
        vitals_phase: Option<String>,
        vitals_trust: Option<f32>,
        memory_project_id: Option<&str>,
    ) -> Option<Uuid> {
        let kind = if delegation_to.is_some() {
            "delegation"
        } else {
            "episode"
        };
        let trace_id_for_hook = trace_json.as_ref().and_then(|v| {
            v.get("trace_id")
                .and_then(|x| x.as_str())
                .map(std::string::ToString::to_string)
        });
        let res = {
            let inner = self.inner.lock().await;
            let prev_id = inner
                .recall_recent(60 * 60 * 24 * 7)
                .ok()
                .and_then(|nodes| nodes.into_iter().next())
                .map(|n| n.id);
            let turn_id = Uuid::new_v4();
            let timestamp = chrono::Utc::now().timestamp();
            let mut node = AinlMemoryNode::new_episode(
                turn_id,
                timestamp,
                tool_calls.clone(),
                delegation_to.clone(),
                trace_json,
            );
            node.agent_id = self.agent_id.clone();
            crate::memory_project_scope::apply_memory_project_id_to_node(
                &mut node,
                memory_project_id,
            );
            if let AinlNodeType::Episode { ref mut episodic } = node.node_type {
                if let Some(prev) = prev_id {
                    episodic.follows_episode_id = Some(prev.to_string());
                }
                for t in episode_tags {
                    if !episodic.tags.iter().any(|x| x == t) {
                        episodic.tags.push(t.clone());
                    }
                }
                episodic.vitals_gate = vitals_gate.clone();
                episodic.vitals_phase = vitals_phase.clone();
                episodic.vitals_trust = vitals_trust;
            }
            let new_id = node.id;
            match inner.write_node(&node) {
                Ok(()) => {
                    if let Some(prev) = prev_id {
                        let _ = inner.write_edge(new_id, prev, "follows");
                    }
                    Ok(new_id)
                }
                Err(e) => Err(e),
            }
        };
        match res {
            Ok(id) => {
                debug!(
                    agent_id = %self.agent_id,
                    episode_id = %id,
                    tools = ?tool_calls,
                    delegation_to = ?delegation_to,
                    "AINL graph memory: episode written"
                );
                let summary = if tool_calls.is_empty() {
                    format!("Episode recorded ({id})")
                } else {
                    format!("Tools: {}", tool_calls.join(", "))
                };
                let reason = if delegation_to.is_some() {
                    "delegation_episode"
                } else {
                    "turn_complete"
                };
                self.fire_write_hook(
                    kind,
                    Some(GraphMemoryWriteProvenance {
                        node_ids: vec![id.to_string()],
                        node_kind: Some("episode".to_string()),
                        reason: Some(reason.to_string()),
                        summary: Some(summary),
                        trace_id: trace_id_for_hook,
                        tool_name: None,
                    }),
                );
                Some(id)
            }
            Err(e) => {
                warn!(
                    agent_id = %self.agent_id,
                    error = %e,
                    "AINL graph memory: failed to write episode"
                );
                None
            }
        }
    }

    /// Persist a [`TrajectoryNode`] for the given episode graph row and link it with edge
    /// `trajectory_of` → episode. No-op when [`trajectory_env_enabled`] is false.
    ///
    /// `episode_graph_id` is the episode **node id** returned from [`Self::record_turn`] (same id
    /// space as semantic `source_turn_id`).
    ///
    /// When `detailed_steps` is `Some`, per-tool steps (from `execute_tool_with_trajectory`) are
    /// persisted; otherwise coarse steps are derived from `tools_fallback`.
    #[allow(clippy::too_many_arguments)] // mirrors the trajectory schema columns
    pub async fn record_trajectory_for_episode(
        &self,
        episode_graph_id: Uuid,
        tools_fallback: &[String],
        detailed_steps: Option<Vec<TrajectoryStep>>,
        outcome: TrajectoryOutcome,
        session_id: &str,
        project_id: Option<&str>,
        duration_ms: u64,
        ainl_source_hash: Option<&str>,
        trajectory_frame_vars: Option<serde_json::Value>,
        trajectory_fitness_delta: Option<f32>,
    ) -> Option<Uuid> {
        if !trajectory_env_enabled() {
            return None;
        }
        let steps = detailed_steps.unwrap_or_else(|| trajectory_steps_from_tools(tools_fallback));
        let step_count = steps.len();
        let res = {
            let inner = self.inner.lock().await;
            ainl_memory::persist_trajectory_for_episode(
                &inner,
                &self.agent_id,
                episode_graph_id,
                steps,
                outcome,
                session_id,
                project_id,
                ainl_source_hash,
                duration_ms,
                trajectory_frame_vars,
                trajectory_fitness_delta,
            )
        };
        match res {
            Ok((graph_tid, _detail_id)) => {
                debug!(
                    agent_id = %self.agent_id,
                    trajectory_id = %graph_tid,
                    episode_id = %episode_graph_id,
                    "AINL graph memory: trajectory written (graph + ainl_trajectories row)"
                );
                self.fire_write_hook(
                    "trajectory",
                    Some(GraphMemoryWriteProvenance {
                        node_ids: vec![graph_tid.to_string(), episode_graph_id.to_string()],
                        node_kind: Some("trajectory".to_string()),
                        reason: Some("turn_trajectory".to_string()),
                        summary: Some(format!(
                            "Trajectory for episode {} ({} steps)",
                            episode_graph_id, step_count
                        )),
                        trace_id: None,
                        tool_name: None,
                    }),
                );
                Some(graph_tid)
            }
            Err(e) => {
                warn!(
                    agent_id = %self.agent_id,
                    error = %e,
                    "AINL graph memory: failed to write trajectory"
                );
                None
            }
        }
    }

    /// Persist a [`ainl_memory::FailureNode`] for a loop-guard outcome (`verdict_label`: `block` | `circuit_break`).
    pub async fn record_loop_guard_failure(
        &self,
        verdict_label: &str,
        tool_name: Option<&str>,
        message: &str,
        session_id: Option<&str>,
        memory_project_id: Option<&str>,
    ) -> Option<Uuid> {
        if !failure_learning_env_enabled() {
            return None;
        }
        let res = {
            let inner = self.inner.lock().await;
            let mut node = AinlMemoryNode::new_loop_guard_failure(
                verdict_label,
                tool_name,
                message,
                session_id,
            );
            node.agent_id = self.agent_id.clone();
            crate::memory_project_scope::apply_memory_project_id_to_node(
                &mut node,
                memory_project_id,
            );
            let id = node.id;
            inner.write_node(&node).map(|()| id)
        };
        match res {
            Ok(id) => {
                debug!(
                    agent_id = %self.agent_id,
                    failure_id = %id,
                    verdict = %verdict_label,
                    "AINL graph memory: loop-guard failure written"
                );
                self.fire_write_hook(
                    "failure",
                    Some(GraphMemoryWriteProvenance {
                        node_ids: vec![id.to_string()],
                        node_kind: Some("failure".to_string()),
                        reason: Some(format!("loop_guard_{verdict_label}")),
                        summary: Some(format!(
                            "Loop guard {verdict_label}: {}",
                            openfang_types::truncate_str(message, 200)
                        )),
                        trace_id: None,
                        tool_name: tool_name.map(str::to_string),
                    }),
                );
                Some(id)
            }
            Err(e) => {
                warn!(
                    agent_id = %self.agent_id,
                    error = %e,
                    "AINL graph memory: failed to write loop-guard failure node"
                );
                None
            }
        }
    }

    /// Persist a [`ainl_memory::FailureNode`] after a tool returned `is_error` (execution, timeout, MCP, etc.).
    pub async fn record_tool_execution_failure(
        &self,
        tool_name: &str,
        message: &str,
        session_id: Option<&str>,
        memory_project_id: Option<&str>,
    ) -> Option<Uuid> {
        self.record_tool_execution_failure_with_source(
            tool_name,
            message,
            session_id,
            memory_project_id,
            None,
            None,
        )
        .await
    }

    /// Like [`Self::record_tool_execution_failure`], with optional MCP-style `source_namespace` / `source_tool` for analytics.
    pub async fn record_tool_execution_failure_with_source(
        &self,
        tool_name: &str,
        message: &str,
        session_id: Option<&str>,
        memory_project_id: Option<&str>,
        source_namespace: Option<&str>,
        source_tool: Option<&str>,
    ) -> Option<Uuid> {
        if !failure_learning_env_enabled() {
            return None;
        }
        let msg = openfang_types::truncate_str(message, 8000);
        let res = {
            let inner = self.inner.lock().await;
            let mut node = AinlMemoryNode::new_tool_execution_failure_with_source(
                tool_name,
                msg,
                session_id,
                source_namespace,
                source_tool,
            );
            node.agent_id = self.agent_id.clone();
            crate::memory_project_scope::apply_memory_project_id_to_node(
                &mut node,
                memory_project_id,
            );
            let id = node.id;
            inner.write_node(&node).map(|()| id)
        };
        match res {
            Ok(id) => {
                debug!(
                    agent_id = %self.agent_id,
                    failure_id = %id,
                    tool = %tool_name,
                    "AINL graph memory: tool execution failure written"
                );
                self.fire_write_hook(
                    "failure",
                    Some(GraphMemoryWriteProvenance {
                        node_ids: vec![id.to_string()],
                        node_kind: Some("failure".to_string()),
                        reason: Some("tool_runner:error".to_string()),
                        summary: Some(openfang_types::truncate_str(message, 200).to_string()),
                        trace_id: None,
                        tool_name: Some(tool_name.to_string()),
                    }),
                );
                Some(id)
            }
            Err(e) => {
                warn!(
                    agent_id = %self.agent_id,
                    error = %e,
                    "AINL graph memory: failed to write tool execution failure node"
                );
                None
            }
        }
    }

    /// Persist a [`ainl_memory::FailureNode`] for a tool rejected before dispatch (`kind`: e.g.
    /// `hook_blocked`, `param_validation` → source `agent_loop:{kind}`).
    pub async fn record_agent_loop_tool_precheck_failure(
        &self,
        kind: &str,
        tool_name: &str,
        message: &str,
        session_id: Option<&str>,
        memory_project_id: Option<&str>,
    ) -> Option<Uuid> {
        if !failure_learning_env_enabled() {
            return None;
        }
        let msg = openfang_types::truncate_str(message, 8000);
        let source = format!("agent_loop:{kind}");
        let res = {
            let inner = self.inner.lock().await;
            let mut node =
                AinlMemoryNode::new_agent_loop_precheck_failure(kind, tool_name, msg, session_id);
            node.agent_id = self.agent_id.clone();
            crate::memory_project_scope::apply_memory_project_id_to_node(
                &mut node,
                memory_project_id,
            );
            let id = node.id;
            inner.write_node(&node).map(|()| id)
        };
        match res {
            Ok(id) => {
                debug!(
                    agent_id = %self.agent_id,
                    failure_id = %id,
                    tool = %tool_name,
                    kind = %kind,
                    "AINL graph memory: agent-loop tool precheck failure written"
                );
                self.fire_write_hook(
                    "failure",
                    Some(GraphMemoryWriteProvenance {
                        node_ids: vec![id.to_string()],
                        node_kind: Some("failure".to_string()),
                        reason: Some(source),
                        summary: Some(openfang_types::truncate_str(message, 200).to_string()),
                        trace_id: None,
                        tool_name: Some(tool_name.to_string()),
                    }),
                );
                Some(id)
            }
            Err(e) => {
                warn!(
                    agent_id = %self.agent_id,
                    error = %e,
                    "AINL graph memory: failed to write agent-loop precheck failure node"
                );
                None
            }
        }
    }

    /// Persist a [`ainl_memory::FailureNode`] when `ainl-runtime` fails graph validation before a turn.
    pub async fn record_ainl_runtime_graph_validation_failure(
        &self,
        message: &str,
        session_id: Option<&str>,
        memory_project_id: Option<&str>,
    ) -> Option<Uuid> {
        if !failure_learning_env_enabled() {
            return None;
        }
        let msg = openfang_types::truncate_str(message, 8000);
        let res = {
            let inner = self.inner.lock().await;
            let mut node =
                AinlMemoryNode::new_ainl_runtime_graph_validation_failure(msg, session_id);
            node.agent_id = self.agent_id.clone();
            crate::memory_project_scope::apply_memory_project_id_to_node(
                &mut node,
                memory_project_id,
            );
            let id = node.id;
            inner.write_node(&node).map(|()| id)
        };
        match res {
            Ok(id) => {
                debug!(
                    agent_id = %self.agent_id,
                    failure_id = %id,
                    "AINL graph memory: ainl-runtime graph validation failure written"
                );
                self.fire_write_hook(
                    "failure",
                    Some(GraphMemoryWriteProvenance {
                        node_ids: vec![id.to_string()],
                        node_kind: Some("failure".to_string()),
                        reason: Some("ainl_runtime:graph_validation".to_string()),
                        summary: Some(openfang_types::truncate_str(message, 200).to_string()),
                        trace_id: None,
                        tool_name: None,
                    }),
                );
                Some(id)
            }
            Err(e) => {
                warn!(
                    agent_id = %self.agent_id,
                    error = %e,
                    "AINL graph memory: failed to write ainl-runtime graph validation failure node"
                );
                None
            }
        }
    }

    /// Record a procedural pattern node (named tool workflow).
    ///
    /// When `trace_id` is set, it is stored on [`ainl_memory::ProceduralNode::trace_id`] for export / Python bridge correlation.
    pub async fn record_pattern(
        &self,
        name: &str,
        tool_sequence: Vec<String>,
        confidence: f32,
        trace_id: Option<String>,
        memory_project_id: Option<&str>,
    ) {
        let seq_preview = tool_sequence.join(" → ");
        let res = {
            let inner = self.inner.lock().await;
            let found = match inner.find_procedural_by_tool_sequence(&self.agent_id, &tool_sequence)
            {
                Ok(n) => n,
                Err(e) => {
                    warn!(
                        agent_id = %self.agent_id,
                        error = %e,
                        "find_procedural_by_tool_sequence failed; writing new pattern row"
                    );
                    None
                }
            };
            if let Some(mut node) = found {
                if let AinlNodeType::Procedural { ref mut procedural } = node.node_type {
                    procedural.pattern_observation_count =
                        procedural.pattern_observation_count.saturating_add(1);
                    let new_ema =
                        pattern_promotion::ema_fitness_update(procedural.fitness, confidence);
                    procedural.fitness = Some(new_ema);
                    procedural.confidence = Some(confidence.clamp(0.0, 1.0));
                    if !name.is_empty() {
                        procedural.pattern_name = name.to_string();
                    }
                    if pattern_promotion::should_promote(
                        procedural.pattern_observation_count,
                        new_ema,
                    ) {
                        procedural.prompt_eligible = true;
                    }
                    if let Some(t) = trace_id.as_ref() {
                        procedural.trace_id = Some(t.clone());
                    }
                }
                crate::memory_project_scope::apply_memory_project_id_to_node(
                    &mut node,
                    memory_project_id,
                );
                let id = node.id;
                inner.write_node(&node).map(|()| id)
            } else {
                let mut node = AinlMemoryNode::new_procedural_tools(
                    name.to_string(),
                    tool_sequence,
                    confidence,
                );
                node.agent_id = self.agent_id.clone();
                crate::memory_project_scope::apply_memory_project_id_to_node(
                    &mut node,
                    memory_project_id,
                );
                if let AinlNodeType::Procedural { ref mut procedural } = node.node_type {
                    procedural.trace_id = trace_id.clone();
                }
                let id = node.id;
                inner.write_node(&node).map(|()| id)
            }
        };
        match res {
            Ok(id) => {
                debug!(
                    agent_id = %self.agent_id,
                    pattern_id = %id,
                    pattern_name = %name,
                    "AINL graph memory: procedural pattern written"
                );
                let summary = format!("Pattern “{name}”: {seq_preview}");
                self.fire_write_hook(
                    "procedural",
                    Some(GraphMemoryWriteProvenance {
                        node_ids: vec![id.to_string()],
                        node_kind: Some("procedural".to_string()),
                        reason: Some("pattern_persisted".to_string()),
                        summary: Some(summary),
                        trace_id,
                        tool_name: None,
                    }),
                );
            }
            Err(e) => warn!(
                agent_id = %self.agent_id,
                error = %e,
                "AINL graph memory: failed to write procedural pattern"
            ),
        }
    }

    /// Record a semantic fact learned during a turn.
    pub async fn record_fact(
        &self,
        fact: String,
        confidence: f32,
        source_turn_id: Uuid,
        memory_project_id: Option<&str>,
    ) {
        self.record_fact_with_tags(fact, confidence, source_turn_id, &[], memory_project_id)
            .await;
    }

    /// Like [`Self::record_fact`], with extra semantic tags (e.g. `trace_id:<uuid>` for correlation).
    pub async fn record_fact_with_tags(
        &self,
        fact: String,
        confidence: f32,
        source_turn_id: Uuid,
        extra_tags: &[String],
        memory_project_id: Option<&str>,
    ) {
        let res = {
            let inner = self.inner.lock().await;
            let mut node = AinlMemoryNode::new_fact(fact.clone(), confidence, source_turn_id);
            node.agent_id = self.agent_id.clone();
            crate::memory_project_scope::apply_memory_project_id_to_node(
                &mut node,
                memory_project_id,
            );
            if let AinlNodeType::Semantic { ref mut semantic } = node.node_type {
                semantic.source_episode_id = source_turn_id.to_string();
                for t in extra_tags {
                    if !semantic.tags.iter().any(|x| x == t) {
                        semantic.tags.push(t.clone());
                    }
                }
            }
            let id = node.id;
            inner.write_node(&node).map(|()| id)
        };
        match res {
            Ok(id) => {
                debug!(
                    agent_id = %self.agent_id,
                    fact_id = %id,
                    confidence = confidence,
                    "AINL graph memory: fact written"
                );
                let fact_preview = openfang_types::truncate_str(fact.as_str(), 160).to_string();
                self.fire_write_hook(
                    "fact",
                    Some(GraphMemoryWriteProvenance {
                        node_ids: vec![id.to_string()],
                        node_kind: Some("semantic".to_string()),
                        reason: Some("fact_extracted".to_string()),
                        summary: Some(format!("Fact: {fact_preview}")),
                        trace_id: None,
                        tool_name: None,
                    }),
                );
            }
            Err(e) => warn!(
                agent_id = %self.agent_id,
                error = %e,
                "AINL graph memory: failed to write fact"
            ),
        }
    }

    /// Export this agent’s subgraph as JSON (same shape as `ainl_memory::AgentGraphSnapshot`).
    pub async fn export_graph_json(&self) -> Result<serde_json::Value, String> {
        let inner = self.inner.lock().await;
        let snap = inner.export_graph(&self.agent_id)?;
        serde_json::to_value(&snap).map_err(|e| format!("export_graph json: {e}"))
    }

    /// Read `~/.armaraos/agents/<agent_id>/ainl_memory.db` and export the agent subgraph (blocking-friendly).
    pub fn export_graph_json_for_agent(agent_id: &str) -> Result<serde_json::Value, String> {
        let path = Self::db_path(agent_id)?;
        if !path.is_file() {
            return Err(format!(
                "AINL graph memory DB not found at {} (expected per-agent SQLite)",
                path.display()
            ));
        }
        let memory = GraphMemory::new(&path).map_err(|e| format!("open graph memory: {e}"))?;
        let snap = memory.export_graph(agent_id)?;
        serde_json::to_value(&snap).map_err(|e| format!("export_graph json: {e}"))
    }

    /// Write nodes produced by `ainl_agent_snapshot::apply_graph_writes` using a **synchronous**
    /// SQLite connection (same file as [`Self::open`]).
    ///
    /// Used from sync [`ainl_runtime::GraphPatchHostDispatch::on_patch_dispatch`] where the host
    /// cannot `.await` the async [`GraphMemoryWriter`] mutex. Dashboard live hooks are not fired
    /// here; the next graph-memory read sees the new rows.
    pub fn write_snapshot_nodes_sync_for_agent(
        agent_id: &str,
        nodes: &[AinlMemoryNode],
    ) -> Result<(), String> {
        let path = Self::db_path(agent_id)?;
        let memory = GraphMemory::new(&path).map_err(|e| format!("open graph memory: {e}"))?;
        for node in nodes {
            memory.write_node(node).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Record an A2A delegation as an EpisodeNode with delegation_to set.
    pub async fn record_delegation(&self, target_agent_id: &str, tool_calls: Vec<String>) {
        let _ = self
            .record_turn(
                tool_calls,
                Some(target_agent_id.to_string()),
                None,
                &[],
                None,
                None,
                None,
                None,
            )
            .await;
    }

    /// Recall recent episodes (last N seconds).
    pub async fn recall_recent(&self, seconds_ago: i64) -> Vec<ainl_memory::AinlMemoryNode> {
        let inner = self.inner.lock().await;
        inner.recall_recent(seconds_ago).unwrap_or_default()
    }

    /// Recall PersonaNode entries from AINL graph memory within the last N seconds.
    pub async fn recall_persona(&self, seconds_ago: i64) -> Vec<ainl_memory::AinlMemoryNode> {
        let inner = self.inner.lock().await;
        inner
            .recall_by_type(ainl_memory::AinlNodeKind::Persona, seconds_ago)
            .unwrap_or_default()
    }

    /// Persona payloads for `agent_id` within the lookback window (same SQL path as
    /// [`Self::recall_persona`], filtered to matching rows).
    pub async fn recall_persona_for_agent(
        &self,
        agent_id: &str,
        lookback_secs: i64,
    ) -> Vec<ainl_memory::PersonaNode> {
        self.recall_persona(lookback_secs)
            .await
            .into_iter()
            .filter(|n| n.agent_id == agent_id)
            .filter_map(|n| match n.node_type {
                ainl_memory::AinlNodeType::Persona { persona } => Some(persona),
                _ => None,
            })
            .collect()
    }

    /// Post-turn persona evolution: run `ainl-graph-extractor`’s [`GraphExtractorTask::run_pass`]
    /// (semantic recurrence, graph `RawSignal`s, `extract_pass`, ingest, optional evolution write).
    ///
    /// When the merged signal batch is empty (cold graph / no extractable cues), applies a
    /// lightweight [`ainl_persona::EvolutionEngine::correction_tick`] on every axis toward `0.5`
    /// and persists **only if** this agent already had at least one [`PersonaNode`] in the store
    /// before this pass — skipping the meaningless neutral bootstrap on a brand-new graph.
    ///
    /// **ArmaraOS / openfang-runtime:** this method is the **active** evolution write path for
    /// agents backed by `~/.armaraos/agents/<id>/ainl_memory.db`. Any other host that persists the
    /// same [`ainl_persona::EVOLUTION_TRAIT_NAME`] row for that DB (for example the `ainl-runtime`
    /// crate’s `AinlRuntime::persist_evolution_snapshot` / `evolve_persona_from_graph_signals`) must
    /// coordinate so those calls are not concurrent with this pass, or disable the other writer
    /// (see the `ainl-runtime` README, including optional `async` / `run_turn_async` and why the graph
    /// uses `Arc<std::sync::Mutex<GraphMemory>>` instead of `tokio::sync::Mutex`).
    ///
    /// Call after episode + fact writes so `GraphExtractor` sees fresh nodes. Intended to be
    /// `tokio::spawn`’d from the agent loop so the user-visible turn is not blocked.
    pub async fn run_persona_evolution_pass(&self) -> PersonaEvolutionExtractionReport {
        #[cfg(feature = "ainl-extractor")]
        {
            let report = {
                let inner = self.inner.lock().await;
                let store = inner.sqlite_store();
                let had_persona_before_pass = inner
                    .recall_by_type(
                        ainl_memory::AinlNodeKind::Persona,
                        PERSONA_PRIOR_LOOKBACK_SECS,
                    )
                    .unwrap_or_default()
                    .iter()
                    .any(|n| n.agent_id == self.agent_id);
                let mut task = GraphExtractorTask::new(&self.agent_id);
                let mut report = task.run_pass(store);

                if let Some(ref e) = report.extract_error {
                    warn!(
                        agent_id = %self.agent_id,
                        error = %e,
                        "graph extractor signal merge failed"
                    );
                }
                if let Some(ref e) = report.pattern_error {
                    warn!(
                        agent_id = %self.agent_id,
                        error = %e,
                        "graph extractor pattern persistence failed"
                    );
                }
                if let Some(ref e) = report.persona_error {
                    warn!(
                        agent_id = %self.agent_id,
                        error = %e,
                        "graph extractor persona evolution write failed"
                    );
                }

                if report.merged_signals.is_empty() {
                    #[cfg(feature = "ainl-persona-evolution")]
                    {
                        for ax in PersonaAxis::ALL {
                            task.evolution_engine.correction_tick(ax, 0.5);
                        }
                        let snapshot = task.evolution_engine.snapshot();
                        if had_persona_before_pass {
                            if let Err(e) =
                                task.evolution_engine.write_persona_node(store, &snapshot)
                            {
                                warn!(
                                    agent_id = %self.agent_id,
                                    error = %e,
                                    "graph extractor persona evolution write failed"
                                );
                                merge_persona_err(&mut report.persona_error, e);
                            }
                        }
                    }
                }
                report
            };

            if report.persona_error.is_none() {
                let reason = if report.merged_signals.is_empty() {
                    "persona_correction_tick"
                } else {
                    "graph_extractor_pass"
                };
                let summary = format!(
                    "Persona evolution: {} signals merged, {} semantic rows touched",
                    report.merged_signals.len(),
                    report.semantic_nodes_updated
                );
                self.fire_write_hook(
                    "persona",
                    Some(GraphMemoryWriteProvenance {
                        node_ids: vec![],
                        node_kind: Some("persona".to_string()),
                        reason: Some(reason.to_string()),
                        summary: Some(summary),
                        trace_id: None,
                        tool_name: None,
                    }),
                );
            }
            report
        }
        #[cfg(not(feature = "ainl-extractor"))]
        {
            PersonaEvolutionExtractionReport {
                agent_id: self.agent_id.clone(),
            }
        }
    }

    /// Background hygiene pass:
    /// - dedupe semantic facts by normalized fact text for this agent,
    /// - keep the highest-confidence (and newest tie-breaker) row,
    /// - drop duplicate rows to reduce stale/noisy recall.
    ///
    /// Returns the number of deleted semantic rows.
    pub async fn run_background_memory_consolidation(&self) -> usize {
        let now = chrono::Utc::now().timestamp();
        {
            let mut tr = match consolidation_tracker().lock() {
                Ok(g) => g,
                Err(_) => return 0,
            };
            if let Some(prev) = tr.get(&self.agent_id) {
                if now - *prev < CONSOLIDATION_MIN_INTERVAL_SECS {
                    return 0;
                }
            }
            tr.insert(self.agent_id.clone(), now);
        }

        let mut deleted = 0usize;
        let mut seen: HashSet<String> = HashSet::new();
        let mut ids_to_delete: HashSet<String> = HashSet::new();
        {
            let mut inner = self.inner.lock().await;
            let mut rows: Vec<(f32, String, Uuid)> = inner
                .recall_by_type(ainl_memory::AinlNodeKind::Semantic, 60 * 60 * 24 * 365)
                .unwrap_or_default()
                .into_iter()
                .filter_map(|n| {
                    if n.agent_id != self.agent_id {
                        return None;
                    }
                    match &n.node_type {
                        AinlNodeType::Semantic { semantic } => Some((
                            semantic.confidence,
                            semantic.fact.trim().to_ascii_lowercase(),
                            n.id,
                        )),
                        _ => None,
                    }
                })
                .collect();
            rows.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
            for (_conf, key, id) in rows {
                if key.is_empty() {
                    continue;
                }
                if !seen.insert(key) {
                    ids_to_delete.insert(id.to_string());
                }
            }
            if !ids_to_delete.is_empty() {
                if let Ok(mut snapshot) = inner.export_graph(&self.agent_id) {
                    let before = snapshot.nodes.len();
                    snapshot
                        .nodes
                        .retain(|n| !ids_to_delete.contains(&n.id.to_string()));
                    if snapshot.nodes.len() < before {
                        let _ = inner.import_graph(&snapshot, false);
                        deleted = before - snapshot.nodes.len();
                    }
                }
            }
        }
        if deleted > 0 {
            self.fire_write_hook(
                "fact",
                Some(GraphMemoryWriteProvenance {
                    node_ids: vec![],
                    node_kind: Some("semantic".to_string()),
                    reason: Some("background_consolidation".to_string()),
                    summary: Some(format!(
                        "Consolidation removed {deleted} duplicate semantic row(s)"
                    )),
                    trace_id: None,
                    tool_name: None,
                }),
            );
            debug!(
                agent_id = %self.agent_id,
                deleted = deleted,
                "AINL graph memory: background consolidation removed duplicate semantic rows"
            );
        }
        deleted
    }
}

#[cfg(feature = "ainl-extractor")]
fn merge_persona_err(slot: &mut Option<String>, e: String) {
    match slot {
        None => *slot = Some(e),
        Some(prev) => {
            prev.push_str("; correction write: ");
            prev.push_str(&e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ainl_contracts::TrajectoryOutcome;
    use ainl_memory::AinlNodeType;
    #[cfg(feature = "ainl-persona-evolution")]
    use ainl_persona::EVOLUTION_TRAIT_NAME;
    use openfang_types::agent::AgentManifest;
    use serde_json::json;
    use std::sync::Mutex as StdMutex;

    fn env_lock() -> &'static tokio::sync::Mutex<()> {
        crate::runtime_env_test_lock()
    }

    fn restore_trajectory_env(prev: Option<String>) {
        if let Some(p) = prev {
            std::env::set_var("AINL_TRAJECTORY_ENABLED", p);
        } else {
            std::env::remove_var("AINL_TRAJECTORY_ENABLED");
        }
    }

    fn restore_source_hash_env(prev_src: Option<String>, prev_bundle: Option<String>) {
        match prev_src {
            Some(p) => std::env::set_var("AINL_SOURCE_HASH", p),
            None => std::env::remove_var("AINL_SOURCE_HASH"),
        }
        match prev_bundle {
            Some(p) => std::env::set_var("AINL_BUNDLE_SHA256", p),
            None => std::env::remove_var("AINL_BUNDLE_SHA256"),
        }
    }

    #[tokio::test]
    async fn ainl_source_hash_prefers_env_then_manifest_metadata() {
        let _guard = env_lock().lock().await;
        let prev_src = std::env::var("AINL_SOURCE_HASH").ok();
        let prev_bundle = std::env::var("AINL_BUNDLE_SHA256").ok();
        std::env::remove_var("AINL_SOURCE_HASH");
        std::env::remove_var("AINL_BUNDLE_SHA256");

        let mut m = AgentManifest::default();
        m.metadata.insert(
            "ainl_source_hash".into(),
            serde_json::Value::String("from-manifest".into()),
        );
        assert_eq!(
            ainl_source_hash_for_trajectory_persist(&m).as_deref(),
            Some("from-manifest")
        );

        std::env::set_var("AINL_SOURCE_HASH", "  env-val  ");
        assert_eq!(
            ainl_source_hash_for_trajectory_persist(&m).as_deref(),
            Some("env-val")
        );

        restore_source_hash_env(prev_src, prev_bundle);
    }

    #[tokio::test]
    #[cfg(feature = "ainl-persona-evolution")]
    async fn correction_tick_all_axes_no_panic() {
        let mut engine = ainl_persona::EvolutionEngine::new("tick-test-agent");
        for ax in ainl_persona::PersonaAxis::ALL {
            engine.correction_tick(ax, 0.5);
        }
        let snap = engine.snapshot();
        assert_eq!(snap.agent_id, "tick-test-agent");
    }

    #[tokio::test]
    #[cfg(feature = "ainl-extractor")]
    async fn persona_evolution_writes_evolution_node_after_tool_turn() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("evo_persona.db");
        let memory = GraphMemory::new(&db_path).expect("open");
        let writer = GraphMemoryWriter {
            inner: Arc::new(Mutex::new(memory)),
            agent_id: "evo-agent".to_string(),
            on_write: None,
        };

        {
            let inner = writer.inner.lock().await;
            inner
                .write_persona("seed_trait", 0.6, vec![])
                .expect("seed baseline persona so evolution can persist on cold signal passes");
        }

        assert!(writer
            .record_turn(
                vec!["shell_exec".into()],
                None,
                Some(json!({ "outcome": "success" })),
                &[],
                None,
                None,
                None,
                None,
            )
            .await
            .is_some());
        let evolve_report = writer.run_persona_evolution_pass().await;
        assert!(
            !evolve_report.has_errors(),
            "unexpected extraction errors: {evolve_report:?}"
        );

        let nodes = writer.recall_persona(3600).await;
        let evo = nodes.iter().find(|n| {
            matches!(
                &n.node_type,
                ainl_memory::AinlNodeType::Persona { persona }
                    if persona.trait_name == EVOLUTION_TRAIT_NAME
            )
        });
        assert!(
            evo.is_some(),
            "expected evolution PersonaNode, got {nodes:?}"
        );
        let ainl_memory::AinlNodeType::Persona { persona } = &evo.unwrap().node_type else {
            panic!();
        };
        assert!(
            !persona.axis_scores.is_empty(),
            "axis_scores should be populated"
        );
    }

    #[tokio::test]
    #[cfg(feature = "ainl-extractor")]
    async fn persona_evolution_cycle_increments_over_two_passes() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("evo_cycle.db");
        let memory = GraphMemory::new(&db_path).expect("open");
        let writer = GraphMemoryWriter {
            inner: Arc::new(Mutex::new(memory)),
            agent_id: "evo-cycle-agent".to_string(),
            on_write: None,
        };

        {
            let inner = writer.inner.lock().await;
            inner
                .write_persona("seed_trait", 0.6, vec![])
                .expect("seed baseline persona so evolution can persist on cold signal passes");
        }

        async fn evolution_cycle(w: &GraphMemoryWriter) -> u32 {
            let nodes = w.recall_persona(3600).await;
            nodes
                .iter()
                .find_map(|n| {
                    if let ainl_memory::AinlNodeType::Persona { persona } = &n.node_type {
                        (persona.trait_name == EVOLUTION_TRAIT_NAME)
                            .then_some(persona.evolution_cycle)
                    } else {
                        None
                    }
                })
                .unwrap_or(0)
        }

        writer
            .record_turn(
                vec!["shell_exec".into()],
                None,
                Some(json!({ "outcome": "success" })),
                &[],
                None,
                None,
                None,
                None,
            )
            .await;
        let r1 = writer.run_persona_evolution_pass().await;
        assert!(!r1.has_errors(), "{r1:?}");
        let c1 = evolution_cycle(&writer).await;
        assert!(c1 >= 1);

        writer
            .record_turn(
                vec!["web_search".into()],
                None,
                Some(json!({ "outcome": "success" })),
                &[],
                None,
                None,
                None,
                None,
            )
            .await;
        let r2_pass = writer.run_persona_evolution_pass().await;
        assert!(!r2_pass.has_errors(), "{r2_pass:?}");
        let c2 = evolution_cycle(&writer).await;
        assert!(
            c2 > c1,
            "evolution_cycle should increase, got {c1} then {c2}"
        );
    }

    #[tokio::test]
    async fn test_graph_memory_writer_records_episode() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("test_ainl.db");

        // Use from_path directly for testing
        let memory = GraphMemory::new(&db_path).expect("open");
        let writer = GraphMemoryWriter {
            inner: Arc::new(Mutex::new(memory)),
            agent_id: "test-agent".to_string(),
            on_write: None,
        };

        assert!(writer
            .record_turn(
                vec!["web_search".to_string(), "file_read".to_string()],
                None,
                None,
                &[],
                None,
                None,
                None,
                None,
            )
            .await
            .is_some());

        let recent = writer.recall_recent(60).await;
        assert_eq!(recent.len(), 1);

        if let ainl_memory::AinlNodeType::Episode { episodic } = &recent[0].node_type {
            assert_eq!(episodic.tool_calls.len(), 2);
            assert!(episodic.tool_calls.contains(&"web_search".to_string()));
        } else {
            panic!("wrong node type");
        }
    }

    #[tokio::test]
    async fn emit_write_observed_triggers_notify_hook() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("notify_hook.db");
        let memory = GraphMemory::new(&db_path).expect("open");
        let writes: Arc<StdMutex<Vec<String>>> = Arc::new(StdMutex::new(Vec::new()));
        let writes_for_hook = Arc::clone(&writes);
        let hook: Arc<dyn Fn(String, String, Option<GraphMemoryWriteProvenance>) + Send + Sync> =
            Arc::new(
                move |_agent_id: String,
                      kind: String,
                      _prov: Option<GraphMemoryWriteProvenance>| {
                    if let Ok(mut v) = writes_for_hook.lock() {
                        v.push(kind);
                    }
                },
            );
        let writer = GraphMemoryWriter {
            inner: Arc::new(Mutex::new(memory)),
            agent_id: "notify-agent".to_string(),
            on_write: Some(hook),
        };

        writer.emit_write_observed("episode", None);

        let seen = writes.lock().unwrap().clone();
        assert_eq!(seen, vec!["episode".to_string()]);
    }

    #[tokio::test]
    async fn test_graph_memory_writer_records_delegation() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("test_ainl_deleg.db");
        let memory = GraphMemory::new(&db_path).expect("open");
        let writer = GraphMemoryWriter {
            inner: Arc::new(Mutex::new(memory)),
            agent_id: "test-agent".to_string(),
            on_write: None,
        };

        writer
            .record_delegation("agent-B", vec!["delegate".to_string()])
            .await;

        let recent = writer.recall_recent(60).await;
        assert_eq!(recent.len(), 1);
        if let ainl_memory::AinlNodeType::Episode { episodic } = &recent[0].node_type {
            assert_eq!(episodic.delegation_to, Some("agent-B".to_string()));
        } else {
            panic!("wrong node type");
        }
    }

    #[tokio::test]
    async fn export_graph_json_round_trips_after_episode_write() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("export_rt.db");
        let memory = GraphMemory::new(&db_path).expect("open");
        let writer = GraphMemoryWriter {
            inner: Arc::new(Mutex::new(memory)),
            agent_id: "export-agent".to_string(),
            on_write: None,
        };
        let trace = json!({"agent_id": "export-agent", "trace_id": "tr-abc"});
        assert!(writer
            .record_turn(
                vec!["shell_exec".into()],
                None,
                Some(trace.clone()),
                &[],
                None,
                None,
                None,
                None,
            )
            .await
            .is_some());
        let v = writer.export_graph_json().await.expect("export");
        assert_eq!(v["agent_id"], "export-agent");
        assert_eq!(v["schema_version"], "1.0");
        let nodes = v["nodes"].as_array().expect("nodes array");
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0]["agent_id"], "export-agent");
        let nt = &nodes[0]["node_type"];
        assert_eq!(nt["type"], "episode");
        assert_eq!(nt["trace_event"], trace);
    }

    #[tokio::test]
    async fn test_record_turn_writes_follows_edge_between_episodes() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("follows_test.db");
        let memory = GraphMemory::new(&db_path).expect("open");
        let writer = GraphMemoryWriter {
            inner: Arc::new(Mutex::new(memory)),
            agent_id: "test-agent".to_string(),
            on_write: None,
        };

        let id1 = writer
            .record_turn(
                vec!["a".to_string()],
                None,
                None,
                &[],
                None,
                None,
                None,
                None,
            )
            .await
            .expect("ep1");
        let id2 = writer
            .record_turn(
                vec!["b".to_string()],
                None,
                None,
                &[],
                None,
                None,
                None,
                None,
            )
            .await
            .expect("ep2");

        let store = GraphMemory::new(&db_path).expect("reopen");
        let prev = store.store().walk_edges(id2, "follows").expect("walk");
        assert_eq!(prev.len(), 1);
        assert_eq!(prev[0].id, id1);
    }

    #[tokio::test]
    async fn test_recall_persona_returns_persona_nodes() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("persona_test.db");
        let memory = GraphMemory::new(&db_path).expect("open");
        let writer = GraphMemoryWriter {
            inner: Arc::new(Mutex::new(memory)),
            agent_id: "test".to_string(),
            on_write: None,
        };

        {
            let inner = writer.inner.lock().await;
            inner
                .write_persona("prefers_brevity", 0.9, vec![])
                .expect("write persona");
        }

        let nodes = writer.recall_persona(3600).await;
        assert_eq!(nodes.len(), 1);
        if let ainl_memory::AinlNodeType::Persona { persona } = &nodes[0].node_type {
            assert_eq!(persona.trait_name, "prefers_brevity");
            assert!((persona.strength - 0.9).abs() < 0.01);
        } else {
            panic!("wrong node type");
        }
    }

    #[tokio::test]
    async fn record_pattern_with_trace_id_stores_trace() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("record_pat_trace.db");
        let memory = GraphMemory::new(&db_path).expect("open");
        let writer = GraphMemoryWriter {
            inner: Arc::new(Mutex::new(memory)),
            agent_id: "pat-trace-agent".to_string(),
            on_write: None,
        };
        writer
            .record_pattern(
                "demo_pat",
                vec!["tool_a".into(), "tool_b".into()],
                0.85,
                Some("trace-z99".into()),
                None,
            )
            .await;
        let v = writer.export_graph_json().await.expect("export");
        let nodes = v["nodes"].as_array().expect("nodes");
        let proc_json = nodes
            .iter()
            .find(|n| n["node_type"]["type"] == "procedural")
            .expect("procedural in export");
        assert_eq!(proc_json["node_type"]["trace_id"], "trace-z99");
    }

    #[tokio::test]
    async fn record_pattern_merge_updates_single_row_and_promotes() {
        let _guard = env_lock().lock().await;
        let prev_min = std::env::var("AINL_PATTERN_PROMOTION_MIN_OBSERVATIONS").ok();
        let prev_floor = std::env::var("AINL_PATTERN_PROMOTION_FITNESS_FLOOR").ok();
        std::env::set_var("AINL_PATTERN_PROMOTION_MIN_OBSERVATIONS", "3");
        std::env::set_var("AINL_PATTERN_PROMOTION_FITNESS_FLOOR", "0.7");

        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("record_pat_merge.db");
        let memory = GraphMemory::new(&db_path).expect("open");
        let writer = GraphMemoryWriter {
            inner: Arc::new(Mutex::new(memory)),
            agent_id: "pat-merge-agent".to_string(),
            on_write: None,
        };
        let seq = vec!["tool_a".into(), "tool_b".into()];
        for _ in 0..3 {
            writer
                .record_pattern("demo_pat", seq.clone(), 0.85, None, None)
                .await;
        }
        let v = writer.export_graph_json().await.expect("export");
        let nodes = v["nodes"].as_array().expect("nodes");
        let procedurals: Vec<_> = nodes
            .iter()
            .filter(|n| n["node_type"]["type"] == "procedural")
            .collect();
        assert_eq!(
            procedurals.len(),
            1,
            "expected merged single procedural row"
        );
        assert_eq!(procedurals[0]["node_type"]["prompt_eligible"], true);
        assert_eq!(procedurals[0]["node_type"]["pattern_observation_count"], 3);

        match prev_min {
            Some(p) => std::env::set_var("AINL_PATTERN_PROMOTION_MIN_OBSERVATIONS", p),
            None => std::env::remove_var("AINL_PATTERN_PROMOTION_MIN_OBSERVATIONS"),
        }
        match prev_floor {
            Some(p) => std::env::set_var("AINL_PATTERN_PROMOTION_FITNESS_FLOOR", p),
            None => std::env::remove_var("AINL_PATTERN_PROMOTION_FITNESS_FLOOR"),
        }
    }

    #[tokio::test]
    async fn trajectory_env_enabled_opt_out_semantics() {
        let _guard = env_lock().lock().await;
        let prev = std::env::var("AINL_TRAJECTORY_ENABLED").ok();
        std::env::remove_var("AINL_TRAJECTORY_ENABLED");
        assert!(trajectory_env_enabled());
        std::env::set_var("AINL_TRAJECTORY_ENABLED", "0");
        assert!(!trajectory_env_enabled());
        std::env::set_var("AINL_TRAJECTORY_ENABLED", "false");
        assert!(!trajectory_env_enabled());
        std::env::set_var("AINL_TRAJECTORY_ENABLED", " no ");
        assert!(!trajectory_env_enabled());
        std::env::set_var("AINL_TRAJECTORY_ENABLED", "1");
        assert!(trajectory_env_enabled());
        restore_trajectory_env(prev);
    }

    #[tokio::test]
    async fn record_trajectory_for_episode_writes_node_and_trajectory_of_edge() {
        let _guard = env_lock().lock().await;
        let prev = std::env::var("AINL_TRAJECTORY_ENABLED").ok();
        std::env::remove_var("AINL_TRAJECTORY_ENABLED");

        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("traj_path.db");
        let memory = GraphMemory::new(&db_path).expect("open");
        let hooks: Arc<StdMutex<Vec<String>>> = Arc::new(StdMutex::new(Vec::new()));
        let hooks_for_cb = Arc::clone(&hooks);
        let hook: GraphMemoryWriteNotifyFn = Arc::new(
            move |_agent: String, kind: String, _prov: Option<GraphMemoryWriteProvenance>| {
                if let Ok(mut v) = hooks_for_cb.lock() {
                    v.push(kind);
                }
            },
        );
        let writer = GraphMemoryWriter {
            inner: Arc::new(Mutex::new(memory)),
            agent_id: "traj-agent".to_string(),
            on_write: Some(hook),
        };

        let tools = vec!["tool_a".to_string(), "tool_b".to_string()];
        let episode_id = writer
            .record_turn(tools.clone(), None, None, &[], None, None, None, None)
            .await
            .expect("episode");

        let traj_id = writer
            .record_trajectory_for_episode(
                episode_id,
                &tools,
                None,
                TrajectoryOutcome::Success,
                "sess-ci",
                Some("proj-ci"),
                0,
                None,
                None,
                None,
            )
            .await
            .expect("trajectory id");

        let gm = GraphMemory::new(&db_path).expect("reopen");
        let targets = gm
            .store()
            .walk_edges(traj_id, "trajectory_of")
            .expect("walk trajectory_of");
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].id, episode_id);

        let traj_node = gm.store().read_node(traj_id).expect("read").expect("traj");
        match &traj_node.node_type {
            AinlNodeType::Trajectory { trajectory } => {
                assert_eq!(trajectory.episode_id, episode_id);
                assert_eq!(trajectory.session_id, "sess-ci");
                assert_eq!(trajectory.project_id.as_deref(), Some("proj-ci"));
                assert_eq!(trajectory.steps.len(), 2);
                assert_eq!(trajectory.steps[0].operation, "tool_a");
                assert_eq!(trajectory.steps[1].operation, "tool_b");
                assert_eq!(trajectory.outcome, TrajectoryOutcome::Success);
            }
            _ => panic!("expected Trajectory node"),
        }

        let kinds = hooks.lock().expect("hook lock").clone();
        assert!(kinds.iter().any(|k| k == "trajectory"));

        restore_trajectory_env(prev);
    }

    #[tokio::test]
    async fn record_trajectory_for_episode_noop_when_env_disabled() {
        let _guard = env_lock().lock().await;
        let prev = std::env::var("AINL_TRAJECTORY_ENABLED").ok();
        std::env::set_var("AINL_TRAJECTORY_ENABLED", "off");

        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("traj_off.db");
        let memory = GraphMemory::new(&db_path).expect("open");
        let writer = GraphMemoryWriter {
            inner: Arc::new(Mutex::new(memory)),
            agent_id: "traj-off-agent".to_string(),
            on_write: None,
        };

        let episode_id = writer
            .record_turn(vec!["only".into()], None, None, &[], None, None, None, None)
            .await
            .expect("episode");

        let out = writer
            .record_trajectory_for_episode(
                episode_id,
                &[String::from("only")],
                None,
                TrajectoryOutcome::Success,
                "sess",
                None,
                0,
                None,
                None,
                None,
            )
            .await;
        assert!(out.is_none());

        let gm = GraphMemory::new(&db_path).expect("reopen");
        assert!(gm
            .store()
            .find_by_type("trajectory")
            .expect("query")
            .is_empty());

        restore_trajectory_env(prev);
    }
}
