//! Built-in tool execution.
//!
//! Provides filesystem, web, shell, and inter-agent tools. Agent tools
//! (agent_send, agent_spawn, etc.) require a KernelHandle to be passed in.

use crate::kernel_handle::KernelHandle;
use crate::mcp;
use crate::web_search::{parse_ddg_results, WebToolsContext};
use openfang_skills::registry::SkillRegistry;
use openfang_types::orchestration::orchestration_context_from_claimed_task;
use openfang_types::taint::{TaintLabel, TaintSink, TaintedValue};
use openfang_types::task_queue::TaskClaimStrategy;
use openfang_types::tool::{ToolDefinition, ToolResult};
use openfang_types::tool_compat::normalize_tool_name;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{debug, warn};

/// Fallback max inter-agent depth when not running inside an agent task scope.
const DEFAULT_MAX_AGENT_CALL_DEPTH: u32 = 5;

/// Helper to get efficient_mode from orchestration context for inheritance.
/// Returns the existing orchestration context's efficient_mode, or None if starting a new tree.
/// When None, child agents will use their manifest metadata or global config default.
fn get_efficient_mode(
    orchestration_ctx: Option<&openfang_types::orchestration::OrchestrationContext>,
) -> Option<String> {
    orchestration_ctx.and_then(|ctx| ctx.efficient_mode.clone())
}

/// Check if a tool name refers to a shell execution tool.
///
/// Used to determine whether exec_policy settings should bypass the approval gate.
fn is_shell_tool(name: &str) -> bool {
    name == "shell_exec"
}

/// Check if a shell command should be blocked by taint tracking.
///
/// Layer 1: Shell metacharacter injection (backticks, `$(`, `${`, etc.)
/// Layer 2: Heuristic patterns for injected external data (piped curl, base64, eval)
///
/// This implements the TaintSink::shell_exec() policy from SOTA 2.
fn check_taint_shell_exec(command: &str) -> Option<String> {
    // Layer 1: Block shell metacharacters that enable command injection.
    // Uses the same validator as subprocess_sandbox and docker_sandbox.
    if let Some(reason) = crate::subprocess_sandbox::contains_shell_metacharacters(command) {
        return Some(format!("Shell metacharacter injection blocked: {reason}"));
    }

    // Layer 2: Heuristic patterns for injected external URLs / base64 payloads
    let suspicious_patterns = ["curl ", "wget ", "| sh", "| bash", "base64 -d", "eval "];
    for pattern in &suspicious_patterns {
        if command.contains(pattern) {
            let mut labels = HashSet::new();
            labels.insert(TaintLabel::ExternalNetwork);
            let tainted = TaintedValue::new(command, labels, "llm_tool_call");
            if let Err(violation) = tainted.check_sink(&TaintSink::shell_exec()) {
                warn!(command = crate::str_utils::safe_truncate_str(command, 80), %violation, "Shell taint check failed");
                return Some(violation.to_string());
            }
        }
    }
    None
}

/// Check if a URL should be blocked by taint tracking before network fetch.
///
/// Blocks URLs that appear to contain API keys, tokens, or other secrets
/// in query parameters (potential data exfiltration). Implements TaintSink::net_fetch().
fn check_taint_net_fetch(url: &str) -> Option<String> {
    let exfil_patterns = [
        "api_key=",
        "apikey=",
        "token=",
        "secret=",
        "password=",
        "Authorization:",
    ];
    for pattern in &exfil_patterns {
        if url.to_lowercase().contains(&pattern.to_lowercase()) {
            let mut labels = HashSet::new();
            labels.insert(TaintLabel::Secret);
            let tainted = TaintedValue::new(url, labels, "llm_tool_call");
            if let Err(violation) = tainted.check_sink(&TaintSink::net_fetch()) {
                warn!(url = crate::str_utils::safe_truncate_str(url, 80), %violation, "Net fetch taint check failed");
                return Some(violation.to_string());
            }
        }
    }
    None
}

/// Detect hallucinated `shell_exec` calls that are actually direct MCP tool names.
///
/// Some models occasionally emit `shell_exec` with `command: "mcp_server_tool"` instead of
/// calling the MCP tool directly. When that command is a single token with the `mcp_` prefix,
/// we transparently re-route to the MCP tool path.
fn direct_mcp_tool_from_shell_command(input: &serde_json::Value) -> Option<String> {
    let raw = input.get("command")?.as_str()?.trim();
    if raw.is_empty() {
        return None;
    }
    let candidate = raw.trim_matches(|c| c == '"' || c == '\'').trim();
    if candidate.split_whitespace().count() != 1 {
        return None;
    }
    if !mcp::is_mcp_tool(candidate) {
        return None;
    }
    Some(candidate.to_string())
}

/// Extract missing required top-level keys from a tool's JSON schema.
fn missing_required_schema_keys(
    input_schema: &serde_json::Value,
    input: &serde_json::Value,
) -> Vec<String> {
    let required = input_schema
        .get("required")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let input_obj = input.as_object();
    required
        .iter()
        .filter_map(|v| v.as_str())
        .filter(|key| input_obj.is_none_or(|obj| !obj.contains_key(*key)))
        .map(|s| s.to_string())
        .collect()
}

async fn dispatch_mcp_tool_by_name(
    tool_name: &str,
    input: &serde_json::Value,
    mcp_connections: Option<&tokio::sync::Mutex<Vec<mcp::McpConnection>>>,
) -> Result<String, String> {
    if !mcp::is_mcp_tool(tool_name) {
        return Err(format!("Invalid MCP tool name: {tool_name}"));
    }
    let Some(mcp_conns) = mcp_connections else {
        return Err(format!("MCP not available for tool: {tool_name}"));
    };

    let mut conns = mcp_conns.lock().await;
    let known_names: Vec<String> = conns.iter().map(|c| c.name().to_string()).collect();
    let known_refs: Vec<&str> = known_names.iter().map(|s| s.as_str()).collect();
    let Some(server_name) = mcp::extract_mcp_server_from_known(tool_name, &known_refs) else {
        return Err(format!("Invalid MCP tool name: {tool_name}"));
    };

    let Some(conn) = conns.iter_mut().find(|c| c.name() == server_name) else {
        return Err(format!("MCP server '{server_name}' not connected"));
    };

    let missing_required = conn
        .tools()
        .iter()
        .find(|d| d.name == tool_name)
        .map(|d| missing_required_schema_keys(&d.input_schema, input))
        .unwrap_or_default();
    if !missing_required.is_empty() {
        return Err(format!(
            "MCP tool '{tool_name}' missing required arguments: {}",
            missing_required.join(", ")
        ));
    }

    debug!(
        tool = tool_name,
        server = server_name,
        "Dispatching to MCP server"
    );
    conn.call_tool(tool_name, input)
        .await
        .map_err(|e| format!("MCP tool call failed: {e}"))
}

async fn tool_mcp_resource_read(
    input: &serde_json::Value,
    mcp_connections: Option<&tokio::sync::Mutex<Vec<mcp::McpConnection>>>,
) -> Result<String, String> {
    let server = input
        .get("mcp_server")
        .or_else(|| input.get("server"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing required field: mcp_server (or server)".to_string())?;
    let uri = input
        .get("uri")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing required field: uri".to_string())?;
    const DEFAULT_MAX: usize = 65_536;
    let max_bytes = input
        .get("max_bytes")
        .and_then(serde_json::Value::as_u64)
        .map(|n| n.min(2_000_000) as usize)
        .unwrap_or(DEFAULT_MAX);
    let offset = input
        .get("offset")
        .or_else(|| input.get("char_offset"))
        .and_then(serde_json::Value::as_u64)
        .map(|n| n as usize)
        .unwrap_or(0);
    let allow_binary = input
        .get("allow_binary")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let Some(mcp_mtx) = mcp_connections else {
        return Err("MCP is not available in this context".to_string());
    };
    let mut conns = mcp_mtx.lock().await;
    let Some(conn) = conns.iter_mut().find(|c| c.name() == server) else {
        return Err(format!("MCP server '{server}' is not connected"));
    };
    conn.read_resource_by_uri_limited(uri, max_bytes, offset, allow_binary)
        .await
}

tokio::task_local! {
    /// Tracks the current inter-agent call depth within a task.
    static AGENT_CALL_DEPTH: std::cell::Cell<u32>;
    /// Effective max depth for this agent turn (from `[runtime_limits]` + manifest).
    pub static MAX_AGENT_CALL_DEPTH_LIMIT: std::cell::Cell<u32>;
    /// Canvas max HTML size in bytes (set from kernel config at loop start).
    pub static CANVAS_MAX_BYTES: usize;
}

/// Shared orchestration context for the current agent turn (wall-clock budget + `shared_vars`).
pub type OrchestrationLive =
    Arc<tokio::sync::RwLock<openfang_types::orchestration::OrchestrationContext>>;

async fn orch_snapshot(
    orch: Option<&OrchestrationLive>,
) -> Option<openfang_types::orchestration::OrchestrationContext> {
    match orch {
        Some(a) => Some(a.read().await.clone()),
        None => None,
    }
}

fn effective_max_agent_call_depth() -> u32 {
    MAX_AGENT_CALL_DEPTH_LIMIT
        .try_with(|c| c.get())
        .unwrap_or(DEFAULT_MAX_AGENT_CALL_DEPTH)
        .max(1)
}

/// Get the current inter-agent call depth from the task-local context.
/// Returns 0 if called outside an agent task.
pub fn current_agent_depth() -> u32 {
    AGENT_CALL_DEPTH.try_with(|d| d.get()).unwrap_or(0)
}

/// Execute a tool by name with the given input, returning a ToolResult.
///
/// The optional `kernel` handle enables inter-agent tools. If `None`,
/// agent tools will return an error indicating the kernel is not available.
///
/// `allowed_tools` enforces capability-based security: if provided, only
/// tools in the list may execute. This prevents an LLM from hallucinating
/// tool names outside the agent's capability grants.
///
/// `ainl_library_root`: host `~/.../ainl-library` path for virtual `ainl-library/...` reads
/// (`file_read`, `file_list`, `document_extract`).
#[allow(clippy::too_many_arguments)]
pub async fn execute_tool(
    tool_use_id: &str,
    tool_name: &str,
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    allowed_tools: Option<&[String]>,
    caller_agent_id: Option<&str>,
    skill_registry: Option<&SkillRegistry>,
    mcp_connections: Option<&tokio::sync::Mutex<Vec<mcp::McpConnection>>>,
    web_ctx: Option<&WebToolsContext>,
    browser_ctx: Option<&crate::browser::BrowserManager>,
    allowed_env_vars: Option<&[String]>,
    workspace_root: Option<&Path>,
    ainl_library_root: Option<&Path>,
    media_engine: Option<&crate::media_understanding::MediaEngine>,
    exec_policy: Option<&openfang_types::config::ExecPolicy>,
    tts_engine: Option<&crate::tts::TtsEngine>,
    docker_config: Option<&openfang_types::config::DockerSandboxConfig>,
    process_manager: Option<&crate::process_manager::ProcessManager>,
    orchestration_live: Option<&OrchestrationLive>,
) -> ToolResult {
    execute_tool_with_trajectory(
        tool_use_id,
        tool_name,
        input,
        kernel,
        allowed_tools,
        caller_agent_id,
        skill_registry,
        mcp_connections,
        web_ctx,
        browser_ctx,
        allowed_env_vars,
        workspace_root,
        ainl_library_root,
        media_engine,
        exec_policy,
        tts_engine,
        docker_config,
        process_manager,
        orchestration_live,
        None,
    )
    .await
}

/// Like [`execute_tool`] with optional per-slot trajectory recording (OpenFang self-learning).
#[allow(clippy::too_many_arguments)]
pub async fn execute_tool_with_trajectory(
    tool_use_id: &str,
    tool_name: &str,
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    allowed_tools: Option<&[String]>,
    caller_agent_id: Option<&str>,
    skill_registry: Option<&SkillRegistry>,
    mcp_connections: Option<&tokio::sync::Mutex<Vec<mcp::McpConnection>>>,
    web_ctx: Option<&WebToolsContext>,
    browser_ctx: Option<&crate::browser::BrowserManager>,
    allowed_env_vars: Option<&[String]>,
    workspace_root: Option<&Path>,
    ainl_library_root: Option<&Path>,
    media_engine: Option<&crate::media_understanding::MediaEngine>,
    exec_policy: Option<&openfang_types::config::ExecPolicy>,
    tts_engine: Option<&crate::tts::TtsEngine>,
    docker_config: Option<&openfang_types::config::DockerSandboxConfig>,
    process_manager: Option<&crate::process_manager::ProcessManager>,
    orchestration_live: Option<&OrchestrationLive>,
    trajectory_capture: Option<(
        std::sync::Arc<crate::trajectory_turn::TrajectoryTurnBuffer>,
        usize,
    )>,
) -> ToolResult {
    // Normalize the tool name through compat mappings so LLM-hallucinated aliases
    // (e.g. "fs-write" → "file_write") resolve to the canonical OpenFang name.
    let tool_name = normalize_tool_name(tool_name);
    let t_tool = std::time::Instant::now();
    let finish_traj = |tr: ToolResult| -> ToolResult {
        crate::trajectory_turn::record_trajectory_tool_step(
            &trajectory_capture,
            tool_name,
            tool_use_id,
            t_tool,
            &tr,
        );
        tr
    };

    // Auto-correct a common hallucination where models run MCP tools via shell_exec.
    // Re-route before capability checks so agents that grant MCP but not shell_exec still work.
    if tool_name == "shell_exec" {
        if let Some(mcp_tool_name) = direct_mcp_tool_from_shell_command(input) {
            if let Some(allowed) = allowed_tools {
                if !allowed.iter().any(|t| t == &mcp_tool_name) {
                    warn!(tool = %mcp_tool_name, "Capability denied: auto-routed MCP tool not in allowed list");
                    return finish_traj(ToolResult {
                        tool_use_id: tool_use_id.to_string(),
                        content: format!(
                            "Permission denied: agent does not have capability to use tool '{}'",
                            mcp_tool_name
                        ),
                        is_error: true,
                    });
                }
            }

            debug!(
                shell_command = %input["command"].as_str().unwrap_or(""),
                mcp_tool = %mcp_tool_name,
                "Auto-routing shell_exec MCP command to MCP tool dispatch"
            );
            let empty_args = serde_json::json!({});
            let rerouted =
                dispatch_mcp_tool_by_name(&mcp_tool_name, &empty_args, mcp_connections).await;
            return match rerouted {
                Ok(content) => finish_traj(ToolResult {
                    tool_use_id: tool_use_id.to_string(),
                    content,
                    is_error: false,
                }),
                Err(err) => finish_traj(ToolResult {
                    tool_use_id: tool_use_id.to_string(),
                    content: format!("Error: {err}"),
                    is_error: true,
                }),
            };
        }
    }

    // Capability enforcement: reject tools not in the allowed list
    if let Some(allowed) = allowed_tools {
        if !allowed.iter().any(|t| t == tool_name) {
            warn!(tool_name, "Capability denied: tool not in allowed list");
            return finish_traj(ToolResult {
                tool_use_id: tool_use_id.to_string(),
                content: format!(
                    "Permission denied: agent does not have capability to use tool '{tool_name}'"
                ),
                is_error: true,
            });
        }
    }

    // Approval gate: check if this tool requires human approval before execution.
    //
    // When exec_policy.mode = "full" (or allowlist with allowed_commands = ["*"]),
    // the user has explicitly opted into unrestricted shell access. In that case,
    // shell_exec should bypass the approval gate — requiring approval for commands
    // the user already whitelisted is contradictory (GitHub issue #772).
    let exec_policy_bypasses_approval = is_shell_tool(tool_name)
        && exec_policy.is_some_and(|p| {
            p.mode == openfang_types::config::ExecSecurityMode::Full
                || (p.mode == openfang_types::config::ExecSecurityMode::Allowlist
                    && p.allowed_commands.iter().any(|c| c == "*"))
        });

    if exec_policy_bypasses_approval {
        debug!(
            tool_name,
            "Approval bypassed: exec_policy grants unrestricted shell access"
        );
    }

    if let Some(kh) = kernel {
        if !exec_policy_bypasses_approval && kh.requires_approval(tool_name) {
            let agent_id_str = caller_agent_id.unwrap_or("unknown");
            let input_str = input.to_string();
            let summary = format!(
                "{}: {}",
                tool_name,
                openfang_types::truncate_str(&input_str, 200)
            );
            match kh.request_approval(agent_id_str, tool_name, &summary).await {
                Ok(true) => {
                    debug!(tool_name, "Approval granted — proceeding with execution");
                }
                Ok(false) => {
                    warn!(tool_name, "Approval denied — blocking tool execution");
                    return finish_traj(ToolResult {
                        tool_use_id: tool_use_id.to_string(),
                        content: format!(
                            "Execution denied: '{}' requires human approval and was denied or timed out. The operation was not performed.",
                            tool_name
                        ),
                        is_error: true,
                    });
                }
                Err(e) => {
                    warn!(tool_name, error = %e, "Approval system error");
                    return finish_traj(ToolResult {
                        tool_use_id: tool_use_id.to_string(),
                        content: format!("Approval system error: {e}"),
                        is_error: true,
                    });
                }
            }
        }
    }

    debug!(tool_name, "Executing tool");
    let result = match tool_name {
        // Filesystem tools
        "file_read" => tool_file_read(input, workspace_root, ainl_library_root).await,
        "file_write" => tool_file_write(input, workspace_root).await,
        "file_list" => tool_file_list(input, workspace_root, ainl_library_root).await,
        "apply_patch" => tool_apply_patch(input, workspace_root).await,
        "document_extract" => {
            crate::document_tools::tool_document_extract(input, workspace_root, ainl_library_root)
                .await
        }
        "spreadsheet_build" => {
            crate::document_tools::tool_spreadsheet_build(input, workspace_root).await
        }

        // GitHub subtree download — single-call clone of a public-or-tokened
        // GitHub subdirectory. Replaces the failure-prone "loop on web_fetch"
        // pattern; returns a manifest of every file written so the agent
        // cannot claim a partial download as a success.
        "github_subtree_download" => {
            crate::github_subtree::run(input, workspace_root).await
        }
        // Web tools (upgraded: multi-provider search, SSRF-protected fetch)
        "web_fetch" => {
            // Taint check: block URLs containing secrets/PII from being exfiltrated
            let url = input["url"].as_str().unwrap_or("");
            if let Some(violation) = check_taint_net_fetch(url) {
                return finish_traj(ToolResult {
                    tool_use_id: tool_use_id.to_string(),
                    content: format!("Taint violation: {violation}"),
                    is_error: true,
                });
            }
            let method = input["method"].as_str().unwrap_or("GET");
            let headers = input.get("headers").and_then(|v| v.as_object());
            let body = input["body"].as_str();
            if let Some(ctx) = web_ctx {
                ctx.fetch
                    .fetch_with_options(url, method, headers, body)
                    .await
            } else {
                tool_web_fetch_legacy(input).await
            }
        }
        "web_search" => {
            if let Some(ctx) = web_ctx {
                let query = input["query"].as_str().unwrap_or("");
                let max_results = input["max_results"].as_u64().unwrap_or(5) as usize;
                ctx.search.search(query, max_results).await
            } else {
                tool_web_search_legacy(input).await
            }
        }

        // Shell tool — metacharacter check + exec policy + taint check
        "shell_exec" => {
            let command = input["command"].as_str().unwrap_or("");

            // Full exec mode uses `sh -c`, which natively handles pipes, redirects, etc.
            // Metacharacter restrictions only apply in Allowlist mode where commands run
            // via direct exec (no shell interpreter).
            let is_full_exec = exec_policy
                .is_some_and(|p| p.mode == openfang_types::config::ExecSecurityMode::Full);

            if !is_full_exec {
                if let Some(reason) =
                    crate::subprocess_sandbox::contains_shell_metacharacters(command)
                {
                    return finish_traj(ToolResult {
                        tool_use_id: tool_use_id.to_string(),
                        content: format!(
                            "shell_exec blocked: command contains {reason}. \
                             Shell metacharacters (pipes, redirects, etc.) require \
                             exec_policy.mode = 'full' in the agent manifest."
                        ),
                        is_error: true,
                    });
                }
            }

            // Exec policy enforcement (allowlist / deny / full)
            if let Some(policy) = exec_policy {
                if let Err(reason) =
                    crate::subprocess_sandbox::validate_command_allowlist(command, policy)
                {
                    return finish_traj(ToolResult {
                        tool_use_id: tool_use_id.to_string(),
                        content: format!(
                            "shell_exec blocked: {reason}. Current exec_policy.mode = '{:?}'. \
                             To allow shell commands, set exec_policy.mode = 'full' in the agent manifest or config.toml.",
                            policy.mode
                        ),
                        is_error: true,
                    });
                }
            }
            // Skip heuristic taint patterns for Full exec policy (e.g. hand agents that need curl)
            if !is_full_exec {
                if let Some(violation) = check_taint_shell_exec(command) {
                    return finish_traj(ToolResult {
                        tool_use_id: tool_use_id.to_string(),
                        content: format!("Taint violation: {violation}"),
                        is_error: true,
                    });
                }
            }
            tool_shell_exec(
                input,
                allowed_env_vars.unwrap_or(&[]),
                workspace_root,
                ainl_library_root,
                caller_agent_id,
                process_manager,
                exec_policy,
                kernel,
            )
            .await
        }

        // Inter-agent tools (require kernel handle)
        "agent_send" => tool_agent_send(input, kernel, orchestration_live, caller_agent_id).await,
        "agent_spawn" => tool_agent_spawn(input, kernel, caller_agent_id, orchestration_live).await,
        "agent_delegate" => {
            tool_agent_delegate(input, kernel, orchestration_live, caller_agent_id).await
        }
        "agent_map_reduce" => {
            tool_agent_map_reduce(input, kernel, orchestration_live, caller_agent_id).await
        }
        "agent_supervise" => {
            tool_agent_supervise(input, kernel, orchestration_live, caller_agent_id).await
        }
        "agent_coordinate" => {
            tool_agent_coordinate(input, kernel, orchestration_live, caller_agent_id).await
        }
        "agent_list" => tool_agent_list(kernel),
        "agent_kill" => tool_agent_kill(input, kernel),

        // Shared memory tools
        "memory_store" => tool_memory_store(input, kernel),
        "memory_recall" => tool_memory_recall(input, kernel),
        "memory_list" => tool_memory_list(input, kernel),

        // Collaboration tools
        "agent_find" => tool_agent_find(input, kernel),
        "agent_find_capabilities" => tool_agent_find_capabilities(input, kernel),
        "agent_pool_list" => tool_agent_pool_list(kernel).await,
        "agent_pool_spawn" => tool_agent_pool_spawn(input, kernel, caller_agent_id).await,
        "task_post" => tool_task_post(input, kernel, caller_agent_id, orchestration_live).await,
        "task_claim" => tool_task_claim(input, kernel, caller_agent_id, orchestration_live).await,
        "orchestration_shared_merge" => {
            tool_orchestration_shared_merge(input, orchestration_live).await
        }
        "task_complete" => tool_task_complete(input, kernel).await,
        "task_list" => tool_task_list(input, kernel).await,
        "event_publish" => tool_event_publish(input, kernel).await,

        // Scheduling tools (aliases for kernel cron — persisted in ~/.armaraos/cron_jobs.json)
        "schedule_create" => tool_schedule_create(input, kernel, caller_agent_id).await,
        "schedule_action_create" => tool_schedule_action_create(input, kernel, caller_agent_id).await,
        "schedule_list" => tool_schedule_list(kernel, caller_agent_id).await,
        "schedule_delete" => tool_schedule_delete(input, kernel).await,
        "channels_list" => Ok(tool_channels_list(kernel)),

        // Knowledge graph tools
        "knowledge_add_entity" => tool_knowledge_add_entity(input, kernel).await,
        "knowledge_add_relation" => tool_knowledge_add_relation(input, kernel).await,
        "knowledge_query" => tool_knowledge_query(input, kernel).await,

        // Image analysis tool
        "image_analyze" => tool_image_analyze(input).await,

        // Media understanding tools
        "media_describe" => tool_media_describe(input, media_engine).await,
        "media_transcribe" => {
            tool_media_transcribe(input, media_engine, workspace_root, ainl_library_root).await
        }

        // Image generation tool
        "image_generate" => tool_image_generate(input, workspace_root).await,

        // TTS/STT tools
        "text_to_speech" => tool_text_to_speech(input, tts_engine, workspace_root).await,
        "speech_to_text" => tool_speech_to_text(input, media_engine, workspace_root).await,

        // Docker sandbox tool
        "docker_exec" => {
            tool_docker_exec(input, docker_config, workspace_root, caller_agent_id).await
        }

        // Location tool
        "location_get" => tool_location_get().await,

        // System time tool
        "system_time" => Ok(tool_system_time()),

        // Cron scheduling tools
        "cron_create" => tool_cron_create(input, kernel, caller_agent_id).await,
        "cron_list" => tool_cron_list(kernel, caller_agent_id).await,
        "cron_cancel" => tool_cron_cancel(input, kernel).await,

        // Channel send tool (proactive outbound messaging)
        "channel_send" => tool_channel_send(input, kernel, workspace_root).await,
        "channel_stream" => tool_channel_stream(input, kernel).await,

        // Persistent process tools
        "process_start" => tool_process_start(input, process_manager, caller_agent_id).await,
        "process_poll" => tool_process_poll(input, process_manager, caller_agent_id).await,
        "process_write" => tool_process_write(input, process_manager, caller_agent_id).await,
        "process_kill" => tool_process_kill(input, process_manager, caller_agent_id).await,
        "process_list" => tool_process_list(process_manager, caller_agent_id).await,
        "workspace_actions_list" => tool_workspace_actions_list(workspace_root).await,
        "workspace_action_set" => tool_workspace_action_set(input, workspace_root).await,
        "workspace_action_delete" => tool_workspace_action_delete(input, workspace_root).await,
        "workspace_action" => {
            tool_workspace_action(
                input,
                allowed_env_vars.unwrap_or(&[]),
                workspace_root,
                ainl_library_root,
                caller_agent_id,
                process_manager,
                exec_policy,
            )
            .await
        }

        // Deterministic script runner — picks venv / tsx / bun / etc. so the model never
        // has to compose `source venv && python …` or `nohup node … &` itself.
        "script_run" => {
            tool_script_run(
                input,
                allowed_env_vars.unwrap_or(&[]),
                workspace_root,
                ainl_library_root,
                caller_agent_id,
                process_manager,
                exec_policy,
            )
            .await
        }

        // Read-only “what should I run?” helper — returns ranked script candidates.
        "script_detect" => tool_script_detect(input, workspace_root).await,

        // Hand tools (curated autonomous capability packages)
        "hand_list" => tool_hand_list(kernel).await,
        "hand_activate" => tool_hand_activate(input, kernel).await,
        "hand_status" => tool_hand_status(input, kernel).await,
        "hand_deactivate" => tool_hand_deactivate(input, kernel).await,

        // A2A outbound tools (cross-instance agent communication)
        "a2a_discover" => tool_a2a_discover(input).await,
        "a2a_send" => tool_a2a_send(input, kernel, caller_agent_id).await,
        "hermes_a2a_status" => tool_hermes_a2a_status().await,
        "a2a_discover_hermes" => tool_a2a_discover_hermes().await,
        "a2a_send_hermes" => tool_a2a_send_hermes(input, kernel, caller_agent_id).await,

        // Browser automation tools
        "browser_navigate" => {
            let url = input["url"].as_str().unwrap_or("");
            if let Some(violation) = check_taint_net_fetch(url) {
                return finish_traj(ToolResult {
                    tool_use_id: tool_use_id.to_string(),
                    content: format!("Taint violation: {violation}"),
                    is_error: true,
                });
            }
            match browser_ctx {
                Some(mgr) => {
                    let aid = caller_agent_id.unwrap_or("default");
                    crate::browser::tool_browser_navigate(input, mgr, aid).await
                }
                None => Err(
                    "Browser tools not available. Ensure Chrome/Chromium is installed.".to_string(),
                ),
            }
        }
        "browser_click" => match browser_ctx {
            Some(mgr) => {
                let aid = caller_agent_id.unwrap_or("default");
                crate::browser::tool_browser_click(input, mgr, aid).await
            }
            None => {
                Err("Browser tools not available. Ensure Chrome/Chromium is installed.".to_string())
            }
        },
        "browser_type" => match browser_ctx {
            Some(mgr) => {
                let aid = caller_agent_id.unwrap_or("default");
                crate::browser::tool_browser_type(input, mgr, aid).await
            }
            None => {
                Err("Browser tools not available. Ensure Chrome/Chromium is installed.".to_string())
            }
        },
        "browser_screenshot" => match browser_ctx {
            Some(mgr) => {
                let aid = caller_agent_id.unwrap_or("default");
                crate::browser::tool_browser_screenshot(input, mgr, aid).await
            }
            None => {
                Err("Browser tools not available. Ensure Chrome/Chromium is installed.".to_string())
            }
        },
        "browser_read_page" => match browser_ctx {
            Some(mgr) => {
                let aid = caller_agent_id.unwrap_or("default");
                crate::browser::tool_browser_read_page(input, mgr, aid).await
            }
            None => {
                Err("Browser tools not available. Ensure Chrome/Chromium is installed.".to_string())
            }
        },
        "browser_close" => match browser_ctx {
            Some(mgr) => {
                let aid = caller_agent_id.unwrap_or("default");
                crate::browser::tool_browser_close(input, mgr, aid).await
            }
            None => {
                Err("Browser tools not available. Ensure Chrome/Chromium is installed.".to_string())
            }
        },
        "browser_scroll" => match browser_ctx {
            Some(mgr) => {
                let aid = caller_agent_id.unwrap_or("default");
                crate::browser::tool_browser_scroll(input, mgr, aid).await
            }
            None => {
                Err("Browser tools not available. Ensure Chrome/Chromium is installed.".to_string())
            }
        },
        "browser_wait" => match browser_ctx {
            Some(mgr) => {
                let aid = caller_agent_id.unwrap_or("default");
                crate::browser::tool_browser_wait(input, mgr, aid).await
            }
            None => {
                Err("Browser tools not available. Ensure Chrome/Chromium is installed.".to_string())
            }
        },
        "browser_run_js" => match browser_ctx {
            Some(mgr) => {
                let aid = caller_agent_id.unwrap_or("default");
                crate::browser::tool_browser_run_js(input, mgr, aid).await
            }
            None => {
                Err("Browser tools not available. Ensure Chrome/Chromium is installed.".to_string())
            }
        },
        "browser_back" => match browser_ctx {
            Some(mgr) => {
                let aid = caller_agent_id.unwrap_or("default");
                crate::browser::tool_browser_back(input, mgr, aid).await
            }
            None => {
                Err("Browser tools not available. Ensure Chrome/Chromium is installed.".to_string())
            }
        },
        "browser_session_start" => match browser_ctx {
            Some(mgr) => {
                let aid = caller_agent_id.unwrap_or("default");
                crate::browser::tool_browser_session_start(input, mgr, aid).await
            }
            None => Err(
                "Browser tools not available. Ensure Chrome/Chromium is installed (or, for mode=attach, that Chrome was started with --remote-debugging-port).".to_string(),
            ),
        },
        "browser_session_status" => match browser_ctx {
            Some(mgr) => {
                let aid = caller_agent_id.unwrap_or("default");
                crate::browser::tool_browser_session_status(input, mgr, aid).await
            }
            None => Err(
                "Browser tools not available. Ensure Chrome/Chromium is installed.".to_string(),
            ),
        },

        // Canvas / A2UI tool
        "canvas_present" => tool_canvas_present(input, workspace_root).await,

        // Email tools
        "email_send" => tool_email_send(input, kernel, mcp_connections).await,
        "email_read" => tool_email_read(input, kernel, mcp_connections).await,
        "email_search" => tool_email_search(input, kernel, mcp_connections).await,
        "email_reply" => tool_email_reply(input, kernel, mcp_connections).await,
        "email_draft" => tool_email_draft(input, mcp_connections).await,

        // Read MCP `resources/read` (e.g. `ainl://…` from `mcp_ainl_ainl_capabilities` mcp_resources).
        "mcp_resource_read" => tool_mcp_resource_read(input, mcp_connections).await,

        other => {
            // Fallback 1: MCP tools (mcp_{server}_{tool} prefix)
            if mcp::is_mcp_tool(other) {
                dispatch_mcp_tool_by_name(other, input, mcp_connections).await
            }
            // Fallback 2: Skill registry tool providers
            else if let Some(registry) = skill_registry {
                if let Some(skill) = registry.find_tool_provider(other) {
                    debug!(tool = other, skill = %skill.manifest.skill.name, "Dispatching to skill");
                    match openfang_skills::loader::execute_skill_tool(
                        &skill.manifest,
                        &skill.path,
                        other,
                        input,
                    )
                    .await
                    {
                        Ok(skill_result) => {
                            let content = serde_json::to_string(&skill_result.output)
                                .unwrap_or_else(|_| skill_result.output.to_string());
                            if skill_result.is_error {
                                Err(content)
                            } else {
                                Ok(content)
                            }
                        }
                        Err(e) => Err(format!("Skill execution failed: {e}")),
                    }
                } else {
                    Err(format!("Unknown tool: {other}"))
                }
            } else {
                Err(format!("Unknown tool: {other}"))
            }
        }
    };

    match result {
        Ok(content) => {
            // For AINL MCP tools, the wire call may succeed (HTTP 200, well-formed JSON) while
            // the body itself reports `ok: false` (e.g. invalid AINL syntax from `ainl_validate`,
            // compile failure from `ainl_compile`, or pre-execution policy/runtime rejection from
            // `ainl_run`). Convert these into real tool errors so the LLM sees a clear failure,
            // `loop_guard` can detect repeated calls, and `failure_learning` records the issue.
            // Non-AINL MCP tools and tools without an `ok` field are unaffected.
            if let Some(err_msg) =
                crate::mcp_ainl_session::ainl_mcp_soft_failure_message(tool_name, &content)
            {
                debug!(
                    tool_name,
                    "AINL MCP tool reported ok: false in body — promoting to tool error"
                );
                finish_traj(ToolResult {
                    tool_use_id: tool_use_id.to_string(),
                    content: err_msg,
                    is_error: true,
                })
            } else {
                finish_traj(ToolResult {
                    tool_use_id: tool_use_id.to_string(),
                    content,
                    is_error: false,
                })
            }
        }
        Err(err) => finish_traj(ToolResult {
            tool_use_id: tool_use_id.to_string(),
            content: format!("Error: {err}"),
            is_error: true,
        }),
    }
}

/// Get definitions for all built-in tools.
pub fn builtin_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        // --- Filesystem tools ---
        ToolDefinition {
            name: "file_read".to_string(),
            description: "Read the contents of a text file. Paths are relative to the agent workspace, or use the virtual prefix `ainl-library/...` for the synced AINL library tree. For .pdf / .xlsx / .docx use `document_extract`.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path (e.g. `notes.txt` or `ainl-library/examples/foo.ainl`)" }
                },
                "required": ["path"]
            }),
        },
        ToolDefinition {
            name: "file_write".to_string(),
            description: "Write content to a file. Paths are relative to the agent workspace.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "The file path to write to" },
                    "content": { "type": "string", "description": "The content to write" }
                },
                "required": ["path", "content"]
            }),
        },
        ToolDefinition {
            name: "file_list".to_string(),
            description: "List files in a directory. Workspace-relative paths, or `ainl-library/...` for the synced AINL library.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Directory path to list" }
                },
                "required": ["path"]
            }),
        },
        ToolDefinition {
            name: "apply_patch".to_string(),
            description: "Apply a multi-hunk diff patch to add, update, move, or delete files. Use this for targeted edits instead of full file overwrites.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "patch": {
                        "type": "string",
                        "description": "The patch in *** Begin Patch / *** End Patch format. Use *** Add File:, *** Update File:, *** Delete File: markers. Hunks use @@ headers with space (context), - (remove), + (add) prefixed lines."
                    }
                },
                "required": ["patch"]
            }),
        },
        ToolDefinition {
            name: "document_extract".to_string(),
            description: "Extract text or tabular data from a document. Supports .pdf (text extraction), .docx (body text), and spreadsheets .xlsx / .xls / .xlsb / .ods (tab-separated rows per sheet). Paths are workspace-relative or `ainl-library/...`. Spreadsheet cell values are usually cached; original formulas may not appear. Use optional max_sheets, max_rows_per_sheet, max_cols to limit output size.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the file (e.g. `uploads/file.xlsx` or `ainl-library/...`)" },
                    "max_sheets": { "type": "integer", "description": "Max worksheets to include (default 8, cap 20)" },
                    "max_rows_per_sheet": { "type": "integer", "description": "Max rows per sheet (default 400, cap 2000)" },
                    "max_cols": { "type": "integer", "description": "Max columns per row (default 40, cap 100)" }
                },
                "required": ["path"]
            }),
        },
        ToolDefinition {
            name: "spreadsheet_build".to_string(),
            description: "Create a new .xlsx file in the workspace from JSON. Each sheet has a name and rows as arrays of cells. Numbers and booleans are written as typed cells; strings starting with '=' are written as formulas; null skips the cell. Use for delivering corrected spreadsheets or formula fixes.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Output path ending in .xlsx (workspace-relative)" },
                    "sheets": {
                        "type": "array",
                        "description": "Sheet objects: { \"name\": string, \"rows\": [ [ cell, ... ], ... ] }",
                        "items": { "type": "object" }
                    }
                },
                "required": ["path", "sheets"]
            }),
        },
        // --- GitHub subtree download ---
        ToolDefinition {
            name: "github_subtree_download".to_string(),
            description: "Download an entire subdirectory of a public (or token-authenticated) GitHub repo into the agent workspace in a single call. Enumerates files server-side via the GitHub Trees API, then fetches each blob from raw.githubusercontent.com. Returns a manifest (including `token_source`: how auth was resolved). You cannot tell public vs private from the URL alone — unauthenticated access to a private repo often returns **404**. Auth resolution order: optional `token` in this call, else process env `GITHUB_TOKEN`, else `GH_TOKEN` (same as the `gh` CLI). The tool does **not** read workspace `.env` or agent memory; put tokens in the daemon environment (e.g. `~/.armaraos/.env`) or pass `token` explicitly. For private repos or higher rate limits, ask the user for a PAT or use env. PREFER THIS over looping `web_fetch` on raw URLs — that pattern silently misses files.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "repo": { "type": "string", "description": "Repository as `owner/name` or a github.com URL (https or git@). Path/branch suffixes in the URL are ignored." },
                    "path": { "type": "string", "description": "Subdirectory inside the repo to mirror (e.g. `apollo-x-bot`). Empty string = whole repo." },
                    "dest": { "type": "string", "description": "Workspace-relative output directory. Defaults to the basename of `path` (or repo name when `path` is empty)." },
                    "branch": { "type": "string", "description": "Branch / ref. Default `main`; falls back to `master` on 404 only when this argument is omitted." },
                    "token": { "type": "string", "description": "Optional GitHub PAT (`repo` for private, `public_repo` for rate-lift on public). If omitted, `GITHUB_TOKEN` / `GH_TOKEN` in the daemon process environment are used. **Omit for public repos** — do not pass `\"\"` (invalid header → 401)." },
                    "extensions": { "type": "array", "items": { "type": "string" }, "description": "Optional allowlist of file extensions (e.g. [\"ainl\", \"md\"]). Files not matching any extension are skipped with a reason." },
                    "exclude": { "type": "array", "items": { "type": "string" }, "description": "Optional substrings; any tree path containing one is skipped." },
                    "max_files": { "type": "integer", "description": "Cap on files written (default 500, hard max 5000)." },
                    "max_total_bytes": { "type": "integer", "description": "Cap on total bytes written (default 50 MiB, hard max 200 MiB)." }
                },
                "required": ["repo"]
            }),
        },
        // --- Web tools ---
        ToolDefinition {
            name: "web_fetch".to_string(),
            description: "Fetch a URL with SSRF protection. Supports GET/POST/PUT/PATCH/DELETE. For GET, HTML is converted to Markdown. For other methods, returns raw response body.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "The URL to fetch (http/https only)" },
                    "method": { "type": "string", "enum": ["GET","POST","PUT","PATCH","DELETE"], "description": "HTTP method (default: GET)" },
                    "headers": { "type": "object", "description": "Custom HTTP headers as key-value pairs" },
                    "body": { "type": "string", "description": "Request body for POST/PUT/PATCH" }
                },
                "required": ["url"]
            }),
        },
        ToolDefinition {
            name: "web_search".to_string(),
            description: "Search the web using multiple providers (Tavily, Brave, Perplexity, DuckDuckGo) with automatic fallback. Returns structured results with titles, URLs, and snippets.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "The search query" },
                    "max_results": { "type": "integer", "description": "Maximum number of results to return (default: 5, max: 20)" }
                },
                "required": ["query"]
            }),
        },
        // --- Shell tool ---
        ToolDefinition {
            name: "shell_exec".to_string(),
            description: "Execute a shell command and return its combined stdout/stderr once it exits. Hard ceiling: 300 s. For commands that may take longer (Playwright, npm install, long Python scripts, build steps), use process_start + process_poll instead so you can read output incrementally without hitting the timeout.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "The command to execute" },
                    "timeout_seconds": { "type": "integer", "description": "Timeout in seconds (default: 30, max: 300). If the command might run longer than 30 s, pass an explicit value up to 300. For jobs expected to take more than 5 minutes, use process_start instead." }
                },
                "required": ["command"]
            }),
        },
        // --- Inter-agent tools ---
        ToolDefinition {
            name: "agent_send".to_string(),
            description: "Send a simple message to a specific agent you already know. USE WHEN: You know exactly which agent to talk to (by name/ID) and just need to exchange information. For capability-based selection, use agent_delegate instead.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string", "description": "The target agent's UUID or name" },
                    "message": { "type": "string", "description": "The message to send to the agent" }
                },
                "required": ["agent_id", "message"]
            }),
        },
        ToolDefinition {
            name: "agent_spawn".to_string(),
            description: "Spawn a new agent from a TOML manifest. Returns the new agent's ID and name.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "manifest_toml": {
                        "type": "string",
                        "description": "The agent manifest in TOML format (must include name, module, [model], and [capabilities])"
                    }
                },
                "required": ["manifest_toml"]
            }),
        },
        ToolDefinition {
            name: "agent_delegate".to_string(),
            description: "Delegate a task to the most capable agent based on required capabilities. USE WHEN: You need specialized skills you lack (e.g., web research, code analysis). The task is well-defined and can be completed independently. NOT FOR: Simple tasks you can do yourself, or when you need tight collaboration.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "task": { "type": "string", "description": "Message/instruction for the selected agent" },
                    "required_capabilities": {
                        "type": "array",
                        "description": "Tool name strings and/or objects like {\"tool_invoke\":\"web_fetch\"}, {\"memory_read\":\"*\"}, {\"agent_spawn\": true}",
                        "items": { "type": ["string", "object"] }
                    },
                    "preferred_tags": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional tags; agents matching more tags rank higher"
                    },
                    "strategy": {
                        "type": "string",
                        "enum": ["best_match", "round_robin", "random", "least_busy", "cost_efficient"],
                        "description": "How to choose among candidates (default: best_match)"
                    },
                    "semantic_ranking": { "type": "boolean", "description": "When true (default), blend embedding similarity into ranking if the host has an embedding driver" },
                    "auto_spawn_pool": {
                        "type": "string",
                        "description": "Pool name to auto-spawn workers from when all matching agents are busy (requires [[agent_pools]] config)"
                    },
                    "auto_spawn_threshold": {
                        "type": "integer",
                        "description": "Minimum in-flight tasks to consider agent 'busy' for auto-spawn (default: 1)"
                    },
                    "delegate_options": {
                        "type": "object",
                        "properties": {
                            "semantic_ranking": { "type": "boolean" },
                            "auto_spawn_pool": { "type": "string" },
                            "auto_spawn_threshold": { "type": "integer" }
                        }
                    }
                },
                "required": ["task"]
            }),
        },
        ToolDefinition {
            name: "agent_map_reduce".to_string(),
            description: "Process multiple independent items in parallel (swarm of up to 3 agents). USE WHEN: You have 3+ similar tasks that can run independently (e.g., analyzing multiple documents, processing data chunks). NOT FOR: Single tasks, or when results must build on each other. Items are processed in parallel waves.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "items": { "type": "array", "items": { "type": "string" }, "description": "Work items to process" },
                    "map_prompt_template": { "type": "string", "description": "Prompt with {{item}} replaced per item" },
                    "map_agent": { "type": "string", "description": "Target agent id or name for map step" },
                    "max_parallelism": { "type": "integer", "description": "Parallel map calls per wave (default 3, max 3)" },
                    "reduce_prompt_template": { "type": "string", "description": "Optional; {{results}} replaced with concatenated map outputs" },
                    "reduce_agent": { "type": "string", "description": "Agent for reduce, or \"self\" to finish in current agent" }
                },
                "required": ["items", "map_prompt_template", "map_agent"]
            }),
        },
        ToolDefinition {
            name: "agent_supervise".to_string(),
            description: "Delegate with oversight and validation. USE WHEN: The task is critical and needs verification (success_criteria), or may take too long (timeout protection). You're acting as a supervisor ensuring quality. NOT FOR: Simple delegation without quality requirements.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string", "description": "Target agent UUID or name" },
                    "task": { "type": "string", "description": "Message/instruction" },
                    "max_duration_secs": { "type": "integer", "description": "Timeout in seconds (default 600)" },
                    "success_criteria": { "type": "string", "description": "If set, response must contain this substring (case-insensitive)" }
                },
                "required": ["agent_id", "task"]
            }),
        },
        ToolDefinition {
            name: "agent_coordinate".to_string(),
            description: "Orchestrate a workflow where tasks depend on each other's outputs. USE WHEN: You have a multi-step plan where later steps need earlier results (e.g., 'research topic' → 'write summary' → 'create presentation'). Tasks automatically run in parallel when dependencies allow. NOT FOR: Independent tasks (use map_reduce) or single-step delegation.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "tasks": {
                        "type": "array",
                        "description": "Nodes: { id, agent, prompt, depends_on? }",
                        "items": { "type": "object" }
                    },
                    "timeout_per_task": { "type": "integer", "description": "Per-task timeout seconds (default 300)" }
                },
                "required": ["tasks"]
            }),
        },
        ToolDefinition {
            name: "agent_list".to_string(),
            description: "List all currently running agents with their IDs, names, states, and models.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolDefinition {
            name: "agent_kill".to_string(),
            description: "Kill (terminate) another agent by its ID.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string", "description": "The agent's UUID to kill" }
                },
                "required": ["agent_id"]
            }),
        },
        // --- Shared memory tools ---
        ToolDefinition {
            name: "memory_store".to_string(),
            description: "Store a value in shared memory accessible by all agents. Use for cross-agent coordination and data sharing.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "key": { "type": "string", "description": "The storage key" },
                    "value": { "type": "string", "description": "The value to store (JSON-encode objects/arrays, or pass a plain string)" }
                },
                "required": ["key", "value"]
            }),
        },
        ToolDefinition {
            name: "memory_recall".to_string(),
            description: "Recall a value from shared memory by key.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "key": { "type": "string", "description": "The storage key to recall" }
                },
                "required": ["key"]
            }),
        },
        ToolDefinition {
            name: "memory_list".to_string(),
            description: "List all keys stored in shared memory, with their current values. Optional prefix filter (e.g. 'project.' to see only project-related keys). Use this to browse what has been remembered before recalling specific values.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "prefix": { "type": "string", "description": "Optional prefix to filter keys (e.g. 'project.', 'user.'). Omit to list all keys." }
                }
            }),
        },
        // --- Collaboration tools ---
        ToolDefinition {
            name: "agent_find".to_string(),
            description: "Discover agents by name, tag, tool, or description. Use to find specialists before delegating work.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query (matches agent name, tags, tools, description)" }
                },
                "required": ["query"]
            }),
        },
        ToolDefinition {
            name: "agent_find_capabilities".to_string(),
            description: "List agents whose manifest grants satisfy all required capabilities (same matching rules as agent_delegate). Use preferred_tags and exclude_agent_ids to narrow results.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "required_capabilities": {
                        "type": "array",
                        "description": "Tool strings and/or capability objects (see agent_delegate)",
                        "items": { "type": ["string", "object"] }
                    },
                    "preferred_tags": { "type": "array", "items": { "type": "string" } },
                    "exclude_agent_ids": { "type": "array", "items": { "type": "string" }, "description": "Agent UUIDs to skip" }
                },
                "required": ["required_capabilities"]
            }),
        },
        ToolDefinition {
            name: "agent_pool_list".to_string(),
            description: "List configured [[agent_pools]] entries with running worker counts and agent IDs.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolDefinition {
            name: "agent_pool_spawn".to_string(),
            description: "Spawn a worker agent from a named pool manifest (respects max_instances).".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "pool_name": { "type": "string", "description": "Name from `[[agent_pools]]`" }
                },
                "required": ["pool_name"]
            }),
        },
        ToolDefinition {
            name: "task_post".to_string(),
            description: "Post a task to the shared task queue for another agent to pick up. When running inside an orchestration, trace metadata is stored for sticky routing on claim.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "Short task title" },
                    "description": { "type": "string", "description": "Detailed task description" },
                    "assigned_to": { "type": "string", "description": "Agent name or ID to assign the task to (optional)" },
                    "payload": { "type": "object", "description": "Optional JSON merged into the task payload (e.g. custom routing hints)" },
                    "priority": { "type": "integer", "description": "Higher runs first when claiming (default 0)" }
                },
                "required": ["title", "description"]
            }),
        },
        ToolDefinition {
            name: "task_claim".to_string(),
            description: "Claim the next available task from the task queue assigned to you or unassigned. Prefer tasks for the current orchestration trace when in an orchestrated turn (or pass prefer_orchestration_trace_id). When the claimed task payload includes orchestration.trace_id (from task_post), the runtime rebuilds OrchestrationContext: it updates the live orchestration lock for the rest of this turn when present, and queues the same context for the agent's next user turn when not.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "prefer_orchestration_trace_id": { "type": "string", "description": "Optional trace_id to prefer sticky tasks posted under that orchestration" },
                    "strategy": { "type": "string", "description": "default | prefer_unassigned | sticky_only" }
                }
            }),
        },
        ToolDefinition {
            name: "orchestration_shared_merge".to_string(),
            description: "Merge key/value pairs into the live orchestration shared_vars map (visible to this agent and propagated to delegated calls).".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "patch": { "type": "object", "description": "Object whose keys are merged into shared_vars" }
                },
                "required": ["patch"]
            }),
        },
        ToolDefinition {
            name: "task_complete".to_string(),
            description: "Mark a previously claimed task as completed with a result.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string", "description": "The task ID to complete" },
                    "result": { "type": "string", "description": "The result or outcome of the task" }
                },
                "required": ["task_id", "result"]
            }),
        },
        ToolDefinition {
            name: "task_list".to_string(),
            description: "List tasks in the shared queue, optionally filtered by status (pending, in_progress, completed).".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "status": { "type": "string", "description": "Filter by status: pending, in_progress, completed (optional)" }
                }
            }),
        },
        ToolDefinition {
            name: "event_publish".to_string(),
            description: "Publish a custom event that can trigger proactive agents. Use to broadcast signals to the agent fleet.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "event_type": { "type": "string", "description": "Type identifier for the event (e.g., 'code_review_requested')" },
                    "payload": { "type": "object", "description": "JSON payload data for the event" }
                },
                "required": ["event_type"]
            }),
        },
        // --- Scheduling tools (friendly aliases → same kernel cron as cron_create) ---
        ToolDefinition {
            name: "schedule_create".to_string(),
            description: "Create a recurring job in the ArmaraOS kernel scheduler (same as cron_create; persisted under ~/.armaraos/cron_jobs.json). Pass natural-language schedule or cron expr. Prefer `program_path` + `delivery` for AINL + alerts; use `cron_create` for full control.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "description": { "type": "string", "description": "Short name/label for the job (used as the cron job name after sanitizing)" },
                    "schedule": { "type": "string", "description": "Natural language or 5-field cron (e.g. 'every 5 minutes', 'daily at 9am', '0 */5 * * *')" },
                    "program_path": { "type": "string", "description": "If set, action is ainl_run on this path (under ainl-library). Otherwise defaults to agent_turn with `message`." },
                    "message": { "type": "string", "description": "Agent turn message when program_path is omitted (default: description)" },
                    "action": { "type": "object", "description": "Optional: full cron action JSON to override program_path/message" },
                    "delivery": { "type": "object", "description": "Optional: same as cron_create delivery (none, last_channel, channel, webhook)" },
                    "timeout_secs": { "type": "integer", "description": "Timeout for agent_turn or ainl_run (default 300)" },
                    "enabled": { "type": "boolean" }
                },
                "required": ["description", "schedule"]
            }),
        },
        ToolDefinition {
            name: "schedule_action_create".to_string(),
            description: "Create a recurring kernel schedule that runs a named workspace action from armaraos.toml. Wrapper around schedule_create with action.kind='workspace_action'.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "description": { "type": "string", "description": "Short label for the job (used as cron job name)." },
                    "schedule": { "type": "string", "description": "Natural language or 5-field cron." },
                    "action": { "type": "string", "description": "Workspace action name under [actions.<name>] in armaraos.toml." },
                    "args": { "type": "array", "items": { "type": "string" }, "description": "Optional args appended at runtime." },
                    "env": { "type": "object", "additionalProperties": { "type": "string" }, "description": "Optional env overrides merged at runtime." },
                    "mode": { "type": "string", "enum": ["oneshot", "daemon"], "description": "Optional mode override for this schedule." },
                    "timeout_secs": { "type": "integer", "description": "Optional oneshot timeout in seconds." },
                    "delivery": { "type": "object", "description": "Optional delivery object (same as schedule_create)." },
                    "enabled": { "type": "boolean", "description": "Whether the job starts enabled (default true)." }
                },
                "required": ["description", "schedule", "action"]
            }),
        },
        ToolDefinition {
            name: "schedule_list".to_string(),
            description: "List kernel cron jobs for this agent (same data as cron_list / Dashboard Scheduler).".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolDefinition {
            name: "schedule_delete".to_string(),
            description: "Remove a kernel cron job by job_id (same as cron_cancel).".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Cron job UUID (from schedule_list or cron_list)" },
                    "job_id": { "type": "string", "description": "Alias for id" }
                },
                "required": []
            }),
        },
        ToolDefinition {
            name: "channels_list".to_string(),
            description: "List registered outbound channel adapter names (telegram, discord, …) for channel_send and cron delivery. Call before channel_send or when wiring alerts.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        // --- Knowledge graph tools ---
        ToolDefinition {
            name: "knowledge_add_entity".to_string(),
            description: "Add an entity to the knowledge graph. Entities represent people, organizations, projects, concepts, locations, tools, etc.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Display name of the entity" },
                    "entity_type": { "type": "string", "description": "Type: person, organization, project, concept, event, location, document, tool, or a custom type" },
                    "properties": { "type": "object", "description": "Arbitrary key-value properties (optional)" }
                },
                "required": ["name", "entity_type"]
            }),
        },
        ToolDefinition {
            name: "knowledge_add_relation".to_string(),
            description: "Add a relation between two entities in the knowledge graph.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "source": { "type": "string", "description": "Source entity ID or name" },
                    "relation": { "type": "string", "description": "Relation type: works_at, knows_about, related_to, depends_on, owned_by, created_by, located_in, part_of, uses, produces, or a custom type" },
                    "target": { "type": "string", "description": "Target entity ID or name" },
                    "confidence": { "type": "number", "description": "Confidence score 0.0-1.0 (default: 1.0)" },
                    "properties": { "type": "object", "description": "Arbitrary key-value properties (optional)" }
                },
                "required": ["source", "relation", "target"]
            }),
        },
        ToolDefinition {
            name: "knowledge_query".to_string(),
            description: "Query the knowledge graph. Filter by source entity, relation type, and/or target entity. Returns matching entity-relation-entity triples.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "source": { "type": "string", "description": "Filter by source entity name or ID (optional)" },
                    "relation": { "type": "string", "description": "Filter by relation type (optional)" },
                    "target": { "type": "string", "description": "Filter by target entity name or ID (optional)" },
                    "max_depth": { "type": "integer", "description": "Maximum traversal depth (default: 1)" }
                }
            }),
        },
        // --- Image analysis tool ---
        ToolDefinition {
            name: "image_analyze".to_string(),
            description: "Analyze an image file — returns format, dimensions, file size, and a base64 preview. For vision-model analysis, include a prompt.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the image file" },
                    "prompt": { "type": "string", "description": "Optional prompt for vision analysis (e.g., 'Describe what you see')" }
                },
                "required": ["path"]
            }),
        },
        // --- Location tool ---
        ToolDefinition {
            name: "location_get".to_string(),
            description: "Get approximate geographic location based on IP address. Returns city, country, coordinates, and timezone.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        // --- Browser automation tools ---
        ToolDefinition {
            name: "browser_navigate".to_string(),
            description: "Navigate a browser to a URL. Returns the page title and readable content as markdown. Opens a persistent browser session if one does not already exist. Optional `mode` selects the browser context: \"headless\" (default, fastest, no window), \"headed\" (visible Chrome — use when the user wants to watch, when sites detect headless, or for debugging), or \"attach\" (connect to a Chrome the user already started with --remote-debugging-port=9222 — preserves their real cookies, sign-ins and profile). Switching `mode` while a session is open will close and reopen the browser in the new mode.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "The URL to navigate to (http/https only)" },
                    "mode": { "type": "string", "description": "Optional browser mode: \"headless\" (default), \"headed\" (visible window), or \"attach\" (use user's running Chrome on --remote-debugging-port=9222). Leave unset to keep current session as-is." }
                },
                "required": ["url"]
            }),
        },
        ToolDefinition {
            name: "browser_click".to_string(),
            description: "Click an element on the current browser page by CSS selector or visible text. Returns the resulting page state.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "selector": { "type": "string", "description": "CSS selector (e.g., '#submit-btn', '.add-to-cart') or visible text to click" }
                },
                "required": ["selector"]
            }),
        },
        ToolDefinition {
            name: "browser_type".to_string(),
            description: "Type text into an input field on the current browser page.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "selector": { "type": "string", "description": "CSS selector for the input field (e.g., 'input[name=\"email\"]', '#search-box')" },
                    "text": { "type": "string", "description": "The text to type into the field" }
                },
                "required": ["selector", "text"]
            }),
        },
        ToolDefinition {
            name: "browser_screenshot".to_string(),
            description: "Take a screenshot of the current browser page. Returns a base64-encoded PNG image.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolDefinition {
            name: "browser_read_page".to_string(),
            description: "Read the current browser page content as structured markdown. Use after clicking or navigating to see the updated page.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolDefinition {
            name: "browser_close".to_string(),
            description: "Close the browser session. The browser will also auto-close when the agent loop ends.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolDefinition {
            name: "browser_scroll".to_string(),
            description: "Scroll the browser page. Use this to see content below the fold or navigate long pages.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "direction": { "type": "string", "description": "Scroll direction: 'up', 'down', 'left', 'right' (default: 'down')" },
                    "amount": { "type": "integer", "description": "Pixels to scroll (default: 600)" }
                }
            }),
        },
        ToolDefinition {
            name: "browser_wait".to_string(),
            description: "Wait for a CSS selector to appear on the page. Useful for dynamic content that loads asynchronously.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "selector": { "type": "string", "description": "CSS selector to wait for" },
                    "timeout_ms": { "type": "integer", "description": "Max wait time in milliseconds (default: 5000, max: 30000)" }
                },
                "required": ["selector"]
            }),
        },
        ToolDefinition {
            name: "browser_run_js".to_string(),
            description: "Run JavaScript on the current browser page and return the result. For advanced interactions that other browser tools cannot handle.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "expression": { "type": "string", "description": "JavaScript expression to run in the page context" }
                },
                "required": ["expression"]
            }),
        },
        ToolDefinition {
            name: "browser_back".to_string(),
            description: "Go back to the previous page in browser history.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolDefinition {
            name: "browser_session_start".to_string(),
            description: "Open (or restart) the browser session in a specific mode without navigating. Use this when you need to switch modes deliberately — for example, start in \"headless\" for speed, then call browser_session_start with mode=\"headed\" so the user can watch the next step. \"attach\" connects to a Chrome the user started with --remote-debugging-port (default 9222) so the agent drives their real browser (existing cookies, sign-ins, profile). Returns the active mode.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "mode": { "type": "string", "description": "\"headless\" (no window, fastest), \"headed\" (visible window — better for sites that block headless / for letting the user watch), or \"attach\" (connect to user's running Chrome via CDP on --remote-debugging-port). Defaults to the configured default_mode." }
                }
            }),
        },
        ToolDefinition {
            name: "browser_session_status".to_string(),
            description: "Report this agent's current browser session: active mode (or \"none\" if no session is open) and the configured default mode. Use before driving the browser to decide whether you need browser_session_start to switch modes.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        // --- Media understanding tools ---
        ToolDefinition {
            name: "media_describe".to_string(),
            description: "Describe an image using a vision-capable LLM. Auto-selects the best available provider (Anthropic, OpenAI, or Gemini). Returns a text description of the image content.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the image file (relative to workspace)" },
                    "prompt": { "type": "string", "description": "Optional prompt to guide the description (e.g., 'Extract all text from this image')" }
                },
                "required": ["path"]
            }),
        },
        ToolDefinition {
            name: "media_transcribe".to_string(),
            description: "Transcribe audio to text using speech-to-text. Auto-selects the best available provider (Groq Whisper or OpenAI Whisper). For dashboard/voice uploads, `file_id` and `content_type` are in the `[ARMAVOS_VOICE_CONTEXT]` block at the **start** of the current user message (lines `ARMAVOS_VOICE file_id=… content_type=…`). **Do not ask the user to paste a UUID** — it is not visible in the chat UI. Never use the browser display name (e.g. `voice_123.webm`) as `file_id`. Call this tool in the same turn as the voice message so the temp file is still on the server.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the audio file (workspace-relative or absolute). Supported extensions: mp3, wav, ogg, flac, m4a, webm." },
                    "file_id": { "type": "string", "description": "Server upload UUID (xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx) from the `[ARMAVOS_VOICE_CONTEXT]` block in the current user message — not the `voice_….webm` label." },
                    "content_type": { "type": "string", "description": "MIME from `[ARMAVOS_VOICE_CONTEXT]` (e.g. audio/webm). Defaults to audio/webm if omitted." },
                    "language": { "type": "string", "description": "Optional ISO-639-1 language code (e.g., 'en', 'es', 'ja')" }
                }
            }),
        },
        // --- Image generation tool ---
        ToolDefinition {
            name: "image_generate".to_string(),
            description: "Generate images from a text prompt using DALL-E 3, DALL-E 2, or GPT-Image-1. Requires OPENAI_API_KEY. Generated images are saved to the workspace output/ directory.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "prompt": { "type": "string", "description": "Text description of the image to generate (max 4000 chars)" },
                    "model": { "type": "string", "description": "Model to use: 'dall-e-3' (default), 'dall-e-2', or 'gpt-image-1'" },
                    "size": { "type": "string", "description": "Image size: '1024x1024' (default), '1024x1792', '1792x1024', '256x256', '512x512'" },
                    "quality": { "type": "string", "description": "Quality: 'hd' (default for dall-e-3) or 'standard'" },
                    "count": { "type": "integer", "description": "Number of images to generate (1-4, default: 1). DALL-E 3 only supports 1." }
                },
                "required": ["prompt"]
            }),
        },
        // --- Cron scheduling tools ---
        ToolDefinition {
            name: "cron_create".to_string(),
            description: "Create a scheduled job in the ArmaraOS kernel scheduler (persists to ~/.armaraos/cron_jobs.json; not OS cron). Supports one-shot (at), recurring (every N seconds), and 5-field cron expressions. Max 50 jobs per agent.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Job name (max 128 chars, alphanumeric + spaces/hyphens/underscores)" },
                    "schedule": {
                        "type": "object",
                        "description": "When: {\"kind\":\"at\",\"at\":\"2025-01-01T00:00:00Z\"} | {\"kind\":\"every\",\"every_secs\":300} | {\"kind\":\"cron\",\"expr\":\"0 */6 * * *\",\"tz\":null}"
                    },
                    "action": {
                        "type": "object",
                        "description": "What to run: {\"kind\":\"system_event\",\"text\":\"...\"} | {\"kind\":\"agent_turn\",\"message\":\"...\",\"timeout_secs\":300,\"model_override\":null} | {\"kind\":\"workflow_run\",\"workflow_id\":\"...\",\"input\":null,\"timeout_secs\":120} | {\"kind\":\"ainl_run\",\"program_path\":\"path.ainl\",\"cwd\":null,\"timeout_secs\":300,\"json_output\":false,\"frame\":null}"
                    },
                    "delivery": {
                        "type": "object",
                        "description": "Where to send output: {\"kind\":\"none\"} | {\"kind\":\"last_channel\"} | {\"kind\":\"channel\",\"channel\":\"telegram\",\"to\":\"chat_id\"} | {\"kind\":\"webhook\",\"url\":\"https://...\"}"
                    },
                    "one_shot": { "type": "boolean", "description": "If true, auto-delete after execution. Default: false" },
                    "enabled": { "type": "boolean", "description": "Default true" }
                },
                "required": ["name", "schedule", "action"]
            }),
        },
        ToolDefinition {
            name: "cron_list".to_string(),
            description: "List all scheduled/cron jobs for the current agent.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolDefinition {
            name: "cron_cancel".to_string(),
            description: "Cancel a scheduled/cron job by its ID.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "job_id": { "type": "string", "description": "The UUID of the cron job to cancel" }
                },
                "required": ["job_id"]
            }),
        },
        // --- Channel send tool (proactive outbound messaging) ---
        ToolDefinition {
            name: "channel_send".to_string(),
            description: "Send a message or media to a user on a configured channel (email, telegram, slack, etc). For email: recipient is the email address; optionally set subject. For media: set image_url, file_url, or file_path to send an image or file instead of (or alongside) text. Use thread_id to reply in a specific thread/topic.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "channel": { "type": "string", "description": "Channel adapter name (e.g., 'email', 'telegram', 'slack', 'discord')" },
                    "recipient": { "type": "string", "description": "Platform-specific recipient identifier (email address, user ID, etc.)" },
                    "subject": { "type": "string", "description": "Optional subject line (used for email; ignored for other channels)" },
                    "message": { "type": "string", "description": "The message body to send (required for text, optional caption for media)" },
                    "image_url": { "type": "string", "description": "URL of an image to send (supported on Telegram, Discord, Slack)" },
                    "file_url": { "type": "string", "description": "URL of a file to send as attachment" },
                    "file_path": { "type": "string", "description": "Local file path to send as attachment (reads from disk; use instead of file_url for local files)" },
                    "filename": { "type": "string", "description": "Filename for file attachments (defaults to the basename of file_path, or 'file')" },
                    "thread_id": { "type": "string", "description": "Thread/topic ID to reply in (e.g., Telegram message_thread_id, Slack thread_ts)" }
                },
                "required": ["channel", "recipient"]
            }),
        },
        ToolDefinition {
            name: "channel_stream".to_string(),
            description: "Push a real-time progress update to a channel mid-task. Use this during long-running jobs to keep stakeholders informed without waiting for the task to finish. Sends immediately and returns without blocking. Same channel/recipient as channel_send. Prefer this over channel_send when the message is a status update rather than a final result.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "channel": { "type": "string", "description": "Channel adapter name (e.g., 'telegram', 'slack', 'discord', 'email')" },
                    "recipient": { "type": "string", "description": "Platform-specific recipient identifier. Omit to use the channel's default_chat_id." },
                    "message": { "type": "string", "description": "Progress update text to send. Keep it concise — this is a status ping, not a final report." },
                    "thread_id": { "type": "string", "description": "Thread/topic ID to reply in (e.g., Telegram message_thread_id, Slack thread_ts). Use the same thread as the original task message for clean threading." }
                },
                "required": ["channel", "message"]
            }),
        },
        // --- Hand tools (curated autonomous capability packages) ---
        ToolDefinition {
            name: "hand_list".to_string(),
            description: "List available Hands (curated autonomous packages) and their activation status.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolDefinition {
            name: "hand_activate".to_string(),
            description: "Activate a Hand — spawns a specialized autonomous agent with curated tools and skills.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "hand_id": { "type": "string", "description": "The ID of the hand to activate (e.g. 'researcher', 'clip', 'browser')" },
                    "config": { "type": "object", "description": "Optional configuration overrides for the hand's settings" }
                },
                "required": ["hand_id"]
            }),
        },
        ToolDefinition {
            name: "hand_status".to_string(),
            description: "Check the status and metrics of an active Hand.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "hand_id": { "type": "string", "description": "The ID of the hand to check status for" }
                },
                "required": ["hand_id"]
            }),
        },
        ToolDefinition {
            name: "hand_deactivate".to_string(),
            description: "Deactivate a running Hand and stop its agent.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "instance_id": { "type": "string", "description": "The UUID of the hand instance to deactivate" }
                },
                "required": ["instance_id"]
            }),
        },
        // --- A2A outbound tools ---
        ToolDefinition {
            name: "a2a_discover".to_string(),
            description: "Discover an external A2A agent by fetching its agent card from a URL. Returns the agent's name, description, skills, and supported protocols.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "Base URL of the remote OpenFang/A2A-compatible agent (e.g., 'https://agent.example.com')" }
                },
                "required": ["url"]
            }),
        },
        ToolDefinition {
            name: "a2a_send".to_string(),
            description: "Send a task/message to an external A2A agent and get the response. Use agent_name to send to a previously discovered agent, or agent_url for direct addressing.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "message": { "type": "string", "description": "The task/message to send to the remote agent" },
                    "agent_url": { "type": "string", "description": "Direct URL of the remote agent's A2A endpoint" },
                    "agent_name": { "type": "string", "description": "Name of a previously discovered A2A agent (looked up from kernel)" },
                    "session_id": { "type": "string", "description": "Optional session ID for multi-turn conversations" }
                },
                "required": ["message"]
            }),
        },
        ToolDefinition {
            name: "hermes_a2a_status".to_string(),
            description: "Check whether a local Hermes A2A config exists (~/.hermes/a2a.json or HERMES_HOME/a2a.json) and return base_url when valid.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: "a2a_discover_hermes".to_string(),
            description: "Discover an A2A agent using the base_url from local Hermes config (a2a.json). Skips generic SSRF URL checks because the URL is operator-controlled on disk; still blocks cloud metadata hosts in the file.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: "a2a_send_hermes".to_string(),
            description: "Send a task to the A2A peer in Hermes a2a.json: discover Agent Card, then send via ArmaraOS JSON-RPC (tasks/send to card.url when same-origin) and/or Linux Foundation HTTP (POST …/message:send) per optional send_binding (default auto).".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "message": { "type": "string", "description": "Task/message for the Hermes A2A peer" },
                    "session_id": { "type": "string", "description": "Optional session ID for multi-turn conversations" }
                },
                "required": ["message"]
            }),
        },
        // --- TTS/STT tools ---
        ToolDefinition {
            name: "text_to_speech".to_string(),
            description: "Convert text to speech audio. Auto-selects OpenAI or ElevenLabs. Saves audio to workspace output/ directory.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "text": { "type": "string", "description": "The text to convert to speech (max 4096 chars)" },
                    "voice": { "type": "string", "description": "Voice name: 'alloy', 'echo', 'fable', 'onyx', 'nova', 'shimmer' (default: 'alloy')" },
                    "format": { "type": "string", "description": "Output format: 'mp3', 'opus', 'aac', 'flac' (default: 'mp3')" }
                },
                "required": ["text"]
            }),
        },
        ToolDefinition {
            name: "speech_to_text".to_string(),
            description: "Transcribe audio to text using speech-to-text. Auto-selects Groq Whisper or OpenAI Whisper. Supported formats: mp3, wav, ogg, flac, m4a, webm.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the audio file (relative to workspace)" },
                    "language": { "type": "string", "description": "Optional ISO-639-1 language code (e.g., 'en', 'es', 'ja')" }
                },
                "required": ["path"]
            }),
        },
        // --- Docker sandbox tool ---
        ToolDefinition {
            name: "docker_exec".to_string(),
            description: "Execute a command inside a Docker container sandbox. Provides OS-level isolation with resource limits, network isolation, and capability dropping. Requires Docker to be installed and docker.enabled=true.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "The command to execute inside the container" }
                },
                "required": ["command"]
            }),
        },
        // --- Persistent process tools ---
        ToolDefinition {
            name: "process_start".to_string(),
            description: "Start a long-running or slow process (REPL, HTTP gateway, watcher, Playwright scraper, npm install, build step, Python script). Returns a process_id; then call process_poll repeatedly to read buffered output until the job finishes. Use this instead of shell_exec when the command may exceed ~30s **or** when you need a daemon/listener (do not use shell_exec with `nohup … &` — use process_start + poll). Max 5 processes per agent.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "The executable to run (e.g. 'python3', 'node', 'npm')" },
                    "args": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Command-line arguments (e.g. ['bot.py'] or ['-i'])"
                    },
                    "env": {
                        "type": "object",
                        "description": "Optional environment variables to set for the process (string→string). Use this for ports, API base URLs, feature flags, etc.",
                        "additionalProperties": { "type": "string" }
                    },
                    "cwd": { "type": "string", "description": "Working directory for the process. Use an absolute path. Required when the script uses relative imports, reads local .env files, or relies on os.getcwd(). Example: '/Users/me/.armaraos/workspaces/MyBot'" }
                },
                "required": ["command"]
            }),
        },
        ToolDefinition {
            name: "process_poll".to_string(),
            description: "Read accumulated stdout/stderr from a running process. Non-blocking: returns whatever output has buffered since the last poll.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "process_id": { "type": "string", "description": "The process ID returned by process_start" }
                },
                "required": ["process_id"]
            }),
        },
        ToolDefinition {
            name: "process_write".to_string(),
            description: "Write data to a running process's stdin. A newline is appended automatically if not present.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "process_id": { "type": "string", "description": "The process ID returned by process_start" },
                    "data": { "type": "string", "description": "The data to write to stdin" }
                },
                "required": ["process_id", "data"]
            }),
        },
        ToolDefinition {
            name: "process_kill".to_string(),
            description: "Terminate a running process and clean up its resources.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "process_id": { "type": "string", "description": "The process ID returned by process_start" }
                },
                "required": ["process_id"]
            }),
        },
        ToolDefinition {
            name: "process_list".to_string(),
            description: "List all running processes for the current agent, including their IDs, commands, uptime, and alive status.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolDefinition {
            name: "workspace_actions_list".to_string(),
            description: "List named workspace actions from `<workspace>/armaraos.toml` (`[actions.<name>]`).".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolDefinition {
            name: "workspace_action_set".to_string(),
            description: "Create or update a named workspace action in `<workspace>/armaraos.toml`. Use this when an action is missing/outdated so future runs use `workspace_action` deterministically.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "description": "Action name under `[actions.<name>]`." },
                    "description": { "type": "string", "description": "Optional human-friendly description." },
                    "script": { "type": "string", "description": "Script path (relative to workspace, or allowed absolute path)." },
                    "args": { "type": "array", "items": { "type": "string" } },
                    "env": { "type": "object", "additionalProperties": { "type": "string" } },
                    "cwd": { "type": "string" },
                    "language": { "type": "string" },
                    "mode": { "type": "string", "enum": ["oneshot", "daemon"] },
                    "timeout_seconds": { "type": "number" },
                    "health_check": {
                        "type": "object",
                        "properties": {
                            "url": { "type": "string" },
                            "timeout_seconds": { "type": "number" },
                            "expect_status": { "type": "number" }
                        }
                    }
                },
                "required": ["action", "script"]
            }),
        },
        ToolDefinition {
            name: "workspace_action_delete".to_string(),
            description: "Delete a named workspace action from `<workspace>/armaraos.toml`. If it was the last action, removes the contract file.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "description": "Action name to remove." }
                },
                "required": ["action"]
            }),
        },
        ToolDefinition {
            name: "workspace_action".to_string(),
            description: "Execute a named action from `<workspace>/armaraos.toml` deterministically (via script_run). Use this for stable 'start gateway/run script' workflows.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "description": "Action name under `[actions.<name>]`." },
                    "args": { "type": "array", "items": { "type": "string" }, "description": "Optional extra args appended to contract args." },
                    "env": { "type": "object", "additionalProperties": { "type": "string" }, "description": "Optional env overrides merged over contract env." },
                    "mode": { "type": "string", "enum": ["oneshot", "daemon"], "description": "Optional mode override." },
                    "timeout_seconds": { "type": "number", "description": "Optional oneshot timeout override." }
                },
                "required": ["action"]
            }),
        },
        // --- Deterministic script runner (preferred over hand-rolled shell_exec) ---
        ToolDefinition {
            name: "script_run".to_string(),
            description: "Run a project script (.py / .sh / .ts / .js / .mjs / .cjs / .tsx) by file path. \
                          The runtime auto-selects the right interpreter: prefers a project venv (`.venv/bin/python3`, \
                          `venv/bin/python3`) for Python; `node_modules/.bin/tsx` (then `npx --yes tsx`) for TypeScript; \
                          `bun`/`deno` when their lockfiles are present. \
                          **Use this instead of composing `source venv && python ...` or `nohup node ... &` in `shell_exec`.** \
                          `mode: \"oneshot\"` (default) runs to completion (~30s, max 600s) and returns stdout/stderr. \
                          `mode: \"daemon\"` launches a managed background process (returns `process_id`; use `process_poll` / \
                          `process_kill` to follow up). For services, also pass `health_check.url` (e.g. \
                          `http://127.0.0.1:8080/health`) and the runtime will probe it until ready so you don't have to \
                          chain `curl` calls yourself.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "script": {
                        "type": "string",
                        "description": "Path to the script. Either workspace-relative (e.g. 'server.py', 'scripts/start.sh') or absolute under the workspace / AINL library."
                    },
                    "args": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional positional args passed to the script after the file path."
                    },
                    "env": {
                        "type": "object",
                        "additionalProperties": { "type": "string" },
                        "description": "Optional environment variables (string→string). Use this for ports, API base URLs, feature flags, etc. Strongly preferred over wrapping with `export FOO=…` in shell_exec."
                    },
                    "cwd": {
                        "type": "string",
                        "description": "Optional working directory override (absolute path). Defaults to the script's parent directory, which is usually correct."
                    },
                    "mode": {
                        "type": "string",
                        "enum": ["oneshot", "daemon"],
                        "description": "'oneshot' (default) runs to completion and returns stdout/stderr. 'daemon' launches as a managed background process and returns a process_id."
                    },
                    "language": {
                        "type": "string",
                        "description": "Optional override of auto-detected language. Supported: 'python', 'shell', 'bash', 'zsh', 'node', 'typescript', 'bun', 'deno'."
                    },
                    "timeout_seconds": {
                        "type": "number",
                        "description": "Override timeout for oneshot mode (default 30, max 600). Ignored in daemon mode."
                    },
                    "health_check": {
                        "type": "object",
                        "description": "Daemon-mode readiness probe. The runtime will GET this URL until it returns the expected status, with backoff.",
                        "properties": {
                            "url": { "type": "string", "description": "URL to probe (e.g. 'http://127.0.0.1:8080/health')." },
                            "timeout_seconds": { "type": "number", "description": "Total wait budget (default 15, max 60)." },
                            "expect_status": { "type": "number", "description": "Expected HTTP status code (default 200)." }
                        },
                        "required": ["url"]
                    }
                },
                "required": ["script"]
            }),
        },
        ToolDefinition {
            name: "script_detect".to_string(),
            description: "Detect what script to run in the current workspace. Read-only: scans for likely entrypoints \
                          (gateway/server/start scripts) and `package.json` scripts, returns ranked candidates. \
                          Use this when the user says \"start the gateway\" but doesn't provide a filename. \
                          Next: pick a `file` result and call `script_run`; or pick a `package_script` and run it via \
                          `process_start` (e.g. command: 'npm', args: ['run', '<name>']).".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "What you're looking for (e.g. 'gateway', 'server', 'dev', 'api'). Used to rank results." },
                    "max_results": { "type": "number", "description": "Maximum results to return (default 20, max 20)." }
                }
            }),
        },
        // --- System time tool ---
        ToolDefinition {
            name: "system_time".to_string(),
            description: "Get the current date, time, and timezone. Returns ISO 8601 timestamp, Unix epoch seconds, and timezone info.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
        // --- Canvas / A2UI tool ---
        ToolDefinition {
            name: "canvas_present".to_string(),
            description: "Present an interactive HTML canvas to the user. The HTML is sanitized (no scripts, no event handlers) and saved to the workspace. The dashboard will render it in a panel. Use for rich data visualizations, formatted reports, or interactive UI.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "html": { "type": "string", "description": "The HTML content to present. Must not contain <script> tags, event handlers, or javascript: URLs." },
                    "title": { "type": "string", "description": "Optional title for the canvas panel" }
                },
                "required": ["html"]
            }),
        },
        // --- Email tools ---
        ToolDefinition {
            name: "email_send".to_string(),
            description: "Send an email via SMTP. Supports Gmail, Outlook, Yahoo, ProtonMail, and other standard SMTP providers. Requires email channel configuration or MCP email integration.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "to": { "type": "string", "description": "Recipient email address (e.g., 'user@example.com')" },
                    "subject": { "type": "string", "description": "Email subject line" },
                    "body": { "type": "string", "description": "Email body (plain text or HTML)" },
                    "cc": { "type": "string", "description": "Optional CC recipients (comma-separated)" },
                    "bcc": { "type": "string", "description": "Optional BCC recipients (comma-separated)" },
                    "provider": { "type": "string", "description": "Optional email provider hint: 'gmail', 'outlook', 'yahoo', 'smtp', 'mcp'. Auto-detected if omitted." }
                },
                "required": ["to", "subject", "body"]
            }),
        },
        ToolDefinition {
            name: "email_read".to_string(),
            description: "Read recent emails from inbox via IMAP or MCP. Returns email metadata and body content. Requires email channel configuration or MCP email integration.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "folder": { "type": "string", "description": "Email folder to read from (default: 'INBOX')" },
                    "limit": { "type": "number", "description": "Maximum number of emails to return (default: 10, max: 50)" },
                    "unread_only": { "type": "boolean", "description": "Only return unread emails (default: true)" },
                    "from": { "type": "string", "description": "Filter by sender email address" },
                    "subject_contains": { "type": "string", "description": "Filter by subject keyword" },
                    "provider": { "type": "string", "description": "Optional provider hint: 'gmail', 'outlook', 'imap', 'mcp'" }
                }
            }),
        },
        ToolDefinition {
            name: "email_search".to_string(),
            description: "Search emails using provider-specific query syntax. Supports Gmail search operators, Outlook filters, and IMAP SEARCH. Returns matching email metadata.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query (provider-specific syntax, e.g., Gmail: 'from:user@example.com subject:urgent', Outlook: 'subject:meeting', IMAP: 'SUBJECT \"meeting\"')" },
                    "limit": { "type": "number", "description": "Maximum results to return (default: 20, max: 100)" },
                    "folder": { "type": "string", "description": "Folder to search (default: all folders or INBOX)" },
                    "provider": { "type": "string", "description": "Provider hint: 'gmail', 'outlook', 'imap', 'mcp'" }
                },
                "required": ["query"]
            }),
        },
        ToolDefinition {
            name: "email_reply".to_string(),
            description: "Reply to an email thread, maintaining conversation context and threading headers (In-Reply-To, References).".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "message_id": { "type": "string", "description": "Original message ID to reply to" },
                    "body": { "type": "string", "description": "Reply body text" },
                    "reply_all": { "type": "boolean", "description": "Reply to all recipients (default: false)" },
                    "provider": { "type": "string", "description": "Provider hint: 'gmail', 'outlook', 'smtp', 'mcp'" }
                },
                "required": ["message_id", "body"]
            }),
        },
        ToolDefinition {
            name: "email_draft".to_string(),
            description: "Create or update an email draft without sending. Useful for composing emails that need review before sending.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "to": { "type": "string", "description": "Recipient email address" },
                    "subject": { "type": "string", "description": "Email subject" },
                    "body": { "type": "string", "description": "Email body" },
                    "draft_id": { "type": "string", "description": "Optional existing draft ID to update" },
                    "provider": { "type": "string", "description": "Provider hint: 'gmail', 'outlook', 'mcp'" }
                },
                "required": ["to", "subject", "body"]
            }),
        },
        ToolDefinition {
            name: "mcp_resource_read".to_string(),
            description: "Read a resource body from a connected MCP server via the MCP `resources/read` request. Use for `ainl://…` URIs listed under `mcp_resources` in `mcp_ainl_ainl_capabilities` (e.g. `ainl://authoring-cheatsheet`, integration docs).".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "mcp_server": { "type": "string", "description": "MCP server id as configured in the host (e.g. `ainl`)" },
                    "server": { "type": "string", "description": "Alias for `mcp_server`" },
                    "uri": { "type": "string", "description": "Resource URI to read (e.g. `ainl://integrations-http-machine-payments`)" },
                    "max_bytes": { "type": "integer", "description": "Max UTF-8 **bytes** of resource text returned after offset (default 65536; capped at 2_000_000). Truncation is on a char boundary; a one-line truncation notice may be appended." },
                    "offset": { "type": "integer", "description": "Skip this many leading Unicode scalar values before applying the byte cap" },
                    "char_offset": { "type": "integer", "description": "Alias for `offset`" },
                    "allow_binary": { "type": "boolean", "description": "If true, allow binary resources (placeholder text). Default false: binary-only resources return an error." }
                },
                "required": ["uri"]
            }),
        },
    ]
}

// ---------------------------------------------------------------------------
// Filesystem tools
// ---------------------------------------------------------------------------

/// SECURITY: Reject path traversal attempts. Forbids `..` components in file paths.
fn validate_path(path: &str) -> Result<&str, String> {
    for component in std::path::Path::new(path).components() {
        if matches!(component, std::path::Component::ParentDir) {
            return Err("Path traversal denied: '..' components are forbidden".to_string());
        }
    }
    Ok(path)
}

/// Resolve a file path through the workspace sandbox (if available) or legacy validation (writes).
pub(crate) fn resolve_file_path(
    raw_path: &str,
    workspace_root: Option<&Path>,
) -> Result<PathBuf, String> {
    if let Some(root) = workspace_root {
        crate::workspace_sandbox::resolve_sandbox_path(raw_path, root)
    } else {
        let _ = validate_path(raw_path)?;
        Ok(PathBuf::from(raw_path))
    }
}

/// Resolve a path for **reads**: workspace sandbox and/or virtual `ainl-library/...` tree.
pub(crate) fn resolve_file_path_read(
    raw_path: &str,
    workspace_root: Option<&Path>,
    ainl_library_root: Option<&Path>,
) -> Result<PathBuf, String> {
    let _ = validate_path(raw_path)?;
    if let Some(root) = workspace_root {
        crate::workspace_sandbox::resolve_sandbox_path_read(raw_path, root, ainl_library_root)
    } else {
        Ok(PathBuf::from(raw_path))
    }
}

async fn tool_file_read(
    input: &serde_json::Value,
    workspace_root: Option<&Path>,
    ainl_library_root: Option<&Path>,
) -> Result<String, String> {
    let raw_path = input["path"].as_str().ok_or("Missing 'path' parameter")?;
    let resolved = resolve_file_path_read(raw_path, workspace_root, ainl_library_root)?;

    // If the agent points file_read at a directory, transparently return a
    // file_list-style listing instead of failing. This avoids a wasted turn
    // (the previous behavior was to error out and force the agent to retry
    // with file_list — observed repeatedly across browser-hand / collector-hand
    // sessions). The output is clearly labelled so the model knows what it got.
    if let Ok(meta) = tokio::fs::metadata(&resolved).await {
        if meta.is_dir() {
            return list_directory_as_text(&resolved, raw_path).await;
        }
    }

    let bytes = tokio::fs::read(&resolved)
        .await
        .map_err(|e| format!("Failed to read file {}: {e}", resolved.display()))?;

    const MAX_READ_BYTES: usize = 2_000_000;
    if bytes.len() > MAX_READ_BYTES {
        return Err(format!(
            "File too large to read ({} bytes; max {} MB).",
            bytes.len(),
            MAX_READ_BYTES / (1024 * 1024)
        ));
    }

    match String::from_utf8(bytes) {
        Ok(text) => Ok(text),
        Err(e) => {
            let bytes = e.into_bytes();
            let lower = resolved.to_string_lossy().to_lowercase();
            if lower.ends_with(".pdf") || bytes.starts_with(b"%PDF") {
                Ok(format!(
                    "[PDF / binary document: {} bytes. file_read returns plain text only; use the document_extract tool for text, or export to text.]",
                    bytes.len()
                ))
            } else {
                Ok(format!(
                    "[Binary or non-UTF-8 file: {} bytes. For .xlsx/.docx/.pdf use document_extract.]",
                    bytes.len()
                ))
            }
        }
    }
}

async fn tool_file_write(
    input: &serde_json::Value,
    workspace_root: Option<&Path>,
) -> Result<String, String> {
    let raw_path = input["path"].as_str().ok_or("Missing 'path' parameter")?;
    let resolved = resolve_file_path(raw_path, workspace_root)?;
    let content = input["content"]
        .as_str()
        .ok_or("Missing 'content' parameter")?;

    // Hard-block writes that are obviously placeholder text the agent intended
    // to fill in but skipped (e.g. `[... full content truncated for brevity ...]`).
    // Without this guard the file lands on disk in a broken state and downstream
    // tools (`ainl validate`, compilers, etc.) fail in ways the agent can't trace
    // back to the original write call. The error message tells the agent exactly
    // what to do next.
    if let Some(snippet) = detect_truncation_placeholder(content) {
        return Err(format!(
            "Refused to write file: content contains a placeholder/truncation marker \
             ('{snippet}'). Re-emit `file_write` with the COMPLETE content of the file — \
             do not summarize, abbreviate, or insert '...truncated...' markers. If the \
             file is large, write it in multiple `apply_patch` chunks instead."
        ));
    }

    if let Some(parent) = resolved.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("Failed to create directories: {e}"))?;
    }
    tokio::fs::write(&resolved, content)
        .await
        .map_err(|e| format!("Failed to write file: {e}"))?;
    Ok(format!(
        "Successfully wrote {} bytes to {}",
        content.len(),
        resolved.display()
    ))
}

async fn tool_file_list(
    input: &serde_json::Value,
    workspace_root: Option<&Path>,
    ainl_library_root: Option<&Path>,
) -> Result<String, String> {
    let raw_path = input["path"].as_str().ok_or("Missing 'path' parameter")?;
    let resolved = resolve_file_path_read(raw_path, workspace_root, ainl_library_root)?;
    let mut entries = tokio::fs::read_dir(&resolved)
        .await
        .map_err(|e| format!("Failed to list directory {}: {e}", resolved.display()))?;
    let mut files = Vec::new();
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| format!("Failed to read entry: {e}"))?
    {
        let name = entry.file_name().to_string_lossy().to_string();
        let metadata = entry.metadata().await;
        let suffix = match metadata {
            Ok(m) if m.is_dir() => "/",
            _ => "",
        };
        files.push(format!("{name}{suffix}"));
    }
    files.sort();
    Ok(files.join("\n"))
}

/// Render a directory as a labelled text listing — used when `file_read` is
/// pointed at a directory so the agent gets the listing inline instead of
/// having to retry with `file_list`.
async fn list_directory_as_text(resolved: &Path, raw_path: &str) -> Result<String, String> {
    let mut entries = tokio::fs::read_dir(resolved)
        .await
        .map_err(|e| format!("Failed to list directory {}: {e}", resolved.display()))?;
    let mut files = Vec::new();
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| format!("Failed to read entry: {e}"))?
    {
        let name = entry.file_name().to_string_lossy().to_string();
        let metadata = entry.metadata().await;
        let suffix = match metadata {
            Ok(m) if m.is_dir() => "/",
            _ => "",
        };
        files.push(format!("{name}{suffix}"));
    }
    files.sort();
    let body = if files.is_empty() {
        "(empty)".to_string()
    } else {
        files.join("\n")
    };
    Ok(format!(
        "[file_read was given a directory at '{raw_path}'. Returned `file_list` output below; \
         call `file_read` again with one of these entries.]\n{body}"
    ))
}

/// Detect content the model wrote into `file_write` that is clearly a placeholder
/// the agent intended to fill in but skipped — e.g. literal `[... truncated for
/// brevity ...]`. Returns the offending fragment (lower-cased, ≤80 chars) so the
/// caller can surface it. Observed in production sessions where agents wrote a
/// broken `record_decision.ainl` containing `[... full content truncated for
/// brevity ...]` and downstream `validate`/`run` then failed in confusing ways.
fn detect_truncation_placeholder(content: &str) -> Option<String> {
    let lower = content.to_lowercase();
    const PATTERNS: &[&str] = &[
        "truncated for brevity",
        "truncated for readability",
        "[... full content",
        "[... rest of",
        "[... remaining",
        "// ... rest of file ...",
        "# ... rest of file ...",
        "<!-- truncated -->",
        "(content truncated)",
        "...truncated...",
    ];
    for p in PATTERNS {
        if let Some(idx) = lower.find(p) {
            let start = idx.saturating_sub(16);
            let end = (idx + p.len() + 16).min(content.len());
            let slice = &content[start..end];
            return Some(slice.replace('\n', " ").trim().to_string());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Patch tool
// ---------------------------------------------------------------------------

async fn tool_apply_patch(
    input: &serde_json::Value,
    workspace_root: Option<&Path>,
) -> Result<String, String> {
    let patch_str = input["patch"].as_str().ok_or("Missing 'patch' parameter")?;
    let root = workspace_root.ok_or("apply_patch requires a workspace root")?;
    let ops = crate::apply_patch::parse_patch(patch_str)?;
    let result = crate::apply_patch::apply_patch(&ops, root).await;
    if result.is_ok() {
        Ok(result.summary())
    } else {
        Err(format!(
            "Patch partially applied: {}. Errors: {}",
            result.summary(),
            result.errors.join("; ")
        ))
    }
}

// ---------------------------------------------------------------------------
// Web tools
// ---------------------------------------------------------------------------

/// Legacy web fetch (no SSRF protection, no readability). Used when WebToolsContext is unavailable.
async fn tool_web_fetch_legacy(input: &serde_json::Value) -> Result<String, String> {
    let url = input["url"].as_str().ok_or("Missing 'url' parameter")?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {e}"))?;
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {e}"))?;
    let status = resp.status();
    // Reject responses larger than 10MB to prevent memory exhaustion
    if let Some(len) = resp.content_length() {
        if len > 10 * 1024 * 1024 {
            return Err(format!("Response too large: {len} bytes (max 10MB)"));
        }
    }
    let body = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read response body: {e}"))?;
    let max_len = 50_000;
    let truncated = if body.len() > max_len {
        format!(
            "{}... [truncated, {} total bytes]",
            crate::str_utils::safe_truncate_str(&body, max_len),
            body.len()
        )
    } else {
        body
    };
    Ok(format!("HTTP {status}\n\n{truncated}"))
}

/// Legacy web search via DuckDuckGo HTML only. Used when WebToolsContext is unavailable.
async fn tool_web_search_legacy(input: &serde_json::Value) -> Result<String, String> {
    let query = input["query"].as_str().ok_or("Missing 'query' parameter")?;
    let max_results = input["max_results"].as_u64().unwrap_or(5) as usize;

    debug!(query, "Executing web search via DuckDuckGo HTML");

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {e}"))?;

    let resp = client
        .get("https://html.duckduckgo.com/html/")
        .query(&[("q", query)])
        .header("User-Agent", "Mozilla/5.0 (compatible; OpenFangAgent/0.1)")
        .send()
        .await
        .map_err(|e| format!("Search request failed: {e}"))?;

    let body = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read search response: {e}"))?;

    // Parse DuckDuckGo HTML results
    let results = parse_ddg_results(&body, max_results);

    if results.is_empty() {
        return Ok(format!("No results found for '{query}'."));
    }

    let mut output = format!("Search results for '{query}':\n\n");
    for (i, (title, url, snippet)) in results.iter().enumerate() {
        output.push_str(&format!(
            "{}. {}\n   URL: {}\n   {}\n\n",
            i + 1,
            title,
            url,
            snippet
        ));
    }

    Ok(output)
}

// ---------------------------------------------------------------------------
// Shell tool
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
async fn tool_shell_exec(
    input: &serde_json::Value,
    allowed_env: &[String],
    workspace_root: Option<&Path>,
    ainl_library_root: Option<&Path>,
    caller_agent_id: Option<&str>,
    process_manager: Option<&crate::process_manager::ProcessManager>,
    exec_policy: Option<&openfang_types::config::ExecPolicy>,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let command = input["command"]
        .as_str()
        .ok_or("Missing 'command' parameter")?;
    // Use LLM-specified timeout, or fall back to exec policy timeout, or default 30s
    let policy_timeout = exec_policy.map(|p| p.timeout_secs).unwrap_or(30);
    let timeout_secs = input["timeout_seconds"].as_u64().unwrap_or(policy_timeout);

    // SECURITY: Determine execution strategy based on exec policy.
    //
    // In Allowlist mode (default): Use direct execution via shlex argv splitting.
    // This avoids invoking a shell interpreter, which eliminates an entire class
    // of injection attacks (encoding tricks, $IFS, glob expansion, etc.).
    //
    // In Full mode: User explicitly opted into unrestricted shell access,
    // so we use sh -c / cmd /C as before.
    let use_direct_exec = exec_policy
        .map(|p| p.mode == openfang_types::config::ExecSecurityMode::Allowlist)
        .unwrap_or(true); // Default to safe mode

    let path_mode = exec_policy
        .map(|p| p.shell_path_guard)
        .unwrap_or(openfang_types::config::ShellPathGuardMode::Enforce);
    let pid_mode = exec_policy
        .map(|p| p.shell_pid_guard)
        .unwrap_or(openfang_types::config::ShellPidGuardMode::Enforce);
    let extra_allowed_path_prefixes = exec_policy
        .map(|p| p.extra_allowed_path_prefixes.as_slice())
        .unwrap_or(&[]);

    crate::shell_job_guard::preflight_shell_job_control(command, use_direct_exec)?;

    crate::shell_argv_guard::preflight_shell_exec(
        command,
        use_direct_exec,
        workspace_root,
        ainl_library_root,
        extra_allowed_path_prefixes,
        path_mode,
        pid_mode,
        caller_agent_id,
        process_manager,
        kernel,
    )?;

    let mut cmd = if use_direct_exec {
        // SAFE PATH: Split command into argv using POSIX shell lexer rules,
        // then execute the binary directly — no shell interpreter involved.
        let argv = shlex::split(command).ok_or_else(|| {
            "Command contains unmatched quotes or invalid shell syntax".to_string()
        })?;
        if argv.is_empty() {
            return Err("Empty command after parsing".to_string());
        }
        let mut c = tokio::process::Command::new(&argv[0]);
        if argv.len() > 1 {
            c.args(&argv[1..]);
        }
        c
    } else {
        // UNSAFE PATH: Full mode — user explicitly opted in to shell interpretation.
        // Shell resolution: prefer sh (Git Bash/MSYS2) on Windows.
        #[cfg(windows)]
        let git_sh: Option<&str> = {
            const SH_PATHS: &[&str] = &[
                "C:\\Program Files\\Git\\usr\\bin\\sh.exe",
                "C:\\Program Files (x86)\\Git\\usr\\bin\\sh.exe",
            ];
            SH_PATHS
                .iter()
                .copied()
                .find(|p| std::path::Path::new(p).exists())
        };
        let (shell, shell_arg) = if cfg!(windows) {
            #[cfg(windows)]
            {
                if let Some(sh) = git_sh {
                    (sh, "-c")
                } else {
                    ("cmd", "/C")
                }
            }
            #[cfg(not(windows))]
            {
                ("sh", "-c")
            }
        } else {
            ("sh", "-c")
        };
        let mut c = tokio::process::Command::new(shell);
        c.arg(shell_arg).arg(command);
        c
    };

    // Set working directory to agent workspace so files are created there
    if let Some(ws) = workspace_root {
        cmd.current_dir(ws);
    }

    // SECURITY: Isolate environment to prevent credential leakage.
    // Hand settings may grant access to specific provider API keys.
    crate::subprocess_sandbox::sandbox_command(&mut cmd, allowed_env);

    // Ensure UTF-8 output on Windows
    #[cfg(windows)]
    cmd.env("PYTHONIOENCODING", "utf-8");

    // Prevent child from inheriting stdin (avoids blocking on Windows)
    cmd.stdin(std::process::Stdio::null());

    // Best-effort: if this looks like a local dev server, ensure candidate listen ports are free
    // on loopback before spawning (avoids opaque EADDRINUSE loops).
    crate::shell_port_preflight::preflight_shell_listen_ports(command)?;

    let result =
        tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), cmd.output()).await;

    match result {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let exit_code = output.status.code().unwrap_or(-1);

            // Truncate very long outputs to prevent memory issues
            let max_output = 100_000;
            let mut stdout_str = if stdout.len() > max_output {
                format!(
                    "{}...\n[truncated, {} total bytes]",
                    crate::str_utils::safe_truncate_str(&stdout, max_output),
                    stdout.len()
                )
            } else {
                stdout.to_string()
            };
            let stderr_str = if stderr.len() > max_output {
                format!(
                    "{}...\n[truncated, {} total bytes]",
                    crate::str_utils::safe_truncate_str(&stderr, max_output),
                    stderr.len()
                )
            } else {
                stderr.to_string()
            };

            if exit_code == 0 && stdout_str.is_empty() {
                stdout_str = "Command executed successfully".to_string();
            }

            Ok(format!(
                "Exit code: {exit_code}\n\nSTDOUT:\n{stdout_str}\nSTDERR:\n{stderr_str}"
            ))
        }
        Ok(Err(e)) => Err(format!("Failed to execute command: {e}")),
        Err(_) => Err(format!("Command timed out after {timeout_secs}s")),
    }
}

// ---------------------------------------------------------------------------
// Inter-agent tools
// ---------------------------------------------------------------------------

fn require_kernel(
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<&Arc<dyn KernelHandle>, String> {
    kernel.ok_or_else(|| {
        "Kernel handle not available. Inter-agent tools require a running kernel.".to_string()
    })
}

async fn tool_agent_send(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    orchestration_live: Option<&OrchestrationLive>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let orchestration_ctx = orch_snapshot(orchestration_live).await;
    let agent_id = input["agent_id"]
        .as_str()
        .ok_or("Missing 'agent_id' parameter")?;
    let message = input["message"]
        .as_str()
        .ok_or("Missing 'message' parameter")?;

    if let Some(ref ctx) = orchestration_ctx {
        if ctx.budget_exhausted() {
            return Err(
                "Orchestration wall-clock budget is exhausted; cannot agent_send.".to_string(),
            );
        }
    }

    // Check + increment inter-agent call depth
    let current_depth = AGENT_CALL_DEPTH.try_with(|d| d.get()).unwrap_or(0);
    let max_depth = effective_max_agent_call_depth();
    if current_depth >= max_depth {
        return Err(format!(
            "Inter-agent call depth exceeded (max {}). \
             A->B->C chain is too deep. Use the task queue instead.",
            max_depth
        ));
    }

    let target_id = kh.resolve_agent_id(agent_id)?;
    let child_ctx =
        match caller_agent_id.and_then(|s| s.parse::<openfang_types::agent::AgentId>().ok()) {
            Some(caller_id) => {
                let base = orchestration_ctx.clone().unwrap_or_else(|| {
                    let efficient_mode = get_efficient_mode(orchestration_ctx.as_ref());
                    openfang_types::orchestration::OrchestrationContext::new_root(
                        caller_id,
                        openfang_types::orchestration::OrchestrationPattern::AdHoc,
                        efficient_mode,
                    )
                });
                Some(base.child(target_id))
            }
            None => orchestration_ctx.clone().map(|ctx| ctx.child(target_id)),
        };

    AGENT_CALL_DEPTH
        .scope(std::cell::Cell::new(current_depth + 1), async {
            kh.send_to_agent_with_context(agent_id, message, child_ctx)
                .await
        })
        .await
}

async fn tool_agent_spawn(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    parent_id: Option<&str>,
    orchestration_live: Option<&OrchestrationLive>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let manifest_toml = input["manifest_toml"]
        .as_str()
        .ok_or("Missing 'manifest_toml' parameter")?;
    let spawn_ctx = orch_snapshot(orchestration_live).await;
    let (id, name) = kh
        .spawn_agent_with_context(manifest_toml, parent_id, spawn_ctx)
        .await?;
    Ok(format!(
        "Agent spawned successfully.\n  ID: {id}\n  Name: {name}"
    ))
}

async fn tool_agent_delegate(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    orchestration_live: Option<&OrchestrationLive>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let orchestration_ctx = orch_snapshot(orchestration_live).await;
    if let Some(ref ctx) = orchestration_ctx {
        if ctx.budget_exhausted() {
            return Err(
                "Orchestration wall-clock budget is exhausted; cannot delegate.".to_string(),
            );
        }
    }
    let task = input["task"].as_str().ok_or("Missing 'task'")?;
    let required_caps = match input
        .get("required_capabilities")
        .and_then(|v| v.as_array())
    {
        Some(arr) => openfang_types::capability::parse_capability_requirements_array(arr)?,
        None => Vec::new(),
    };
    let preferred_tags: Vec<String> = input["preferred_tags"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let strategy = input["strategy"]
        .as_str()
        .and_then(|s| match s {
            "round_robin" => Some(openfang_types::orchestration::SelectionStrategy::RoundRobin),
            "least_busy" => Some(openfang_types::orchestration::SelectionStrategy::LeastBusy),
            "cost_efficient" => {
                Some(openfang_types::orchestration::SelectionStrategy::CostEfficient)
            }
            "best_match" => Some(openfang_types::orchestration::SelectionStrategy::BestMatch),
            "random" => Some(openfang_types::orchestration::SelectionStrategy::Random),
            _ => None,
        })
        .unwrap_or_default();
    let delegate_options =
        if let Some(o) = input.get("delegate_options").and_then(|v| v.as_object()) {
            openfang_types::orchestration::DelegateSelectionOptions {
                semantic_ranking: o
                    .get("semantic_ranking")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true),
                auto_spawn_pool: o
                    .get("auto_spawn_pool")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                auto_spawn_threshold: o
                    .get("auto_spawn_threshold")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(1) as u32,
            }
        } else {
            openfang_types::orchestration::DelegateSelectionOptions {
                semantic_ranking: input
                    .get("semantic_ranking")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true),
                auto_spawn_pool: input
                    .get("auto_spawn_pool")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                auto_spawn_threshold: input
                    .get("auto_spawn_threshold")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(1) as u32,
            }
        };
    let selected = kh
        .select_agent_for_task(
            task,
            &required_caps,
            &preferred_tags,
            strategy,
            delegate_options,
        )
        .await?;
    let delegator = caller_agent_id
        .and_then(|s| s.parse().ok())
        .unwrap_or(selected);
    let cap_str = format!("{required_caps:?}");
    let mut child = orchestration_ctx
        .clone()
        .unwrap_or_else(|| {
            let efficient_mode = get_efficient_mode(orchestration_ctx.as_ref());
            openfang_types::orchestration::OrchestrationContext::new_root(
                delegator,
                openfang_types::orchestration::OrchestrationPattern::AdHoc,
                efficient_mode,
            )
        })
        .child(selected);
    child.pattern = openfang_types::orchestration::OrchestrationPattern::Delegation {
        delegator_id: delegator,
        capability_required: cap_str,
    };
    let trace_id = child.trace_id.clone();
    let orchestrator_id = child.orchestrator_id;
    let parent_of_delegator = orchestration_ctx.as_ref().and_then(|c| {
        let n = c.call_chain.len();
        if n >= 2 {
            c.call_chain.get(n - 2).copied()
        } else {
            None
        }
    });
    let out = kh
        .send_to_agent_with_context(&selected.to_string(), task, Some(child))
        .await?;

    // AINL graph-memory write using GraphMemoryWriter
    if let Ok(gm) = crate::graph_memory_writer::GraphMemoryWriter::open(&delegator.to_string()) {
        let trace_event = serde_json::to_value(
            &openfang_types::orchestration_trace::OrchestrationTraceEvent {
                trace_id: trace_id.clone(),
                orchestrator_id,
                agent_id: delegator,
                parent_agent_id: parent_of_delegator,
                event_type: openfang_types::orchestration_trace::TraceEventType::AgentDelegated {
                    target_agent: selected,
                    task: task.to_string(),
                },
                timestamp: chrono::Utc::now(),
                metadata: std::collections::HashMap::new(),
            },
        )
        .ok();

        let mem = crate::memory_project_scope::memory_project_id_from_process_env();
        let _ = gm
            .record_turn(
                vec!["agent_delegate".to_string()],
                Some(selected.to_string()),
                trace_event,
                &[],
                None,
                None,
                None,
                mem.as_deref(),
            )
            .await;
    }

    kh.record_orchestration_trace(
        openfang_types::orchestration_trace::OrchestrationTraceEvent {
            trace_id,
            orchestrator_id,
            agent_id: delegator,
            parent_agent_id: parent_of_delegator,
            event_type: openfang_types::orchestration_trace::TraceEventType::AgentDelegated {
                target_agent: selected,
                task: task.to_string(),
            },
            timestamp: chrono::Utc::now(),
            metadata: std::collections::HashMap::new(),
        },
    );
    Ok(out)
}

async fn tool_agent_map_reduce(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    orchestration_live: Option<&OrchestrationLive>,
    _caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let orchestration_ctx = orch_snapshot(orchestration_live).await;
    // Note: map_reduce uses child contexts, inheriting efficient_mode through ctx.child()
    // If no orchestration context exists, agents use their own manifest settings
    let items: Vec<String> = input["items"]
        .as_array()
        .ok_or("Missing 'items'")?
        .iter()
        .filter_map(|v| v.as_str().map(String::from))
        .collect();
    if items.is_empty() {
        return Err("items array is empty".to_string());
    }
    let map_prompt_template = input["map_prompt_template"]
        .as_str()
        .ok_or("Missing 'map_prompt_template'")?;
    let map_agent = input["map_agent"].as_str().ok_or("Missing 'map_agent'")?;
    let max_parallelism = input["max_parallelism"].as_u64().unwrap_or(3).clamp(1, 3) as usize;
    let job_id = uuid::Uuid::new_v4().to_string();
    let mut map_results: Vec<serde_json::Value> = Vec::new();
    let map_target_id = kh
        .resolve_agent_id(map_agent)
        .unwrap_or_else(|_| openfang_types::agent::AgentId::new());
    let kh_arc = Arc::clone(kh);
    let mut global_idx: usize = 0;

    for chunk in items.chunks(max_parallelism) {
        let mut futs = Vec::new();
        for item in chunk.iter() {
            let idx = global_idx;
            global_idx += 1;
            let prompt = map_prompt_template.replace("{{item}}", item);
            let child_ctx = orchestration_ctx.as_ref().map(|ctx| {
                let mut c = ctx.child(map_target_id);
                c.pattern = openfang_types::orchestration::OrchestrationPattern::MapReduce {
                    job_id: job_id.clone(),
                    phase: openfang_types::orchestration::MapReducePhase::Map,
                    item_index: Some(idx),
                };
                c
            });
            let kh2 = Arc::clone(&kh_arc);
            let target = map_agent.to_string();
            let item_owned = item.clone();
            futs.push(async move {
                let r = kh2
                    .send_to_agent_with_context(&target, &prompt, child_ctx)
                    .await;
                (item_owned, r)
            });
        }
        for (item, r) in futures::future::join_all(futs).await {
            let text = r?;
            map_results.push(serde_json::json!({"item": item, "result": text}));
        }
    }

    let reduce_template = input["reduce_prompt_template"].as_str();
    let Some(reduce_template) = reduce_template else {
        return serde_json::to_string_pretty(&serde_json::json!({
            "map_results": map_results,
            "job_id": job_id,
        }))
        .map_err(|e| e.to_string());
    };

    let combined = map_results
        .iter()
        .filter_map(|v| v.get("result").and_then(|x| x.as_str()))
        .collect::<Vec<_>>()
        .join("\n\n---\n\n");
    let reduce_prompt = reduce_template.replace("{{results}}", &combined);
    let reduce_agent = input["reduce_agent"].as_str().unwrap_or("self");
    if reduce_agent == "self" {
        return serde_json::to_string_pretty(&serde_json::json!({
            "map_results": map_results,
            "reduce_prompt": reduce_prompt,
            "note": "reduce_agent=self: continue in your reasoning using reduce_prompt to produce the final answer.",
            "job_id": job_id,
        }))
        .map_err(|e| e.to_string());
    }

    let reduce_target_id = kh
        .resolve_agent_id(reduce_agent)
        .unwrap_or_else(|_| openfang_types::agent::AgentId::new());
    let reduce_ctx = orchestration_ctx.as_ref().map(|ctx| {
        let mut c = ctx.child(reduce_target_id);
        c.pattern = openfang_types::orchestration::OrchestrationPattern::MapReduce {
            job_id: job_id.clone(),
            phase: openfang_types::orchestration::MapReducePhase::Reduce,
            item_index: None,
        };
        c
    });
    let reduced = kh
        .send_to_agent_with_context(reduce_agent, &reduce_prompt, reduce_ctx)
        .await?;
    serde_json::to_string_pretty(&serde_json::json!({
        "map_results": map_results,
        "reduce_result": reduced,
        "job_id": job_id,
    }))
    .map_err(|e| e.to_string())
}

async fn tool_agent_supervise(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    orchestration_live: Option<&OrchestrationLive>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let orchestration_ctx = orch_snapshot(orchestration_live).await;
    let agent_id = input["agent_id"].as_str().ok_or("Missing 'agent_id'")?;
    let task = input["task"].as_str().ok_or("Missing 'task'")?;
    let max_duration = input["max_duration_secs"].as_u64().unwrap_or(600);
    let target = kh.resolve_agent_id(agent_id)?;
    let supervisor_id = caller_agent_id
        .and_then(|s| s.parse().ok())
        .unwrap_or(target);
    let mut ctx = orchestration_ctx
        .clone()
        .unwrap_or_else(|| {
            let efficient_mode = get_efficient_mode(orchestration_ctx.as_ref());
            openfang_types::orchestration::OrchestrationContext::new_root(
                supervisor_id,
                openfang_types::orchestration::OrchestrationPattern::AdHoc,
                efficient_mode,
            )
        })
        .child(target);
    ctx.pattern = openfang_types::orchestration::OrchestrationPattern::Supervisor {
        supervisor_id,
        task_type: "supervised_task".to_string(),
    };
    let fut = kh.send_to_agent_with_context(agent_id, task, Some(ctx));
    match tokio::time::timeout(std::time::Duration::from_secs(max_duration), fut).await {
        Ok(Ok(response)) => {
            if let Some(crit) = input.get("success_criteria").and_then(|v| v.as_str()) {
                let lc = crit.to_lowercase();
                if !response.to_lowercase().contains(&lc) {
                    return Ok(format!(
                        "Supervised task completed but success_criteria '{crit}' not found in response.\n\n{response}"
                    ));
                }
            }
            Ok(response)
        }
        Ok(Err(e)) => Err(e),
        Err(_) => Err(format!("Supervised task timed out after {max_duration}s")),
    }
}

async fn tool_agent_coordinate(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    orchestration_live: Option<&OrchestrationLive>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let orchestration_ctx = orch_snapshot(orchestration_live).await;
    // Get efficient_mode for orchestration inheritance
    let efficient_mode = get_efficient_mode(orchestration_ctx.as_ref());
    let tasks = input["tasks"].as_array().ok_or("Missing 'tasks'")?;
    if tasks.is_empty() {
        return Err("tasks array is empty".to_string());
    }
    let timeout_per_task = input["timeout_per_task"].as_u64().unwrap_or(300);
    let coordinator_id = caller_agent_id
        .and_then(|s| s.parse().ok())
        .unwrap_or_default();
    let task_group_id = uuid::Uuid::new_v4().to_string();

    #[derive(Debug, Clone)]
    struct Node {
        id: String,
        agent: String,
        prompt: String,
        deps: Vec<String>,
    }
    let mut nodes: Vec<Node> = Vec::new();
    for t in tasks {
        let id = t["id"].as_str().ok_or("task missing id")?.to_string();
        let agent = t["agent"].as_str().ok_or("task missing agent")?.to_string();
        let prompt = t["prompt"]
            .as_str()
            .ok_or("task missing prompt")?
            .to_string();
        let deps: Vec<String> = t["depends_on"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        nodes.push(Node {
            id,
            agent,
            prompt,
            deps,
        });
    }

    let mut outputs: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut pending: std::collections::HashSet<String> =
        nodes.iter().map(|n| n.id.clone()).collect();
    let mut results_json = Vec::new();

    while !pending.is_empty() {
        let ready: Vec<Node> = nodes
            .iter()
            .filter(|n| pending.contains(&n.id) && n.deps.iter().all(|d| outputs.contains_key(d)))
            .cloned()
            .collect();
        if ready.is_empty() {
            return Err("Coordinate: cyclic dependency or missing task id".to_string());
        }
        let mut wave = Vec::new();
        for node in ready {
            pending.remove(&node.id);
            let mut prompt = node.prompt.clone();
            for (k, v) in &outputs {
                let placeholder = ["{{", k.as_str(), "}}"].concat();
                prompt = prompt.replace(&placeholder, v);
            }
            let kh2 = Arc::clone(kh);
            let oid = orchestration_ctx.clone();
            let cid = coordinator_id;
            let gid = task_group_id.clone();
            let id_copy = node.id.clone();
            let agent_copy = node.agent.clone();
            let efficient_mode_copy = efficient_mode.clone();
            wave.push(async move {
                let target_id = kh2
                    .resolve_agent_id(&agent_copy)
                    .unwrap_or_else(|_| openfang_types::agent::AgentId::new());
                let mut c = oid
                    .unwrap_or_else(|| {
                        openfang_types::orchestration::OrchestrationContext::new_root(
                            cid,
                            openfang_types::orchestration::OrchestrationPattern::AdHoc,
                            efficient_mode_copy,
                        )
                    })
                    .child(target_id);
                c.pattern = openfang_types::orchestration::OrchestrationPattern::Coordination {
                    coordinator_id: cid,
                    task_id: gid,
                };
                let r = tokio::time::timeout(
                    std::time::Duration::from_secs(timeout_per_task),
                    kh2.send_to_agent_with_context(&agent_copy, &prompt, Some(c)),
                )
                .await;
                (id_copy, r)
            });
        }
        for (id, r) in futures::future::join_all(wave).await {
            match r {
                Ok(Ok(text)) => {
                    outputs.insert(id.clone(), text.clone());
                    results_json.push(serde_json::json!({"id": id, "output": text}));
                }
                Ok(Err(e)) => return Err(format!("Task {id} failed: {e}")),
                Err(_) => return Err(format!("Task {id} timed out after {timeout_per_task}s")),
            }
        }
    }

    serde_json::to_string_pretty(&serde_json::json!({ "results": results_json }))
        .map_err(|e| e.to_string())
}

fn tool_agent_list(kernel: Option<&Arc<dyn KernelHandle>>) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agents = kh.list_agents();
    if agents.is_empty() {
        return Ok("No agents currently running.".to_string());
    }
    let mut output = format!("Running agents ({}):\n", agents.len());
    for a in &agents {
        output.push_str(&format!(
            "  - {} (id: {}, state: {}, model: {}:{})\n",
            a.name, a.id, a.state, a.model_provider, a.model_name
        ));
        if !a.description.is_empty() {
            output.push_str(&format!("    description: {}\n", a.description));
        }
        if !a.tags.is_empty() {
            output.push_str(&format!("    tags: {}\n", a.tags.join(", ")));
        }
        if !a.tools.is_empty() {
            // Show first 8 tools to keep output readable; a full list is available via agent_find
            let shown: Vec<&String> = a.tools.iter().take(8).collect();
            let suffix = if a.tools.len() > 8 {
                format!(" (+{} more)", a.tools.len() - 8)
            } else {
                String::new()
            };
            output.push_str(&format!(
                "    tools: {}{}\n",
                shown
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
                suffix
            ));
        }
    }
    Ok(output)
}

fn tool_agent_kill(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agent_id = input["agent_id"]
        .as_str()
        .ok_or("Missing 'agent_id' parameter")?;
    kh.kill_agent(agent_id)?;
    Ok(format!("Agent {agent_id} killed successfully."))
}

// ---------------------------------------------------------------------------
// Shared memory tools
// ---------------------------------------------------------------------------

fn tool_memory_store(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let key = input["key"].as_str().ok_or("Missing 'key' parameter")?;
    let value = input.get("value").ok_or("Missing 'value' parameter")?;
    kh.memory_store(key, value.clone())?;
    Ok(format!("Stored value under key '{key}'."))
}

fn tool_memory_recall(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let key = input["key"].as_str().ok_or("Missing 'key' parameter")?;
    match kh.memory_recall(key)? {
        Some(val) => Ok(serde_json::to_string_pretty(&val).unwrap_or_else(|_| val.to_string())),
        None => Ok(format!("No value found for key '{key}'.")),
    }
}

fn tool_memory_list(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let prefix = input["prefix"].as_str();
    let entries = kh.memory_list(prefix)?;
    if entries.is_empty() {
        return Ok(match prefix {
            Some(p) => format!("No memory keys found matching prefix '{p}'."),
            None => "No memory keys stored yet.".to_string(),
        });
    }
    let result: Vec<serde_json::Value> = entries
        .into_iter()
        .map(|(k, v)| serde_json::json!({ "key": k, "value": v }))
        .collect();
    serde_json::to_string_pretty(&result).map_err(|e| format!("Serialize error: {e}"))
}

// ---------------------------------------------------------------------------
// Collaboration tools
// ---------------------------------------------------------------------------

fn tool_agent_find(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let query = input["query"].as_str().ok_or("Missing 'query' parameter")?;
    let agents = kh.find_agents(query);
    if agents.is_empty() {
        return Ok(format!("No agents found matching '{query}'."));
    }
    let result: Vec<serde_json::Value> = agents
        .iter()
        .map(|a| {
            serde_json::json!({
                "id": a.id,
                "name": a.name,
                "state": a.state,
                "description": a.description,
                "tags": a.tags,
                "tools": a.tools,
                "model": format!("{}:{}", a.model_provider, a.model_name),
            })
        })
        .collect();
    serde_json::to_string_pretty(&result).map_err(|e| format!("Serialize error: {e}"))
}

fn tool_agent_find_capabilities(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let caps = match input
        .get("required_capabilities")
        .and_then(|v| v.as_array())
    {
        Some(arr) => openfang_types::capability::parse_capability_requirements_array(arr)?,
        None => return Err("Missing 'required_capabilities' array".to_string()),
    };
    let preferred_tags: Vec<String> = input["preferred_tags"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let exclude: Vec<openfang_types::agent::AgentId> = input["exclude_agent_ids"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().and_then(|s| s.parse().ok()))
                .collect()
        })
        .unwrap_or_default();
    let agents = kh.find_by_capabilities(&caps, &preferred_tags, &exclude);
    let result: Vec<serde_json::Value> = agents
        .iter()
        .map(|a| {
            serde_json::json!({
                "id": a.id,
                "name": a.name,
                "state": a.state,
                "description": a.description,
                "tags": a.tags,
                "tools": a.tools,
                "model": format!("{}:{}", a.model_provider, a.model_name),
            })
        })
        .collect();
    serde_json::to_string_pretty(&result).map_err(|e| format!("Serialize error: {e}"))
}

async fn tool_agent_pool_list(kernel: Option<&Arc<dyn KernelHandle>>) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let rows = kh.list_agent_pools();
    serde_json::to_string_pretty(&rows).map_err(|e| e.to_string())
}

async fn tool_agent_pool_spawn(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let pool_name = input["pool_name"]
        .as_str()
        .or_else(|| input["name"].as_str())
        .ok_or("Missing 'pool_name'")?;
    let (id, name) = kh
        .spawn_agent_pool_worker(pool_name, caller_agent_id)
        .await?;
    Ok(format!("Spawned pool worker {name} ({id})."))
}

async fn tool_orchestration_shared_merge(
    input: &serde_json::Value,
    orch: Option<&OrchestrationLive>,
) -> Result<String, String> {
    let Some(a) = orch else {
        return Err(
            "orchestration_shared_merge requires an active orchestration context".to_string(),
        );
    };
    let patch = input
        .get("patch")
        .and_then(|v| v.as_object())
        .ok_or("Missing 'patch' object")?;
    let n = patch.len();
    let mut w = a.write().await;
    let mut m = HashMap::new();
    for (k, v) in patch {
        m.insert(k.clone(), v.clone());
    }
    w.merge_shared_vars(m);
    let total = w.shared_vars.len();
    Ok(format!(
        "Merged {n} key(s) into orchestration shared_vars ({total} total)."
    ))
}

async fn tool_task_post(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
    orchestration_live: Option<&OrchestrationLive>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let title = input["title"].as_str().ok_or("Missing 'title' parameter")?;
    let description = input["description"]
        .as_str()
        .ok_or("Missing 'description' parameter")?;
    let assigned_to = input["assigned_to"].as_str();
    let mut meta = serde_json::Map::new();
    if let Some(extra) = input.get("payload").and_then(|v| v.as_object()) {
        for (k, v) in extra {
            meta.insert(k.clone(), v.clone());
        }
    }
    if let Some(o) = orchestration_live {
        let g = o.read().await;
        meta.insert(
            "orchestration".to_string(),
            serde_json::json!({
                "trace_id": g.trace_id,
                "orchestrator_id": g.orchestrator_id.to_string(),
            }),
        );
    }
    let payload = if meta.is_empty() {
        None
    } else {
        Some(serde_json::Value::Object(meta))
    };
    let priority = input.get("priority").and_then(|v| v.as_i64()).unwrap_or(0);
    let task_id = kh
        .task_post(
            title,
            description,
            assigned_to,
            caller_agent_id,
            payload,
            priority,
        )
        .await?;
    Ok(format!("Task created with ID: {task_id}"))
}

async fn tool_task_claim(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
    orchestration_live: Option<&OrchestrationLive>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agent_id = caller_agent_id.unwrap_or("");
    let prefer = input
        .get("prefer_orchestration_trace_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let prefer = if let Some(s) = prefer {
        Some(s)
    } else if let Some(o) = orchestration_live {
        Some(o.read().await.trace_id.clone())
    } else {
        None
    };
    let strategy = input
        .get("strategy")
        .and_then(|v| v.as_str())
        .map(|s| match s {
            "prefer_unassigned" => TaskClaimStrategy::PreferUnassigned,
            "sticky_only" => TaskClaimStrategy::StickyOnly,
            _ => TaskClaimStrategy::Default,
        })
        .unwrap_or_default();
    match kh.task_claim(agent_id, prefer.as_deref(), strategy).await? {
        Some(task) => {
            if let Ok(claimant) = kh.resolve_agent_id(agent_id) {
                if let Some(ctx) = orchestration_context_from_claimed_task(&task, claimant) {
                    let _ = kh.set_pending_orchestration_ctx(agent_id, ctx.clone());
                    if let Some(live) = orchestration_live {
                        *live.write().await = ctx;
                    }
                }
            }
            serde_json::to_string_pretty(&task).map_err(|e| format!("Serialize error: {e}"))
        }
        None => Ok("No tasks available.".to_string()),
    }
}

async fn tool_task_complete(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let task_id = input["task_id"]
        .as_str()
        .ok_or("Missing 'task_id' parameter")?;
    let result = input["result"]
        .as_str()
        .ok_or("Missing 'result' parameter")?;
    kh.task_complete(task_id, result).await?;
    Ok(format!("Task {task_id} marked as completed."))
}

async fn tool_task_list(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let status = input["status"].as_str();
    let tasks = kh.task_list(status).await?;
    if tasks.is_empty() {
        return Ok("No tasks found.".to_string());
    }
    serde_json::to_string_pretty(&tasks).map_err(|e| format!("Serialize error: {e}"))
}

async fn tool_event_publish(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let event_type = input["event_type"]
        .as_str()
        .ok_or("Missing 'event_type' parameter")?;
    let payload = input
        .get("payload")
        .cloned()
        .unwrap_or(serde_json::json!({}));
    kh.publish_event(event_type, payload).await?;
    Ok(format!("Event '{event_type}' published successfully."))
}

// ---------------------------------------------------------------------------
// Knowledge graph tools
// ---------------------------------------------------------------------------

fn parse_entity_type(s: &str) -> openfang_types::memory::EntityType {
    use openfang_types::memory::EntityType;
    match s.to_lowercase().as_str() {
        "person" => EntityType::Person,
        "organization" | "org" => EntityType::Organization,
        "project" => EntityType::Project,
        "concept" => EntityType::Concept,
        "event" => EntityType::Event,
        "location" => EntityType::Location,
        "document" | "doc" => EntityType::Document,
        "tool" => EntityType::Tool,
        other => EntityType::Custom(other.to_string()),
    }
}

fn parse_relation_type(s: &str) -> openfang_types::memory::RelationType {
    use openfang_types::memory::RelationType;
    match s.to_lowercase().as_str() {
        "works_at" | "worksat" => RelationType::WorksAt,
        "knows_about" | "knowsabout" | "knows" => RelationType::KnowsAbout,
        "related_to" | "relatedto" | "related" => RelationType::RelatedTo,
        "depends_on" | "dependson" | "depends" => RelationType::DependsOn,
        "owned_by" | "ownedby" => RelationType::OwnedBy,
        "created_by" | "createdby" => RelationType::CreatedBy,
        "located_in" | "locatedin" => RelationType::LocatedIn,
        "part_of" | "partof" => RelationType::PartOf,
        "uses" => RelationType::Uses,
        "produces" => RelationType::Produces,
        other => RelationType::Custom(other.to_string()),
    }
}

async fn tool_knowledge_add_entity(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let name = input["name"].as_str().ok_or("Missing 'name' parameter")?;
    let entity_type_str = input["entity_type"]
        .as_str()
        .ok_or("Missing 'entity_type' parameter")?;
    let properties = input
        .get("properties")
        .and_then(|v| v.as_object())
        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default();

    let entity = openfang_types::memory::Entity {
        id: String::new(), // kernel/store assigns a real ID
        entity_type: parse_entity_type(entity_type_str),
        name: name.to_string(),
        properties,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };

    let id = kh.knowledge_add_entity(entity).await?;
    Ok(format!("Entity '{name}' added with ID: {id}"))
}

async fn tool_knowledge_add_relation(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let source = input["source"]
        .as_str()
        .ok_or("Missing 'source' parameter")?;
    let relation_str = input["relation"]
        .as_str()
        .ok_or("Missing 'relation' parameter")?;
    let target = input["target"]
        .as_str()
        .ok_or("Missing 'target' parameter")?;
    let confidence = input["confidence"].as_f64().unwrap_or(1.0) as f32;
    let properties = input
        .get("properties")
        .and_then(|v| v.as_object())
        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default();

    let relation = openfang_types::memory::Relation {
        source: source.to_string(),
        relation: parse_relation_type(relation_str),
        target: target.to_string(),
        properties,
        confidence,
        created_at: chrono::Utc::now(),
    };

    let id = kh.knowledge_add_relation(relation).await?;
    Ok(format!(
        "Relation '{source}' --[{relation_str}]--> '{target}' added with ID: {id}"
    ))
}

async fn tool_knowledge_query(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let source = input["source"].as_str().map(|s| s.to_string());
    let target = input["target"].as_str().map(|s| s.to_string());
    let relation = input["relation"].as_str().map(parse_relation_type);
    let max_depth = input["max_depth"].as_u64().unwrap_or(1) as u32;

    let pattern = openfang_types::memory::GraphPattern {
        source,
        relation,
        target,
        max_depth,
    };

    let matches = kh.knowledge_query(pattern).await?;
    if matches.is_empty() {
        return Ok("No matching knowledge graph entries found.".to_string());
    }

    let mut output = format!("Found {} match(es):\n", matches.len());
    for m in &matches {
        output.push_str(&format!(
            "\n  {} ({:?}) --[{:?} ({:.0}%)]--> {} ({:?})",
            m.source.name,
            m.source.entity_type,
            m.relation.relation,
            m.relation.confidence * 100.0,
            m.target.name,
            m.target.entity_type,
        ));
    }
    Ok(output)
}

// ---------------------------------------------------------------------------
// Scheduling tools
// ---------------------------------------------------------------------------

/// Parse a natural language schedule into a cron expression.
fn parse_schedule_to_cron(input: &str) -> Result<String, String> {
    let input = input.trim().to_lowercase();

    // If it already looks like a cron expression (5 space-separated fields), pass through
    let parts: Vec<&str> = input.split_whitespace().collect();
    if parts.len() == 5
        && parts
            .iter()
            .all(|p| p.chars().all(|c| c.is_ascii_digit() || "*/,-".contains(c)))
    {
        return Ok(input);
    }

    // Natural language patterns
    if let Some(rest) = input.strip_prefix("every ") {
        if rest == "minute" || rest == "1 minute" {
            return Ok("* * * * *".to_string());
        }
        if let Some(mins) = rest.strip_suffix(" minutes") {
            let n: u32 = mins
                .trim()
                .parse()
                .map_err(|_| format!("Invalid number in '{input}'"))?;
            if n == 0 || n > 59 {
                return Err(format!("Minutes must be 1-59, got {n}"));
            }
            return Ok(format!("*/{n} * * * *"));
        }
        if rest == "hour" || rest == "1 hour" {
            return Ok("0 * * * *".to_string());
        }
        if let Some(hrs) = rest.strip_suffix(" hours") {
            let n: u32 = hrs
                .trim()
                .parse()
                .map_err(|_| format!("Invalid number in '{input}'"))?;
            if n == 0 || n > 23 {
                return Err(format!("Hours must be 1-23, got {n}"));
            }
            return Ok(format!("0 */{n} * * *"));
        }
        if rest == "day" || rest == "1 day" {
            return Ok("0 0 * * *".to_string());
        }
        if rest == "week" || rest == "1 week" {
            return Ok("0 0 * * 0".to_string());
        }
    }

    // "daily at Xam/pm"
    if let Some(time_str) = input.strip_prefix("daily at ") {
        let hour = parse_time_to_hour(time_str)?;
        return Ok(format!("0 {hour} * * *"));
    }

    // "weekdays at Xam/pm"
    if let Some(time_str) = input.strip_prefix("weekdays at ") {
        let hour = parse_time_to_hour(time_str)?;
        return Ok(format!("0 {hour} * * 1-5"));
    }

    // "weekends at Xam/pm"
    if let Some(time_str) = input.strip_prefix("weekends at ") {
        let hour = parse_time_to_hour(time_str)?;
        return Ok(format!("0 {hour} * * 0,6"));
    }

    // "hourly" / "daily" / "weekly" / "monthly"
    match input.as_str() {
        "hourly" => return Ok("0 * * * *".to_string()),
        "daily" => return Ok("0 0 * * *".to_string()),
        "weekly" => return Ok("0 0 * * 0".to_string()),
        "monthly" => return Ok("0 0 1 * *".to_string()),
        _ => {}
    }

    Err(format!(
        "Could not parse schedule '{input}'. Try: 'every 5 minutes', 'daily at 9am', 'weekdays at 6pm', or a cron expression like '0 */5 * * *'"
    ))
}

/// Parse a time string like "9am", "6pm", "14:00", "9:30am" into an hour (0-23).
fn parse_time_to_hour(s: &str) -> Result<u32, String> {
    let s = s.trim().to_lowercase();

    // Handle "9am", "6pm", "12pm", "12am"
    if let Some(h) = s.strip_suffix("am") {
        let hour: u32 = h.trim().parse().map_err(|_| format!("Invalid time: {s}"))?;
        return match hour {
            12 => Ok(0),
            1..=11 => Ok(hour),
            _ => Err(format!("Invalid hour: {hour}")),
        };
    }
    if let Some(h) = s.strip_suffix("pm") {
        let hour: u32 = h.trim().parse().map_err(|_| format!("Invalid time: {s}"))?;
        return match hour {
            12 => Ok(12),
            1..=11 => Ok(hour + 12),
            _ => Err(format!("Invalid hour: {hour}")),
        };
    }

    // Handle "14:00" or "9:30"
    if let Some((h, _m)) = s.split_once(':') {
        let hour: u32 = h.trim().parse().map_err(|_| format!("Invalid time: {s}"))?;
        if hour > 23 {
            return Err(format!("Hour must be 0-23, got {hour}"));
        }
        return Ok(hour);
    }

    // Plain number
    let hour: u32 = s.parse().map_err(|_| format!("Invalid time: {s}"))?;
    if hour > 23 {
        return Err(format!("Hour must be 0-23, got {hour}"));
    }
    Ok(hour)
}

const SCHEDULES_KEY: &str = "__openfang_schedules";

fn sanitize_cron_job_name(description: &str) -> String {
    let cleaned: String = description
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let mut name = cleaned
        .split_whitespace()
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if name.is_empty() {
        name = "scheduled-job".to_string();
    }
    if name.len() > 120 {
        name.truncate(120);
    }
    name
}

/// Friendly wrapper around [`KernelHandle::cron_create`] — registers the real kernel scheduler.
async fn tool_schedule_create(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agent_id = caller_agent_id.ok_or("Agent ID required for schedule_create")?;
    let description = input["description"]
        .as_str()
        .ok_or("Missing 'description' parameter")?;
    let schedule_str = input["schedule"]
        .as_str()
        .ok_or("Missing 'schedule' parameter")?;
    let cron_expr = parse_schedule_to_cron(schedule_str)?;
    let timeout_secs = input["timeout_secs"]
        .as_u64()
        .unwrap_or(300)
        .clamp(10, 3600);

    let name = sanitize_cron_job_name(description);

    let action = if let Some(p) = input["program_path"].as_str() {
        serde_json::json!({
            "kind": "ainl_run",
            "program_path": p,
            "timeout_secs": timeout_secs
        })
    } else if input.get("action").map(|v| v.is_object()).unwrap_or(false) {
        input["action"].clone()
    } else {
        let msg = input["message"].as_str().unwrap_or(description);
        serde_json::json!({
            "kind": "agent_turn",
            "message": format!("[Scheduled] {msg}"),
            "timeout_secs": timeout_secs
        })
    };

    let delivery = if input
        .get("delivery")
        .map(|v| v.is_object())
        .unwrap_or(false)
    {
        input["delivery"].clone()
    } else {
        serde_json::json!({"kind": "none"})
    };

    let enabled = input["enabled"].as_bool().unwrap_or(true);

    let body = serde_json::json!({
        "name": name,
        "agent_id": agent_id,
        "schedule": { "kind": "cron", "expr": cron_expr },
        "action": action,
        "delivery": delivery,
        "enabled": enabled,
    });

    kh.cron_create(agent_id, body).await
}

async fn tool_schedule_list(
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agent_id = caller_agent_id.ok_or("Agent ID required for schedule_list")?;
    let jobs = kh.cron_list(agent_id).await?;
    let mut out = serde_json::to_string_pretty(&jobs)
        .map_err(|e| format!("Failed to serialize jobs: {e}"))?;
    if let Ok(Some(serde_json::Value::Array(arr))) = kh.memory_recall(SCHEDULES_KEY) {
        if !arr.is_empty() {
            out.push_str("\n\n(Legacy note: old memory-only schedule entries exist under __openfang_schedules — they never ran on a timer. New jobs use the kernel cron file; ignore or clear stale memory keys if you migrated.)");
        }
    }
    Ok(out)
}

async fn tool_schedule_delete(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let job_id = input["job_id"]
        .as_str()
        .or_else(|| input["id"].as_str())
        .ok_or("Missing 'job_id' or 'id' parameter (cron job UUID)")?;
    tool_cron_cancel(&serde_json::json!({ "job_id": job_id }), kernel).await
}

fn tool_channels_list(kernel: Option<&Arc<dyn KernelHandle>>) -> String {
    let kh = match require_kernel(kernel) {
        Ok(k) => k,
        Err(e) => return e,
    };
    kh.list_channels_summary()
}

// ---------------------------------------------------------------------------
// Cron scheduling tools (delegated to kernel via KernelHandle trait)
// ---------------------------------------------------------------------------

async fn tool_cron_create(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agent_id = caller_agent_id.ok_or("Agent ID required for cron_create")?;
    kh.cron_create(agent_id, input.clone()).await
}

async fn tool_cron_list(
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agent_id = caller_agent_id.ok_or("Agent ID required for cron_list")?;
    let jobs = kh.cron_list(agent_id).await?;
    serde_json::to_string_pretty(&jobs).map_err(|e| format!("Failed to serialize cron jobs: {e}"))
}

async fn tool_cron_cancel(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let job_id = input["job_id"]
        .as_str()
        .ok_or("Missing 'job_id' parameter")?;
    kh.cron_cancel(job_id).await?;
    Ok(format!("Cron job '{job_id}' cancelled."))
}

// ---------------------------------------------------------------------------
// Channel send tool (proactive outbound messaging via configured adapters)
// ---------------------------------------------------------------------------

async fn tool_channel_send(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    workspace_root: Option<&Path>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;

    let channel = input["channel"]
        .as_str()
        .ok_or("Missing 'channel' parameter")?
        .trim()
        .to_lowercase();
    let recipient_input = input["recipient"]
        .as_str()
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    // If recipient is empty, resolve from channel's default_chat_id config.
    let recipient = if recipient_input.is_empty() {
        let default_id = kh.get_channel_default_recipient(&channel).await;
        match default_id {
            Some(id) => id,
            None => {
                return Err(format!(
                "Missing 'recipient' parameter. Set default_chat_id in [channels.{channel}] config \
                 or pass recipient explicitly."
            ))
            }
        }
    } else {
        recipient_input
    };
    let recipient = recipient.as_str();

    let thread_id = input["thread_id"].as_str().filter(|s| !s.is_empty());

    // Check for media content (image_url, file_url, or file_path)
    let image_url = input["image_url"].as_str().filter(|s| !s.is_empty());
    let file_url = input["file_url"].as_str().filter(|s| !s.is_empty());
    let file_path = input["file_path"].as_str().filter(|s| !s.is_empty());

    if let Some(url) = image_url {
        let caption = input["message"].as_str().filter(|s| !s.is_empty());
        return kh
            .send_channel_media(&channel, recipient, "image", url, caption, None, thread_id)
            .await;
    }

    if let Some(url) = file_url {
        let caption = input["message"].as_str().filter(|s| !s.is_empty());
        let filename = input["filename"].as_str();
        return kh
            .send_channel_media(
                &channel, recipient, "file", url, caption, filename, thread_id,
            )
            .await;
    }

    // Local file attachment: read from disk and send as FileData
    if let Some(raw_path) = file_path {
        let resolved = resolve_file_path(raw_path, workspace_root)?;
        let data = tokio::fs::read(&resolved)
            .await
            .map_err(|e| format!("Failed to read file '{}': {e}", resolved.display()))?;

        // Derive filename from the path if not explicitly provided
        let filename = input["filename"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                resolved
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("file")
                    .to_string()
            });

        // Determine MIME type from extension
        let ext = resolved
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let mime_type = match ext.as_str() {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "svg" => "image/svg+xml",
            "pdf" => "application/pdf",
            "txt" => "text/plain",
            "csv" => "text/csv",
            "json" => "application/json",
            "xml" => "application/xml",
            "zip" => "application/zip",
            "gz" | "gzip" => "application/gzip",
            "tar" => "application/x-tar",
            "mp3" => "audio/mpeg",
            "wav" => "audio/wav",
            "mp4" => "video/mp4",
            "doc" => "application/msword",
            "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "xls" => "application/vnd.ms-excel",
            "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            _ => "application/octet-stream",
        };

        return kh
            .send_channel_file_data(&channel, recipient, data, &filename, mime_type, thread_id)
            .await;
    }

    // Text-only message
    let message = input["message"]
        .as_str()
        .ok_or("Missing 'message' parameter (required for text messages)")?;

    if message.is_empty() {
        return Err("Message cannot be empty".to_string());
    }

    // For email channels, validate email format and prepend subject
    let final_message = if channel == "email" {
        if !recipient.contains('@') || !recipient.contains('.') {
            return Err(format!("Invalid email address: '{recipient}'"));
        }
        if let Some(subject) = input["subject"].as_str() {
            if !subject.is_empty() {
                format!("Subject: {subject}\n\n{message}")
            } else {
                message.to_string()
            }
        } else {
            message.to_string()
        }
    } else {
        message.to_string()
    };

    kh.send_channel_message(&channel, recipient, &final_message, thread_id)
        .await
}

/// Send a real-time progress update to a channel mid-task.
/// Functionally identical to channel_send but semantically scoped to status pings —
/// the description and parameter names guide the LLM to use it appropriately.
async fn tool_channel_stream(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;

    let channel = input["channel"]
        .as_str()
        .ok_or("Missing 'channel' parameter")?
        .trim()
        .to_lowercase();
    let message = input["message"]
        .as_str()
        .ok_or("Missing 'message' parameter")?;
    let thread_id = input["thread_id"].as_str().filter(|s| !s.is_empty());

    let recipient_input = input["recipient"]
        .as_str()
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    let recipient_owned = if recipient_input.is_empty() {
        match kh.get_channel_default_recipient(&channel).await {
            Some(id) => id,
            None => {
                return Err(format!(
                    "Missing 'recipient' and no default_chat_id configured for channel '{channel}'."
                ))
            }
        }
    } else {
        recipient_input
    };

    kh.send_channel_message(&channel, &recipient_owned, message, thread_id)
        .await
        .map(|_| {
            serde_json::json!({
                "sent": true,
                "channel": channel,
                "recipient": recipient_owned,
            })
            .to_string()
        })
}

// ---------------------------------------------------------------------------
// Hand tools (delegated to kernel via KernelHandle trait)
// ---------------------------------------------------------------------------

async fn tool_hand_list(kernel: Option<&Arc<dyn KernelHandle>>) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let hands = kh.hand_list().await?;

    if hands.is_empty() {
        return Ok(
            "No Hands available. Install hands to enable curated autonomous packages.".to_string(),
        );
    }

    let mut lines = vec!["Available Hands:".to_string(), String::new()];
    for h in &hands {
        let icon = h["icon"].as_str().unwrap_or("");
        let name = h["name"].as_str().unwrap_or("?");
        let id = h["id"].as_str().unwrap_or("?");
        let status = h["status"].as_str().unwrap_or("unknown");
        let desc = h["description"].as_str().unwrap_or("");

        let status_marker = match status {
            "Active" => "[ACTIVE]",
            "Paused" => "[PAUSED]",
            _ => "[available]",
        };

        lines.push(format!("{} {} ({}) {}", icon, name, id, status_marker));
        if !desc.is_empty() {
            lines.push(format!("  {}", desc));
        }
        if let Some(iid) = h["instance_id"].as_str() {
            lines.push(format!("  Instance: {}", iid));
        }
        lines.push(String::new());
    }

    Ok(lines.join("\n"))
}

async fn tool_hand_activate(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let hand_id = input["hand_id"]
        .as_str()
        .ok_or("Missing 'hand_id' parameter")?;
    let config: std::collections::HashMap<String, serde_json::Value> =
        if let Some(obj) = input["config"].as_object() {
            obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
        } else {
            std::collections::HashMap::new()
        };

    let result = kh.hand_activate(hand_id, config).await?;

    let instance_id = result["instance_id"].as_str().unwrap_or("?");
    let agent_name = result["agent_name"].as_str().unwrap_or("?");
    let status = result["status"].as_str().unwrap_or("?");

    Ok(format!(
        "Hand '{}' activated!\n  Instance: {}\n  Agent: {} ({})",
        hand_id, instance_id, agent_name, status
    ))
}

async fn tool_hand_status(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let hand_id = input["hand_id"]
        .as_str()
        .ok_or("Missing 'hand_id' parameter")?;

    let result = kh.hand_status(hand_id).await?;

    let icon = result["icon"].as_str().unwrap_or("");
    let name = result["name"].as_str().unwrap_or(hand_id);
    let status = result["status"].as_str().unwrap_or("unknown");
    let instance_id = result["instance_id"].as_str().unwrap_or("?");
    let agent_name = result["agent_name"].as_str().unwrap_or("?");
    let activated = result["activated_at"].as_str().unwrap_or("?");

    Ok(format!(
        "{} {} — {}\n  Instance: {}\n  Agent: {}\n  Activated: {}",
        icon, name, status, instance_id, agent_name, activated
    ))
}

async fn tool_hand_deactivate(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let instance_id = input["instance_id"]
        .as_str()
        .ok_or("Missing 'instance_id' parameter")?;
    kh.hand_deactivate(instance_id).await?;
    Ok(format!("Hand instance '{}' deactivated.", instance_id))
}

// ---------------------------------------------------------------------------
// A2A outbound tools (cross-instance agent communication)
// ---------------------------------------------------------------------------

/// Discover an external A2A agent by fetching its agent card.
async fn tool_a2a_discover(input: &serde_json::Value) -> Result<String, String> {
    let url = input["url"].as_str().ok_or("Missing 'url' parameter")?;

    // SSRF protection: block private/metadata IPs
    if crate::web_fetch::check_ssrf(url, &[]).is_err() {
        return Err("SSRF blocked: URL resolves to a private or metadata address".to_string());
    }

    let client = crate::a2a::A2aClient::new();
    let card = client.discover(url).await?;

    serde_json::to_string_pretty(&card).map_err(|e| format!("Serialization error: {e}"))
}

/// Send a task to an external A2A agent.
async fn tool_a2a_send(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let message = input["message"]
        .as_str()
        .ok_or("Missing 'message' parameter")?;

    // Resolve agent URL: either directly provided or looked up by name
    let url = if let Some(url) = input["agent_url"].as_str() {
        // SSRF protection
        if crate::web_fetch::check_ssrf(url, &[]).is_err() {
            return Err("SSRF blocked: URL resolves to a private or metadata address".to_string());
        }
        url.to_string()
    } else if let Some(name) = input["agent_name"].as_str() {
        kh.get_a2a_agent_url(name)
            .ok_or_else(|| format!("No known A2A agent with name '{name}'. Use a2a_discover first or provide agent_url directly."))?
    } else {
        return Err("Missing 'agent_url' or 'agent_name' parameter".to_string());
    };

    let session_id = input["session_id"].as_str();
    let client = crate::a2a::A2aClient::new();
    let task = client.send_task(&url, message, session_id).await?;

    // Record A2A delegation in AINL graph memory
    if let Some(caller_id) = caller_agent_id {
        if let Ok(gm) = crate::graph_memory_writer::GraphMemoryWriter::open(caller_id) {
            let target = input["agent_name"].as_str().unwrap_or("external-a2a-agent");
            gm.record_delegation(target, vec!["a2a_send".to_string()])
                .await;
        }
    }

    serde_json::to_string_pretty(&task).map_err(|e| format!("Serialization error: {e}"))
}

async fn tool_hermes_a2a_status() -> Result<String, String> {
    let root = crate::hermes_a2a::hermes_root();
    let path = crate::hermes_a2a::hermes_a2a_config_path();
    let mut out = serde_json::json!({
        "hermes_root": root.display().to_string(),
        "config_path": path.display().to_string(),
        "config_exists": path.is_file(),
    });
    if path.is_file() {
        match crate::hermes_a2a::load_hermes_a2a_config() {
            Ok(cfg) => {
                out["base_url"] = serde_json::Value::String(cfg.base_url);
                out["send_binding"] =
                    serde_json::Value::String(cfg.send_binding.as_str().to_string());
                out["ok"] = serde_json::json!(true);
            }
            Err(e) => {
                out["ok"] = serde_json::json!(false);
                out["error"] = serde_json::Value::String(e);
            }
        }
    } else {
        out["ok"] = serde_json::json!(false);
        out["hint"] = serde_json::Value::String(
            "Nous Hermes Agent does not ship an A2A listener yet (see hermes-agent issue #514). When you run an A2A-capable server, create a2a.json with base_url (+ optional send_binding: auto | armaraos_jsonrpc | a2a_http).".to_string(),
        );
    }
    serde_json::to_string_pretty(&out).map_err(|e| format!("Serialization error: {e}"))
}

async fn tool_a2a_discover_hermes() -> Result<String, String> {
    let (base, _cfg_path) = crate::hermes_a2a::load_hermes_a2a_base_url()?;
    let client = crate::a2a::A2aClient::new();
    let card = client.discover(&base).await?;
    serde_json::to_string_pretty(&card).map_err(|e| format!("Serialization error: {e}"))
}

async fn tool_a2a_send_hermes(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let _kh = require_kernel(kernel)?;
    let message = input["message"]
        .as_str()
        .ok_or("Missing 'message' parameter")?;
    let cfg = crate::hermes_a2a::load_hermes_a2a_config()?;
    let session_id = input["session_id"].as_str();
    let client = crate::a2a::A2aClient::new();
    let card = client.discover(&cfg.base_url).await?;

    let mut last_err = String::new();

    let try_jsonrpc = matches!(
        cfg.send_binding,
        crate::hermes_a2a::HermesSendBinding::Auto
            | crate::hermes_a2a::HermesSendBinding::ArmaraosJsonRpc
    );
    if try_jsonrpc {
        match crate::hermes_a2a::assert_hermes_rpc_matches_base(&cfg.base_url, &card.url) {
            Ok(()) => match client.send_task(&card.url, message, session_id).await {
                Ok(task) => {
                    if let Some(caller_id) = caller_agent_id {
                        if let Ok(gm) =
                            crate::graph_memory_writer::GraphMemoryWriter::open(caller_id)
                        {
                            let target = format!("hermes-a2a:{}", card.name);
                            gm.record_delegation(&target, vec!["a2a_send_hermes".to_string()])
                                .await;
                        }
                    }
                    return serde_json::to_string_pretty(&task)
                        .map_err(|e| format!("Serialization error: {e}"));
                }
                Err(e) => {
                    if matches!(
                        cfg.send_binding,
                        crate::hermes_a2a::HermesSendBinding::ArmaraosJsonRpc
                    ) {
                        return Err(e);
                    }
                    last_err = e;
                }
            },
            Err(e) => {
                if matches!(
                    cfg.send_binding,
                    crate::hermes_a2a::HermesSendBinding::ArmaraosJsonRpc
                ) {
                    return Err(e);
                }
            }
        }
    }

    let try_http = matches!(
        cfg.send_binding,
        crate::hermes_a2a::HermesSendBinding::Auto | crate::hermes_a2a::HermesSendBinding::A2aHttp
    );
    if try_http {
        for ep in crate::a2a::A2aClient::message_send_endpoints(&cfg.base_url, &card) {
            match client.post_message_send(&ep, message).await {
                Ok(v) => {
                    if let Some(caller_id) = caller_agent_id {
                        if let Ok(gm) =
                            crate::graph_memory_writer::GraphMemoryWriter::open(caller_id)
                        {
                            let target = format!("hermes-a2a:{}", card.name);
                            gm.record_delegation(&target, vec!["a2a_send_hermes".to_string()])
                                .await;
                        }
                    }
                    return serde_json::to_string_pretty(&v)
                        .map_err(|e| format!("Serialization error: {e}"));
                }
                Err(e) => last_err = e,
            }
        }
    }

    Err(if last_err.is_empty() {
        "Hermes A2A: no working send path. For Linux Foundation HTTP binding ensure POST {base_url}/message:send exists; for ArmaraOS-style peers use send_binding=armaraos_jsonrpc and matching AgentCard.url origin."
            .to_string()
    } else {
        last_err
    })
}

// ---------------------------------------------------------------------------
// Image analysis tool
// ---------------------------------------------------------------------------

async fn tool_image_analyze(input: &serde_json::Value) -> Result<String, String> {
    let path = input["path"].as_str().ok_or("Missing 'path' parameter")?;
    let prompt = input["prompt"].as_str().unwrap_or("");

    let data = tokio::fs::read(path)
        .await
        .map_err(|e| format!("Failed to read image '{path}': {e}"))?;

    let file_size = data.len();

    // Detect image format from magic bytes
    let format = detect_image_format(&data);

    // Extract dimensions for common formats
    let dimensions = extract_image_dimensions(&data, &format);

    // Base64-encode (truncate for very large images in the response)
    let base64_preview = if file_size <= 512 * 1024 {
        // Under 512KB — include full base64
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(&data)
    } else {
        // Over 512KB — include first 64KB preview
        use base64::Engine;
        let preview_bytes = &data[..64 * 1024];
        format!(
            "{}... [truncated, {} total bytes]",
            base64::engine::general_purpose::STANDARD.encode(preview_bytes),
            file_size
        )
    };

    let mut result = serde_json::json!({
        "path": path,
        "format": format,
        "file_size_bytes": file_size,
        "file_size_human": format_file_size(file_size),
    });

    if let Some((w, h)) = dimensions {
        result["width"] = serde_json::json!(w);
        result["height"] = serde_json::json!(h);
    }

    if !prompt.is_empty() {
        result["prompt"] = serde_json::json!(prompt);
        result["note"] = serde_json::json!(
            "Vision analysis requires a vision-capable LLM. The base64 data is included for downstream processing."
        );
    }

    result["base64_preview"] = serde_json::json!(base64_preview);

    serde_json::to_string_pretty(&result).map_err(|e| format!("Serialize error: {e}"))
}

/// Detect image format from magic bytes.
fn detect_image_format(data: &[u8]) -> String {
    if data.len() < 4 {
        return "unknown".to_string();
    }
    if data.starts_with(b"\x89PNG") {
        "png".to_string()
    } else if data.starts_with(b"\xFF\xD8\xFF") {
        "jpeg".to_string()
    } else if data.starts_with(b"GIF8") {
        "gif".to_string()
    } else if data.starts_with(b"RIFF") && data.len() > 12 && &data[8..12] == b"WEBP" {
        "webp".to_string()
    } else if data.starts_with(b"BM") {
        "bmp".to_string()
    } else if data.starts_with(b"\x00\x00\x01\x00") {
        "ico".to_string()
    } else {
        "unknown".to_string()
    }
}

/// Extract image dimensions from common formats.
fn extract_image_dimensions(data: &[u8], format: &str) -> Option<(u32, u32)> {
    match format {
        "png" => {
            // PNG: IHDR chunk starts at byte 16, width at 16-19, height at 20-23
            if data.len() >= 24 {
                let w = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
                let h = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
                Some((w, h))
            } else {
                None
            }
        }
        "gif" => {
            // GIF: width at bytes 6-7, height at bytes 8-9 (little-endian)
            if data.len() >= 10 {
                let w = u16::from_le_bytes([data[6], data[7]]) as u32;
                let h = u16::from_le_bytes([data[8], data[9]]) as u32;
                Some((w, h))
            } else {
                None
            }
        }
        "bmp" => {
            // BMP: width at bytes 18-21, height at bytes 22-25 (little-endian)
            if data.len() >= 26 {
                let w = u32::from_le_bytes([data[18], data[19], data[20], data[21]]);
                let h = u32::from_le_bytes([data[22], data[23], data[24], data[25]]);
                Some((w, h))
            } else {
                None
            }
        }
        "jpeg" => {
            // JPEG: scan for SOF0 marker (0xFF 0xC0) to find dimensions
            extract_jpeg_dimensions(data)
        }
        _ => None,
    }
}

/// Extract JPEG dimensions by scanning for SOF markers.
fn extract_jpeg_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    let mut i = 2; // Skip SOI marker
    while i + 1 < data.len() {
        if data[i] != 0xFF {
            i += 1;
            continue;
        }
        let marker = data[i + 1];
        // SOF0-SOF3 markers contain dimensions
        if (0xC0..=0xC3).contains(&marker) && i + 9 < data.len() {
            let h = u16::from_be_bytes([data[i + 5], data[i + 6]]) as u32;
            let w = u16::from_be_bytes([data[i + 7], data[i + 8]]) as u32;
            return Some((w, h));
        }
        if i + 3 < data.len() {
            let seg_len = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
            i += 2 + seg_len;
        } else {
            break;
        }
    }
    None
}

/// Format file size in human-readable form.
fn format_file_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

// ---------------------------------------------------------------------------
// Location tool
// ---------------------------------------------------------------------------

async fn tool_location_get() -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {e}"))?;

    // Use ip-api.com (free, no API key, JSON response)
    let resp = client
        .get("https://ip-api.com/json/?fields=status,message,country,regionName,city,zip,lat,lon,timezone,isp,query")
        .header("User-Agent", "ArmaraOS/0.1")
        .send()
        .await
        .map_err(|e| format!("Location request failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("Location API returned {}", resp.status()));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse location response: {e}"))?;

    if body["status"].as_str() != Some("success") {
        let msg = body["message"].as_str().unwrap_or("Unknown error");
        return Err(format!("Location lookup failed: {msg}"));
    }

    let result = serde_json::json!({
        "lat": body["lat"],
        "lon": body["lon"],
        "city": body["city"],
        "region": body["regionName"],
        "country": body["country"],
        "zip": body["zip"],
        "timezone": body["timezone"],
        "isp": body["isp"],
        "ip": body["query"],
    });

    serde_json::to_string_pretty(&result).map_err(|e| format!("Serialize error: {e}"))
}

// ---------------------------------------------------------------------------
// System time tool
// ---------------------------------------------------------------------------

/// Return current date, time, timezone, and Unix epoch.
fn tool_system_time() -> String {
    let now_utc = chrono::Utc::now();
    let now_local = chrono::Local::now();
    let result = serde_json::json!({
        "utc": now_utc.to_rfc3339(),
        "local": now_local.to_rfc3339(),
        "unix_epoch": now_utc.timestamp(),
        "timezone": now_local.format("%Z").to_string(),
        "utc_offset": now_local.format("%:z").to_string(),
        "date": now_local.format("%Y-%m-%d").to_string(),
        "time": now_local.format("%H:%M:%S").to_string(),
        "day_of_week": now_local.format("%A").to_string(),
    });
    serde_json::to_string_pretty(&result).unwrap_or_else(|_| now_utc.to_rfc3339())
}

// ---------------------------------------------------------------------------
// Media understanding tools
// ---------------------------------------------------------------------------

/// Describe an image using a vision-capable LLM provider.
async fn tool_media_describe(
    input: &serde_json::Value,
    media_engine: Option<&crate::media_understanding::MediaEngine>,
) -> Result<String, String> {
    use base64::Engine;
    let engine = media_engine.ok_or("Media engine not available. Check media configuration.")?;
    let path = input["path"].as_str().ok_or("Missing 'path' parameter")?;
    let _ = validate_path(path)?;

    // Read image file
    let data = tokio::fs::read(path)
        .await
        .map_err(|e| format!("Failed to read image file: {e}"))?;

    // Detect MIME type from extension
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let mime = match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        _ => return Err(format!("Unsupported image format: .{ext}")),
    };

    let attachment = openfang_types::media::MediaAttachment {
        media_type: openfang_types::media::MediaType::Image,
        mime_type: mime.to_string(),
        source: openfang_types::media::MediaSource::Base64 {
            data: base64::engine::general_purpose::STANDARD.encode(&data),
            mime_type: mime.to_string(),
        },
        size_bytes: data.len() as u64,
    };

    let understanding = engine.describe_image(&attachment).await?;
    serde_json::to_string_pretty(&understanding).map_err(|e| format!("Serialize error: {e}"))
}

/// Transcribe audio to text using speech-to-text.
async fn tool_media_transcribe(
    input: &serde_json::Value,
    media_engine: Option<&crate::media_understanding::MediaEngine>,
    workspace_root: Option<&std::path::Path>,
    ainl_library_root: Option<&std::path::Path>,
) -> Result<String, String> {
    use base64::Engine;
    use std::path::PathBuf;
    let engine = media_engine.ok_or("Media engine not available. Check media configuration.")?;

    let file_id = input["file_id"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let path_str = input["path"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let (resolved, mime): (PathBuf, String) = match (file_id, path_str) {
        (Some(fid), _) => {
            if uuid::Uuid::parse_str(fid).is_err() {
                return Err(
                    "Invalid file_id: expected the upload UUID from the message (format xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx). \
If you used the display filename such as voice_….webm, use the file_id from the **Voice/audio attachments** block instead — not the synthetic name."
                        .to_string(),
                );
            }
            let p = std::env::temp_dir().join("openfang_uploads").join(fid);
            let ct = input["content_type"]
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("audio/webm")
                .to_string();
            let ct = openfang_types::media::normalize_mime_type(&ct);
            (p, ct)
        }
        (None, Some(p)) => {
            let _ = validate_path(p)?;
            let resolved = resolve_file_path_read(p, workspace_root, ainl_library_root)?;
            let ext = resolved
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            let mime = match ext.as_str() {
                "mp3" => "audio/mpeg",
                "wav" => "audio/wav",
                "ogg" => "audio/ogg",
                "flac" => "audio/flac",
                "m4a" => "audio/mp4",
                "webm" => "audio/webm",
                _ => return Err(format!("Unsupported audio format: .{ext}")),
            };
            (resolved, mime.to_string())
        }
        (None, None) => {
            return Err(
                "Missing 'path' or 'file_id'. For voice uploads use file_id from the message hint."
                    .to_string(),
            );
        }
    };

    // Read audio file
    let data = tokio::fs::read(&resolved)
        .await
        .map_err(|e| format!("Failed to read audio file: {e}"))?;

    let attachment = openfang_types::media::MediaAttachment {
        media_type: openfang_types::media::MediaType::Audio,
        mime_type: mime.clone(),
        source: openfang_types::media::MediaSource::Base64 {
            data: base64::engine::general_purpose::STANDARD.encode(&data),
            mime_type: mime,
        },
        size_bytes: data.len() as u64,
    };

    let understanding = engine.transcribe_audio(&attachment).await?;
    serde_json::to_string_pretty(&understanding).map_err(|e| format!("Serialize error: {e}"))
}

// ---------------------------------------------------------------------------
// Image generation tool
// ---------------------------------------------------------------------------

/// Generate images from a text prompt.
async fn tool_image_generate(
    input: &serde_json::Value,
    workspace_root: Option<&Path>,
) -> Result<String, String> {
    let prompt = input["prompt"]
        .as_str()
        .ok_or("Missing 'prompt' parameter")?;

    let model_str = input["model"].as_str().unwrap_or("dall-e-3");
    let model = match model_str {
        "dall-e-3" | "dalle3" | "dalle-3" => openfang_types::media::ImageGenModel::DallE3,
        "dall-e-2" | "dalle2" | "dalle-2" => openfang_types::media::ImageGenModel::DallE2,
        "gpt-image-1" | "gpt_image_1" => openfang_types::media::ImageGenModel::GptImage1,
        _ => {
            return Err(format!(
                "Unknown image model: {model_str}. Use 'dall-e-3', 'dall-e-2', or 'gpt-image-1'."
            ))
        }
    };

    let size = input["size"].as_str().unwrap_or("1024x1024").to_string();
    let quality = input["quality"].as_str().unwrap_or("hd").to_string();
    let count = input["count"].as_u64().unwrap_or(1).min(4) as u8;

    let request = openfang_types::media::ImageGenRequest {
        prompt: prompt.to_string(),
        model,
        size,
        quality,
        count,
    };

    let result = crate::image_gen::generate_image(&request).await?;

    // Save images to workspace if available
    let saved_paths = if let Some(workspace) = workspace_root {
        match crate::image_gen::save_images_to_workspace(&result, workspace) {
            Ok(paths) => paths,
            Err(e) => {
                warn!("Failed to save images to workspace: {e}");
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };

    // Also save to the uploads temp dir so the web UI can serve them via
    // GET /api/uploads/{file_id}.  Each image gets a UUID filename.
    let mut image_urls: Vec<String> = Vec::new();
    {
        use base64::Engine;
        let upload_dir = std::env::temp_dir().join("openfang_uploads");
        let _ = std::fs::create_dir_all(&upload_dir);
        for img in &result.images {
            let file_id = uuid::Uuid::new_v4().to_string();
            if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(&img.data_base64)
            {
                let path = upload_dir.join(&file_id);
                if std::fs::write(&path, &decoded).is_ok() {
                    image_urls.push(format!("/api/uploads/{file_id}"));
                }
            }
        }
    }

    // Build response — include image_urls so the dashboard can render <img> tags
    let response = serde_json::json!({
        "model": result.model,
        "images_generated": result.images.len(),
        "saved_to": saved_paths,
        "revised_prompt": result.revised_prompt,
        "image_urls": image_urls,
    });

    serde_json::to_string_pretty(&response).map_err(|e| format!("Serialize error: {e}"))
}

// ---------------------------------------------------------------------------
// TTS / STT tools
// ---------------------------------------------------------------------------

async fn tool_text_to_speech(
    input: &serde_json::Value,
    tts_engine: Option<&crate::tts::TtsEngine>,
    workspace_root: Option<&Path>,
) -> Result<String, String> {
    let engine =
        tts_engine.ok_or("TTS engine not available. Ensure tts.enabled=true in config.")?;
    let text = input["text"].as_str().ok_or("Missing 'text' parameter")?;
    let voice = input["voice"].as_str();
    let format = input["format"].as_str();

    let result = engine.synthesize(text, voice, format).await?;

    // Save audio to workspace
    let saved_path = if let Some(workspace) = workspace_root {
        let output_dir = workspace.join("output");
        tokio::fs::create_dir_all(&output_dir)
            .await
            .map_err(|e| format!("Failed to create output dir: {e}"))?;

        let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S").to_string();
        let filename = format!("tts_{timestamp}.{}", result.format);
        let path = output_dir.join(&filename);

        tokio::fs::write(&path, &result.audio_data)
            .await
            .map_err(|e| format!("Failed to write audio file: {e}"))?;

        Some(path.display().to_string())
    } else {
        None
    };

    // Also copy to the uploads temp dir so the dashboard can play it directly
    // via GET /api/uploads/{file_id} — same pattern as image_generate.
    let audio_url: Option<String> = {
        let upload_dir = std::env::temp_dir().join("openfang_uploads");
        let _ = std::fs::create_dir_all(&upload_dir);
        let ext = &result.format;
        let file_id = format!("{}.{ext}", uuid::Uuid::new_v4());
        let upload_path = upload_dir.join(&file_id);
        if std::fs::write(&upload_path, &result.audio_data).is_ok() {
            Some(format!("/api/uploads/{file_id}"))
        } else {
            None
        }
    };

    let response = serde_json::json!({
        "saved_to": saved_path,
        "audio_url": audio_url,
        "format": result.format,
        "provider": result.provider,
        "duration_estimate_ms": result.duration_estimate_ms,
        "size_bytes": result.audio_data.len(),
    });

    serde_json::to_string_pretty(&response).map_err(|e| format!("Serialize error: {e}"))
}

async fn tool_speech_to_text(
    input: &serde_json::Value,
    media_engine: Option<&crate::media_understanding::MediaEngine>,
    workspace_root: Option<&Path>,
) -> Result<String, String> {
    let engine = media_engine.ok_or("Media engine not available for speech-to-text")?;
    let raw_path = input["path"].as_str().ok_or("Missing 'path' parameter")?;
    let _language = input["language"].as_str();

    let resolved = resolve_file_path(raw_path, workspace_root)?;

    // Read the audio file
    let data = tokio::fs::read(&resolved)
        .await
        .map_err(|e| format!("Failed to read audio file: {e}"))?;

    // Determine MIME type from extension
    let ext = resolved
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("mp3");
    let mime_type = match ext {
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "flac" => "audio/flac",
        "m4a" => "audio/mp4",
        "webm" => "audio/webm",
        _ => "audio/mpeg",
    };

    use openfang_types::media::{MediaAttachment, MediaSource, MediaType};
    let attachment = MediaAttachment {
        media_type: MediaType::Audio,
        mime_type: mime_type.to_string(),
        source: MediaSource::Base64 {
            data: {
                use base64::Engine;
                base64::engine::general_purpose::STANDARD.encode(&data)
            },
            mime_type: mime_type.to_string(),
        },
        size_bytes: data.len() as u64,
    };

    let understanding = engine.transcribe_audio(&attachment).await?;

    let response = serde_json::json!({
        "transcript": understanding.description,
        "provider": understanding.provider,
        "model": understanding.model,
    });

    serde_json::to_string_pretty(&response).map_err(|e| format!("Serialize error: {e}"))
}

// ---------------------------------------------------------------------------
// Docker sandbox tool
// ---------------------------------------------------------------------------

async fn tool_docker_exec(
    input: &serde_json::Value,
    docker_config: Option<&openfang_types::config::DockerSandboxConfig>,
    workspace_root: Option<&Path>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let config = docker_config.ok_or("Docker sandbox not configured")?;

    if !config.enabled {
        return Err("Docker sandbox is disabled. Set docker.enabled=true in config.".into());
    }

    let command = input["command"]
        .as_str()
        .ok_or("Missing 'command' parameter")?;

    let workspace = workspace_root.ok_or("Docker exec requires a workspace directory")?;
    let agent_id = caller_agent_id.unwrap_or("default");

    // Check Docker availability
    if !crate::docker_sandbox::is_docker_available().await {
        return Err(
            "Docker is not available on this system. Install Docker to use docker_exec.".into(),
        );
    }

    // Create sandbox container
    let container = crate::docker_sandbox::create_sandbox(config, agent_id, workspace).await?;

    // Execute command with timeout
    let timeout = std::time::Duration::from_secs(config.timeout_secs);
    let result = crate::docker_sandbox::exec_in_sandbox(&container, command, timeout).await;

    // Always destroy the container after execution
    if let Err(e) = crate::docker_sandbox::destroy_sandbox(&container).await {
        warn!("Failed to destroy Docker sandbox: {e}");
    }

    let exec_result = result?;

    let response = serde_json::json!({
        "exit_code": exec_result.exit_code,
        "stdout": exec_result.stdout,
        "stderr": exec_result.stderr,
        "container_id": container.container_id,
    });

    serde_json::to_string_pretty(&response).map_err(|e| format!("Serialize error: {e}"))
}

// ---------------------------------------------------------------------------
// Persistent process tools
// ---------------------------------------------------------------------------

/// Start a long-running process (REPL, server, watcher).
async fn tool_process_start(
    input: &serde_json::Value,
    pm: Option<&crate::process_manager::ProcessManager>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let pm = pm.ok_or("Process manager not available")?;
    let agent_id = caller_agent_id.unwrap_or("default");
    let command = input["command"]
        .as_str()
        .ok_or("Missing 'command' parameter")?;
    let args: Vec<String> = input["args"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let env: Option<std::collections::HashMap<String, String>> =
        input.get("env").and_then(|v| v.as_object()).map(|obj| {
            obj.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        });
    let cwd = input["cwd"].as_str();

    let proc_id = pm
        .start(agent_id, command, &args, env.as_ref(), cwd)
        .await?;
    Ok(serde_json::json!({
        "process_id": proc_id,
        "status": "started"
    })
    .to_string())
}

/// Read accumulated stdout/stderr from a process (non-blocking drain).
async fn tool_process_poll(
    input: &serde_json::Value,
    pm: Option<&crate::process_manager::ProcessManager>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let pm = pm.ok_or("Process manager not available")?;
    let agent_id = caller_agent_id.unwrap_or("default");
    let proc_id = input["process_id"]
        .as_str()
        .ok_or("Missing 'process_id' parameter")?;
    let (stdout, stderr) = pm.read_for_agent(proc_id, agent_id).await?;
    Ok(serde_json::json!({
        "stdout": stdout,
        "stderr": stderr,
    })
    .to_string())
}

/// Write data to a process's stdin.
async fn tool_process_write(
    input: &serde_json::Value,
    pm: Option<&crate::process_manager::ProcessManager>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let pm = pm.ok_or("Process manager not available")?;
    let agent_id = caller_agent_id.unwrap_or("default");
    let proc_id = input["process_id"]
        .as_str()
        .ok_or("Missing 'process_id' parameter")?;
    let data = input["data"].as_str().ok_or("Missing 'data' parameter")?;
    // Always append newline if not present (common expectation for REPLs)
    let data = if data.ends_with('\n') {
        data.to_string()
    } else {
        format!("{data}\n")
    };
    pm.write_for_agent(proc_id, agent_id, &data).await?;
    Ok(r#"{"status": "written"}"#.to_string())
}

/// Terminate a process.
async fn tool_process_kill(
    input: &serde_json::Value,
    pm: Option<&crate::process_manager::ProcessManager>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let pm = pm.ok_or("Process manager not available")?;
    let agent_id = caller_agent_id.unwrap_or("default");
    let proc_id = input["process_id"]
        .as_str()
        .ok_or("Missing 'process_id' parameter")?;
    pm.kill_for_agent(proc_id, agent_id).await?;
    Ok(r#"{"status": "killed"}"#.to_string())
}

/// List processes for the current agent.
async fn tool_process_list(
    pm: Option<&crate::process_manager::ProcessManager>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let pm = pm.ok_or("Process manager not available")?;
    let agent_id = caller_agent_id.unwrap_or("default");
    let procs = pm.list(agent_id);
    let list: Vec<serde_json::Value> = procs
        .iter()
        .map(|p| {
            serde_json::json!({
                "id": p.id,
                "command": p.command,
                "alive": p.alive,
                "uptime_secs": p.uptime_secs,
            })
        })
        .collect();
    Ok(serde_json::Value::Array(list).to_string())
}

// ---------------------------------------------------------------------------
// Script run tool — deterministic interpreter selection (Python / shell / TS / JS)
// ---------------------------------------------------------------------------

/// Hard caps for `script_detect` (read-only workspace scan).
const SCRIPT_DETECT_MAX_RESULTS: usize = 20;
const SCRIPT_DETECT_MAX_FILES_SCANNED: usize = 8_000;
const SCRIPT_DETECT_MAX_DEPTH: usize = 8;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "kind")]
enum ScriptDetectCandidate {
    /// A runnable file path (preferred; feed directly into `script_run`).
    #[serde(rename = "file")]
    File {
        path: String,
        score: i32,
        reason: String,
        language_hint: Option<String>,
    },
    /// A `package.json` script name + its command (use `process_start` with `npm`/`pnpm`/`yarn`).
    #[serde(rename = "package_script")]
    PackageScript {
        name: String,
        command: String,
        score: i32,
        reason: String,
    },
}

fn is_script_extension(ext: &str) -> bool {
    matches!(
        ext,
        "py" | "sh" | "bash" | "zsh" | "js" | "mjs" | "cjs" | "ts" | "tsx" | "mts" | "cts"
    )
}

fn should_skip_walk_entry(path: &Path) -> bool {
    // Keep this list small and obvious; we want deterministic behavior and fast scans.
    // (node_modules + target dominate; .git is never relevant.)
    path.components().any(|c| {
        let s = c.as_os_str().to_string_lossy();
        matches!(
            s.as_ref(),
            "node_modules" | "target" | ".git" | ".next" | "dist" | "build" | "output"
        )
    })
}

fn score_file_candidate(path: &Path, query_lc: &str) -> Option<(i32, String, Option<String>)> {
    let file_name = path.file_name()?.to_string_lossy().to_ascii_lowercase();
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !is_script_extension(&ext) {
        return None;
    }

    let mut score: i32 = 0;
    let mut reasons: Vec<&str> = Vec::new();

    // Base score by extension family (gateway/server scripts tend to be TS/JS/PY).
    match ext.as_str() {
        "ts" | "tsx" | "js" | "mjs" | "cjs" => {
            score += 25;
            reasons.push("js/ts entrypoint");
        }
        "py" => {
            score += 20;
            reasons.push("python entrypoint");
        }
        "sh" | "bash" | "zsh" => {
            score += 12;
            reasons.push("shell script");
        }
        _ => {}
    }

    // Filename heuristics.
    for (needle, bump, why) in [
        ("gateway", 40, "filename contains 'gateway'"),
        ("server", 30, "filename contains 'server'"),
        ("api", 12, "filename contains 'api'"),
        ("start", 10, "filename contains 'start'"),
        ("main", 8, "filename contains 'main'"),
        ("index", 5, "filename contains 'index'"),
    ] {
        if file_name.contains(needle) {
            score += bump;
            reasons.push(why);
        }
    }

    // Query match bump.
    if !query_lc.is_empty() {
        if file_name.contains(query_lc) {
            score += 50;
            reasons.push("matches query");
        } else {
            // Tokenized query: "gateway server" etc.
            let tokens: Vec<&str> = query_lc.split_whitespace().collect();
            let hits = tokens.iter().filter(|t| file_name.contains(**t)).count() as i32;
            if hits > 0 {
                score += 15 * hits;
                reasons.push("partially matches query");
            }
        }
    }

    // Prefer scripts under conventional dirs.
    let path_lc = path.to_string_lossy().to_ascii_lowercase();
    if path_lc.contains("/scripts/") || path_lc.contains("\\scripts\\") {
        score += 6;
        reasons.push("under scripts/");
    }
    if path_lc.contains("/src/") || path_lc.contains("\\src\\") {
        score += 4;
        reasons.push("under src/");
    }

    let language_hint = match ext.as_str() {
        "py" => Some("python".to_string()),
        "ts" | "tsx" | "mts" | "cts" => Some("typescript".to_string()),
        "js" | "mjs" | "cjs" => Some("node".to_string()),
        "sh" => Some("shell".to_string()),
        "bash" => Some("bash".to_string()),
        "zsh" => Some("zsh".to_string()),
        _ => None,
    };

    Some((
        score,
        reasons
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
            .join(", "),
        language_hint,
    ))
}

async fn try_read_package_json_scripts(
    workspace_root: &Path,
    query_lc: &str,
) -> Vec<ScriptDetectCandidate> {
    let pkg = workspace_root.join("package.json");
    let Ok(bytes) = tokio::fs::read(&pkg).await else {
        return vec![];
    };
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return vec![];
    };
    let Some(scripts) = v.get("scripts").and_then(|s| s.as_object()) else {
        return vec![];
    };

    let mut out = Vec::new();
    for (name, cmd) in scripts.iter() {
        let Some(cmd) = cmd.as_str() else { continue };
        let name_lc = name.to_ascii_lowercase();
        let cmd_lc = cmd.to_ascii_lowercase();

        let mut score = 0i32;
        let mut reasons = Vec::new();

        // Common dev/start scripts.
        if matches!(name_lc.as_str(), "dev" | "start" | "serve" | "gateway") {
            score += 45;
            reasons.push("common start script");
        }
        if name_lc.contains("gateway") || cmd_lc.contains("gateway") {
            score += 35;
            reasons.push("mentions gateway");
        }
        if name_lc.contains("server") || cmd_lc.contains("server") {
            score += 20;
            reasons.push("mentions server");
        }

        if !query_lc.is_empty() && (name_lc.contains(query_lc) || cmd_lc.contains(query_lc)) {
            score += 40;
            reasons.push("matches query");
        }

        if score > 0 {
            out.push(ScriptDetectCandidate::PackageScript {
                name: name.clone(),
                command: cmd.to_string(),
                score,
                reason: reasons.join(", "),
            });
        }
    }
    out
}

/// Read-only helper for “what should I run?” questions.
///
/// Returns a ranked list of candidate runnable files (preferred) and `package.json` scripts.
async fn tool_script_detect(
    input: &serde_json::Value,
    workspace_root: Option<&Path>,
) -> Result<String, String> {
    let ws = workspace_root.ok_or("script_detect: workspace_root is required")?;
    let query = input
        .get("query")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let query_lc = query.to_ascii_lowercase();
    let max_results = input
        .get("max_results")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
        .unwrap_or(SCRIPT_DETECT_MAX_RESULTS)
        .min(SCRIPT_DETECT_MAX_RESULTS);

    let mut candidates: Vec<ScriptDetectCandidate> = Vec::new();

    // package.json scripts (fast, high signal)
    candidates.extend(try_read_package_json_scripts(ws, &query_lc).await);

    // bounded filesystem walk
    let mut scanned = 0usize;
    for entry in walkdir::WalkDir::new(ws)
        .follow_links(false)
        .max_depth(SCRIPT_DETECT_MAX_DEPTH)
        .into_iter()
        .filter_map(Result::ok)
    {
        if scanned >= SCRIPT_DETECT_MAX_FILES_SCANNED {
            break;
        }
        scanned += 1;
        let path = entry.path();
        if entry.file_type().is_dir() {
            if should_skip_walk_entry(path) {
                // WalkDir doesn't support dynamic prune without `filter_entry`; keep it simple:
                // we'll still visit children but they will be quickly skipped by the same check.
            }
            continue;
        }
        if should_skip_walk_entry(path) {
            continue;
        }
        if let Some((score, reason, language_hint)) = score_file_candidate(path, &query_lc) {
            // Prefer workspace-relative paths in output.
            let rel = path
                .strip_prefix(ws)
                .unwrap_or(path)
                .to_string_lossy()
                .to_string();
            candidates.push(ScriptDetectCandidate::File {
                path: rel,
                score,
                reason,
                language_hint,
            });
        }
    }

    // Sort by score desc, stable tie-break by name.
    candidates.sort_by(|a, b| {
        let (sa, ka) = match a {
            ScriptDetectCandidate::File { score, path, .. } => (*score, format!("file:{path}")),
            ScriptDetectCandidate::PackageScript { score, name, .. } => {
                (*score, format!("pkg:{name}"))
            }
        };
        let (sb, kb) = match b {
            ScriptDetectCandidate::File { score, path, .. } => (*score, format!("file:{path}")),
            ScriptDetectCandidate::PackageScript { score, name, .. } => {
                (*score, format!("pkg:{name}"))
            }
        };
        sb.cmp(&sa).then_with(|| ka.cmp(&kb))
    });

    candidates.truncate(max_results);

    let payload = serde_json::json!({
        "query": query,
        "workspace": ws.to_string_lossy(),
        "scanned_entries": scanned,
        "results": candidates,
        "next_step_hint": "Pick a `file` result and call `script_run` with that path. If you pick a `package_script`, run it with `process_start` (e.g. command: 'npm', args: ['run', '<name>'])."
    });
    Ok(payload.to_string())
}

/// List declarative workspace actions from `<workspace>/armaraos.toml`.
async fn tool_workspace_actions_list(workspace_root: Option<&Path>) -> Result<String, String> {
    let ws = workspace_root.ok_or("workspace_actions_list: workspace_root is required")?;
    let contract = crate::workspace_action_contract::load_workspace_contract(ws)?;
    let actions = crate::workspace_action_contract::summarize_actions(&contract);
    Ok(serde_json::json!({
        "workspace": ws.to_string_lossy(),
        "contract_path": crate::workspace_action_contract::contract_path(ws).to_string_lossy(),
        "actions": actions,
    })
    .to_string())
}

/// Create/update a named workspace action in `<workspace>/armaraos.toml`.
async fn tool_workspace_action_set(
    input: &serde_json::Value,
    workspace_root: Option<&Path>,
) -> Result<String, String> {
    let ws = workspace_root.ok_or("workspace_action_set: workspace_root is required")?;
    let action_name = input
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or("workspace_action_set: missing required `action` parameter")?;
    let script = input
        .get("script")
        .and_then(|v| v.as_str())
        .ok_or("workspace_action_set: missing required `script` parameter")?;

    let action = crate::workspace_action_contract::WorkspaceAction {
        description: input
            .get("description")
            .and_then(|v| v.as_str())
            .map(ToString::to_string),
        script: script.to_string(),
        args: input
            .get("args")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(ToString::to_string))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
        env: input
            .get("env")
            .and_then(|v| v.as_object())
            .map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect::<std::collections::HashMap<_, _>>()
            })
            .unwrap_or_default(),
        cwd: input
            .get("cwd")
            .and_then(|v| v.as_str())
            .map(ToString::to_string),
        language: input
            .get("language")
            .and_then(|v| v.as_str())
            .map(ToString::to_string),
        mode: input
            .get("mode")
            .and_then(|v| v.as_str())
            .map(ToString::to_string),
        timeout_seconds: input.get("timeout_seconds").and_then(|v| v.as_u64()),
        health_check: input
            .get("health_check")
            .filter(|v| v.is_object())
            .map(|v| serde_json::from_value(v.clone()))
            .transpose()
            .map_err(|e| format!("workspace_action_set: invalid health_check object: {e}"))?,
    };

    let contract =
        crate::workspace_action_contract::upsert_workspace_action(ws, action_name, action)?;
    let actions = crate::workspace_action_contract::summarize_actions(&contract);
    Ok(serde_json::json!({
        "ok": true,
        "status": "updated",
        "action": action_name,
        "contract_path": crate::workspace_action_contract::contract_path(ws).to_string_lossy(),
        "actions": actions,
    })
    .to_string())
}

/// Delete a named workspace action from `<workspace>/armaraos.toml`.
async fn tool_workspace_action_delete(
    input: &serde_json::Value,
    workspace_root: Option<&Path>,
) -> Result<String, String> {
    let ws = workspace_root.ok_or("workspace_action_delete: workspace_root is required")?;
    let action_name = input
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or("workspace_action_delete: missing required `action` parameter")?;

    let outcome = crate::workspace_action_contract::delete_workspace_action(ws, action_name)?;
    Ok(serde_json::json!({
        "ok": true,
        "action": action_name,
        "status": match outcome {
            crate::workspace_action_contract::DeleteWorkspaceActionOutcome::Deleted => "deleted",
            crate::workspace_action_contract::DeleteWorkspaceActionOutcome::DeletedAndRemovedContract => "deleted_last_action_removed_contract",
        },
        "contract_path": crate::workspace_action_contract::contract_path(ws).to_string_lossy(),
    })
    .to_string())
}

/// Execute a named workspace action declared in `<workspace>/armaraos.toml`.
///
/// This is intentionally a thin, deterministic layer over `script_run`.
/// The model chooses `action` by name; the runtime maps it to script/cwd/env/mode.
#[allow(clippy::too_many_arguments)]
async fn tool_workspace_action(
    input: &serde_json::Value,
    allowed_env: &[String],
    workspace_root: Option<&Path>,
    ainl_library_root: Option<&Path>,
    caller_agent_id: Option<&str>,
    process_manager: Option<&crate::process_manager::ProcessManager>,
    exec_policy: Option<&openfang_types::config::ExecPolicy>,
) -> Result<String, String> {
    let ws = workspace_root.ok_or("workspace_action: workspace_root is required")?;
    let action_name = input
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or("workspace_action: missing required `action` parameter")?;

    execute_workspace_action_direct(
        ws,
        action_name,
        input.get("args").and_then(|v| v.as_array()).map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(ToString::to_string))
                .collect::<Vec<_>>()
        }),
        input.get("env").and_then(|v| v.as_object()).map(|m| {
            m.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect::<std::collections::HashMap<String, String>>()
        }),
        input
            .get("mode")
            .and_then(|v| v.as_str())
            .map(ToString::to_string),
        input
            .get("timeout_seconds")
            .and_then(|v| v.as_u64())
            .or_else(|| input.get("timeout_secs").and_then(|v| v.as_u64())),
        allowed_env,
        ainl_library_root,
        caller_agent_id,
        process_manager,
        exec_policy,
    )
    .await
}

/// Shared implementation used both by the `workspace_action` tool and kernel cron
/// (`CronAction::workspace_action`) so scheduled actions run deterministically without LLM turns.
#[allow(clippy::too_many_arguments)]
pub async fn execute_workspace_action_direct(
    workspace_root: &Path,
    action_name: &str,
    args_override: Option<Vec<String>>,
    env_override: Option<std::collections::HashMap<String, String>>,
    mode_override: Option<String>,
    timeout_secs_override: Option<u64>,
    allowed_env: &[String],
    ainl_library_root: Option<&Path>,
    caller_agent_id: Option<&str>,
    process_manager: Option<&crate::process_manager::ProcessManager>,
    exec_policy: Option<&openfang_types::config::ExecPolicy>,
) -> Result<String, String> {
    let contract = crate::workspace_action_contract::load_workspace_contract(workspace_root)?;
    let action = contract.actions.get(action_name).ok_or_else(|| {
        format!("workspace_action: action `{action_name}` not found in armaraos.toml")
    })?;

    // Merge args/env: contract defaults first, call-site overrides appended/overridden.
    let mut merged_args = action.args.clone();
    if let Some(extra) = args_override {
        merged_args.extend(extra);
    }
    let mut merged_env = action.env.clone();
    if let Some(ovr) = env_override {
        for (k, v) in ovr {
            merged_env.insert(k, v);
        }
    }

    let mut run_input = serde_json::json!({
        "script": action.script,
    });
    if !merged_args.is_empty() {
        run_input["args"] = serde_json::to_value(merged_args).unwrap_or_default();
    }
    if !merged_env.is_empty() {
        run_input["env"] = serde_json::to_value(merged_env).unwrap_or_default();
    }
    if let Some(cwd) = &action.cwd {
        run_input["cwd"] = serde_json::Value::String(cwd.clone());
    }
    if let Some(lang) = &action.language {
        run_input["language"] = serde_json::Value::String(lang.clone());
    }
    if let Some(hc) = &action.health_check {
        run_input["health_check"] = serde_json::to_value(hc).unwrap_or_default();
    }

    let mode = mode_override
        .or_else(|| action.mode.clone())
        .unwrap_or_else(|| "oneshot".to_string());
    run_input["mode"] = serde_json::Value::String(mode);

    if let Some(t) = timeout_secs_override.or(action.timeout_seconds) {
        run_input["timeout_seconds"] = serde_json::Value::Number(serde_json::Number::from(t));
    }

    let result = tool_script_run(
        &run_input,
        allowed_env,
        Some(workspace_root),
        ainl_library_root,
        caller_agent_id,
        process_manager,
        exec_policy,
    )
    .await?;

    // Add action metadata to payload for easier downstream auditing.
    let mut parsed = serde_json::from_str::<serde_json::Value>(&result).unwrap_or_else(|_| {
        serde_json::json!({
            "raw": result
        })
    });
    parsed["workspace_action"] = serde_json::json!({
        "name": action_name,
        "contract_path": crate::workspace_action_contract::contract_path(workspace_root)
            .to_string_lossy(),
    });
    Ok(parsed.to_string())
}

/// Convenience wrapper around `schedule_create` for declarative workspace actions.
///
/// Accepts the same `schedule` / `description` / `delivery` shape as `schedule_create`,
/// but generates:
/// `action = { kind = "workspace_action", action_name, args?, env?, mode?, timeout_secs? }`.
async fn tool_schedule_action_create(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let action_name = input
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or("schedule_action_create: missing required `action` parameter")?;

    let mut action_obj = serde_json::json!({
        "kind": "workspace_action",
        "action_name": action_name,
    });
    if let Some(args) = input.get("args").and_then(|v| v.as_array()) {
        action_obj["args"] = serde_json::Value::Array(args.clone());
    }
    if let Some(env) = input.get("env").and_then(|v| v.as_object()) {
        action_obj["env"] = serde_json::Value::Object(env.clone());
    }
    if let Some(mode) = input.get("mode").and_then(|v| v.as_str()) {
        action_obj["mode"] = serde_json::Value::String(mode.to_string());
    }
    if let Some(t) = input.get("timeout_secs").and_then(|v| v.as_u64()) {
        action_obj["timeout_secs"] = serde_json::Value::Number(serde_json::Number::from(t));
    }

    let mut forwarded = input.clone();
    forwarded["action"] = action_obj;
    tool_schedule_create(&forwarded, kernel, caller_agent_id).await
}

/// Cap how long a oneshot script may run inside the agent loop. Mirrors `shell_exec`'s
/// default upper bound — if the model needs longer than this, it should pass `mode: "daemon"`
/// instead of cranking the timeout.
const SCRIPT_RUN_ONESHOT_MAX_TIMEOUT_SECS: u64 = 600;

/// Default oneshot timeout when the model omits `timeout_seconds` — matches `shell_exec`.
const SCRIPT_RUN_ONESHOT_DEFAULT_TIMEOUT_SECS: u64 = 30;

/// Default daemon health-probe budget when the model passes `health_check.url` without a
/// `timeout_seconds`. Long enough for cold-start (venv import, `tsx` first-run, etc.) but
/// short enough that the agent loop doesn't wait forever.
const SCRIPT_RUN_DAEMON_HEALTH_DEFAULT_TIMEOUT_SECS: u64 = 15;

/// Maximum stdout/stderr bytes captured per stream for oneshot mode. Same cap as
/// `shell_exec` so the resulting tool message stays within token budgets.
const SCRIPT_RUN_OUTPUT_MAX_BYTES: usize = 100_000;

/// Implementation of the `script_run` builtin tool.
///
/// **Why this exists:** the LLM should not be composing
/// `source venv/bin/activate && python script.py …` or `nohup node server.js &`.
/// Those compositions are where models hallucinate file paths, get shell quoting wrong,
/// and end up dumping copy-paste-Terminal blocks back at the user. With `script_run` the
/// model only states **intent** (`script`, optional `args` / `env` / `cwd` / `mode`) and
/// the runtime decides **how** — picking a project venv when present, `node_modules/.bin/tsx`
/// over `npx`, `bun`/`deno` when their lockfiles are detected, etc.
///
/// `mode: "oneshot"` runs to completion under the same env sandbox as `shell_exec`.
/// `mode: "daemon"` hands off to [`crate::process_manager::ProcessManager`] (the same
/// path as `process_start`) and optionally probes a `health_check.url` until ready —
/// so agents that start an HTTP gateway no longer need to invent `lsof`/`curl` calls
/// just to confirm it bound a port.
#[allow(clippy::too_many_arguments)]
async fn tool_script_run(
    input: &serde_json::Value,
    allowed_env: &[String],
    workspace_root: Option<&Path>,
    ainl_library_root: Option<&Path>,
    caller_agent_id: Option<&str>,
    process_manager: Option<&crate::process_manager::ProcessManager>,
    exec_policy: Option<&openfang_types::config::ExecPolicy>,
) -> Result<String, String> {
    let script = input
        .get("script")
        .and_then(|v| v.as_str())
        .ok_or("script_run: missing required `script` parameter")?;

    let caller_args: Vec<String> = input
        .get("args")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let env_overrides: std::collections::HashMap<String, String> = input
        .get("env")
        .and_then(|v| v.as_object())
        .map(|obj| {
            obj.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();

    let cwd_override = input.get("cwd").and_then(|v| v.as_str()).map(PathBuf::from);

    let explicit_mode = input
        .get("mode")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_ascii_lowercase());
    let mode = explicit_mode
        .clone()
        .unwrap_or_else(|| infer_script_run_mode(script, &caller_args, input));

    let language_hint = input
        .get("language")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty());

    // Resolve runner deterministically.
    let mut allowed_prefixes: Vec<PathBuf> = Vec::new();
    if let Some(lib) = ainl_library_root {
        allowed_prefixes.push(lib.to_path_buf());
    }
    if let Some(policy) = exec_policy {
        for raw in &policy.extra_allowed_path_prefixes {
            let p = PathBuf::from(raw);
            if p.is_absolute() && !p.as_os_str().is_empty() {
                allowed_prefixes.push(p);
            }
        }
    }

    let probe = crate::script_runner::FsProbe;
    let resolved = match crate::script_runner::resolve_runner(
        script,
        workspace_root,
        language_hint,
        &allowed_prefixes,
        &probe,
    ) {
        Ok(r) => r,
        Err(err) => return Err(err.user_message()),
    };

    // Effective cwd: caller override > script's parent > workspace root.
    let effective_cwd = cwd_override.unwrap_or_else(|| resolved.default_cwd.clone());

    let runner_meta = serde_json::json!({
        "interpreter": resolved.interpreter,
        "interpreter_args": resolved.interpreter_args,
        "script_path": resolved.script_path.to_string_lossy(),
        "language": resolved.language.as_str(),
        "decision_source": resolved.decision_source,
        "mode_source": if explicit_mode.is_some() { "explicit" } else { "inferred" },
        "cwd": effective_cwd.to_string_lossy(),
    });

    match mode.as_str() {
        "oneshot" => {
            run_oneshot(
                &resolved,
                &caller_args,
                &env_overrides,
                &effective_cwd,
                allowed_env,
                input,
                &runner_meta,
            )
            .await
        }
        "daemon" => {
            let pm = process_manager.ok_or(
                "script_run: process_manager not available — daemon mode requires the persistent \
                 process subsystem to be enabled",
            )?;
            run_daemon(
                &resolved,
                &caller_args,
                &env_overrides,
                &effective_cwd,
                input,
                &runner_meta,
                pm,
                caller_agent_id,
            )
            .await
        }
        other => Err(format!(
            "script_run: unknown `mode` value `{other}`. Supported: 'oneshot' (default), 'daemon'."
        )),
    }
}

/// Execute the resolved script to completion under the same env sandbox as `shell_exec`.
#[allow(clippy::too_many_arguments)]
async fn run_oneshot(
    resolved: &crate::script_runner::ResolvedRunner,
    caller_args: &[String],
    env_overrides: &std::collections::HashMap<String, String>,
    effective_cwd: &Path,
    allowed_env: &[String],
    input: &serde_json::Value,
    runner_meta: &serde_json::Value,
) -> Result<String, String> {
    let timeout_secs = input
        .get("timeout_seconds")
        .and_then(|v| v.as_u64())
        .unwrap_or(SCRIPT_RUN_ONESHOT_DEFAULT_TIMEOUT_SECS)
        .min(SCRIPT_RUN_ONESHOT_MAX_TIMEOUT_SECS);

    let argv = resolved.full_argv(caller_args);
    let mut cmd = tokio::process::Command::new(&resolved.interpreter);
    if !argv.is_empty() {
        cmd.args(&argv);
    }
    cmd.current_dir(effective_cwd);
    cmd.stdin(std::process::Stdio::null());

    crate::subprocess_sandbox::sandbox_command(&mut cmd, allowed_env);
    for (k, v) in env_overrides {
        cmd.env(k, v);
    }
    #[cfg(windows)]
    cmd.env("PYTHONIOENCODING", "utf-8");

    debug!(
        interpreter = %resolved.interpreter,
        argv = ?argv,
        cwd = %effective_cwd.display(),
        timeout_secs,
        "script_run oneshot spawn"
    );

    let started = std::time::Instant::now();
    let outcome =
        tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), cmd.output()).await;

    match outcome {
        Ok(Ok(out)) => {
            let stdout = truncate_stream(&String::from_utf8_lossy(&out.stdout));
            let stderr = truncate_stream(&String::from_utf8_lossy(&out.stderr));
            let exit_code = out.status.code().unwrap_or(-1);
            let payload = serde_json::json!({
                "mode": "oneshot",
                "ok": exit_code == 0,
                "exit_code": exit_code,
                "duration_ms": started.elapsed().as_millis() as u64,
                "stdout": stdout,
                "stderr": stderr,
                "runner": runner_meta,
            });
            Ok(payload.to_string())
        }
        Ok(Err(e)) => Err(format!(
            "script_run: failed to spawn `{}`: {e}. Hint: check the interpreter exists \
             (the runner picked `{}`); pass `language` to override or install the missing tool.",
            resolved.interpreter, resolved.interpreter
        )),
        Err(_) => Err(format!(
            "script_run: oneshot timed out after {timeout_secs}s. Hint: pass `mode: \"daemon\"` \
             for long-running services, or raise `timeout_seconds` (max \
             {SCRIPT_RUN_ONESHOT_MAX_TIMEOUT_SECS}s)."
        )),
    }
}

/// Infer a safer default when the caller omits `mode`.
///
/// Heuristic:
/// - `health_check` present => daemon
/// - script/args mention server-ish terms (`gateway`, `server`, `serve`, etc.) => daemon
/// - otherwise oneshot
fn infer_script_run_mode(script: &str, args: &[String], input: &serde_json::Value) -> String {
    if input
        .get("health_check")
        .map(|v| v.is_object())
        .unwrap_or(false)
    {
        return "daemon".to_string();
    }
    let mut haystack = script.to_ascii_lowercase();
    if !args.is_empty() {
        haystack.push(' ');
        haystack.push_str(&args.join(" ").to_ascii_lowercase());
    }
    let service_markers = [
        "gateway",
        "server",
        "serve",
        "daemon",
        "watch",
        "uvicorn",
        "gunicorn",
        "flask run",
    ];
    if service_markers.iter().any(|m| haystack.contains(m)) {
        "daemon".to_string()
    } else {
        "oneshot".to_string()
    }
}

/// Truncate stdout/stderr like `shell_exec` does so the tool reply stays within token budgets.
fn truncate_stream(s: &str) -> String {
    if s.len() > SCRIPT_RUN_OUTPUT_MAX_BYTES {
        format!(
            "{}...\n[truncated, {} total bytes]",
            crate::str_utils::safe_truncate_str(s, SCRIPT_RUN_OUTPUT_MAX_BYTES),
            s.len()
        )
    } else {
        s.to_string()
    }
}

/// Hand off to `ProcessManager` and optionally probe `health_check.url` until ready.
#[allow(clippy::too_many_arguments)]
async fn run_daemon(
    resolved: &crate::script_runner::ResolvedRunner,
    caller_args: &[String],
    env_overrides: &std::collections::HashMap<String, String>,
    effective_cwd: &Path,
    input: &serde_json::Value,
    runner_meta: &serde_json::Value,
    pm: &crate::process_manager::ProcessManager,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let agent_id = caller_agent_id.unwrap_or("default");
    let argv = resolved.full_argv(caller_args);

    debug!(
        interpreter = %resolved.interpreter,
        argv = ?argv,
        cwd = %effective_cwd.display(),
        agent = %agent_id,
        "script_run daemon spawn"
    );

    let env_for_pm = if env_overrides.is_empty() {
        None
    } else {
        Some(env_overrides.clone())
    };

    let cwd_str = effective_cwd.to_string_lossy().to_string();
    let proc_id = pm
        .start(
            agent_id,
            &resolved.interpreter,
            &argv,
            env_for_pm.as_ref(),
            Some(cwd_str.as_str()),
        )
        .await?;

    let mut payload = serde_json::json!({
        "mode": "daemon",
        "ok": true,
        "process_id": proc_id,
        "status": "started",
        "runner": runner_meta,
        "next_steps": "Call `process_poll` with this process_id to drain stdout/stderr; \
                       `process_kill` to stop.",
    });

    if let Some(hc) = input.get("health_check").and_then(|v| v.as_object()) {
        if let Some(url) = hc.get("url").and_then(|v| v.as_str()) {
            let timeout_secs = hc
                .get("timeout_seconds")
                .and_then(|v| v.as_u64())
                .unwrap_or(SCRIPT_RUN_DAEMON_HEALTH_DEFAULT_TIMEOUT_SECS)
                .min(60);
            let expect_status = hc
                .get("expect_status")
                .and_then(|v| v.as_u64())
                .unwrap_or(200) as u16;

            let probe = probe_http_until_ready(url, timeout_secs, expect_status).await;
            if let Some(obj) = payload.as_object_mut() {
                obj.insert(
                    "health".to_string(),
                    serde_json::to_value(&probe).unwrap_or_default(),
                );
                obj.insert(
                    "status".to_string(),
                    serde_json::Value::String(
                        if probe.ready { "ready" } else { "unreachable" }.to_string(),
                    ),
                );
            }
        }
    }

    Ok(payload.to_string())
}

/// Outcome of the optional daemon-mode HTTP readiness probe.
#[derive(Debug, serde::Serialize)]
struct HealthProbe {
    url: String,
    ready: bool,
    attempts: u32,
    elapsed_ms: u64,
    last_status: Option<u16>,
    last_error: Option<String>,
}

/// Repeatedly GET `url` until we see `expect_status` (or `timeout_secs` elapses).
/// Uses an increasing backoff (250ms → 500ms → 1s, capped) so a fast-starting service
/// returns immediately while a slow one (cold venv import, vite dev server) gets some grace.
async fn probe_http_until_ready(url: &str, timeout_secs: u64, expect_status: u16) -> HealthProbe {
    let started = std::time::Instant::now();
    let deadline = started + std::time::Duration::from_secs(timeout_secs);
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return HealthProbe {
                url: url.to_string(),
                ready: false,
                attempts: 0,
                elapsed_ms: started.elapsed().as_millis() as u64,
                last_status: None,
                last_error: Some(format!("failed to build http client: {e}")),
            };
        }
    };

    let mut attempts: u32 = 0;
    // Initial `None` values are immediately overwritten on the first probe attempt;
    // we keep them so the deadline-exit path (which reads both) sees defined values
    // even when the very first iteration short-circuits.
    #[allow(unused_assignments)]
    let mut last_status: Option<u16> = None;
    #[allow(unused_assignments)]
    let mut last_error: Option<String> = None;
    let mut backoff = std::time::Duration::from_millis(250);

    loop {
        attempts += 1;
        match client.get(url).send().await {
            Ok(resp) => {
                let status = resp.status().as_u16();
                last_status = Some(status);
                last_error = None;
                if status == expect_status {
                    return HealthProbe {
                        url: url.to_string(),
                        ready: true,
                        attempts,
                        elapsed_ms: started.elapsed().as_millis() as u64,
                        last_status,
                        last_error,
                    };
                }
            }
            Err(e) => {
                last_status = None;
                last_error = Some(e.to_string());
            }
        }

        let now = std::time::Instant::now();
        if now >= deadline {
            return HealthProbe {
                url: url.to_string(),
                ready: false,
                attempts,
                elapsed_ms: started.elapsed().as_millis() as u64,
                last_status,
                last_error,
            };
        }
        let remaining = deadline.saturating_duration_since(now);
        tokio::time::sleep(backoff.min(remaining)).await;
        backoff = (backoff * 2).min(std::time::Duration::from_secs(1));
    }
}

// ---------------------------------------------------------------------------
// Canvas / A2UI tool
// ---------------------------------------------------------------------------

/// Sanitize HTML for canvas presentation.
///
/// SECURITY: Strips dangerous elements and attributes to prevent XSS:
/// - Rejects <script>, <iframe>, <object>, <embed>, <applet> tags
/// - Strips all on* event attributes (onclick, onload, onerror, etc.)
/// - Strips javascript:, data:text/html, vbscript: URLs
/// - Enforces size limit
pub fn sanitize_canvas_html(html: &str, max_bytes: usize) -> Result<String, String> {
    if html.is_empty() {
        return Err("Empty HTML content".to_string());
    }
    if html.len() > max_bytes {
        return Err(format!(
            "HTML too large: {} bytes (max {})",
            html.len(),
            max_bytes
        ));
    }

    let lower = html.to_lowercase();

    // Reject dangerous tags
    let dangerous_tags = [
        "<script", "</script", "<iframe", "</iframe", "<object", "</object", "<embed", "<applet",
        "</applet",
    ];
    for tag in &dangerous_tags {
        if lower.contains(tag) {
            return Err(format!("Forbidden HTML tag detected: {tag}"));
        }
    }

    // Reject event handler attributes (on*)
    // Match patterns like: onclick=, onload=, onerror=, onmouseover=, etc.
    static EVENT_PATTERN: std::sync::LazyLock<regex_lite::Regex> =
        std::sync::LazyLock::new(|| regex_lite::Regex::new(r"(?i)\bon[a-z]+\s*=").unwrap());
    if EVENT_PATTERN.is_match(html) {
        return Err(
            "Forbidden event handler attribute detected (on* attributes are not allowed)"
                .to_string(),
        );
    }

    // Reject dangerous URL schemes
    let dangerous_schemes = ["javascript:", "vbscript:", "data:text/html"];
    for scheme in &dangerous_schemes {
        if lower.contains(scheme) {
            return Err(format!("Forbidden URL scheme detected: {scheme}"));
        }
    }

    Ok(html.to_string())
}

/// Canvas presentation tool handler.
async fn tool_canvas_present(
    input: &serde_json::Value,
    workspace_root: Option<&Path>,
) -> Result<String, String> {
    let html = input["html"].as_str().ok_or("Missing 'html' parameter")?;
    let title = input["title"].as_str().unwrap_or("Canvas");

    // Use configured max from task-local (set by agent_loop from KernelConfig), or default 512KB.
    let max_bytes = CANVAS_MAX_BYTES.try_with(|v| *v).unwrap_or(512 * 1024);
    let sanitized = sanitize_canvas_html(html, max_bytes)?;

    // Generate canvas ID
    let canvas_id = uuid::Uuid::new_v4().to_string();

    // Save to workspace output directory
    let output_dir = if let Some(root) = workspace_root {
        root.join("output")
    } else {
        PathBuf::from("output")
    };
    let _ = tokio::fs::create_dir_all(&output_dir).await;

    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    let filename = format!(
        "canvas_{timestamp}_{}.html",
        crate::str_utils::safe_truncate_str(&canvas_id, 8)
    );
    let filepath = output_dir.join(&filename);

    // Write the full HTML document
    let full_html = format!(
        "<!DOCTYPE html>\n<html>\n<head><meta charset=\"utf-8\"><title>{title}</title></head>\n<body>\n{sanitized}\n</body>\n</html>"
    );
    tokio::fs::write(&filepath, &full_html)
        .await
        .map_err(|e| format!("Failed to save canvas: {e}"))?;

    let response = serde_json::json!({
        "canvas_id": canvas_id,
        "title": title,
        "saved_to": filepath.to_string_lossy(),
        "size_bytes": full_html.len(),
    });

    serde_json::to_string_pretty(&response).map_err(|e| format!("Serialize error: {e}"))
}

// ---------------------------------------------------------------------------
// Email tools
// ---------------------------------------------------------------------------

/// Send an email via SMTP or MCP email integration.
async fn tool_email_send(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    mcp_connections: Option<&tokio::sync::Mutex<Vec<mcp::McpConnection>>>,
) -> Result<String, String> {
    let to = input["to"].as_str().ok_or("Missing 'to' parameter")?;
    let subject = input["subject"]
        .as_str()
        .ok_or("Missing 'subject' parameter")?;
    let body = input["body"].as_str().ok_or("Missing 'body' parameter")?;
    let cc = input["cc"].as_str();
    let bcc = input["bcc"].as_str();
    let provider_hint = input["provider"].as_str();

    // Strategy: Try MCP email servers first, then fall back to SMTP via email channel config

    // 1. Check for MCP email integrations (Gmail, Outlook, etc.)
    if let Some(mcp_conns) = mcp_connections {
        let conns = mcp_conns.lock().await;

        // Look for email-capable MCP servers (gmail, outlook, etc.)
        let email_servers: Vec<&str> = conns
            .iter()
            .filter(|c| {
                let name = c.name().to_lowercase();
                name.contains("gmail") || name.contains("outlook") || name.contains("email")
            })
            .map(|c| c.name())
            .collect();

        if !email_servers.is_empty() {
            // Found MCP email server - use provider hint or first available
            let server_name_ref = if let Some(hint) = provider_hint {
                email_servers
                    .iter()
                    .find(|&&s| s.to_lowercase().contains(&hint.to_lowercase()))
                    .unwrap_or(&email_servers[0])
            } else {
                email_servers[0]
            };

            // Clone to owned String before dropping lock
            let server_name = server_name_ref.to_string();

            drop(conns); // Release lock before async call

            // Try to find a send_email tool on the MCP server
            let tool_name = format!("{}_send_email", server_name);
            let mcp_input = serde_json::json!({
                "to": to,
                "subject": subject,
                "body": body,
                "cc": cc,
                "bcc": bcc,
            });

            let mut conns = mcp_connections.unwrap().lock().await;
            if let Some(conn) = conns.iter_mut().find(|c| c.name() == server_name) {
                match conn.call_tool(&tool_name, &mcp_input).await {
                    Ok(_result) => {
                        return Ok(serde_json::json!({
                            "status": "sent",
                            "provider": server_name,
                            "method": "mcp",
                            "to": to,
                            "subject": subject,
                        })
                        .to_string());
                    }
                    Err(e) => {
                        warn!("MCP email send failed, falling back to SMTP: {}", e);
                    }
                }
            }
        }
    }

    // 2. Fall back to SMTP using email channel config (if available)
    // TODO: Add support for direct SMTP via email channel config when kernel provides access to config
    let _ = kernel; // Suppress unused warning
    Err("No MCP email integration found. Configure an email channel or install an MCP email server (e.g., Gmail, Outlook).".to_string())
}

/// Read emails via IMAP or MCP email integration.
async fn tool_email_read(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    mcp_connections: Option<&tokio::sync::Mutex<Vec<mcp::McpConnection>>>,
) -> Result<String, String> {
    let folder = input["folder"].as_str().unwrap_or("INBOX");
    let limit = input["limit"].as_u64().unwrap_or(10).min(50) as usize;
    let unread_only = input["unread_only"].as_bool().unwrap_or(true);
    let from_filter = input["from"].as_str();
    let subject_filter = input["subject_contains"].as_str();
    let provider_hint = input["provider"].as_str();

    // 1. Try MCP email servers first
    if let Some(mcp_conns) = mcp_connections {
        let conns = mcp_conns.lock().await;

        let email_servers: Vec<&str> = conns
            .iter()
            .filter(|c| {
                let name = c.name().to_lowercase();
                name.contains("gmail") || name.contains("outlook") || name.contains("email")
            })
            .map(|c| c.name())
            .collect();

        if !email_servers.is_empty() {
            let server_name_ref = if let Some(hint) = provider_hint {
                email_servers
                    .iter()
                    .find(|&&s| s.to_lowercase().contains(&hint.to_lowercase()))
                    .unwrap_or(&email_servers[0])
            } else {
                email_servers[0]
            };

            let server_name = server_name_ref.to_string();

            drop(conns);

            let tool_name = format!("{}_read_emails", server_name);
            let mcp_input = serde_json::json!({
                "folder": folder,
                "limit": limit,
                "unread_only": unread_only,
                "from": from_filter,
                "subject_contains": subject_filter,
            });

            let mut conns = mcp_connections.unwrap().lock().await;
            if let Some(conn) = conns.iter_mut().find(|c| c.name() == server_name) {
                match conn.call_tool(&tool_name, &mcp_input).await {
                    Ok(result) => return Ok(result),
                    Err(e) => {
                        warn!("MCP email read failed, falling back to IMAP: {}", e);
                    }
                }
            }
        }
    }

    // 2. Fall back to IMAP (requires email channel config)
    let _ = kernel; // Suppress unused warning
    Err("No MCP email integration found. Configure an email channel or install an MCP email server.".to_string())
}

/// Search emails using provider-specific query syntax.
async fn tool_email_search(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    mcp_connections: Option<&tokio::sync::Mutex<Vec<mcp::McpConnection>>>,
) -> Result<String, String> {
    let query = input["query"].as_str().ok_or("Missing 'query' parameter")?;
    let limit = input["limit"].as_u64().unwrap_or(20).min(100) as usize;
    let folder = input["folder"].as_str();
    let provider_hint = input["provider"].as_str();

    // 1. Try MCP email servers
    if let Some(mcp_conns) = mcp_connections {
        let conns = mcp_conns.lock().await;

        let email_servers: Vec<&str> = conns
            .iter()
            .filter(|c| {
                let name = c.name().to_lowercase();
                name.contains("gmail") || name.contains("outlook") || name.contains("email")
            })
            .map(|c| c.name())
            .collect();

        if !email_servers.is_empty() {
            let server_name_ref = if let Some(hint) = provider_hint {
                email_servers
                    .iter()
                    .find(|&&s| s.to_lowercase().contains(&hint.to_lowercase()))
                    .unwrap_or(&email_servers[0])
            } else {
                email_servers[0]
            };

            let server_name = server_name_ref.to_string();

            drop(conns);

            let tool_name = format!("{}_search_emails", server_name);
            let mcp_input = serde_json::json!({
                "query": query,
                "limit": limit,
                "folder": folder,
            });

            let mut conns = mcp_connections.unwrap().lock().await;
            if let Some(conn) = conns.iter_mut().find(|c| c.name() == server_name) {
                match conn.call_tool(&tool_name, &mcp_input).await {
                    Ok(result) => return Ok(result),
                    Err(e) => {
                        warn!("MCP email search failed: {}", e);
                    }
                }
            }
        }
    }

    let _ = kernel; // Suppress unused warning
    Err("No MCP email integration found. Configure an email channel or install an MCP email server.".to_string())
}

/// Reply to an email thread with proper threading headers.
async fn tool_email_reply(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    mcp_connections: Option<&tokio::sync::Mutex<Vec<mcp::McpConnection>>>,
) -> Result<String, String> {
    let message_id = input["message_id"]
        .as_str()
        .ok_or("Missing 'message_id' parameter")?;
    let body = input["body"].as_str().ok_or("Missing 'body' parameter")?;
    let reply_all = input["reply_all"].as_bool().unwrap_or(false);
    let provider_hint = input["provider"].as_str();

    // 1. Try MCP email servers
    if let Some(mcp_conns) = mcp_connections {
        let conns = mcp_conns.lock().await;

        let email_servers: Vec<&str> = conns
            .iter()
            .filter(|c| {
                let name = c.name().to_lowercase();
                name.contains("gmail") || name.contains("outlook") || name.contains("email")
            })
            .map(|c| c.name())
            .collect();

        if !email_servers.is_empty() {
            let server_name_ref = if let Some(hint) = provider_hint {
                email_servers
                    .iter()
                    .find(|&&s| s.to_lowercase().contains(&hint.to_lowercase()))
                    .unwrap_or(&email_servers[0])
            } else {
                email_servers[0]
            };

            let server_name = server_name_ref.to_string();

            drop(conns);

            let tool_name = format!("{}_reply_email", server_name);
            let mcp_input = serde_json::json!({
                "message_id": message_id,
                "body": body,
                "reply_all": reply_all,
            });

            let mut conns = mcp_connections.unwrap().lock().await;
            if let Some(conn) = conns.iter_mut().find(|c| c.name() == server_name) {
                match conn.call_tool(&tool_name, &mcp_input).await {
                    Ok(_result) => {
                        return Ok(serde_json::json!({
                            "status": "sent",
                            "provider": server_name,
                            "method": "mcp",
                            "message_id": message_id,
                            "reply_all": reply_all,
                        })
                        .to_string());
                    }
                    Err(e) => {
                        warn!("MCP email reply failed: {}", e);
                    }
                }
            }
        }
    }

    let _ = kernel; // Suppress unused warning
    Err("No MCP email integration found. Configure an email channel or install an MCP email server.".to_string())
}

/// Create or update an email draft.
async fn tool_email_draft(
    input: &serde_json::Value,
    mcp_connections: Option<&tokio::sync::Mutex<Vec<mcp::McpConnection>>>,
) -> Result<String, String> {
    let to = input["to"].as_str().ok_or("Missing 'to' parameter")?;
    let subject = input["subject"]
        .as_str()
        .ok_or("Missing 'subject' parameter")?;
    let body = input["body"].as_str().ok_or("Missing 'body' parameter")?;
    let draft_id = input["draft_id"].as_str();
    let provider_hint = input["provider"].as_str();

    // Draft creation/update is MCP-only (not well supported via raw IMAP)
    if let Some(mcp_conns) = mcp_connections {
        let conns = mcp_conns.lock().await;

        let email_servers: Vec<&str> = conns
            .iter()
            .filter(|c| {
                let name = c.name().to_lowercase();
                name.contains("gmail") || name.contains("outlook") || name.contains("email")
            })
            .map(|c| c.name())
            .collect();

        if !email_servers.is_empty() {
            let server_name_ref = if let Some(hint) = provider_hint {
                email_servers
                    .iter()
                    .find(|&&s| s.to_lowercase().contains(&hint.to_lowercase()))
                    .unwrap_or(&email_servers[0])
            } else {
                email_servers[0]
            };

            let server_name = server_name_ref.to_string();

            drop(conns);

            let tool_name = format!("{}_create_draft", server_name);
            let mcp_input = serde_json::json!({
                "to": to,
                "subject": subject,
                "body": body,
                "draft_id": draft_id,
            });

            let mut conns = mcp_connections.unwrap().lock().await;
            if let Some(conn) = conns.iter_mut().find(|c| c.name() == server_name) {
                match conn.call_tool(&tool_name, &mcp_input).await {
                    Ok(result) => return Ok(result),
                    Err(e) => {
                        return Err(format!("MCP draft creation failed: {}", e));
                    }
                }
            }
        }
    }

    Err("Draft creation requires an MCP email integration (Gmail, Outlook). Raw IMAP does not support drafts reliably.".to_string())
}

// ── Planner mode (deterministic plan) tool dispatch — mirrors `agent_loop` timeouts ─────────────

const PLANNER_TOOL_TIMEOUT_SECS: u64 = 300;
const PLANNER_AGENT_TOOL_TIMEOUT_SECS: u64 = 600;

/// Wall-clock timeout for a single tool invocation (same caps as `agent_loop::tool_timeout_for`).
pub fn tool_execution_timeout(tool_name: &str) -> std::time::Duration {
    use std::time::Duration;
    match tool_name {
        "agent_send" | "agent_spawn" => Duration::from_secs(PLANNER_AGENT_TOOL_TIMEOUT_SECS),
        "document_extract" | "spreadsheet_build" => Duration::from_secs(180),
        "channel_send" | "channel_stream" => Duration::from_secs(30),
        "image_generate" | "text_to_speech" | "speech_to_text" | "media_describe"
        | "media_transcribe" => Duration::from_secs(300),
        "a2a_send" | "a2a_discover" | "a2a_discover_hermes" | "a2a_send_hermes" => {
            Duration::from_secs(300)
        }
        "hermes_a2a_status" => Duration::from_secs(30),
        "mcp_resource_read" => Duration::from_secs(90),
        "process_start" | "process_poll" | "process_write" | "process_kill" | "process_list" => {
            Duration::from_secs(30)
        }
        "workspace_actions_list" => Duration::from_secs(30),
        "workspace_action_set" | "workspace_action_delete" => Duration::from_secs(30),
        "workspace_action" => Duration::from_secs(SCRIPT_RUN_ONESHOT_MAX_TIMEOUT_SECS),
        "schedule_action_create" => Duration::from_secs(30),
        "script_detect" => Duration::from_secs(30),
        // script_run oneshot can take up to its 600s upper bound; daemon mode plus
        // health-probe budget tops out around 60s. Cap at the larger to be safe.
        "script_run" => Duration::from_secs(SCRIPT_RUN_ONESHOT_MAX_TIMEOUT_SECS),
        "shell_exec" => Duration::from_secs(PLANNER_TOOL_TIMEOUT_SECS),
        _ => Duration::from_secs(PLANNER_TOOL_TIMEOUT_SECS),
    }
}

/// Dispatch one planner step through the same path as the interactive tool loop.
#[allow(clippy::too_many_arguments)]
pub async fn dispatch_planned_tool_call(
    step_id: &str,
    tool_name: &str,
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    allowed_tools: Option<&[String]>,
    caller_agent_id: Option<&str>,
    skill_registry: Option<&SkillRegistry>,
    mcp_connections: Option<&tokio::sync::Mutex<Vec<mcp::McpConnection>>>,
    web_ctx: Option<&WebToolsContext>,
    browser_ctx: Option<&crate::browser::BrowserManager>,
    allowed_env_vars: Option<&[String]>,
    workspace_root: Option<&Path>,
    ainl_library_root: Option<&Path>,
    media_engine: Option<&crate::media_understanding::MediaEngine>,
    exec_policy: Option<&openfang_types::config::ExecPolicy>,
    tts_engine: Option<&crate::tts::TtsEngine>,
    docker_config: Option<&openfang_types::config::DockerSandboxConfig>,
    process_manager: Option<&crate::process_manager::ProcessManager>,
    orchestration_live: Option<&OrchestrationLive>,
) -> Result<serde_json::Value, ainl_agent_snapshot::PlanStepError> {
    let timeout = tool_execution_timeout(tool_name);
    let fut = execute_tool(
        step_id,
        tool_name,
        input,
        kernel,
        allowed_tools,
        caller_agent_id,
        skill_registry,
        mcp_connections,
        web_ctx,
        browser_ctx,
        allowed_env_vars,
        workspace_root,
        ainl_library_root,
        media_engine,
        exec_policy,
        tts_engine,
        docker_config,
        process_manager,
        orchestration_live,
    );
    match tokio::time::timeout(timeout, fut).await {
        Ok(tr) => {
            if tr.is_error {
                Err(ainl_agent_snapshot::PlanStepError::Deterministic(
                    tr.content,
                ))
            } else {
                Ok(serde_json::json!({ "output": tr.content }))
            }
        }
        Err(_) => Err(ainl_agent_snapshot::PlanStepError::Timeout),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_tool_definitions() {
        let tools = builtin_tool_definitions();
        assert!(
            tools.len() >= 40,
            "Expected at least 40 tools, got {}",
            tools.len()
        );
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        // Original 12
        assert!(names.contains(&"file_read"));
        assert!(names.contains(&"document_extract"));
        assert!(names.contains(&"spreadsheet_build"));
        assert!(names.contains(&"shell_exec"));
        assert!(names.contains(&"agent_send"));
        assert!(names.contains(&"agent_spawn"));
        assert!(names.contains(&"agent_delegate"));
        assert!(names.contains(&"agent_map_reduce"));
        assert!(names.contains(&"agent_supervise"));
        assert!(names.contains(&"agent_coordinate"));
        assert!(names.contains(&"agent_list"));
        assert!(names.contains(&"agent_kill"));
        assert!(names.contains(&"memory_store"));
        assert!(names.contains(&"memory_recall"));
        // Collaboration / orchestration tools
        assert!(names.contains(&"agent_find"));
        assert!(names.contains(&"agent_find_capabilities"));
        assert!(names.contains(&"agent_pool_list"));
        assert!(names.contains(&"agent_pool_spawn"));
        assert!(names.contains(&"task_post"));
        assert!(names.contains(&"task_claim"));
        assert!(names.contains(&"task_complete"));
        assert!(names.contains(&"task_list"));
        assert!(names.contains(&"event_publish"));
        // 5 new Phase 3 tools
        assert!(names.contains(&"schedule_create"));
        assert!(names.contains(&"schedule_list"));
        assert!(names.contains(&"schedule_delete"));
        assert!(names.contains(&"image_analyze"));
        assert!(names.contains(&"location_get"));
        assert!(names.contains(&"system_time"));
        // 6 browser tools
        assert!(names.contains(&"browser_navigate"));
        assert!(names.contains(&"browser_click"));
        assert!(names.contains(&"browser_type"));
        assert!(names.contains(&"browser_screenshot"));
        assert!(names.contains(&"browser_read_page"));
        assert!(names.contains(&"browser_close"));
        assert!(names.contains(&"browser_scroll"));
        assert!(names.contains(&"browser_wait"));
        assert!(names.contains(&"browser_run_js"));
        assert!(names.contains(&"browser_back"));
        // 3 media/image generation tools
        assert!(names.contains(&"media_describe"));
        assert!(names.contains(&"media_transcribe"));
        assert!(names.contains(&"image_generate"));
        // 3 cron tools
        assert!(names.contains(&"cron_create"));
        assert!(names.contains(&"cron_list"));
        assert!(names.contains(&"cron_cancel"));
        // Channel tools
        assert!(names.contains(&"channel_send"));
        assert!(names.contains(&"channels_list"));
        // 4 hand tools
        assert!(names.contains(&"hand_list"));
        assert!(names.contains(&"hand_activate"));
        assert!(names.contains(&"hand_status"));
        assert!(names.contains(&"hand_deactivate"));
        // 3 voice/docker tools
        assert!(names.contains(&"text_to_speech"));
        assert!(names.contains(&"speech_to_text"));
        assert!(names.contains(&"docker_exec"));
        // Canvas tool
        assert!(names.contains(&"canvas_present"));
    }

    /// Every builtin definition must map to a real `execute_tool` arm (not MCP/skill fallback).
    #[tokio::test]
    async fn test_builtin_tools_are_dispatched_not_unknown() {
        let empty = serde_json::json!({});
        // Network tools would call out with `{}` — skip; they are still covered by `builtin_tool_definitions` + match arms.
        let skip_minimal_probe: &[&str] = &["web_search", "web_fetch"];
        for def in builtin_tool_definitions() {
            if skip_minimal_probe.contains(&def.name.as_str()) {
                continue;
            }
            let res = execute_tool(
                "probe",
                def.name.as_str(),
                &empty,
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
                None, // media_engine
                None, // exec_policy
                None, // tts_engine
                None, // docker_config
                None, // process_manager
                None, // orchestration_live
            )
            .await;
            assert!(
                !res.content.contains("Unknown tool"),
                "builtin `{}` fell through to unknown tool: {}",
                def.name,
                res.content
            );
        }
    }

    #[test]
    fn test_builtin_tool_names_unique() {
        let defs = builtin_tool_definitions();
        let mut names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        names.sort_unstable();
        let deduped_len = names
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>()
            .len();
        assert_eq!(
            deduped_len,
            defs.len(),
            "duplicate tool names in builtin_tool_definitions"
        );
    }

    #[test]
    fn test_collaboration_tool_schemas() {
        let tools = builtin_tool_definitions();
        let collab_tools = [
            "agent_find",
            "agent_find_capabilities",
            "agent_pool_list",
            "agent_pool_spawn",
            "agent_delegate",
            "agent_map_reduce",
            "agent_supervise",
            "agent_coordinate",
            "task_post",
            "task_claim",
            "orchestration_shared_merge",
            "task_complete",
            "task_list",
            "event_publish",
        ];
        for name in &collab_tools {
            let tool = tools
                .iter()
                .find(|t| t.name == *name)
                .unwrap_or_else(|| panic!("Tool '{}' not found", name));
            // Verify each has a valid JSON schema
            assert!(
                tool.input_schema.is_object(),
                "Tool '{}' schema should be an object",
                name
            );
            assert_eq!(
                tool.input_schema["type"], "object",
                "Tool '{}' should have type=object",
                name
            );
        }
    }

    #[tokio::test]
    async fn test_file_read_missing() {
        let bad_path = std::env::temp_dir()
            .join("openfang_test_nonexistent_99999")
            .join("file.txt");
        let result = execute_tool(
            "test-id",
            "file_read",
            &serde_json::json!({"path": bad_path.to_str().unwrap()}),
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
            None, // media_engine
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
            None, // orchestration_live
        )
        .await;
        assert!(
            result.is_error,
            "Expected error but got: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_file_read_path_traversal_blocked() {
        let result = execute_tool(
            "test-id",
            "file_read",
            &serde_json::json!({"path": "../../etc/passwd"}),
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
            None, // media_engine
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
            None, // orchestration_live
        )
        .await;
        assert!(result.is_error);
        assert!(result.content.contains("traversal"));
    }

    #[tokio::test]
    async fn test_file_write_path_traversal_blocked() {
        let result = execute_tool(
            "test-id",
            "file_write",
            &serde_json::json!({"path": "../../../tmp/evil.txt", "content": "pwned"}),
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
            None, // media_engine
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
            None, // orchestration_live
        )
        .await;
        assert!(result.is_error);
        assert!(result.content.contains("traversal"));
    }

    #[tokio::test]
    async fn test_file_list_path_traversal_blocked() {
        let result = execute_tool(
            "test-id",
            "file_list",
            &serde_json::json!({"path": "/foo/../../etc"}),
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
            None, // media_engine
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
            None, // orchestration_live
        )
        .await;
        assert!(result.is_error);
        assert!(result.content.contains("traversal"));
    }

    #[tokio::test]
    async fn test_web_search() {
        let result = execute_tool(
            "test-id",
            "web_search",
            &serde_json::json!({"query": "rust programming"}),
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
            None, // media_engine
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
            None, // orchestration_live
        )
        .await;
        // web_search now attempts a real fetch; may succeed or fail depending on network
        assert!(!result.tool_use_id.is_empty());
    }

    #[tokio::test]
    async fn test_unknown_tool() {
        let result = execute_tool(
            "test-id",
            "nonexistent_tool",
            &serde_json::json!({}),
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
            None, // media_engine
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
            None, // orchestration_live
        )
        .await;
        assert!(result.is_error);
        assert!(result.content.contains("Unknown tool"));
    }

    #[tokio::test]
    async fn test_agent_tools_without_kernel() {
        let result = execute_tool(
            "test-id",
            "agent_list",
            &serde_json::json!({}),
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
            None, // media_engine
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
            None, // orchestration_live
        )
        .await;
        assert!(result.is_error);
        assert!(result.content.contains("Kernel handle not available"));
    }

    #[tokio::test]
    async fn test_capability_enforcement_denied() {
        let allowed = vec!["file_read".to_string(), "file_list".to_string()];
        let result = execute_tool(
            "test-id",
            "shell_exec",
            &serde_json::json!({"command": "ls"}),
            None,
            Some(&allowed),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None, // media_engine
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
            None, // orchestration_live
        )
        .await;
        assert!(result.is_error);
        assert!(result.content.contains("Permission denied"));
    }

    #[tokio::test]
    async fn test_capability_enforcement_allowed() {
        let allowed = vec!["file_read".to_string()];
        // Use a cross-platform nonexistent path
        let bad_path = std::env::temp_dir()
            .join("openfang_test_nonexistent_12345")
            .join("file.txt");
        let result = execute_tool(
            "test-id",
            "file_read",
            &serde_json::json!({"path": bad_path.to_str().unwrap()}),
            None,
            Some(&allowed),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None, // media_engine
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
            None, // orchestration_live
        )
        .await;
        // Should fail for file-not-found, NOT for permission denied
        assert!(
            result.is_error,
            "Expected error but got: {}",
            result.content
        );
        assert!(
            result.content.contains("Failed to read")
                || result.content.contains("not found")
                || result.content.contains("No such file"),
            "Unexpected error: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_capability_enforcement_aliased_tool_name() {
        // Agent has "file_write" in allowed tools, but LLM calls "fs-write".
        // After normalization, this should pass the capability check.
        let allowed = vec![
            "file_read".to_string(),
            "file_write".to_string(),
            "file_list".to_string(),
            "shell_exec".to_string(),
        ];
        let result = execute_tool(
            "test-id",
            "fs-write", // LLM-hallucinated alias
            &serde_json::json!({"path": "/nonexistent/file.txt", "content": "hello"}),
            None,
            Some(&allowed),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None, // media_engine
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
            None, // orchestration_live
        )
        .await;
        // Should NOT be the capability-enforcement "Permission denied" — it should
        // normalize to file_write and pass the capability check.  It may still fail
        // for filesystem reasons (e.g. OS "Permission denied (os error 13)"), so we
        // check specifically for the capability-gate message.
        assert!(
            !result.content.contains("Permission denied: agent"),
            "fs-write should normalize to file_write and pass capability check, got: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_capability_enforcement_aliased_denied() {
        // Agent does NOT have file_write, and LLM calls "fs-write" — should be denied.
        let allowed = vec!["file_read".to_string()];
        let result = execute_tool(
            "test-id",
            "fs-write",
            &serde_json::json!({"path": "/tmp/test.txt", "content": "hello"}),
            None,
            Some(&allowed),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None, // media_engine
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
            None, // orchestration_live
        )
        .await;
        assert!(result.is_error);
        assert!(
            result.content.contains("Permission denied"),
            "fs-write should normalize to file_write which is not in allowed list"
        );
    }

    #[test]
    fn test_missing_required_schema_keys_reports_missing_fields() {
        let schema = serde_json::json!({
            "type": "object",
            "required": ["source", "strict"],
            "properties": {
                "source": { "type": "string" },
                "strict": { "type": "boolean" }
            }
        });
        let missing = missing_required_schema_keys(&schema, &serde_json::json!({"source": "x"}));
        assert_eq!(missing, vec!["strict"]);
    }

    #[tokio::test]
    async fn test_shell_exec_mcp_command_reroutes_before_capability_check() {
        let allowed = vec!["mcp_ainl_ainl_capabilities".to_string()];
        let result = execute_tool(
            "test-id",
            "shell_exec",
            &serde_json::json!({"command": "mcp_ainl_ainl_capabilities"}),
            None,
            Some(&allowed),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None, // media_engine
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
            None, // orchestration_live
        )
        .await;
        assert!(result.is_error);
        assert!(
            result
                .content
                .contains("MCP not available for tool: mcp_ainl_ainl_capabilities"),
            "Expected MCP dispatch path, got: {}",
            result.content
        );
    }

    // --- Schedule parser tests ---
    #[test]
    fn test_parse_schedule_every_minutes() {
        assert_eq!(
            parse_schedule_to_cron("every 5 minutes").unwrap(),
            "*/5 * * * *"
        );
        assert_eq!(
            parse_schedule_to_cron("every 1 minute").unwrap(),
            "* * * * *"
        );
        assert_eq!(parse_schedule_to_cron("every minute").unwrap(), "* * * * *");
        assert_eq!(
            parse_schedule_to_cron("every 30 minutes").unwrap(),
            "*/30 * * * *"
        );
    }

    #[test]
    fn test_parse_schedule_every_hours() {
        assert_eq!(parse_schedule_to_cron("every hour").unwrap(), "0 * * * *");
        assert_eq!(parse_schedule_to_cron("every 1 hour").unwrap(), "0 * * * *");
        assert_eq!(
            parse_schedule_to_cron("every 2 hours").unwrap(),
            "0 */2 * * *"
        );
    }

    #[test]
    fn test_parse_schedule_daily() {
        assert_eq!(parse_schedule_to_cron("daily at 9am").unwrap(), "0 9 * * *");
        assert_eq!(
            parse_schedule_to_cron("daily at 6pm").unwrap(),
            "0 18 * * *"
        );
        assert_eq!(
            parse_schedule_to_cron("daily at 12am").unwrap(),
            "0 0 * * *"
        );
        assert_eq!(
            parse_schedule_to_cron("daily at 12pm").unwrap(),
            "0 12 * * *"
        );
    }

    #[test]
    fn test_parse_schedule_weekdays() {
        assert_eq!(
            parse_schedule_to_cron("weekdays at 9am").unwrap(),
            "0 9 * * 1-5"
        );
        assert_eq!(
            parse_schedule_to_cron("weekends at 10am").unwrap(),
            "0 10 * * 0,6"
        );
    }

    #[test]
    fn test_parse_schedule_shorthand() {
        assert_eq!(parse_schedule_to_cron("hourly").unwrap(), "0 * * * *");
        assert_eq!(parse_schedule_to_cron("daily").unwrap(), "0 0 * * *");
        assert_eq!(parse_schedule_to_cron("weekly").unwrap(), "0 0 * * 0");
        assert_eq!(parse_schedule_to_cron("monthly").unwrap(), "0 0 1 * *");
    }

    #[test]
    fn test_parse_schedule_cron_passthrough() {
        assert_eq!(
            parse_schedule_to_cron("0 */5 * * *").unwrap(),
            "0 */5 * * *"
        );
        assert_eq!(
            parse_schedule_to_cron("30 9 * * 1-5").unwrap(),
            "30 9 * * 1-5"
        );
    }

    #[test]
    fn test_parse_schedule_invalid() {
        assert!(parse_schedule_to_cron("whenever I feel like it").is_err());
        assert!(parse_schedule_to_cron("every 0 minutes").is_err());
    }

    // --- Image format detection tests ---
    #[test]
    fn test_detect_image_format_png() {
        let data = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x10\x00\x00\x00\x10";
        assert_eq!(detect_image_format(data), "png");
    }

    #[test]
    fn test_detect_image_format_jpeg() {
        let data = b"\xFF\xD8\xFF\xE0\x00\x10JFIF";
        assert_eq!(detect_image_format(data), "jpeg");
    }

    #[test]
    fn test_detect_image_format_gif() {
        let data = b"GIF89a\x10\x00\x10\x00";
        assert_eq!(detect_image_format(data), "gif");
    }

    #[test]
    fn test_detect_image_format_bmp() {
        let data = b"BM\x00\x00\x00\x00";
        assert_eq!(detect_image_format(data), "bmp");
    }

    #[test]
    fn test_detect_image_format_unknown() {
        let data = b"\x00\x00\x00\x00";
        assert_eq!(detect_image_format(data), "unknown");
    }

    #[test]
    fn test_extract_png_dimensions() {
        // Minimal PNG header: signature (8) + IHDR length (4) + "IHDR" (4) + width (4) + height (4)
        let mut data = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]; // signature
        data.extend_from_slice(&[0x00, 0x00, 0x00, 0x0D]); // IHDR length
        data.extend_from_slice(b"IHDR"); // chunk type
        data.extend_from_slice(&640u32.to_be_bytes()); // width
        data.extend_from_slice(&480u32.to_be_bytes()); // height
        assert_eq!(extract_image_dimensions(&data, "png"), Some((640, 480)));
    }

    #[test]
    fn test_extract_gif_dimensions() {
        let mut data = b"GIF89a".to_vec();
        data.extend_from_slice(&320u16.to_le_bytes()); // width
        data.extend_from_slice(&240u16.to_le_bytes()); // height
        assert_eq!(extract_image_dimensions(&data, "gif"), Some((320, 240)));
    }

    #[test]
    fn test_format_file_size() {
        assert_eq!(format_file_size(500), "500 B");
        assert_eq!(format_file_size(1536), "1.5 KB");
        assert_eq!(format_file_size(2 * 1024 * 1024), "2.0 MB");
    }

    #[tokio::test]
    async fn test_image_analyze_missing_file() {
        let result = execute_tool(
            "test-id",
            "image_analyze",
            &serde_json::json!({"path": "/nonexistent/image.png"}),
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
            None, // media_engine
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
            None, // orchestration_live
        )
        .await;
        assert!(result.is_error);
        assert!(result.content.contains("Failed to read"));
    }

    #[test]
    fn test_depth_limit_default_cap() {
        assert_eq!(DEFAULT_MAX_AGENT_CALL_DEPTH, 5);
    }

    #[test]
    fn test_depth_limit_first_call_succeeds() {
        // Default depth is 0, which is < effective max when task-local unset
        let default_depth = AGENT_CALL_DEPTH.try_with(|d| d.get()).unwrap_or(0);
        assert!(default_depth < effective_max_agent_call_depth());
    }

    #[test]
    fn test_task_local_compiles() {
        // Verify task_local macro works — just ensure the type exists
        let cell = std::cell::Cell::new(0u32);
        assert_eq!(cell.get(), 0);
    }

    #[tokio::test]
    async fn test_schedule_tools_without_kernel() {
        let result = execute_tool(
            "test-id",
            "schedule_list",
            &serde_json::json!({}),
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
            None, // media_engine
            None, // exec_policy
            None, // tts_engine
            None, // docker_config
            None, // process_manager
            None, // orchestration_live
        )
        .await;
        assert!(result.is_error);
        assert!(result.content.contains("Kernel handle not available"));
    }

    // ─── Canvas / A2UI tests ────────────────────────────────────────

    #[test]
    fn test_sanitize_canvas_basic_html() {
        let html = "<h1>Hello World</h1><p>This is a test.</p>";
        let result = sanitize_canvas_html(html, 512 * 1024);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), html);
    }

    #[test]
    fn test_sanitize_canvas_rejects_script() {
        let html = "<div><script>alert('xss')</script></div>";
        let result = sanitize_canvas_html(html, 512 * 1024);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("script"));
    }

    #[test]
    fn test_sanitize_canvas_rejects_iframe() {
        let html = "<iframe src='https://evil.com'></iframe>";
        let result = sanitize_canvas_html(html, 512 * 1024);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("iframe"));
    }

    #[test]
    fn test_sanitize_canvas_rejects_event_handler() {
        let html = "<div onclick=\"alert('xss')\">click me</div>";
        let result = sanitize_canvas_html(html, 512 * 1024);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("event handler"));
    }

    #[test]
    fn test_sanitize_canvas_rejects_onload() {
        let html = "<img src='x' onerror = \"alert(1)\">";
        let result = sanitize_canvas_html(html, 512 * 1024);
        assert!(result.is_err());
    }

    #[test]
    fn test_sanitize_canvas_rejects_javascript_url() {
        let html = "<a href=\"javascript:alert('xss')\">click</a>";
        let result = sanitize_canvas_html(html, 512 * 1024);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("javascript:"));
    }

    #[test]
    fn test_sanitize_canvas_rejects_data_html() {
        let html = "<a href=\"data:text/html,<script>alert(1)</script>\">x</a>";
        let result = sanitize_canvas_html(html, 512 * 1024);
        assert!(result.is_err());
    }

    #[test]
    fn test_sanitize_canvas_rejects_empty() {
        let result = sanitize_canvas_html("", 512 * 1024);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Empty"));
    }

    #[test]
    fn test_sanitize_canvas_size_limit() {
        let html = "x".repeat(1024);
        let result = sanitize_canvas_html(&html, 100);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("too large"));
    }

    #[tokio::test]
    async fn test_canvas_present_tool() {
        let input = serde_json::json!({
            "html": "<h1>Test Canvas</h1><p>Hello world</p>",
            "title": "Test"
        });
        let tmp = std::env::temp_dir().join("openfang_canvas_test");
        let _ = std::fs::create_dir_all(&tmp);
        let result = tool_canvas_present(&input, Some(tmp.as_path())).await;
        assert!(result.is_ok());
        let output: serde_json::Value = serde_json::from_str(&result.unwrap()).unwrap();
        assert!(output["canvas_id"].is_string());
        assert_eq!(output["title"], "Test");
        // Cleanup
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn orchestration_shared_merge_applies_with_orchestration_live() {
        use openfang_types::agent::AgentId;
        use openfang_types::orchestration::{OrchestrationContext, OrchestrationPattern};

        let aid = AgentId::new();
        let ctx = OrchestrationContext::new_root(
            aid,
            OrchestrationPattern::AdHoc,
            Some("balanced".to_string()),
        );
        let live: OrchestrationLive = std::sync::Arc::new(tokio::sync::RwLock::new(ctx));
        let input = serde_json::json!({ "patch": { "k": 42 } });
        let res = execute_tool(
            "tid",
            "orchestration_shared_merge",
            &input,
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
            Some(&live),
        )
        .await;
        assert!(!res.is_error, "{}", res.content);
        assert!(res.content.contains("Merged 1 key"), "{}", res.content);

        let g = live.read().await;
        assert_eq!(g.shared_vars.get("k"), Some(&serde_json::json!(42)));
    }

    // -------------------------------------------------------------------------
    // file_* refinements: deep-nested write, dir-as-file_read, placeholder block
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_file_write_into_nested_missing_dirs_succeeds() {
        // Regression: agents asking to write `apollo-x-bot/modules/common/retry.ainl`
        // into a workspace that only contains the root previously failed with
        // "Failed to resolve parent directory". With the sandbox walk-up fix +
        // the existing `create_dir_all(parent)` in `tool_file_write`, the deep
        // path should be created and written transparently.
        let dir = tempfile::TempDir::new().unwrap();
        let payload = serde_json::json!({
            "path": "apollo-x-bot/modules/common/retry.ainl",
            "content": "# test\nLENTRY:\n  R core.ADD 1 1 ->x\n  J x\n",
        });
        let res = tool_file_write(&payload, Some(dir.path())).await;
        assert!(
            res.is_ok(),
            "expected nested write to succeed, got: {:?}",
            res.err()
        );
        let summary = res.unwrap();
        assert!(summary.contains("Successfully wrote"), "{}", summary);
        let written = dir.path().join("apollo-x-bot/modules/common/retry.ainl");
        assert!(
            written.exists(),
            "expected file to be created: {}",
            written.display()
        );
    }

    #[tokio::test]
    async fn test_file_read_on_directory_returns_listing() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("apollo-x-bot/modules")).unwrap();
        std::fs::write(dir.path().join("apollo-x-bot/README.md"), "hi").unwrap();
        let payload = serde_json::json!({"path": "apollo-x-bot"});
        let res = tool_file_read(&payload, Some(dir.path()), None).await;
        assert!(res.is_ok(), "{:?}", res.err());
        let body = res.unwrap();
        assert!(body.contains("file_read was given a directory"), "{}", body);
        assert!(
            body.contains("README.md"),
            "listing must include files: {}",
            body
        );
        assert!(
            body.contains("modules/"),
            "listing must include subdirs with /: {}",
            body
        );
    }

    #[tokio::test]
    async fn test_file_write_rejects_truncation_placeholder() {
        // Real example pulled from a `browser-hand` session: agent wrote a
        // 170-byte `record_decision.ainl` whose body was literally
        // "[... full content truncated for brevity ...]" — silently broken.
        let dir = tempfile::TempDir::new().unwrap();
        let payload = serde_json::json!({
            "path": "modules_common_record_decision.ainl",
            "content": "# modules/common/record_decision.ainl\n[... full content truncated for brevity ...]",
        });
        let res = tool_file_write(&payload, Some(dir.path())).await;
        assert!(
            res.is_err(),
            "expected truncation placeholder to be rejected"
        );
        let err = res.err().unwrap();
        assert!(err.contains("placeholder/truncation"), "{}", err);
        assert!(
            !dir.path()
                .join("modules_common_record_decision.ainl")
                .exists(),
            "broken file must not land on disk"
        );
    }

    #[test]
    fn test_detect_truncation_placeholder_negatives() {
        // Real prose mentioning the word "truncated" should not trip the guard.
        assert!(
            detect_truncation_placeholder("function clamp(x) { /* clamps not truncated */ }")
                .is_none()
        );
        assert!(
            detect_truncation_placeholder("def parse():\n    # parse JSON\n    pass\n").is_none()
        );
        // Positive controls.
        assert!(detect_truncation_placeholder(
            "hello\n[... full content truncated for brevity ...]\n"
        )
        .is_some());
        assert!(detect_truncation_placeholder("// ... rest of file ...").is_some());
    }

    // -------------------------------------------------------------------------
    // script_run: deterministic interpreter selection (Phase 1 of the long-term
    // "model decides intent, runtime decides how" architecture).
    // -------------------------------------------------------------------------

    /// Oneshot mode runs a real `.sh` script to completion and returns structured JSON
    /// with stdout/stderr + exit_code + a `runner` block describing the interpreter pick.
    /// Skipped on Windows because it relies on a POSIX shell being on PATH.
    #[tokio::test]
    #[cfg(unix)]
    async fn test_script_run_oneshot_shell_returns_stdout_and_runner_meta() {
        let dir = tempfile::TempDir::new().unwrap();
        let script = dir.path().join("hello.sh");
        std::fs::write(&script, "#!/bin/sh\necho hi-from-script\n").unwrap();

        let payload = serde_json::json!({ "script": "hello.sh" });
        let res = tool_script_run(&payload, &[], Some(dir.path()), None, None, None, None).await;

        assert!(
            res.is_ok(),
            "expected oneshot success, got: {:?}",
            res.err()
        );
        let body = res.unwrap();
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["mode"], "oneshot");
        assert_eq!(v["ok"], true);
        assert_eq!(v["exit_code"], 0);
        assert!(
            v["stdout"].as_str().unwrap().contains("hi-from-script"),
            "stdout should contain the echo: {body}"
        );
        let runner = &v["runner"];
        assert_eq!(runner["language"], "shell");
        assert_eq!(runner["decision_source"], "extension");
        assert!(
            runner["script_path"]
                .as_str()
                .unwrap()
                .ends_with("hello.sh"),
            "script_path must point at the resolved file: {body}"
        );
    }

    /// Missing script returns a structured error message that names the path and tells
    /// the agent how to recover (use `file_list`, etc.) — never a bare "command not found".
    #[tokio::test]
    async fn test_script_run_missing_file_returns_actionable_hint() {
        let dir = tempfile::TempDir::new().unwrap();
        let payload = serde_json::json!({ "script": "no-such.py" });
        let res = tool_script_run(&payload, &[], Some(dir.path()), None, None, None, None).await;

        let err = res.expect_err("missing script must error");
        assert!(
            err.contains("script_run: script not found"),
            "must surface the structured prefix: {err}"
        );
        assert!(
            err.contains("no-such.py"),
            "must echo the requested path: {err}"
        );
        assert!(
            err.contains("file_list") || err.contains("workspace"),
            "must include a recovery hint: {err}"
        );
    }

    /// Unknown / unsupported `mode` is rejected with a clear list of accepted values.
    #[tokio::test]
    async fn test_script_run_unknown_mode_rejected() {
        let dir = tempfile::TempDir::new().unwrap();
        let script = dir.path().join("noop.sh");
        std::fs::write(&script, "#!/bin/sh\nexit 0\n").unwrap();

        let payload = serde_json::json!({ "script": "noop.sh", "mode": "spawn-and-pray" });
        let res = tool_script_run(&payload, &[], Some(dir.path()), None, None, None, None).await;

        let err = res.expect_err("invalid mode must error");
        assert!(err.contains("script_run: unknown `mode`"), "{err}");
        assert!(err.contains("oneshot"), "must list accepted modes: {err}");
        assert!(err.contains("daemon"), "must list accepted modes: {err}");
    }

    #[tokio::test]
    async fn test_script_run_infers_daemon_for_gateway_scripts() {
        let dir = tempfile::TempDir::new().unwrap();
        let script = dir.path().join("gateway_server.py");
        std::fs::write(&script, "print('x')\n").unwrap();

        // No explicit mode, but "gateway_server.py" should infer daemon.
        // Without a process manager, that means a deterministic error.
        let payload = serde_json::json!({ "script": "gateway_server.py" });
        let err = tool_script_run(&payload, &[], Some(dir.path()), None, None, None, None)
            .await
            .expect_err("daemon inference requires process manager");
        assert!(err.contains("process_manager not available"), "{err}");
    }

    /// `script_run` must be in the public `builtin_tool_definitions()` list and have
    /// a non-empty schema so the LLM actually sees it as an option.
    #[test]
    fn test_script_run_is_registered_as_builtin() {
        let tools = builtin_tool_definitions();
        let def = tools
            .iter()
            .find(|t| t.name == "script_run")
            .expect("script_run must be registered");
        assert!(
            def.description.contains("script_run") || def.description.contains("project script"),
            "description must mention the tool's purpose: {}",
            def.description
        );
        let required = def
            .input_schema
            .get("required")
            .and_then(|v| v.as_array())
            .expect("script_run schema must declare `required`");
        assert!(
            required.iter().any(|v| v.as_str() == Some("script")),
            "`script` must be required"
        );
    }

    #[tokio::test]
    async fn test_script_detect_finds_files_and_package_scripts() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/gateway.ts"), "console.log('hi')\n").unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"name":"x","scripts":{"dev":"tsx src/gateway.ts","start":"node src/gateway.ts"}}"#,
        )
        .unwrap();

        let payload = serde_json::json!({ "query": "gateway" });
        let body = tool_script_detect(&payload, Some(dir.path()))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let results = v["results"].as_array().unwrap();
        assert!(!results.is_empty(), "expected some results: {body}");
        // Expect at least one file match.
        assert!(
            results
                .iter()
                .any(|r| r["kind"] == "file" && r["path"].as_str().unwrap().contains("gateway.ts")),
            "expected gateway.ts file candidate: {body}"
        );
        // Expect at least one package script match.
        assert!(
            results
                .iter()
                .any(|r| r["kind"] == "package_script" && r["name"] == "dev"),
            "expected dev package_script candidate: {body}"
        );
    }

    #[test]
    fn test_script_detect_is_registered_as_builtin() {
        let tools = builtin_tool_definitions();
        tools
            .iter()
            .find(|t| t.name == "script_detect")
            .expect("script_detect must be registered");
    }

    #[tokio::test]
    async fn test_workspace_actions_list_reads_contract() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("armaraos.toml"),
            r#"
[actions.gateway]
description = "Start gateway"
script = "gateway.sh"
mode = "oneshot"
"#,
        )
        .unwrap();
        let out = tool_workspace_actions_list(Some(dir.path())).await.unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        let actions = v["actions"].as_array().unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0]["name"], "gateway");
    }

    #[tokio::test]
    async fn test_workspace_action_set_creates_contract() {
        let dir = tempfile::TempDir::new().unwrap();
        let input = serde_json::json!({
            "action": "gateway",
            "description": "Start gateway",
            "script": "gateway_server.py",
            "mode": "daemon",
            "env": { "PROMOTER_GATEWAY_PORT": "9011" },
            "health_check": {
                "url": "http://127.0.0.1:9011/v1/promoter.stats",
                "expect_status": 200
            }
        });
        let out = tool_workspace_action_set(&input, Some(dir.path()))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["action"], "gateway");
        let raw = std::fs::read_to_string(dir.path().join("armaraos.toml")).unwrap();
        assert!(raw.contains("[actions.gateway]"), "{raw}");
        assert!(raw.contains("gateway_server.py"), "{raw}");
    }

    #[tokio::test]
    async fn test_workspace_action_delete_removes_last_contract() {
        let dir = tempfile::TempDir::new().unwrap();
        let set_input = serde_json::json!({
            "action": "gateway",
            "script": "gateway_server.py",
        });
        tool_workspace_action_set(&set_input, Some(dir.path()))
            .await
            .unwrap();
        assert!(dir.path().join("armaraos.toml").exists());

        let del_input = serde_json::json!({ "action": "gateway" });
        let out = tool_workspace_action_delete(&del_input, Some(dir.path()))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["status"], "deleted_last_action_removed_contract");
        assert!(!dir.path().join("armaraos.toml").exists());
    }

    #[test]
    fn test_workspace_action_set_delete_are_registered_as_builtins() {
        let tools = builtin_tool_definitions();
        tools
            .iter()
            .find(|t| t.name == "workspace_action_set")
            .expect("workspace_action_set must be registered");
        tools
            .iter()
            .find(|t| t.name == "workspace_action_delete")
            .expect("workspace_action_delete must be registered");
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn test_workspace_action_executes_named_contract_action() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("run.sh"),
            "#!/bin/sh\necho hi-from-contract\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("armaraos.toml"),
            r#"
[actions.gateway]
script = "run.sh"
mode = "oneshot"
"#,
        )
        .unwrap();

        let input = serde_json::json!({ "action": "gateway" });
        let out = tool_workspace_action(
            &input,
            &[],
            Some(dir.path()),
            None,
            Some("agent-1"),
            None,
            None,
        )
        .await
        .unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["ok"], true);
        assert!(v["stdout"].as_str().unwrap().contains("hi-from-contract"));
        assert_eq!(v["workspace_action"]["name"], "gateway");
    }
}
