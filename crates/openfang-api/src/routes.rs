//! Route handlers for the OpenFang API.

use crate::middleware::RequestId;
use crate::types::*;
use axum::extract::{ConnectInfo, Extension, Multipart, Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use dashmap::mapref::entry::Entry;
use dashmap::DashMap;
use globset::{Glob, GlobSet, GlobSetBuilder};
use openfang_channels::bridge::channel_command_specs;
use openfang_hands;
use openfang_kernel::triggers::{TriggerId, TriggerPattern};
use openfang_kernel::workflow::{
    AggregationStrategy, ErrorMode, StepAgent, StepMode, Workflow, WorkflowId, WorkflowStep,
};
use openfang_kernel::OpenFangKernel;
use openfang_runtime::kernel_handle::KernelHandle;
use openfang_runtime::tool_runner::builtin_tool_definitions;
use openfang_runtime::workspace_sandbox::resolve_sandbox_path;
use openfang_types::agent::{AgentId, AgentIdentity, AgentManifest};
use openfang_types::message::Role;
use openfang_types::scheduler::{CronAction, CronDelivery, CronJob, CronJobId, CronSchedule};
use std::collections::{BTreeMap, HashMap};
use std::net::SocketAddr;
use std::path::{Path as FsPath, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::Instant;
use walkdir::WalkDir;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

/// Shared application state.
///
/// The kernel is wrapped in Arc so it can serve as both the main kernel
/// and the KernelHandle for inter-agent tool access.
pub struct AppState {
    pub kernel: Arc<OpenFangKernel>,
    pub started_at: Instant,
    /// Optional peer registry for OFP mesh networking status.
    pub peer_registry: Option<Arc<openfang_wire::registry::PeerRegistry>>,
    /// Channel bridge manager — held behind a Mutex so it can be swapped on hot-reload.
    pub bridge_manager: tokio::sync::Mutex<Option<openfang_channels::bridge::BridgeManager>>,
    /// Live channel config — updated on every hot-reload so list_channels() reflects reality.
    pub channels_config: tokio::sync::RwLock<openfang_types::config::ChannelsConfig>,
    /// Notify handle to trigger graceful HTTP server shutdown from the API.
    pub shutdown_notify: Arc<tokio::sync::Notify>,
    /// ClawHub response cache — prevents 429 rate limiting on rapid dashboard refreshes.
    /// Maps cache key → (fetched_at, response_json) with 120s TTL.
    pub clawhub_cache: DashMap<String, (Instant, serde_json::Value)>,
    /// Probe cache for local provider health checks (ollama/vllm/lmstudio).
    /// Avoids blocking the `/api/providers` endpoint on TCP timeouts to
    /// unreachable local services. 60-second TTL.
    pub provider_probe_cache: openfang_runtime::provider_health::ProbeCache,
    /// Thread-safe mutable budget config. Updated via PUT /api/budget.
    /// Initialized from `kernel.config.budget` at startup.
    pub budget_config: Arc<tokio::sync::RwLock<openfang_types::config::BudgetConfig>>,
    /// Timestamps (unix ms) of recent `POST /api/ainl/library/register-curated` calls per client IP.
    pub ainl_register_hits: DashMap<std::net::IpAddr, Vec<u64>>,
    /// Background sampler for `GET /api/system/daemon-resources` (dashboard footprint strip).
    pub daemon_resources: Arc<crate::daemon_resources::DaemonResources>,
}

#[inline]
fn resolve_request_id(ext: Option<Extension<RequestId>>) -> RequestId {
    ext.map(|e| e.0)
        .unwrap_or_else(|| RequestId("unknown".to_string()))
}

/// PATCH merge for optional identity strings: absent request keeps `current`; empty string clears.
#[inline]
fn patch_merge_identity_opt(req: Option<String>, current: Option<String>) -> Option<String> {
    match req {
        None => current,
        Some(s) if s.is_empty() => None,
        Some(s) => Some(s),
    }
}

/// PATCH merge for color: absent keeps `current`; empty string keeps `current` (invalid client payload).
#[inline]
fn patch_merge_color_opt(req: Option<String>, current: Option<String>) -> Option<String> {
    match req {
        None => current,
        Some(s) if s.is_empty() => current,
        Some(s) => Some(s),
    }
}

/// Structured JSON error for dashboard clients (`error`, `detail`, `path`, `request_id`, optional `hint`).
pub fn api_json_error(
    status: StatusCode,
    req_id: &RequestId,
    path: &str,
    error: &str,
    detail: String,
    hint: Option<&str>,
) -> (StatusCode, Json<serde_json::Value>) {
    let mut v = serde_json::json!({
        "error": error,
        "detail": detail,
        "path": path,
        "request_id": req_id.0,
    });
    if let Some(h) = hint {
        v["hint"] = serde_json::json!(h);
    }
    (status, Json(v))
}

/// POST /api/agents — Spawn a new agent.
pub async fn spawn_agent(
    State(state): State<Arc<AppState>>,
    ext: Option<Extension<RequestId>>,
    Json(req): Json<SpawnRequest>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    // Resolve template name → manifest_toml if template is provided and manifest_toml is empty
    let manifest_toml = if req.manifest_toml.trim().is_empty() {
        if let Some(ref tmpl_name) = req.template {
            // Sanitize template name to prevent path traversal
            let safe_name = tmpl_name
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
                .collect::<String>();
            if safe_name.is_empty() || safe_name != *tmpl_name {
                return api_json_error(
                    StatusCode::BAD_REQUEST,
                    &rid,
                    "/api/agents",
                    "Invalid template name",
                    "Template names may only contain letters, numbers, hyphens, and underscores.".to_string(),
                    Some("Pick a template folder name under ~/.armaraos/agents/<name>/ without special characters."),
                );
            }
            let tmpl_path = state
                .kernel
                .config
                .home_dir
                .join("agents")
                .join(&safe_name)
                .join("agent.toml");
            match std::fs::read_to_string(&tmpl_path) {
                Ok(content) => content,
                Err(_) => {
                    return api_json_error(
                        StatusCode::NOT_FOUND,
                        &rid,
                        "/api/agents",
                        "Template not found",
                        format!(
                            "No agent.toml at ~/.armaraos/agents/{safe_name}/agent.toml (or ARMARAOS_HOME equivalent)."
                        ),
                        Some("Copy an example into agents/ or pass manifest_toml in the request body."),
                    );
                }
            }
        } else {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                "/api/agents",
                "Missing manifest",
                "Either 'manifest_toml' or 'template' is required in the JSON body.".to_string(),
                Some("Paste a manifest TOML or set template to a folder name under ~/.armaraos/agents/."),
            );
        }
    } else {
        req.manifest_toml.clone()
    };

    // SECURITY: Reject oversized manifests to prevent parser memory exhaustion.
    const MAX_MANIFEST_SIZE: usize = 1024 * 1024; // 1MB
    if manifest_toml.len() > MAX_MANIFEST_SIZE {
        return api_json_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            &rid,
            "/api/agents",
            "Manifest too large",
            format!("Manifest exceeds maximum size ({MAX_MANIFEST_SIZE} bytes)."),
            Some("Trim embedded assets or split into smaller manifests."),
        );
    }

    // SECURITY: Verify Ed25519 signature when a signed manifest is provided
    if let Some(ref signed_json) = req.signed_manifest {
        match state.kernel.verify_signed_manifest(signed_json) {
            Ok(verified_toml) => {
                // Ensure the signed manifest matches the provided manifest_toml
                if verified_toml.trim() != manifest_toml.trim() {
                    tracing::warn!("Signed manifest content does not match manifest_toml");
                    return api_json_error(
                        StatusCode::BAD_REQUEST,
                        &rid,
                        "/api/agents",
                        "Signed manifest mismatch",
                        "Verified signed_manifest content does not match manifest_toml."
                            .to_string(),
                        Some("Ensure the TOML you sign is identical to manifest_toml."),
                    );
                }
            }
            Err(e) => {
                tracing::warn!("Manifest signature verification failed: {e}");
                state.kernel.audit_log.record(
                    "system",
                    openfang_runtime::audit::AuditAction::AuthAttempt,
                    "manifest signature verification failed",
                    format!("error: {e}"),
                );
                return api_json_error(
                    StatusCode::FORBIDDEN,
                    &rid,
                    "/api/agents",
                    "Manifest signature verification failed",
                    format!("Signature or public key could not validate this manifest: {e}"),
                    Some("Regenerate the signed manifest or disable signing for development."),
                );
            }
        }
    }

    let manifest: AgentManifest = match toml::from_str(&manifest_toml) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!("Invalid manifest TOML: {e}");
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                "/api/agents",
                "Invalid manifest format",
                format!("TOML parse error: {e}"),
                Some("Validate agent.toml against the AgentManifest schema."),
            );
        }
    };

    let name = manifest.name.clone();
    match state.kernel.spawn_agent(manifest) {
        Ok(id) => {
            // Register in channel router so binding resolution finds the new agent
            if let Some(ref mgr) = *state.bridge_manager.lock().await {
                mgr.router().register_agent(name.clone(), id);
            }
            (
                StatusCode::CREATED,
                Json(serde_json::json!(SpawnResponse {
                    agent_id: id.to_string(),
                    name,
                })),
            )
        }
        Err(e) => {
            tracing::warn!("Spawn failed: {e}");
            api_json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &rid,
                "/api/agents",
                "Agent spawn failed",
                format!("{e}"),
                Some("Check kernel logs, provider configuration, and agent manifest constraints."),
            )
        }
    }
}

/// GET /api/agents — List all agents.
pub async fn list_agents(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // Snapshot catalog once for enrichment
    let catalog = state.kernel.model_catalog.read().ok();
    let dm = &state.kernel.config.default_model;
    let home_dir = state.kernel.config.home_dir.clone();
    let ainl_runtime_compile_flags = openfang_runtime::ainl_integration_compile_flags();
    let ainl_runtime_engine_compiled = ainl_runtime_compile_flags
        .get("ainl_runtime_engine")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let ainl_runtime_engine_forced_by_env =
        std::env::var("AINL_RUNTIME_ENGINE").ok().as_deref() == Some("1");
    let ainl_runtime_engine_env_disabled = openfang_runtime::ainl_runtime_engine_env_disabled();

    let agents: Vec<serde_json::Value> = state
        .kernel
        .registry
        .list()
        .into_iter()
        .map(|e| {
            // Resolve "default" provider/model to actual kernel defaults
            let provider =
                if e.manifest.model.provider.is_empty() || e.manifest.model.provider == "default" {
                    dm.provider.as_str()
                } else {
                    e.manifest.model.provider.as_str()
                };
            let model = if e.manifest.model.model.is_empty() || e.manifest.model.model == "default"
            {
                dm.model.as_str()
            } else {
                e.manifest.model.model.as_str()
            };

            // Enrich from catalog
            let (tier, auth_status) = catalog
                .as_ref()
                .map(|cat| {
                    let tier = cat
                        .find_model(model)
                        .map(|m| format!("{:?}", m.tier).to_lowercase())
                        .unwrap_or_else(|| "unknown".to_string());
                    let auth = cat
                        .get_provider(provider)
                        .map(|p| format!("{:?}", p.auth_status).to_lowercase())
                        .unwrap_or_else(|| "unknown".to_string());
                    (tier, auth)
                })
                .unwrap_or(("unknown".to_string(), "unknown".to_string()));

            let ready = matches!(e.state, openfang_types::agent::AgentState::Running)
                && auth_status != "missing";

            let workspace = e
                .manifest
                .workspace
                .as_ref()
                .map(|p| p.display().to_string());
            let workspace_rel_home = e.manifest.workspace.as_ref().and_then(|p| {
                p.strip_prefix(&home_dir)
                    .ok()
                    .map(|rel| rel.to_string_lossy().replace('\\', "/"))
            });
            let ainl_runtime_engine_effective = ainl_runtime_engine_compiled
                && !ainl_runtime_engine_env_disabled
                && (e.manifest.ainl_runtime_engine || ainl_runtime_engine_forced_by_env);

            serde_json::json!({
                "id": e.id.to_string(),
                "name": e.name,
                "state": format!("{:?}", e.state),
                "mode": e.mode,
                "created_at": e.created_at.to_rfc3339(),
                "last_active": e.last_active.to_rfc3339(),
                "model_provider": provider,
                "model_name": model,
                "model_tier": tier,
                "auth_status": auth_status,
                "ready": ready,
                "workspace": workspace,
                "workspace_rel_home": workspace_rel_home,
                "ainl_runtime_engine": e.manifest.ainl_runtime_engine,
                "ainl_runtime_engine_effective": ainl_runtime_engine_effective,
                "ainl_runtime_engine_forced_by_env": ainl_runtime_engine_forced_by_env,
                "ainl_runtime_engine_env_disabled": ainl_runtime_engine_env_disabled,
                "ainl_runtime_engine_compiled": ainl_runtime_engine_compiled,
                "profile": e.manifest.profile,
                "system_prompt": e.manifest.model.system_prompt,
                "identity": {
                    "emoji": e.identity.emoji,
                    "avatar_url": e.identity.avatar_url,
                    "color": e.identity.color,
                    "archetype": e.identity.archetype,
                    "vibe": e.identity.vibe,
                    "greeting_style": e.identity.greeting_style,
                },
            })
        })
        .collect();

    Json(agents)
}

/// Resolve uploaded file attachments into ContentBlock::Image blocks.
///
/// Reads each file from the upload directory, base64-encodes it, and
/// returns image content blocks ready to insert into a session message.
pub fn resolve_attachments(
    attachments: &[AttachmentRef],
) -> Vec<openfang_types::message::ContentBlock> {
    use base64::Engine;

    let upload_dir = std::env::temp_dir().join("openfang_uploads");
    let mut blocks = Vec::new();

    for att in attachments {
        // Look up metadata from the upload registry
        let meta = UPLOAD_REGISTRY.get(&att.file_id);
        let content_type = if let Some(ref m) = meta {
            m.content_type.clone()
        } else if !att.content_type.is_empty() {
            att.content_type.clone()
        } else {
            continue; // Skip unknown attachments
        };

        // Only process image types
        if !content_type.starts_with("image/") {
            continue;
        }

        // Validate file_id is a UUID to prevent path traversal
        if uuid::Uuid::parse_str(&att.file_id).is_err() {
            continue;
        }

        let file_path = upload_dir.join(&att.file_id);
        match std::fs::read(&file_path) {
            Ok(data) => {
                let b64 = base64::engine::general_purpose::STANDARD.encode(&data);
                blocks.push(openfang_types::message::ContentBlock::Image {
                    media_type: content_type,
                    data: b64,
                });
            }
            Err(e) => {
                tracing::warn!(file_id = %att.file_id, error = %e, "Failed to read upload for attachment");
            }
        }
    }

    blocks
}

/// Sanitize a client-provided filename for a safe destination basename.
fn sanitize_upload_filename(name: &str) -> String {
    let base = FsPath::new(name)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("upload.bin");
    let s: String = base
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() || s == "." {
        "file.bin".to_string()
    } else {
        s
    }
}

/// Copy chat uploads from the temp upload dir into `workspace/uploads/` so
/// `file_list` / `file_read` can resolve them. Skips **audio** only (handled via
/// STT transcription). Images are copied too (vision blocks are separate).
///
/// Returns text to append to the user message listing workspace-relative paths.
pub fn workspace_upload_hints(workspace_root: &FsPath, attachments: &[AttachmentRef]) -> String {
    let upload_dir = std::env::temp_dir().join("openfang_uploads");
    let dest_dir = workspace_root.join("uploads");
    if let Err(e) = std::fs::create_dir_all(&dest_dir) {
        tracing::warn!(error = %e, path = %dest_dir.display(), "Failed to create workspace uploads dir");
        return String::new();
    }

    let mut lines: Vec<String> = Vec::new();
    for att in attachments {
        if uuid::Uuid::parse_str(&att.file_id).is_err() {
            continue;
        }

        let meta = UPLOAD_REGISTRY.get(&att.file_id);
        let content_type = meta
            .as_ref()
            .map(|m| m.content_type.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or(att.content_type.as_str());

        if content_type.starts_with("audio/") {
            continue;
        }

        let src = upload_dir.join(&att.file_id);
        if !src.is_file() {
            tracing::warn!(file_id = %att.file_id, "Upload source missing for workspace materialize");
            continue;
        }

        let orig_name = meta
            .as_ref()
            .map(|m| m.filename.as_str())
            .filter(|s| !s.is_empty())
            .or_else(|| {
                let f = att.filename.as_str();
                if f.is_empty() {
                    None
                } else {
                    Some(f)
                }
            })
            .unwrap_or("file");

        let safe = sanitize_upload_filename(orig_name);
        let dest_name = format!("{}_{}", att.file_id, safe);
        let rel = format!("uploads/{}", dest_name);
        let dest = dest_dir.join(&dest_name);
        if let Err(e) = std::fs::copy(&src, &dest) {
            tracing::warn!(error = %e, dest = %dest.display(), "Failed to copy upload to workspace");
            continue;
        }

        lines.push(format!("- `{}` — original filename: {}", rel, orig_name));
    }

    if lines.is_empty() {
        String::new()
    } else {
        format!(
            "\n\nThe attached file(s) were copied into your agent workspace. Use **file_read** with the path below (relative to the workspace root), not the display name alone:\n{}",
            lines.join("\n")
        )
    }
}

/// Pre-insert image attachments into an agent's session so the LLM can see them.
///
/// This injects image content blocks into the session BEFORE the kernel
/// adds the text user message, so the LLM receives: [..., User(images), User(text)].
pub fn inject_attachments_into_session(
    kernel: &OpenFangKernel,
    agent_id: AgentId,
    image_blocks: Vec<openfang_types::message::ContentBlock>,
) {
    use openfang_types::message::{Message, MessageContent, Role};

    let entry = match kernel.registry.get(agent_id) {
        Some(e) => e,
        None => return,
    };

    let mut session = match kernel.memory.get_session(entry.session_id) {
        Ok(Some(s)) => s,
        _ => openfang_memory::session::Session {
            id: entry.session_id,
            agent_id,
            messages: Vec::new(),
            context_window_tokens: 0,
            label: None,
        },
    };

    session.messages.push(Message {
        role: Role::User,
        content: MessageContent::Blocks(image_blocks),
        orchestration_ctx: None,
    });

    if let Err(e) = kernel.memory.save_session(&session) {
        tracing::warn!(error = %e, "Failed to save session with image attachments");
    }
}

/// POST /api/agents/:id/message — Send a message to an agent.
pub async fn send_message(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
    Json(req): Json<MessageRequest>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    let msg_path = "/api/agents/:id/message";
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                msg_path,
                "Invalid agent ID",
                "Agent id must be a valid UUID.".to_string(),
                Some("Use the id from GET /api/agents or the dashboard agent list."),
            );
        }
    };

    // SECURITY: Reject oversized messages to prevent OOM / LLM token abuse.
    const MAX_MESSAGE_SIZE: usize = 64 * 1024; // 64KB
    if req.message.len() > MAX_MESSAGE_SIZE {
        return api_json_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            &rid,
            msg_path,
            "Message too large",
            format!("Message exceeds {MAX_MESSAGE_SIZE} bytes."),
            Some("Shorten the prompt or split attachments across turns."),
        );
    }

    // Check agent exists before processing
    if state.kernel.registry.get(agent_id).is_none() {
        return api_json_error(
            StatusCode::NOT_FOUND,
            &rid,
            msg_path,
            "Agent not found",
            format!("No agent registered for id {id}."),
            Some("Spawn an agent or pick a valid id from GET /api/agents."),
        );
    }

    // Resolve file attachments into image content blocks.
    // Pass them as content_blocks so the LLM receives them in the current turn
    // (not as a separate session message which the LLM may not process).
    let content_blocks = if !req.attachments.is_empty() {
        let image_blocks = resolve_attachments(&req.attachments);
        if image_blocks.is_empty() {
            None
        } else {
            Some(image_blocks)
        }
    } else {
        None
    };

    let mut message_for_agent = req.message.clone();
    if !req.attachments.is_empty() {
        if let Some(entry) = state.kernel.registry.get(agent_id) {
            if let Some(ref ws) = entry.manifest.workspace {
                let hint = workspace_upload_hints(ws.as_path(), &req.attachments);
                if !hint.is_empty() {
                    message_for_agent.push_str(&hint);
                }
            }
        }
    }

    if message_for_agent.len() > MAX_MESSAGE_SIZE {
        return api_json_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            &rid,
            msg_path,
            "Message too large",
            format!("Message exceeds {MAX_MESSAGE_SIZE} bytes after attachments."),
            Some("Shorten the prompt or split attachments across turns."),
        );
    }

    let kernel_handle: Arc<dyn KernelHandle> = state.kernel.clone() as Arc<dyn KernelHandle>;
    match state
        .kernel
        .send_message_with_handle_and_blocks(
            agent_id,
            &message_for_agent,
            Some(kernel_handle),
            content_blocks,
            req.sender_id,
            req.sender_name,
            None,
            None,
        )
        .await
    {
        Ok(result) => {
            // Strip <think>...</think> blocks from model output
            let cleaned = crate::ws::strip_think_tags(&result.response);

            // If the agent intentionally returned a silent/NO_REPLY response,
            // return an empty string — don't generate debug fallback text.
            let response = if result.silent {
                String::new()
            } else if cleaned.trim().is_empty() {
                if result.total_usage.input_tokens == 0 && result.total_usage.output_tokens == 0 {
                    "[No assistant text (0 tokens). The model call did not complete — usually a missing or invalid provider API key. For OpenRouter: Settings → Providers, or set OPENROUTER_API_KEY for the daemon. If an error line appears below, that is the real cause.]".to_string()
                } else {
                    format!(
                        "[The agent completed processing but returned no text response. ({} in / {} out | {} iter)]",
                        result.total_usage.input_tokens,
                        result.total_usage.output_tokens,
                        result.iterations,
                    )
                }
            } else {
                cleaned
            };

            let skill_draft_path = if let Some(intent) =
                openfang_kernel::skills_staging::learn_prefixed_intent(&req.message)
            {
                let frame = openfang_kernel::skills_staging::frame_from_agent_learn_turn(
                    intent,
                    &req.message,
                    &response,
                    &id,
                    result.silent,
                );
                match openfang_kernel::skills_staging::write_skill_draft_markdown(
                    &state.kernel.config.home_dir,
                    &frame,
                ) {
                    Ok(p) => {
                        tracing::info!(path = %p.display(), agent = %id, "skill draft from [learn] message");
                        Some(p.display().to_string())
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, agent = %id, "skill draft write failed");
                        None
                    }
                }
            } else {
                None
            };
            let ainl_runtime_telemetry = result.ainl_runtime_telemetry.as_ref().map(|t| {
                serde_json::json!({
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
                })
            });

            (
                StatusCode::OK,
                Json(serde_json::json!(MessageResponse {
                    response,
                    input_tokens: result.total_usage.input_tokens,
                    output_tokens: result.total_usage.output_tokens,
                    iterations: result.iterations,
                    cost_usd: result.cost_usd,
                    latency_ms: result.latency_ms,
                    llm_fallback_note: result.llm_fallback_note.clone(),
                    skill_draft_path,
                    compression_savings_pct: result.compression_savings_pct,
                    compressed_input: result.compressed_input.clone(),
                    compression_semantic_score: result.compression_semantic_score,
                    adaptive_confidence: result.adaptive_confidence,
                    eco_counterfactual: result.eco_counterfactual.clone(),
                    adaptive_eco_effective_mode: result.adaptive_eco_effective_mode.clone(),
                    adaptive_eco_recommended_mode: result.adaptive_eco_recommended_mode.clone(),
                    adaptive_eco_reason_codes: result.adaptive_eco_reason_codes.clone(),
                    tools: Vec::new(),
                    ainl_runtime_telemetry,
                })),
            )
        }
        Err(e) => {
            tracing::warn!("send_message failed for agent {id}: {e}");
            let status = if format!("{e}").contains("Agent not found") {
                StatusCode::NOT_FOUND
            } else if format!("{e}").contains("quota") || format!("{e}").contains("Quota") {
                StatusCode::TOO_MANY_REQUESTS
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            let hint = if status == StatusCode::TOO_MANY_REQUESTS {
                Some("Budget or provider rate limit — check /api/budget and provider health.")
            } else if status == StatusCode::NOT_FOUND {
                Some("The agent may have been stopped; refresh GET /api/agents.")
            } else {
                Some(
                    "Check provider keys, model availability, and daemon logs for this request_id.",
                )
            };
            api_json_error(
                status,
                &rid,
                msg_path,
                "Message delivery failed",
                format!("{e}"),
                hint,
            )
        }
    }
}

/// GET /api/agents/:id/session — Get agent session (conversation history).
pub async fn get_agent_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/agents/:id/session";
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid agent ID",
                "Agent id must be a valid UUID.".to_string(),
                Some("Use GET /api/agents to list ids."),
            );
        }
    };

    let entry = match state.kernel.registry.get(agent_id) {
        Some(e) => e,
        None => {
            return api_json_error(
                StatusCode::NOT_FOUND,
                &rid,
                PATH,
                "Agent not found",
                format!("No agent registered for id {id}."),
                Some("Spawn an agent or pick a valid id from GET /api/agents."),
            );
        }
    };

    match state.kernel.memory.get_session(entry.session_id) {
        Ok(Some(session)) => {
            // Two-pass approach: ToolUse blocks live in Assistant messages while
            // ToolResult blocks arrive in subsequent User messages.  Pass 1
            // collects all tool_use entries keyed by id; pass 2 attaches results.

            // Pass 1: build messages and a lookup from tool_use_id → (msg_idx, tool_idx)
            use base64::Engine as _;
            let mut built_messages: Vec<serde_json::Value> = Vec::new();
            let mut tool_use_index: std::collections::HashMap<String, (usize, usize)> =
                std::collections::HashMap::new();

            for m in &session.messages {
                let mut tools: Vec<serde_json::Value> = Vec::new();
                let mut msg_images: Vec<serde_json::Value> = Vec::new();
                let content = match &m.content {
                    openfang_types::message::MessageContent::Text(t) => t.clone(),
                    openfang_types::message::MessageContent::Blocks(blocks) => {
                        let mut texts = Vec::new();
                        for b in blocks {
                            match b {
                                openfang_types::message::ContentBlock::Text { text, .. } => {
                                    texts.push(text.clone());
                                }
                                openfang_types::message::ContentBlock::Image {
                                    media_type,
                                    data,
                                } => {
                                    texts.push("[Image]".to_string());
                                    // Persist image to upload dir so it can be
                                    // served back when loading session history.
                                    let file_id = uuid::Uuid::new_v4().to_string();
                                    let upload_dir = std::env::temp_dir().join("openfang_uploads");
                                    let _ = std::fs::create_dir_all(&upload_dir);
                                    if let Ok(bytes) =
                                        base64::engine::general_purpose::STANDARD.decode(data)
                                    {
                                        let _ = std::fs::write(upload_dir.join(&file_id), &bytes);
                                        UPLOAD_REGISTRY.insert(
                                            file_id.clone(),
                                            UploadMeta {
                                                filename: format!(
                                                    "image.{}",
                                                    media_type.rsplit('/').next().unwrap_or("png")
                                                ),
                                                content_type: media_type.clone(),
                                            },
                                        );
                                        msg_images.push(serde_json::json!({
                                            "file_id": file_id,
                                            "filename": format!("image.{}", media_type.rsplit('/').next().unwrap_or("png")),
                                        }));
                                    }
                                }
                                openfang_types::message::ContentBlock::ToolUse {
                                    id,
                                    name,
                                    input,
                                    ..
                                } => {
                                    let tool_idx = tools.len();
                                    tools.push(serde_json::json!({
                                        "name": name,
                                        "input": input,
                                        "running": false,
                                        "expanded": false,
                                    }));
                                    // Will be filled after this loop when we know msg_idx
                                    tool_use_index.insert(id.clone(), (usize::MAX, tool_idx));
                                }
                                // ToolResult blocks are handled in pass 2
                                openfang_types::message::ContentBlock::ToolResult { .. } => {}
                                _ => {}
                            }
                        }
                        texts.join("\n")
                    }
                };
                // Skip messages that are purely tool results (User role with only ToolResult blocks)
                if content.is_empty() && tools.is_empty() {
                    continue;
                }
                let msg_idx = built_messages.len();
                // Fix up the msg_idx for tool_use entries registered with sentinel
                for (_, (mi, _)) in tool_use_index.iter_mut() {
                    if *mi == usize::MAX {
                        *mi = msg_idx;
                    }
                }
                let mut msg = serde_json::json!({
                    "role": format!("{:?}", m.role),
                    "content": content,
                });
                if !tools.is_empty() {
                    msg["tools"] = serde_json::Value::Array(tools);
                }
                if !msg_images.is_empty() {
                    msg["images"] = serde_json::Value::Array(msg_images);
                }
                built_messages.push(msg);
            }

            // Pass 2: walk messages again and attach ToolResult to the correct tool
            for m in &session.messages {
                if let openfang_types::message::MessageContent::Blocks(blocks) = &m.content {
                    for b in blocks {
                        if let openfang_types::message::ContentBlock::ToolResult {
                            tool_use_id,
                            content: result,
                            is_error,
                            ..
                        } = b
                        {
                            if let Some(&(msg_idx, tool_idx)) = tool_use_index.get(tool_use_id) {
                                if let Some(msg) = built_messages.get_mut(msg_idx) {
                                    if let Some(tools_arr) =
                                        msg.get_mut("tools").and_then(|v| v.as_array_mut())
                                    {
                                        if let Some(tool_obj) = tools_arr.get_mut(tool_idx) {
                                            tool_obj["result"] =
                                                serde_json::Value::String(result.clone());
                                            tool_obj["is_error"] =
                                                serde_json::Value::Bool(*is_error);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            let messages = built_messages;
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "session_id": session.id.0.to_string(),
                    "agent_id": session.agent_id.0.to_string(),
                    "message_count": session.messages.len(),
                    "context_window_tokens": session.context_window_tokens,
                    "label": session.label,
                    "messages": messages,
                })),
            )
        }
        Ok(None) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "session_id": entry.session_id.0.to_string(),
                "agent_id": agent_id.to_string(),
                "message_count": 0,
                "context_window_tokens": 0,
                "messages": [],
            })),
        ),
        Err(e) => {
            tracing::warn!("Session load failed for agent {id}: {e}");
            api_json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &rid,
                PATH,
                "Session load failed",
                format!("{e}"),
                Some("Check database health and GET /api/health/detail."),
            )
        }
    }
}

/// GET /api/agents/:id/session/digest — Small JSON for dashboard polling (unread / sync); avoids full history.
pub async fn get_agent_session_digest(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/agents/:id/session/digest";
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid agent ID",
                "Agent id must be a valid UUID.".to_string(),
                Some("Use GET /api/agents to list ids."),
            );
        }
    };

    let entry = match state.kernel.registry.get(agent_id) {
        Some(e) => e,
        None => {
            return api_json_error(
                StatusCode::NOT_FOUND,
                &rid,
                PATH,
                "Agent not found",
                format!("No agent registered for id {id}."),
                Some("Spawn an agent or pick a valid id from GET /api/agents."),
            );
        }
    };

    match state.kernel.memory.get_session(entry.session_id) {
        Ok(Some(session)) => {
            let assistant_message_count = session
                .messages
                .iter()
                .filter(|m| m.role == Role::Assistant)
                .count();
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "session_id": session.id.0.to_string(),
                    "agent_id": session.agent_id.0.to_string(),
                    "message_count": session.messages.len(),
                    "assistant_message_count": assistant_message_count,
                })),
            )
        }
        Ok(None) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "session_id": entry.session_id.0.to_string(),
                "agent_id": agent_id.to_string(),
                "message_count": 0usize,
                "assistant_message_count": 0usize,
            })),
        ),
        Err(e) => {
            tracing::warn!("Session digest failed for agent {id}: {e}");
            api_json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &rid,
                PATH,
                "Session digest failed",
                format!("{e}"),
                Some("Check database health and GET /api/health/detail."),
            )
        }
    }
}

/// DELETE /api/agents/:id — Kill an agent.
pub async fn kill_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/agents/:id";
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid agent ID",
                "Agent id must be a valid UUID.".to_string(),
                Some("Use GET /api/agents to list ids."),
            );
        }
    };

    match state.kernel.kill_agent(agent_id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "killed", "agent_id": id})),
        ),
        Err(e) => {
            tracing::warn!("kill_agent failed for {id}: {e}");
            api_json_error(
                StatusCode::NOT_FOUND,
                &rid,
                PATH,
                "Agent not found or already terminated",
                format!("{e}"),
                Some("The agent may have exited; refresh GET /api/agents."),
            )
        }
    }
}

/// POST /api/agents/{id}/restart — Restart a crashed/stuck agent.
///
/// Cancels any active task, resets agent state to Running, and updates last_active.
/// Returns the agent's new state.
pub async fn restart_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/agents/:id/restart";
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid agent ID",
                "Agent id must be a valid UUID.".to_string(),
                Some("Use GET /api/agents to list ids."),
            );
        }
    };

    // Check agent exists
    let entry = match state.kernel.registry.get(agent_id) {
        Some(e) => e,
        None => {
            return api_json_error(
                StatusCode::NOT_FOUND,
                &rid,
                PATH,
                "Agent not found",
                format!("No agent registered for id {id}."),
                Some("Spawn an agent or pick a valid id from GET /api/agents."),
            );
        }
    };

    let agent_name = entry.name.clone();
    let previous_state = format!("{:?}", entry.state);
    drop(entry);

    // Cancel any running task
    let was_running = state.kernel.stop_agent_run(agent_id).unwrap_or(false);

    // Reset state to Running (also updates last_active)
    let _ = state
        .kernel
        .registry
        .set_state(agent_id, openfang_types::agent::AgentState::Running);

    tracing::info!(
        agent = %agent_name,
        previous_state = %previous_state,
        task_cancelled = was_running,
        "Agent restarted via API"
    );

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "restarted",
            "agent": agent_name,
            "agent_id": id,
            "previous_state": previous_state,
            "task_cancelled": was_running,
        })),
    )
}

/// GET /api/status — Kernel status.
pub async fn status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let agents: Vec<serde_json::Value> = state
        .kernel
        .registry
        .list()
        .into_iter()
        .map(|e| {
            serde_json::json!({
                "id": e.id.to_string(),
                "name": e.name,
                "state": format!("{:?}", e.state),
                "mode": e.mode,
                "created_at": e.created_at.to_rfc3339(),
                "model_provider": e.manifest.model.provider,
                "model_name": e.manifest.model.model,
                "profile": e.manifest.profile,
            })
        })
        .collect();

    let uptime = state.started_at.elapsed().as_secs();
    let agent_count = agents.len();

    let mut openfang_runtime_ainl = openfang_runtime::ainl_integration_compile_flags();
    if let serde_json::Value::Object(ref mut m) = openfang_runtime_ainl {
        m.insert(
            "ainl_runtime_engine_forced_by_env".to_string(),
            serde_json::Value::Bool(
                std::env::var("AINL_RUNTIME_ENGINE").ok().as_deref() == Some("1"),
            ),
        );
        m.insert(
            "ainl_runtime_engine_env_disabled".to_string(),
            serde_json::Value::Bool(openfang_runtime::ainl_runtime_engine_env_disabled()),
        );
    }
    let eco_compression = state
        .kernel
        .memory
        .usage()
        .query_compression_summary(Some(7))
        .map(serde_json::to_value)
        .ok()
        .and_then(Result::ok)
        .unwrap_or_else(|| serde_json::json!({"window":"7d","modes":{},"agents":[]}));

    let adaptive_eco = state.kernel.adaptive_eco_config();
    let memory_context_metrics = openfang_runtime::graph_memory_context::memory_context_metrics();
    let memory_selection_debug = openfang_runtime::graph_memory_context::latest_selection_debug(20);
    let memory_contract_metrics = openfang_runtime::ainl_inbox_reader::inbox_contract_metrics();
    Json(serde_json::json!({
        "status": "running",
        "version": env!("CARGO_PKG_VERSION"),
        "agent_count": agent_count,
        "default_provider": state.kernel.config.default_model.provider,
        "default_model": state.kernel.config.default_model.model,
        "uptime_seconds": uptime,
        "api_listen": state.kernel.config.api_listen,
        "home_dir": state.kernel.config.home_dir.display().to_string(),
        "log_level": state.kernel.config.log_level,
        "network_enabled": state.kernel.config.network_enabled,
        "config_schema_version": state.kernel.config.config_schema_version,
        "config_schema_version_binary": openfang_types::config::CONFIG_SCHEMA_VERSION,
        "agents": agents,
        "openfang_runtime_ainl": openfang_runtime_ainl,
        "graph_memory_context_metrics": memory_context_metrics,
        "graph_memory_selection_debug": memory_selection_debug,
        "graph_memory_contract_metrics": memory_contract_metrics,
        "eco_compression": eco_compression,
        "adaptive_eco": {
            "enabled": adaptive_eco.enabled,
            "enforce": adaptive_eco.enforce,
            "enforce_min_consecutive_turns": adaptive_eco.enforce_min_consecutive_turns,
            "allow_aggressive_on_structured": adaptive_eco.allow_aggressive_on_structured,
            "semantic_floor": adaptive_eco.semantic_floor,
            "circuit_breaker_enabled": adaptive_eco.circuit_breaker_enabled,
            "circuit_breaker_window": adaptive_eco.circuit_breaker_window,
            "circuit_breaker_min_below_floor": adaptive_eco.circuit_breaker_min_below_floor,
        },
    }))
}

/// POST /api/shutdown — Graceful shutdown.
pub async fn shutdown(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    tracing::info!("Shutdown requested via API");
    // SECURITY: Record shutdown in audit trail
    state.kernel.audit_log.record(
        "system",
        openfang_runtime::audit::AuditAction::ConfigChange,
        "shutdown requested via API",
        "ok",
    );
    state.kernel.shutdown();
    // Signal the HTTP server to initiate graceful shutdown so the process exits.
    state.shutdown_notify.notify_one();
    Json(serde_json::json!({"status": "shutting_down"}))
}

// ---------------------------------------------------------------------------
// Workflow routes
// ---------------------------------------------------------------------------

fn parse_workflow_collect_aggregation(
    step: &serde_json::Value,
) -> Result<Option<AggregationStrategy>, String> {
    match step.get("collect_aggregation") {
        None => Ok(None),
        Some(v) if v.is_null() => Ok(None),
        Some(v) => serde_json::from_value(v.clone())
            .map(Some)
            .map_err(|e| e.to_string()),
    }
}

/// POST /api/workflows — Register a new workflow.
pub async fn create_workflow(
    State(state): State<Arc<AppState>>,
    ext: Option<Extension<RequestId>>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/workflows";
    let name = req["name"].as_str().unwrap_or("unnamed").to_string();
    let description = req["description"].as_str().unwrap_or("").to_string();

    let steps_json = match req["steps"].as_array() {
        Some(s) => s,
        None => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Missing workflow steps",
                "JSON body must include a 'steps' array.".to_string(),
                Some("Each step needs agent_id or agent_name, mode, prompt, etc."),
            );
        }
    };

    let mut steps = Vec::new();
    for s in steps_json {
        let step_name = s["name"].as_str().unwrap_or("step").to_string();
        let agent = if let Some(id) = s["agent_id"].as_str() {
            StepAgent::ById { id: id.to_string() }
        } else if let Some(name) = s["agent_name"].as_str() {
            StepAgent::ByName {
                name: name.to_string(),
            }
        } else {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid workflow step",
                format!("Step '{step_name}' needs 'agent_id' or 'agent_name'."),
                Some("Reference an existing agent by UUID or name."),
            );
        };

        let mode = match s["mode"].as_str().unwrap_or("sequential") {
            "fan_out" => StepMode::FanOut,
            "collect" => StepMode::Collect,
            "conditional" => StepMode::Conditional {
                condition: s["condition"].as_str().unwrap_or("").to_string(),
            },
            "loop" => StepMode::Loop {
                max_iterations: s["max_iterations"].as_u64().unwrap_or(5) as u32,
                until: s["until"].as_str().unwrap_or("").to_string(),
            },
            "adaptive" => StepMode::Adaptive {
                max_iterations: s["max_iterations"].as_u64().unwrap_or(20) as u32,
                tool_allowlist: s["tool_allowlist"].as_array().map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                }),
                allow_subagents: s["allow_subagents"].as_bool().unwrap_or(true),
                max_tokens: s["max_tokens"].as_u64(),
            },
            _ => StepMode::Sequential,
        };

        let error_mode = match s["error_mode"].as_str().unwrap_or("fail") {
            "skip" => ErrorMode::Skip,
            "retry" => ErrorMode::Retry {
                max_retries: s["max_retries"].as_u64().unwrap_or(3) as u32,
            },
            _ => ErrorMode::Fail,
        };

        let collect_aggregation = match parse_workflow_collect_aggregation(s) {
            Ok(v) => v,
            Err(e) => {
                return api_json_error(
                    StatusCode::BAD_REQUEST,
                    &rid,
                    PATH,
                    "Invalid collect_aggregation",
                    e,
                    Some("Use type: concatenate | json_array | consensus | best_of | summarize | custom (see docs)."),
                );
            }
        };

        steps.push(WorkflowStep {
            name: step_name,
            agent,
            prompt_template: s["prompt"].as_str().unwrap_or("{{input}}").to_string(),
            mode,
            timeout_secs: s["timeout_secs"].as_u64().unwrap_or(120),
            error_mode,
            output_var: s["output_var"].as_str().map(String::from),
            collect_aggregation,
        });
    }

    let workflow = Workflow {
        id: WorkflowId::new(),
        name,
        description,
        steps,
        created_at: chrono::Utc::now(),
    };

    let id = state.kernel.register_workflow(workflow.clone()).await;

    // Persist workflow to disk so it survives daemon restarts (#751)
    let wf_dir = state
        .kernel
        .config
        .workflows_dir
        .clone()
        .unwrap_or_else(|| state.kernel.config.home_dir.join("workflows"));
    if let Err(e) = std::fs::create_dir_all(&wf_dir) {
        tracing::warn!("Failed to create workflows dir: {e}");
    } else {
        let wf_path = wf_dir.join(format!("{}.json", id));
        if let Ok(json) = serde_json::to_string_pretty(&workflow) {
            if let Err(e) = std::fs::write(&wf_path, json) {
                tracing::warn!("Failed to persist workflow {id}: {e}");
            }
        }
    }

    (
        StatusCode::CREATED,
        Json(serde_json::json!({"workflow_id": id.to_string()})),
    )
}

/// GET /api/workflows — List all workflows.
pub async fn list_workflows(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let workflows = state.kernel.workflows.list_workflows().await;
    let list: Vec<serde_json::Value> = workflows
        .iter()
        .map(|w| {
            serde_json::json!({
                "id": w.id.to_string(),
                "name": w.name,
                "description": w.description,
                "steps": w.steps.len(),
                "created_at": w.created_at.to_rfc3339(),
            })
        })
        .collect();
    Json(list)
}

/// POST /api/workflows/:id/run — Execute a workflow.
pub async fn run_workflow(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/workflows/:id/run";
    let workflow_id = WorkflowId(match id.parse() {
        Ok(u) => u,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid workflow ID",
                "Workflow id must be a valid UUID.".to_string(),
                Some("Use the id returned by POST /api/workflows or GET /api/workflows."),
            );
        }
    });

    let input = req["input"].as_str().unwrap_or("").to_string();

    match state.kernel.run_workflow(workflow_id, input).await {
        Ok((run_id, output)) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "run_id": run_id.to_string(),
                "output": output,
                "status": "completed",
            })),
        ),
        Err(e) => {
            tracing::warn!("Workflow run failed for {id}: {e}");
            api_json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &rid,
                PATH,
                "Workflow execution failed",
                format!("{e}"),
                Some("Check agent availability, prompts, and kernel logs for this request_id."),
            )
        }
    }
}

/// GET /api/workflows/:id/runs — List runs for a workflow.
pub async fn list_workflow_runs(
    State(state): State<Arc<AppState>>,
    Path(_id): Path<String>,
) -> impl IntoResponse {
    let runs = state.kernel.workflows.list_runs(None).await;
    let list: Vec<serde_json::Value> = runs
        .iter()
        .map(|r| {
            serde_json::json!({
                "id": r.id.to_string(),
                "workflow_name": r.workflow_name,
                "state": serde_json::to_value(&r.state).unwrap_or_default(),
                "steps_completed": r.step_results.len(),
                "started_at": r.started_at.to_rfc3339(),
                "completed_at": r.completed_at.map(|t| t.to_rfc3339()),
            })
        })
        .collect();
    Json(list)
}

/// GET /api/workflows/:id — Get a single workflow by ID.
pub async fn get_workflow(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/workflows/:id";
    let workflow_id = WorkflowId(match id.parse() {
        Ok(u) => u,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid workflow ID",
                "Workflow id must be a valid UUID.".to_string(),
                Some("Use GET /api/workflows to list ids."),
            );
        }
    });

    match state.kernel.workflows.get_workflow(workflow_id).await {
        Some(w) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "id": w.id.to_string(),
                "name": w.name,
                "description": w.description,
                "steps": w.steps,
                "created_at": w.created_at.to_rfc3339(),
            })),
        ),
        None => api_json_error(
            StatusCode::NOT_FOUND,
            &rid,
            PATH,
            "Workflow not found",
            format!("No workflow registered for id {id}."),
            Some("Register a workflow via POST /api/workflows."),
        ),
    }
}

/// PUT /api/workflows/:id — Update a workflow definition.
pub async fn update_workflow(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/workflows/:id";
    let workflow_id = WorkflowId(match id.parse() {
        Ok(u) => u,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid workflow ID",
                "Workflow id must be a valid UUID.".to_string(),
                Some("Use GET /api/workflows to list ids."),
            );
        }
    });

    let name = req["name"].as_str().unwrap_or("unnamed").to_string();
    let description = req["description"].as_str().unwrap_or("").to_string();

    let steps_json = match req["steps"].as_array() {
        Some(s) => s,
        None => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Missing workflow steps",
                "JSON body must include a 'steps' array.".to_string(),
                Some("Each step needs agent_id or agent_name, mode, prompt, etc."),
            );
        }
    };

    let mut steps = Vec::new();
    for s in steps_json {
        let step_name = s["name"].as_str().unwrap_or("step").to_string();
        let agent = if let Some(id) = s["agent_id"].as_str() {
            StepAgent::ById { id: id.to_string() }
        } else if let Some(name) = s["agent_name"].as_str() {
            StepAgent::ByName {
                name: name.to_string(),
            }
        } else {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid workflow step",
                format!("Step '{step_name}' needs 'agent_id' or 'agent_name'."),
                Some("Reference an existing agent by UUID or name."),
            );
        };

        let mode = match s["mode"].as_str().unwrap_or("sequential") {
            "fan_out" => StepMode::FanOut,
            "collect" => StepMode::Collect,
            "conditional" => StepMode::Conditional {
                condition: s["condition"].as_str().unwrap_or("").to_string(),
            },
            "loop" => StepMode::Loop {
                max_iterations: s["max_iterations"].as_u64().unwrap_or(5) as u32,
                until: s["until"].as_str().unwrap_or("").to_string(),
            },
            "adaptive" => StepMode::Adaptive {
                max_iterations: s["max_iterations"].as_u64().unwrap_or(20) as u32,
                tool_allowlist: s["tool_allowlist"].as_array().map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                }),
                allow_subagents: s["allow_subagents"].as_bool().unwrap_or(true),
                max_tokens: s["max_tokens"].as_u64(),
            },
            _ => StepMode::Sequential,
        };

        let error_mode = match s["error_mode"].as_str().unwrap_or("fail") {
            "skip" => ErrorMode::Skip,
            "retry" => ErrorMode::Retry {
                max_retries: s["max_retries"].as_u64().unwrap_or(3) as u32,
            },
            _ => ErrorMode::Fail,
        };

        let collect_aggregation = match parse_workflow_collect_aggregation(s) {
            Ok(v) => v,
            Err(e) => {
                return api_json_error(
                    StatusCode::BAD_REQUEST,
                    &rid,
                    PATH,
                    "Invalid collect_aggregation",
                    e,
                    Some("Use type: concatenate | json_array | consensus | best_of | summarize | custom (see docs)."),
                );
            }
        };

        steps.push(WorkflowStep {
            name: step_name,
            agent,
            prompt_template: s["prompt"].as_str().unwrap_or("{{input}}").to_string(),
            mode,
            timeout_secs: s["timeout_secs"].as_u64().unwrap_or(120),
            error_mode,
            output_var: s["output_var"].as_str().map(String::from),
            collect_aggregation,
        });
    }

    let updated = Workflow {
        id: workflow_id,
        name,
        description,
        steps,
        created_at: chrono::Utc::now(), // preserved by engine
    };

    if state
        .kernel
        .workflows
        .update_workflow(workflow_id, updated)
        .await
    {
        (
            StatusCode::OK,
            Json(serde_json::json!({"status": "updated", "workflow_id": id})),
        )
    } else {
        api_json_error(
            StatusCode::NOT_FOUND,
            &rid,
            PATH,
            "Workflow not found",
            format!("No workflow registered for id {id}."),
            Some("Register a workflow via POST /api/workflows."),
        )
    }
}

/// DELETE /api/workflows/:id — Delete a workflow definition.
pub async fn delete_workflow(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/workflows/:id";
    let workflow_id = WorkflowId(match id.parse() {
        Ok(u) => u,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid workflow ID",
                "Workflow id must be a valid UUID.".to_string(),
                Some("Use GET /api/workflows to list ids."),
            );
        }
    });

    if state.kernel.workflows.remove_workflow(workflow_id).await {
        (
            StatusCode::OK,
            Json(serde_json::json!({"status": "removed", "workflow_id": id})),
        )
    } else {
        api_json_error(
            StatusCode::NOT_FOUND,
            &rid,
            PATH,
            "Workflow not found",
            format!("No workflow registered for id {id}."),
            Some("Register a workflow via POST /api/workflows."),
        )
    }
}

// ---------------------------------------------------------------------------
// Trigger routes
// ---------------------------------------------------------------------------

/// POST /api/triggers — Register a new event trigger.
pub async fn create_trigger(
    State(state): State<Arc<AppState>>,
    ext: Option<Extension<RequestId>>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/triggers";
    let agent_id_str = match req["agent_id"].as_str() {
        Some(id) => id,
        None => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Missing agent_id",
                "JSON body must include 'agent_id' (UUID).".to_string(),
                Some("Use an agent id from GET /api/agents."),
            );
        }
    };

    let agent_id: AgentId = match agent_id_str.parse() {
        Ok(id) => id,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid agent_id",
                "agent_id must be a valid UUID.".to_string(),
                Some("Use GET /api/agents to list valid ids."),
            );
        }
    };

    let pattern: TriggerPattern = match req.get("pattern") {
        Some(p) => match serde_json::from_value(p.clone()) {
            Ok(pat) => pat,
            Err(e) => {
                tracing::warn!("Invalid trigger pattern: {e}");
                return api_json_error(
                    StatusCode::BAD_REQUEST,
                    &rid,
                    PATH,
                    "Invalid trigger pattern",
                    format!("Could not parse pattern: {e}"),
                    Some("See TriggerPattern schema in the API docs / kernel types."),
                );
            }
        },
        None => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Missing pattern",
                "JSON body must include a 'pattern' object.".to_string(),
                Some("Define when the trigger should fire (event type, filters, etc.)."),
            );
        }
    };

    let prompt_template = req["prompt_template"]
        .as_str()
        .unwrap_or("Event: {{event}}")
        .to_string();
    let max_fires = req["max_fires"].as_u64().unwrap_or(0);

    match state
        .kernel
        .register_trigger(agent_id, pattern, prompt_template, max_fires)
    {
        Ok(trigger_id) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "trigger_id": trigger_id.to_string(),
                "agent_id": agent_id.to_string(),
            })),
        ),
        Err(e) => {
            tracing::warn!("Trigger registration failed: {e}");
            api_json_error(
                StatusCode::NOT_FOUND,
                &rid,
                PATH,
                "Trigger registration failed",
                format!("{e}"),
                Some("Verify the agent exists and is running."),
            )
        }
    }
}

/// GET /api/triggers — List all triggers (optionally filter by ?agent_id=...).
pub async fn list_triggers(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let agent_filter = params
        .get("agent_id")
        .and_then(|id| id.parse::<AgentId>().ok());

    let triggers = state.kernel.list_triggers(agent_filter);
    let list: Vec<serde_json::Value> = triggers
        .iter()
        .map(|t| {
            serde_json::json!({
                "id": t.id.to_string(),
                "agent_id": t.agent_id.to_string(),
                "pattern": serde_json::to_value(&t.pattern).unwrap_or_default(),
                "prompt_template": t.prompt_template,
                "enabled": t.enabled,
                "fire_count": t.fire_count,
                "max_fires": t.max_fires,
                "created_at": t.created_at.to_rfc3339(),
            })
        })
        .collect();
    Json(list)
}

/// DELETE /api/triggers/:id — Remove a trigger.
pub async fn delete_trigger(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/triggers/:id";
    let trigger_id = TriggerId(match id.parse() {
        Ok(u) => u,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid trigger ID",
                "Trigger id must be a valid UUID.".to_string(),
                Some("Use GET /api/triggers to list ids."),
            );
        }
    });

    if state.kernel.remove_trigger(trigger_id) {
        (
            StatusCode::OK,
            Json(serde_json::json!({"status": "removed", "trigger_id": id})),
        )
    } else {
        api_json_error(
            StatusCode::NOT_FOUND,
            &rid,
            PATH,
            "Trigger not found",
            format!("No trigger registered for id {id}."),
            Some("List triggers with GET /api/triggers."),
        )
    }
}

// ---------------------------------------------------------------------------
// Profile + Mode endpoints
// ---------------------------------------------------------------------------

/// GET /api/profiles — List all tool profiles and their tool lists.
pub async fn list_profiles() -> impl IntoResponse {
    use openfang_types::agent::ToolProfile;

    let profiles = [
        ("minimal", ToolProfile::Minimal),
        ("coding", ToolProfile::Coding),
        ("research", ToolProfile::Research),
        ("messaging", ToolProfile::Messaging),
        ("automation", ToolProfile::Automation),
        ("full", ToolProfile::Full),
    ];

    let result: Vec<serde_json::Value> = profiles
        .iter()
        .map(|(name, profile)| {
            serde_json::json!({
                "name": name,
                "tools": profile.tools(),
            })
        })
        .collect();

    Json(result)
}

/// PUT /api/agents/:id/mode — Change an agent's operational mode.
pub async fn set_agent_mode(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
    Json(body): Json<SetModeRequest>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/agents/:id/mode";
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid agent ID",
                "Agent id must be a valid UUID.".to_string(),
                Some("Use GET /api/agents to list ids."),
            );
        }
    };

    match state.kernel.registry.set_mode(agent_id, body.mode) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "updated",
                "agent_id": id,
                "mode": body.mode,
            })),
        ),
        Err(_) => api_json_error(
            StatusCode::NOT_FOUND,
            &rid,
            PATH,
            "Agent not found",
            format!("No agent registered for id {id}."),
            Some("Spawn an agent or pick a valid id from GET /api/agents."),
        ),
    }
}

// ---------------------------------------------------------------------------
// Version endpoint
// ---------------------------------------------------------------------------

/// GET /api/version — Build & version info.
pub async fn version() -> impl IntoResponse {
    Json(serde_json::json!({
        "name": "openfang",
        "version": env!("CARGO_PKG_VERSION"),
        "build_date": option_env!("BUILD_DATE").unwrap_or("dev"),
        "git_sha": option_env!("GIT_SHA").unwrap_or("unknown"),
        "rust_version": option_env!("RUSTC_VERSION").unwrap_or("unknown"),
        "platform": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
    }))
}

/// GET /api/version/github-latest — Latest ArmaraOS release on GitHub (server-side fetch).
///
/// The dashboard cannot reliably call `api.github.com` from the browser or embedded webview
/// (CORS / “Load failed”). The daemon proxies this read-only public API.
const GITHUB_ARMARAOS_LATEST: &str =
    "https://api.github.com/repos/sbhooley/armaraos/releases/latest";

pub async fn version_github_latest_release() -> impl IntoResponse {
    let client = match reqwest::Client::builder()
        .user_agent(concat!(
            "ArmaraOS/",
            env!("CARGO_PKG_VERSION"),
            " (daemon; +https://github.com/sbhooley/armaraos)"
        ))
        .timeout(std::time::Duration::from_secs(20))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({ "error": format!("HTTP client: {e}") })),
            )
                .into_response();
        }
    };

    let resp = match client
        .get(GITHUB_ARMARAOS_LATEST)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({ "error": format!("GitHub request failed: {e}") })),
            )
                .into_response();
        }
    };

    let status = resp.status();
    let body = match resp.text().await {
        Ok(t) => t,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({ "error": format!("GitHub response body: {e}") })),
            )
                .into_response();
        }
    };

    if !status.is_success() {
        let preview: String = body.chars().take(240).collect();
        return (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({
                "error": format!("GitHub returned HTTP {}", status.as_u16()),
                "detail": preview
            })),
        )
            .into_response();
    }

    let v: serde_json::Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({ "error": format!("Invalid GitHub JSON: {e}") })),
            )
                .into_response();
        }
    };

    let tag_name = v
        .get("tag_name")
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string();
    let html_url = v
        .get("html_url")
        .and_then(|t| t.as_str())
        .unwrap_or("https://github.com/sbhooley/armaraos/releases")
        .to_string();

    Json(serde_json::json!({
        "tag_name": tag_name,
        "html_url": html_url,
    }))
    .into_response()
}

// ---------------------------------------------------------------------------
// Single agent detail + SSE streaming
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
pub struct GetAgentQuery {
    /// Comma-separated response fields to omit (e.g. `manifest_toml`).
    #[serde(default)]
    pub omit: Option<String>,
}

fn get_agent_query_omits_field(omit: &Option<String>, field: &str) -> bool {
    omit.as_deref().is_some_and(|s| {
        s.split(',')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .any(|p| p == field)
    })
}

/// GET /api/agents/:id — Get a single agent's detailed info.
pub async fn get_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<GetAgentQuery>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/agents/:id";
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid agent ID",
                "Agent id must be a valid UUID.".to_string(),
                Some("Use GET /api/agents to list ids."),
            );
        }
    };

    let entry = match state.kernel.registry.get(agent_id) {
        Some(e) => e,
        None => {
            return api_json_error(
                StatusCode::NOT_FOUND,
                &rid,
                PATH,
                "Agent not found",
                format!("No agent registered for id {id}."),
                Some("Spawn an agent or pick a valid id from GET /api/agents."),
            );
        }
    };

    let turn_total = entry.turn_stats.turns_ok + entry.turn_stats.turns_err;
    let ainl_runtime_compile_flags = openfang_runtime::ainl_integration_compile_flags();
    let ainl_runtime_engine_compiled = ainl_runtime_compile_flags
        .get("ainl_runtime_engine")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let ainl_runtime_engine_forced_by_env =
        std::env::var("AINL_RUNTIME_ENGINE").ok().as_deref() == Some("1");
    let ainl_runtime_engine_env_disabled = openfang_runtime::ainl_runtime_engine_env_disabled();
    let ainl_runtime_engine_effective = ainl_runtime_engine_compiled
        && !ainl_runtime_engine_env_disabled
        && (entry.manifest.ainl_runtime_engine || ainl_runtime_engine_forced_by_env);

    let turn_error_rate: serde_json::Value = if turn_total == 0 {
        serde_json::Value::Null
    } else {
        serde_json::json!(entry.turn_stats.turns_err as f64 / turn_total as f64)
    };

    let omit_manifest_toml = get_agent_query_omits_field(&query.omit, "manifest_toml");
    let manifest_toml = (!omit_manifest_toml).then(|| {
        toml::to_string(&entry.manifest).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "Failed to serialize manifest to TOML for GET /api/agents/:id");
            String::new()
        })
    });

    let mut body = serde_json::json!({
        "id": entry.id.to_string(),
        "name": entry.name,
        "state": format!("{:?}", entry.state),
        "mode": entry.mode,
        "profile": entry.manifest.profile,
        "created_at": entry.created_at.to_rfc3339(),
        "session_id": entry.session_id.0.to_string(),
        "model": {
            "provider": entry.manifest.model.provider,
            "model": entry.manifest.model.model,
        },
        "capabilities": {
            "tools": entry.manifest.capabilities.tools,
            "network": entry.manifest.capabilities.network,
        },
        "description": entry.manifest.description,
        "tags": entry.manifest.tags,
        "system_prompt": entry.manifest.model.system_prompt,
        "identity": {
            "emoji": entry.identity.emoji,
            "avatar_url": entry.identity.avatar_url,
            "color": entry.identity.color,
            "archetype": entry.identity.archetype,
            "vibe": entry.identity.vibe,
            "greeting_style": entry.identity.greeting_style,
        },
        "tool_allowlist": entry.manifest.tool_allowlist,
        "tool_blocklist": entry.manifest.tool_blocklist,
        "ainl_runtime_engine": entry.manifest.ainl_runtime_engine,
        "ainl_runtime_engine_effective": ainl_runtime_engine_effective,
        "ainl_runtime_engine_forced_by_env": ainl_runtime_engine_forced_by_env,
        "ainl_runtime_engine_env_disabled": ainl_runtime_engine_env_disabled,
        "ainl_runtime_engine_compiled": ainl_runtime_engine_compiled,
        "skills": entry.manifest.skills,
        "skills_mode": if entry.manifest.skills.is_empty() { "all" } else { "allowlist" },
        "mcp_servers": entry.manifest.mcp_servers,
        "mcp_servers_mode": if entry.manifest.mcp_servers.is_empty() { "all" } else { "allowlist" },
        "fallback_models": entry.manifest.fallback_models,
        "max_iterations": entry.manifest.autonomous.as_ref().map(|a| a.max_iterations),
        "scheduled_ainl_host_adapter": state.kernel.scheduled_ainl_host_adapter_info(agent_id),
        "turn_stats": {
            "last_latency_ms": entry.turn_stats.last_latency_ms,
            "last_fallback_note": entry.turn_stats.last_fallback_note,
            "last_turn_at": entry.turn_stats.last_turn_at.map(|t| t.to_rfc3339()),
            "last_success_at": entry.turn_stats.last_success_at.map(|t| t.to_rfc3339()),
            "last_error_at": entry.turn_stats.last_error_at.map(|t| t.to_rfc3339()),
            "last_error_summary": entry.turn_stats.last_error_summary,
            "turns_ok": entry.turn_stats.turns_ok,
            "turns_err": entry.turn_stats.turns_err,
            "last_input_tokens": entry.turn_stats.last_input_tokens,
            "last_output_tokens": entry.turn_stats.last_output_tokens,
            "error_rate": turn_error_rate,
        },
    });
    if let Some(mt) = manifest_toml {
        if let Some(obj) = body.as_object_mut() {
            obj.insert("manifest_toml".to_string(), serde_json::Value::String(mt));
        }
    }

    (StatusCode::OK, Json(body))
}

/// POST /api/agents/:id/message/stream — SSE streaming response.
pub async fn send_message_stream(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
    Json(req): Json<MessageRequest>,
) -> axum::response::Response {
    use axum::response::sse::{Event, Sse};
    use futures::stream;
    use openfang_runtime::llm_driver::StreamEvent;

    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/agents/:id/message/stream";

    // SECURITY: Reject oversized messages to prevent OOM / LLM token abuse.
    const MAX_MESSAGE_SIZE: usize = 64 * 1024; // 64KB
    if req.message.len() > MAX_MESSAGE_SIZE {
        return api_json_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            &rid,
            PATH,
            "Message too large",
            format!("Message exceeds {MAX_MESSAGE_SIZE} bytes (64KB)."),
            Some("Shorten the prompt or split it across multiple messages."),
        )
        .into_response();
    }

    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid agent ID",
                "Agent id must be a valid UUID.".to_string(),
                Some("Use GET /api/agents to list ids."),
            )
            .into_response();
        }
    };

    if state.kernel.registry.get(agent_id).is_none() {
        return api_json_error(
            StatusCode::NOT_FOUND,
            &rid,
            PATH,
            "Agent not found",
            format!("No agent registered for id {id}."),
            Some("Spawn an agent or pick a valid id from GET /api/agents."),
        )
        .into_response();
    }

    let mut message_for_agent = req.message.clone();
    if !req.attachments.is_empty() {
        if let Some(entry) = state.kernel.registry.get(agent_id) {
            if let Some(ref ws) = entry.manifest.workspace {
                let hint = workspace_upload_hints(ws.as_path(), &req.attachments);
                if !hint.is_empty() {
                    message_for_agent.push_str(&hint);
                }
            }
        }
    }

    if message_for_agent.len() > MAX_MESSAGE_SIZE {
        return api_json_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            &rid,
            PATH,
            "Message too large",
            format!("Message exceeds {MAX_MESSAGE_SIZE} bytes after attachments."),
            Some("Shorten the prompt or split attachments across turns."),
        )
        .into_response();
    }

    let kernel_handle: Arc<dyn KernelHandle> = state.kernel.clone() as Arc<dyn KernelHandle>;
    let (rx, _handle) = match state.kernel.send_message_streaming(
        agent_id,
        &message_for_agent,
        Some(kernel_handle),
        req.sender_id,
        req.sender_name,
        None, // SSE streaming doesn't support image attachments yet
        None,
    ) {
        Ok(pair) => pair,
        Err(e) => {
            tracing::warn!("Streaming message failed for agent {id}: {e}");
            return api_json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &rid,
                PATH,
                "Streaming message failed",
                format!("{e}"),
                Some("Check agent logs and LLM provider configuration."),
            )
            .into_response();
        }
    };

    let sse_stream = stream::unfold(rx, |mut rx| async move {
        match rx.recv().await {
            Some(event) => {
                let sse_event: Result<Event, std::convert::Infallible> = Ok(match event {
                    StreamEvent::TextDelta { text } => Event::default()
                        .event("chunk")
                        .json_data(serde_json::json!({"content": text, "done": false}))
                        .unwrap_or_else(|_| Event::default().data("error")),
                    StreamEvent::ToolUseStart { name, .. } => Event::default()
                        .event("tool_use")
                        .json_data(serde_json::json!({"tool": name}))
                        .unwrap_or_else(|_| Event::default().data("error")),
                    StreamEvent::ToolUseEnd { name, input, .. } => Event::default()
                        .event("tool_result")
                        .json_data(serde_json::json!({"tool": name, "input": input}))
                        .unwrap_or_else(|_| Event::default().data("error")),
                    StreamEvent::ContentComplete { usage, .. } => Event::default()
                        .event("done")
                        .json_data(serde_json::json!({
                            "done": true,
                            "usage": {
                                "input_tokens": usage.input_tokens,
                                "output_tokens": usage.output_tokens,
                            }
                        }))
                        .unwrap_or_else(|_| Event::default().data("error")),
                    StreamEvent::PhaseChange { phase, detail } => Event::default()
                        .event("phase")
                        .json_data(serde_json::json!({
                            "phase": phase,
                            "detail": detail,
                        }))
                        .unwrap_or_else(|_| Event::default().data("error")),
                    StreamEvent::AinlRuntimeTelemetry { payload } => Event::default()
                        .event("ainl_runtime_telemetry")
                        .json_data(payload)
                        .unwrap_or_else(|_| Event::default().data("error")),
                    _ => Event::default().comment("skip"),
                });
                Some((sse_event, rx))
            }
            None => None,
        }
    });

    Sse::new(sse_stream)
        .keep_alive(axum::response::sse::KeepAlive::default())
        .into_response()
}

// ---------------------------------------------------------------------------
// Channel status endpoints — data-driven registry for all 40 adapters
// ---------------------------------------------------------------------------

/// Field type for the channel configuration form.
#[derive(Clone, Copy, PartialEq)]
enum FieldType {
    Secret,
    Text,
    Number,
    List,
}

impl FieldType {
    fn as_str(self) -> &'static str {
        match self {
            Self::Secret => "secret",
            Self::Text => "text",
            Self::Number => "number",
            Self::List => "list",
        }
    }
}

/// A single configurable field for a channel adapter.
#[derive(Clone)]
struct ChannelField {
    key: &'static str,
    label: &'static str,
    field_type: FieldType,
    env_var: Option<&'static str>,
    required: bool,
    placeholder: &'static str,
    /// If true, this field is hidden under "Show Advanced" in the UI.
    advanced: bool,
}

/// Metadata for one channel adapter.
struct ChannelMeta {
    name: &'static str,
    display_name: &'static str,
    icon: &'static str,
    description: &'static str,
    category: &'static str,
    difficulty: &'static str,
    setup_time: &'static str,
    /// One-line quick setup hint shown in the simple form view.
    quick_setup: &'static str,
    /// Setup type: "form" (default), "qr" (QR code scan + form fallback).
    setup_type: &'static str,
    fields: &'static [ChannelField],
    setup_steps: &'static [&'static str],
    config_template: &'static str,
}

const CHANNEL_REGISTRY: &[ChannelMeta] = &[
    // ── Messaging (12) ──────────────────────────────────────────────
    ChannelMeta {
        name: "telegram", display_name: "Telegram", icon: "TG",
        description: "Telegram Bot API — long-polling adapter",
        category: "messaging", difficulty: "Easy", setup_time: "~2 min",
        quick_setup: "Paste your bot token from @BotFather",
        setup_type: "form",
        fields: &[
            ChannelField { key: "bot_token_env", label: "Bot Token", field_type: FieldType::Secret, env_var: Some("TELEGRAM_BOT_TOKEN"), required: true, placeholder: "123456:ABC-DEF...", advanced: false },
            ChannelField { key: "allowed_users", label: "Allowed User IDs", field_type: FieldType::List, env_var: None, required: false, placeholder: "12345, 67890", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
            ChannelField { key: "poll_interval_secs", label: "Poll Interval (sec)", field_type: FieldType::Number, env_var: None, required: false, placeholder: "1", advanced: true },
        ],
        setup_steps: &["Open @BotFather on Telegram", "Send /newbot and follow the prompts", "Paste the token below"],
        config_template: "[channels.telegram]\nbot_token_env = \"TELEGRAM_BOT_TOKEN\"",
    },
    ChannelMeta {
        name: "discord", display_name: "Discord", icon: "DC",
        description: "Discord Gateway bot adapter",
        category: "messaging", difficulty: "Easy", setup_time: "~3 min",
        quick_setup: "Paste your bot token from the Discord Developer Portal",
        setup_type: "form",
        fields: &[
            ChannelField { key: "bot_token_env", label: "Bot Token", field_type: FieldType::Secret, env_var: Some("DISCORD_BOT_TOKEN"), required: true, placeholder: "MTIz...", advanced: false },
            ChannelField { key: "allowed_guilds", label: "Allowed Guild IDs", field_type: FieldType::List, env_var: None, required: false, placeholder: "123456789, 987654321", advanced: true },
            ChannelField { key: "allowed_users", label: "Allowed User IDs", field_type: FieldType::List, env_var: None, required: false, placeholder: "123456789, 987654321", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
            ChannelField { key: "intents", label: "Intents Bitmask", field_type: FieldType::Number, env_var: None, required: false, placeholder: "37376", advanced: true },
        ],
        setup_steps: &["Go to discord.com/developers/applications", "Create a bot and copy the token", "Paste it below"],
        config_template: "[channels.discord]\nbot_token_env = \"DISCORD_BOT_TOKEN\"",
    },
    ChannelMeta {
        name: "slack", display_name: "Slack", icon: "SL",
        description: "Slack Socket Mode + Events API",
        category: "messaging", difficulty: "Medium", setup_time: "~5 min",
        quick_setup: "Paste your App Token and Bot Token from api.slack.com",
        setup_type: "form",
        fields: &[
            ChannelField { key: "app_token_env", label: "App Token (xapp-)", field_type: FieldType::Secret, env_var: Some("SLACK_APP_TOKEN"), required: true, placeholder: "xapp-1-...", advanced: false },
            ChannelField { key: "bot_token_env", label: "Bot Token (xoxb-)", field_type: FieldType::Secret, env_var: Some("SLACK_BOT_TOKEN"), required: true, placeholder: "xoxb-...", advanced: false },
            ChannelField { key: "allowed_channels", label: "Allowed Channel IDs", field_type: FieldType::List, env_var: None, required: false, placeholder: "C01234, C56789", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
        ],
        setup_steps: &["Create app at api.slack.com/apps", "Enable Socket Mode and copy App Token", "Copy Bot Token from OAuth & Permissions"],
        config_template: "[channels.slack]\napp_token_env = \"SLACK_APP_TOKEN\"\nbot_token_env = \"SLACK_BOT_TOKEN\"",
    },
    ChannelMeta {
        name: "whatsapp", display_name: "WhatsApp", icon: "WA",
        description: "Connect your personal WhatsApp via QR scan",
        category: "messaging", difficulty: "Easy", setup_time: "~1 min",
        quick_setup: "Scan QR code with your phone — no developer account needed",
        setup_type: "qr",
        fields: &[
            // Business API fallback fields — all advanced (hidden behind "Use Business API" toggle)
            ChannelField { key: "access_token_env", label: "Access Token", field_type: FieldType::Secret, env_var: Some("WHATSAPP_ACCESS_TOKEN"), required: false, placeholder: "EAAx...", advanced: true },
            ChannelField { key: "phone_number_id", label: "Phone Number ID", field_type: FieldType::Text, env_var: None, required: false, placeholder: "1234567890", advanced: true },
            ChannelField { key: "verify_token_env", label: "Verify Token", field_type: FieldType::Secret, env_var: Some("WHATSAPP_VERIFY_TOKEN"), required: false, placeholder: "my-verify-token", advanced: true },
            ChannelField { key: "webhook_port", label: "Webhook Port", field_type: FieldType::Number, env_var: None, required: false, placeholder: "8443", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
        ],
        setup_steps: &["Open WhatsApp on your phone", "Go to Linked Devices", "Tap Link a Device and scan the QR code"],
        config_template: "[channels.whatsapp]\naccess_token_env = \"WHATSAPP_ACCESS_TOKEN\"\nphone_number_id = \"\"",
    },
    ChannelMeta {
        name: "signal", display_name: "Signal", icon: "SG",
        description: "Signal via signal-cli REST API",
        category: "messaging", difficulty: "Medium", setup_time: "~10 min",
        quick_setup: "Enter your signal-cli API URL",
        setup_type: "form",
        fields: &[
            ChannelField { key: "api_url", label: "signal-cli API URL", field_type: FieldType::Text, env_var: None, required: true, placeholder: "http://localhost:8080", advanced: false },
            ChannelField { key: "phone_number", label: "Phone Number", field_type: FieldType::Text, env_var: None, required: true, placeholder: "+1234567890", advanced: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
        ],
        setup_steps: &["Install signal-cli-rest-api", "Enter the API URL and your phone number"],
        config_template: "[channels.signal]\napi_url = \"http://localhost:8080\"\nphone_number = \"\"",
    },
    ChannelMeta {
        name: "matrix", display_name: "Matrix", icon: "MX",
        description: "Matrix/Element bot via homeserver",
        category: "messaging", difficulty: "Easy", setup_time: "~3 min",
        quick_setup: "Paste your access token and homeserver URL",
        setup_type: "form",
        fields: &[
            ChannelField { key: "access_token_env", label: "Access Token", field_type: FieldType::Secret, env_var: Some("MATRIX_ACCESS_TOKEN"), required: true, placeholder: "syt_...", advanced: false },
            ChannelField { key: "homeserver_url", label: "Homeserver URL", field_type: FieldType::Text, env_var: None, required: true, placeholder: "https://matrix.org", advanced: false },
            ChannelField { key: "user_id", label: "Bot User ID", field_type: FieldType::Text, env_var: None, required: false, placeholder: "@openfang:matrix.org", advanced: true },
            ChannelField { key: "allowed_rooms", label: "Allowed Room IDs", field_type: FieldType::List, env_var: None, required: false, placeholder: "!abc:matrix.org", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
        ],
        setup_steps: &["Create a bot account on your homeserver", "Generate an access token", "Paste token and homeserver URL below"],
        config_template: "[channels.matrix]\naccess_token_env = \"MATRIX_ACCESS_TOKEN\"\nhomeserver_url = \"https://matrix.org\"",
    },
    ChannelMeta {
        name: "email", display_name: "Email", icon: "EM",
        description: "IMAP/SMTP email adapter",
        category: "messaging", difficulty: "Easy", setup_time: "~3 min",
        quick_setup: "Enter your email, password, and server hosts",
        setup_type: "form",
        fields: &[
            ChannelField { key: "username", label: "Email Address", field_type: FieldType::Text, env_var: None, required: true, placeholder: "bot@example.com", advanced: false },
            ChannelField { key: "password_env", label: "Password / App Password", field_type: FieldType::Secret, env_var: Some("EMAIL_PASSWORD"), required: true, placeholder: "app-password", advanced: false },
            ChannelField { key: "imap_host", label: "IMAP Host", field_type: FieldType::Text, env_var: None, required: true, placeholder: "imap.gmail.com", advanced: false },
            ChannelField { key: "smtp_host", label: "SMTP Host", field_type: FieldType::Text, env_var: None, required: true, placeholder: "smtp.gmail.com", advanced: false },
            ChannelField { key: "imap_port", label: "IMAP Port", field_type: FieldType::Number, env_var: None, required: false, placeholder: "993", advanced: true },
            ChannelField { key: "smtp_port", label: "SMTP Port", field_type: FieldType::Number, env_var: None, required: false, placeholder: "587", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
        ],
        setup_steps: &["Enable IMAP on your email account", "Generate an app password if using Gmail", "Fill in email, password, and hosts below"],
        config_template: "[channels.email]\nimap_host = \"imap.gmail.com\"\nsmtp_host = \"smtp.gmail.com\"\npassword_env = \"EMAIL_PASSWORD\"",
    },
    ChannelMeta {
        name: "line", display_name: "LINE", icon: "LN",
        description: "LINE Messaging API adapter",
        category: "messaging", difficulty: "Easy", setup_time: "~3 min",
        quick_setup: "Paste your Channel Secret and Access Token",
        setup_type: "form",
        fields: &[
            ChannelField { key: "channel_secret_env", label: "Channel Secret", field_type: FieldType::Secret, env_var: Some("LINE_CHANNEL_SECRET"), required: true, placeholder: "abc123...", advanced: false },
            ChannelField { key: "access_token_env", label: "Channel Access Token", field_type: FieldType::Secret, env_var: Some("LINE_CHANNEL_ACCESS_TOKEN"), required: true, placeholder: "xyz789...", advanced: false },
            ChannelField { key: "webhook_port", label: "Webhook Port", field_type: FieldType::Number, env_var: None, required: false, placeholder: "8450", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
        ],
        setup_steps: &["Create a Messaging API channel at LINE Developers", "Copy Channel Secret and Access Token", "Paste them below"],
        config_template: "[channels.line]\nchannel_secret_env = \"LINE_CHANNEL_SECRET\"\naccess_token_env = \"LINE_CHANNEL_ACCESS_TOKEN\"",
    },
    ChannelMeta {
        name: "viber", display_name: "Viber", icon: "VB",
        description: "Viber Bot API adapter",
        category: "messaging", difficulty: "Easy", setup_time: "~2 min",
        quick_setup: "Paste your auth token from partners.viber.com",
        setup_type: "form",
        fields: &[
            ChannelField { key: "auth_token_env", label: "Auth Token", field_type: FieldType::Secret, env_var: Some("VIBER_AUTH_TOKEN"), required: true, placeholder: "4dc...", advanced: false },
            ChannelField { key: "webhook_url", label: "Webhook URL", field_type: FieldType::Text, env_var: None, required: false, placeholder: "https://your-domain.com/viber", advanced: true },
            ChannelField { key: "webhook_port", label: "Webhook Port", field_type: FieldType::Number, env_var: None, required: false, placeholder: "8451", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
        ],
        setup_steps: &["Create a bot at partners.viber.com", "Copy the auth token", "Paste it below"],
        config_template: "[channels.viber]\nauth_token_env = \"VIBER_AUTH_TOKEN\"",
    },
    ChannelMeta {
        name: "messenger", display_name: "Messenger", icon: "FB",
        description: "Facebook Messenger Platform adapter",
        category: "messaging", difficulty: "Medium", setup_time: "~10 min",
        quick_setup: "Paste your Page Access Token from developers.facebook.com",
        setup_type: "form",
        fields: &[
            ChannelField { key: "page_token_env", label: "Page Access Token", field_type: FieldType::Secret, env_var: Some("MESSENGER_PAGE_TOKEN"), required: true, placeholder: "EAAx...", advanced: false },
            ChannelField { key: "verify_token_env", label: "Verify Token", field_type: FieldType::Secret, env_var: Some("MESSENGER_VERIFY_TOKEN"), required: false, placeholder: "my-verify-token", advanced: true },
            ChannelField { key: "webhook_port", label: "Webhook Port", field_type: FieldType::Number, env_var: None, required: false, placeholder: "8452", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
        ],
        setup_steps: &["Create a Facebook App and add Messenger", "Generate a Page Access Token", "Paste it below"],
        config_template: "[channels.messenger]\npage_token_env = \"MESSENGER_PAGE_TOKEN\"",
    },
    ChannelMeta {
        name: "threema", display_name: "Threema", icon: "3M",
        description: "Threema Gateway adapter",
        category: "messaging", difficulty: "Easy", setup_time: "~3 min",
        quick_setup: "Paste your Gateway ID and API secret",
        setup_type: "form",
        fields: &[
            ChannelField { key: "secret_env", label: "API Secret", field_type: FieldType::Secret, env_var: Some("THREEMA_SECRET"), required: true, placeholder: "abc123...", advanced: false },
            ChannelField { key: "threema_id", label: "Gateway ID", field_type: FieldType::Text, env_var: None, required: true, placeholder: "*MYID01", advanced: false },
            ChannelField { key: "webhook_port", label: "Webhook Port", field_type: FieldType::Number, env_var: None, required: false, placeholder: "8454", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
        ],
        setup_steps: &["Register at gateway.threema.ch", "Copy your ID and API secret", "Paste them below"],
        config_template: "[channels.threema]\nthreema_id = \"\"\nsecret_env = \"THREEMA_SECRET\"",
    },
    ChannelMeta {
        name: "keybase", display_name: "Keybase", icon: "KB",
        description: "Keybase chat bot adapter",
        category: "messaging", difficulty: "Easy", setup_time: "~3 min",
        quick_setup: "Enter your username and paper key",
        setup_type: "form",
        fields: &[
            ChannelField { key: "username", label: "Username", field_type: FieldType::Text, env_var: None, required: true, placeholder: "openfang_bot", advanced: false },
            ChannelField { key: "paperkey_env", label: "Paper Key", field_type: FieldType::Secret, env_var: Some("KEYBASE_PAPERKEY"), required: true, placeholder: "word1 word2 word3...", advanced: false },
            ChannelField { key: "allowed_teams", label: "Allowed Teams", field_type: FieldType::List, env_var: None, required: false, placeholder: "team1, team2", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
        ],
        setup_steps: &["Create a Keybase bot account", "Generate a paper key", "Enter username and paper key below"],
        config_template: "[channels.keybase]\nusername = \"\"\npaperkey_env = \"KEYBASE_PAPERKEY\"",
    },
    // ── Social (5) ──────────────────────────────────────────────────
    ChannelMeta {
        name: "reddit", display_name: "Reddit", icon: "RD",
        description: "Reddit API bot adapter",
        category: "social", difficulty: "Medium", setup_time: "~5 min",
        quick_setup: "Paste your Client ID, Secret, and bot credentials",
        setup_type: "form",
        fields: &[
            ChannelField { key: "client_id", label: "Client ID", field_type: FieldType::Text, env_var: None, required: true, placeholder: "abc123def", advanced: false },
            ChannelField { key: "client_secret_env", label: "Client Secret", field_type: FieldType::Secret, env_var: Some("REDDIT_CLIENT_SECRET"), required: true, placeholder: "abc123...", advanced: false },
            ChannelField { key: "username", label: "Bot Username", field_type: FieldType::Text, env_var: None, required: true, placeholder: "openfang_bot", advanced: false },
            ChannelField { key: "password_env", label: "Bot Password", field_type: FieldType::Secret, env_var: Some("REDDIT_PASSWORD"), required: true, placeholder: "password", advanced: false },
            ChannelField { key: "subreddits", label: "Subreddits", field_type: FieldType::List, env_var: None, required: false, placeholder: "openfang, rust", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
        ],
        setup_steps: &["Create a Reddit app at reddit.com/prefs/apps (script type)", "Copy Client ID and Secret", "Enter bot credentials below"],
        config_template: "[channels.reddit]\nclient_id = \"\"\nclient_secret_env = \"REDDIT_CLIENT_SECRET\"\nusername = \"\"\npassword_env = \"REDDIT_PASSWORD\"",
    },
    ChannelMeta {
        name: "mastodon", display_name: "Mastodon", icon: "MA",
        description: "Mastodon Streaming API adapter",
        category: "social", difficulty: "Easy", setup_time: "~2 min",
        quick_setup: "Paste your access token from Settings > Development",
        setup_type: "form",
        fields: &[
            ChannelField { key: "access_token_env", label: "Access Token", field_type: FieldType::Secret, env_var: Some("MASTODON_ACCESS_TOKEN"), required: true, placeholder: "abc123...", advanced: false },
            ChannelField { key: "instance_url", label: "Instance URL", field_type: FieldType::Text, env_var: None, required: true, placeholder: "https://mastodon.social", advanced: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
        ],
        setup_steps: &["Go to Settings > Development on your instance", "Create an app and copy the token", "Paste it below"],
        config_template: "[channels.mastodon]\ninstance_url = \"https://mastodon.social\"\naccess_token_env = \"MASTODON_ACCESS_TOKEN\"",
    },
    ChannelMeta {
        name: "bluesky", display_name: "Bluesky", icon: "BS",
        description: "Bluesky/AT Protocol adapter",
        category: "social", difficulty: "Easy", setup_time: "~1 min",
        quick_setup: "Enter your handle and app password",
        setup_type: "form",
        fields: &[
            ChannelField { key: "identifier", label: "Handle", field_type: FieldType::Text, env_var: None, required: true, placeholder: "user.bsky.social", advanced: false },
            ChannelField { key: "app_password_env", label: "App Password", field_type: FieldType::Secret, env_var: Some("BLUESKY_APP_PASSWORD"), required: true, placeholder: "xxxx-xxxx-xxxx-xxxx", advanced: false },
            ChannelField { key: "service_url", label: "PDS URL", field_type: FieldType::Text, env_var: None, required: false, placeholder: "https://bsky.social", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
        ],
        setup_steps: &["Go to Settings > App Passwords in Bluesky", "Create an app password", "Enter handle and password below"],
        config_template: "[channels.bluesky]\nidentifier = \"\"\napp_password_env = \"BLUESKY_APP_PASSWORD\"",
    },
    ChannelMeta {
        name: "linkedin", display_name: "LinkedIn", icon: "LI",
        description: "LinkedIn Messaging API adapter",
        category: "social", difficulty: "Hard", setup_time: "~15 min",
        quick_setup: "Paste your OAuth2 access token and Organization ID",
        setup_type: "form",
        fields: &[
            ChannelField { key: "access_token_env", label: "Access Token", field_type: FieldType::Secret, env_var: Some("LINKEDIN_ACCESS_TOKEN"), required: true, placeholder: "AQV...", advanced: false },
            ChannelField { key: "organization_id", label: "Organization ID", field_type: FieldType::Text, env_var: None, required: true, placeholder: "12345678", advanced: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
        ],
        setup_steps: &["Create a LinkedIn App at linkedin.com/developers", "Generate an OAuth2 token", "Enter token and org ID below"],
        config_template: "[channels.linkedin]\naccess_token_env = \"LINKEDIN_ACCESS_TOKEN\"\norganization_id = \"\"",
    },
    ChannelMeta {
        name: "nostr", display_name: "Nostr", icon: "NS",
        description: "Nostr relay protocol adapter",
        category: "social", difficulty: "Easy", setup_time: "~2 min",
        quick_setup: "Paste your private key (nsec or hex)",
        setup_type: "form",
        fields: &[
            ChannelField { key: "private_key_env", label: "Private Key", field_type: FieldType::Secret, env_var: Some("NOSTR_PRIVATE_KEY"), required: true, placeholder: "nsec1...", advanced: false },
            ChannelField { key: "relays", label: "Relay URLs", field_type: FieldType::List, env_var: None, required: false, placeholder: "wss://relay.damus.io", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
        ],
        setup_steps: &["Generate or use an existing Nostr keypair", "Paste your private key below"],
        config_template: "[channels.nostr]\nprivate_key_env = \"NOSTR_PRIVATE_KEY\"",
    },
    // ── Enterprise (10) ─────────────────────────────────────────────
    ChannelMeta {
        name: "teams", display_name: "Microsoft Teams", icon: "MS",
        description: "Teams Bot Framework adapter",
        category: "enterprise", difficulty: "Medium", setup_time: "~10 min",
        quick_setup: "Paste your Azure Bot App ID and Password",
        setup_type: "form",
        fields: &[
            ChannelField { key: "app_id", label: "App ID", field_type: FieldType::Text, env_var: None, required: true, placeholder: "00000000-0000-...", advanced: false },
            ChannelField { key: "app_password_env", label: "App Password", field_type: FieldType::Secret, env_var: Some("TEAMS_APP_PASSWORD"), required: true, placeholder: "abc123...", advanced: false },
            ChannelField { key: "webhook_port", label: "Webhook Port", field_type: FieldType::Number, env_var: None, required: false, placeholder: "3978", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
        ],
        setup_steps: &["Create an Azure Bot registration", "Copy App ID and generate a password", "Paste them below"],
        config_template: "[channels.teams]\napp_id = \"\"\napp_password_env = \"TEAMS_APP_PASSWORD\"",
    },
    ChannelMeta {
        name: "mattermost", display_name: "Mattermost", icon: "MM",
        description: "Mattermost WebSocket adapter",
        category: "enterprise", difficulty: "Easy", setup_time: "~2 min",
        quick_setup: "Paste your bot token and server URL",
        setup_type: "form",
        fields: &[
            ChannelField { key: "server_url", label: "Server URL", field_type: FieldType::Text, env_var: None, required: true, placeholder: "https://mattermost.example.com", advanced: false },
            ChannelField { key: "token_env", label: "Bot Token", field_type: FieldType::Secret, env_var: Some("MATTERMOST_TOKEN"), required: true, placeholder: "abc123...", advanced: false },
            ChannelField { key: "allowed_channels", label: "Allowed Channels", field_type: FieldType::List, env_var: None, required: false, placeholder: "abc123, def456", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
        ],
        setup_steps: &["Create a bot in System Console > Bot Accounts", "Copy the token", "Enter server URL and token below"],
        config_template: "[channels.mattermost]\nserver_url = \"\"\ntoken_env = \"MATTERMOST_TOKEN\"",
    },
    ChannelMeta {
        name: "google_chat", display_name: "Google Chat", icon: "GC",
        description: "Google Chat service account adapter (requires Google Workspace + service account JSON key with Chat API enabled)",
        category: "enterprise", difficulty: "Hard", setup_time: "~20 min",
        quick_setup: "Create a Google Cloud project, enable the Chat API, download a service account JSON key, then paste it below",
        setup_type: "form",
        fields: &[
            ChannelField { key: "service_account_env", label: "Service Account JSON", field_type: FieldType::Secret, env_var: Some("GOOGLE_CHAT_SERVICE_ACCOUNT"), required: true, placeholder: "/path/to/key.json", advanced: false },
            ChannelField { key: "space_ids", label: "Space IDs", field_type: FieldType::List, env_var: None, required: false, placeholder: "spaces/AAAA", advanced: true },
            ChannelField { key: "webhook_port", label: "Webhook Port", field_type: FieldType::Number, env_var: None, required: false, placeholder: "8444", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
        ],
        setup_steps: &["Create a Google Cloud project with Chat API", "Download service account JSON key", "Enter the path below"],
        config_template: "[channels.google_chat]\nservice_account_env = \"GOOGLE_CHAT_SERVICE_ACCOUNT\"",
    },
    ChannelMeta {
        name: "webex", display_name: "Webex", icon: "WX",
        description: "Cisco Webex bot adapter",
        category: "enterprise", difficulty: "Easy", setup_time: "~2 min",
        quick_setup: "Paste your bot token from developer.webex.com",
        setup_type: "form",
        fields: &[
            ChannelField { key: "bot_token_env", label: "Bot Token", field_type: FieldType::Secret, env_var: Some("WEBEX_BOT_TOKEN"), required: true, placeholder: "NjI...", advanced: false },
            ChannelField { key: "allowed_rooms", label: "Allowed Rooms", field_type: FieldType::List, env_var: None, required: false, placeholder: "Y2lz...", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
        ],
        setup_steps: &["Create a bot at developer.webex.com", "Copy the token", "Paste it below"],
        config_template: "[channels.webex]\nbot_token_env = \"WEBEX_BOT_TOKEN\"",
    },
    ChannelMeta {
        name: "feishu", display_name: "Feishu/Lark", icon: "FS",
        description: "Feishu/Lark Open Platform adapter (supports China & International)",
        category: "enterprise", difficulty: "Easy", setup_time: "~3 min",
        quick_setup: "Paste your App ID and App Secret",
        setup_type: "form",
        fields: &[
            ChannelField { key: "app_id", label: "App ID", field_type: FieldType::Text, env_var: None, required: true, placeholder: "cli_abc123", advanced: false },
            ChannelField { key: "app_secret_env", label: "App Secret", field_type: FieldType::Secret, env_var: Some("FEISHU_APP_SECRET"), required: true, placeholder: "abc123...", advanced: false },
            ChannelField { key: "region", label: "Region", field_type: FieldType::Text, env_var: None, required: false, placeholder: "cn or intl", advanced: false },
            ChannelField { key: "mode", label: "Receive Mode", field_type: FieldType::Text, env_var: None, required: false, placeholder: "webhook|websocket", advanced: true },
            ChannelField { key: "webhook_port", label: "Webhook Port", field_type: FieldType::Number, env_var: None, required: false, placeholder: "8453", advanced: true },
            ChannelField { key: "webhook_path", label: "Webhook Path", field_type: FieldType::Text, env_var: None, required: false, placeholder: "/feishu/webhook", advanced: true },
            ChannelField { key: "verification_token", label: "Verification Token", field_type: FieldType::Text, env_var: None, required: false, placeholder: "verify-token", advanced: true },
            ChannelField { key: "encrypt_key_env", label: "Encrypt Key", field_type: FieldType::Secret, env_var: Some("FEISHU_ENCRYPT_KEY"), required: false, placeholder: "encrypt-key", advanced: true },
            ChannelField { key: "bot_names", label: "Bot Names", field_type: FieldType::List, env_var: None, required: false, placeholder: "MyBot, Assistant", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
        ],
        setup_steps: &["Create an app at open.feishu.cn (CN) or open.larksuite.com (International)", "Copy App ID and Secret", "Set region: cn (Feishu) or intl (Lark)"],
        config_template: "[channels.feishu]\napp_id = \"\"\napp_secret_env = \"FEISHU_APP_SECRET\"\nregion = \"cn\"\nmode = \"websocket\"",
    },
    ChannelMeta {
        name: "dingtalk", display_name: "DingTalk", icon: "DT",
        description: "DingTalk Robot API adapter",
        category: "enterprise", difficulty: "Easy", setup_time: "~3 min",
        quick_setup: "Paste your webhook token and signing secret",
        setup_type: "form",
        fields: &[
            ChannelField { key: "access_token_env", label: "Access Token", field_type: FieldType::Secret, env_var: Some("DINGTALK_ACCESS_TOKEN"), required: true, placeholder: "abc123...", advanced: false },
            ChannelField { key: "secret_env", label: "Signing Secret", field_type: FieldType::Secret, env_var: Some("DINGTALK_SECRET"), required: true, placeholder: "SEC...", advanced: false },
            ChannelField { key: "webhook_port", label: "Webhook Port", field_type: FieldType::Number, env_var: None, required: false, placeholder: "8457", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
        ],
        setup_steps: &["Create a robot in your DingTalk group", "Copy the token and signing secret", "Paste them below"],
        config_template: "[channels.dingtalk]\naccess_token_env = \"DINGTALK_ACCESS_TOKEN\"\nsecret_env = \"DINGTALK_SECRET\"",
    },
    ChannelMeta {
        name: "dingtalk_stream", display_name: "DingTalk Stream", icon: "DS",
        description: "DingTalk Stream Mode (WebSocket long-connection)",
        category: "enterprise", difficulty: "Easy", setup_time: "~5 min",
        quick_setup: "Create an Enterprise Internal App with Stream Mode enabled",
        setup_type: "form",
        fields: &[
            ChannelField { key: "app_key_env", label: "App Key", field_type: FieldType::Secret, env_var: Some("DINGTALK_APP_KEY"), required: true, placeholder: "ding...", advanced: false },
            ChannelField { key: "app_secret_env", label: "App Secret", field_type: FieldType::Secret, env_var: Some("DINGTALK_APP_SECRET"), required: true, placeholder: "uAn4...", advanced: false },
            ChannelField { key: "robot_code_env", label: "Robot Code", field_type: FieldType::Text, env_var: Some("DINGTALK_ROBOT_CODE"), required: false, placeholder: "ding... (same as App Key)", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
        ],
        setup_steps: &["Create an Enterprise Internal App in DingTalk Open Platform", "Enable Stream Mode in the app settings", "Add robot capability and configure permissions", "Copy App Key and App Secret below"],
        config_template: "[channels.dingtalk_stream]\napp_key_env = \"DINGTALK_APP_KEY\"\napp_secret_env = \"DINGTALK_APP_SECRET\"",
    },
    ChannelMeta {
        name: "pumble", display_name: "Pumble", icon: "PB",
        description: "Pumble bot adapter",
        category: "enterprise", difficulty: "Easy", setup_time: "~1 min",
        quick_setup: "Paste your bot token",
        setup_type: "form",
        fields: &[
            ChannelField { key: "bot_token_env", label: "Bot Token", field_type: FieldType::Secret, env_var: Some("PUMBLE_BOT_TOKEN"), required: true, placeholder: "abc123...", advanced: false },
            ChannelField { key: "webhook_port", label: "Webhook Port", field_type: FieldType::Number, env_var: None, required: false, placeholder: "8455", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
        ],
        setup_steps: &["Create a bot in Pumble Integrations", "Copy the token", "Paste it below"],
        config_template: "[channels.pumble]\nbot_token_env = \"PUMBLE_BOT_TOKEN\"",
    },
    ChannelMeta {
        name: "flock", display_name: "Flock", icon: "FL",
        description: "Flock bot adapter",
        category: "enterprise", difficulty: "Easy", setup_time: "~1 min",
        quick_setup: "Paste your bot token",
        setup_type: "form",
        fields: &[
            ChannelField { key: "bot_token_env", label: "Bot Token", field_type: FieldType::Secret, env_var: Some("FLOCK_BOT_TOKEN"), required: true, placeholder: "abc123...", advanced: false },
            ChannelField { key: "webhook_port", label: "Webhook Port", field_type: FieldType::Number, env_var: None, required: false, placeholder: "8456", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
        ],
        setup_steps: &["Build an app in Flock App Store", "Copy the bot token", "Paste it below"],
        config_template: "[channels.flock]\nbot_token_env = \"FLOCK_BOT_TOKEN\"",
    },
    ChannelMeta {
        name: "twist", display_name: "Twist", icon: "TW",
        description: "Twist API v3 adapter",
        category: "enterprise", difficulty: "Easy", setup_time: "~2 min",
        quick_setup: "Paste your API token and workspace ID",
        setup_type: "form",
        fields: &[
            ChannelField { key: "token_env", label: "API Token", field_type: FieldType::Secret, env_var: Some("TWIST_TOKEN"), required: true, placeholder: "abc123...", advanced: false },
            ChannelField { key: "workspace_id", label: "Workspace ID", field_type: FieldType::Text, env_var: None, required: true, placeholder: "12345", advanced: false },
            ChannelField { key: "allowed_channels", label: "Channel IDs", field_type: FieldType::List, env_var: None, required: false, placeholder: "123, 456", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
        ],
        setup_steps: &["Create an integration in Twist Settings", "Copy the API token", "Enter token and workspace ID below"],
        config_template: "[channels.twist]\ntoken_env = \"TWIST_TOKEN\"\nworkspace_id = \"\"",
    },
    ChannelMeta {
        name: "zulip", display_name: "Zulip", icon: "ZL",
        description: "Zulip event queue adapter",
        category: "enterprise", difficulty: "Easy", setup_time: "~2 min",
        quick_setup: "Paste your API key, server URL, and bot email",
        setup_type: "form",
        fields: &[
            ChannelField { key: "server_url", label: "Server URL", field_type: FieldType::Text, env_var: None, required: true, placeholder: "https://chat.zulip.org", advanced: false },
            ChannelField { key: "bot_email", label: "Bot Email", field_type: FieldType::Text, env_var: None, required: true, placeholder: "bot@zulip.example.com", advanced: false },
            ChannelField { key: "api_key_env", label: "API Key", field_type: FieldType::Secret, env_var: Some("ZULIP_API_KEY"), required: true, placeholder: "abc123...", advanced: false },
            ChannelField { key: "streams", label: "Streams", field_type: FieldType::List, env_var: None, required: false, placeholder: "general, dev", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
        ],
        setup_steps: &["Create a bot in Zulip Settings > Your Bots", "Copy the API key", "Enter server URL, bot email, and key below"],
        config_template: "[channels.zulip]\nserver_url = \"\"\nbot_email = \"\"\napi_key_env = \"ZULIP_API_KEY\"",
    },
    // ── Developer (9) ───────────────────────────────────────────────
    ChannelMeta {
        name: "irc", display_name: "IRC", icon: "IR",
        description: "IRC raw TCP adapter",
        category: "developer", difficulty: "Easy", setup_time: "~2 min",
        quick_setup: "Enter server and nickname",
        setup_type: "form",
        fields: &[
            ChannelField { key: "server", label: "Server", field_type: FieldType::Text, env_var: None, required: true, placeholder: "irc.libera.chat", advanced: false },
            ChannelField { key: "nick", label: "Nickname", field_type: FieldType::Text, env_var: None, required: true, placeholder: "openfang", advanced: false },
            ChannelField { key: "channels", label: "Channels", field_type: FieldType::List, env_var: None, required: false, placeholder: "#openfang, #general", advanced: false },
            ChannelField { key: "port", label: "Port", field_type: FieldType::Number, env_var: None, required: false, placeholder: "6667", advanced: true },
            ChannelField { key: "use_tls", label: "Use TLS", field_type: FieldType::Text, env_var: None, required: false, placeholder: "false", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
        ],
        setup_steps: &["Choose an IRC server", "Enter server, nick, and channels below"],
        config_template: "[channels.irc]\nserver = \"irc.libera.chat\"\nnick = \"openfang\"",
    },
    ChannelMeta {
        name: "xmpp", display_name: "XMPP/Jabber", icon: "XM",
        description: "XMPP/Jabber protocol adapter (coming soon — tokio-xmpp dependency not yet integrated; config is saved but connection will not start)",
        category: "developer", difficulty: "Hard", setup_time: "~10 min",
        quick_setup: "XMPP support is not yet active. Config is saved for when the integration ships.",
        setup_type: "form",
        fields: &[
            ChannelField { key: "jid", label: "JID", field_type: FieldType::Text, env_var: None, required: true, placeholder: "bot@jabber.org", advanced: false },
            ChannelField { key: "password_env", label: "Password", field_type: FieldType::Secret, env_var: Some("XMPP_PASSWORD"), required: true, placeholder: "password", advanced: false },
            ChannelField { key: "server", label: "Server", field_type: FieldType::Text, env_var: None, required: false, placeholder: "jabber.org", advanced: true },
            ChannelField { key: "port", label: "Port", field_type: FieldType::Number, env_var: None, required: false, placeholder: "5222", advanced: true },
            ChannelField { key: "rooms", label: "MUC Rooms", field_type: FieldType::List, env_var: None, required: false, placeholder: "room@conference.jabber.org", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
        ],
        setup_steps: &["Create a bot account on your XMPP server", "Enter JID and password below"],
        config_template: "[channels.xmpp]\njid = \"\"\npassword_env = \"XMPP_PASSWORD\"",
    },
    ChannelMeta {
        name: "gitter", display_name: "Gitter", icon: "GT",
        description: "Gitter Streaming API adapter",
        category: "developer", difficulty: "Easy", setup_time: "~2 min",
        quick_setup: "Paste your auth token and room ID",
        setup_type: "form",
        fields: &[
            ChannelField { key: "token_env", label: "Auth Token", field_type: FieldType::Secret, env_var: Some("GITTER_TOKEN"), required: true, placeholder: "abc123...", advanced: false },
            ChannelField { key: "room_id", label: "Room ID", field_type: FieldType::Text, env_var: None, required: true, placeholder: "abc123def456", advanced: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
        ],
        setup_steps: &["Get a token from developer.gitter.im", "Find your room ID", "Paste both below"],
        config_template: "[channels.gitter]\ntoken_env = \"GITTER_TOKEN\"\nroom_id = \"\"",
    },
    ChannelMeta {
        name: "discourse", display_name: "Discourse", icon: "DS",
        description: "Discourse forum API adapter",
        category: "developer", difficulty: "Easy", setup_time: "~2 min",
        quick_setup: "Paste your API key and forum URL",
        setup_type: "form",
        fields: &[
            ChannelField { key: "base_url", label: "Forum URL", field_type: FieldType::Text, env_var: None, required: true, placeholder: "https://forum.example.com", advanced: false },
            ChannelField { key: "api_key_env", label: "API Key", field_type: FieldType::Secret, env_var: Some("DISCOURSE_API_KEY"), required: true, placeholder: "abc123...", advanced: false },
            ChannelField { key: "api_username", label: "API Username", field_type: FieldType::Text, env_var: None, required: false, placeholder: "system", advanced: true },
            ChannelField { key: "categories", label: "Categories", field_type: FieldType::List, env_var: None, required: false, placeholder: "general, support", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
        ],
        setup_steps: &["Go to Admin > API > Keys", "Generate an API key", "Enter forum URL and key below"],
        config_template: "[channels.discourse]\nbase_url = \"\"\napi_key_env = \"DISCOURSE_API_KEY\"",
    },
    ChannelMeta {
        name: "revolt", display_name: "Revolt", icon: "RV",
        description: "Revolt bot adapter",
        category: "developer", difficulty: "Easy", setup_time: "~1 min",
        quick_setup: "Paste your bot token",
        setup_type: "form",
        fields: &[
            ChannelField { key: "bot_token_env", label: "Bot Token", field_type: FieldType::Secret, env_var: Some("REVOLT_BOT_TOKEN"), required: true, placeholder: "abc123...", advanced: false },
            ChannelField { key: "api_url", label: "API URL", field_type: FieldType::Text, env_var: None, required: false, placeholder: "https://api.revolt.chat", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
        ],
        setup_steps: &["Go to Settings > My Bots in Revolt", "Create a bot and copy the token", "Paste it below"],
        config_template: "[channels.revolt]\nbot_token_env = \"REVOLT_BOT_TOKEN\"",
    },
    ChannelMeta {
        name: "guilded", display_name: "Guilded", icon: "GD",
        description: "Guilded bot adapter",
        category: "developer", difficulty: "Easy", setup_time: "~1 min",
        quick_setup: "Paste your bot token",
        setup_type: "form",
        fields: &[
            ChannelField { key: "bot_token_env", label: "Bot Token", field_type: FieldType::Secret, env_var: Some("GUILDED_BOT_TOKEN"), required: true, placeholder: "abc123...", advanced: false },
            ChannelField { key: "server_ids", label: "Server IDs", field_type: FieldType::List, env_var: None, required: false, placeholder: "abc123", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
        ],
        setup_steps: &["Go to Server Settings > Bots in Guilded", "Create a bot and copy the token", "Paste it below"],
        config_template: "[channels.guilded]\nbot_token_env = \"GUILDED_BOT_TOKEN\"",
    },
    ChannelMeta {
        name: "nextcloud", display_name: "Nextcloud Talk", icon: "NC",
        description: "Nextcloud Talk REST adapter",
        category: "developer", difficulty: "Easy", setup_time: "~2 min",
        quick_setup: "Paste your server URL and auth token",
        setup_type: "form",
        fields: &[
            ChannelField { key: "server_url", label: "Server URL", field_type: FieldType::Text, env_var: None, required: true, placeholder: "https://cloud.example.com", advanced: false },
            ChannelField { key: "token_env", label: "Auth Token", field_type: FieldType::Secret, env_var: Some("NEXTCLOUD_TOKEN"), required: true, placeholder: "abc123...", advanced: false },
            ChannelField { key: "allowed_rooms", label: "Room Tokens", field_type: FieldType::List, env_var: None, required: false, placeholder: "abc123", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
        ],
        setup_steps: &["Create a bot user in Nextcloud", "Generate an app password", "Enter URL and token below"],
        config_template: "[channels.nextcloud]\nserver_url = \"\"\ntoken_env = \"NEXTCLOUD_TOKEN\"",
    },
    ChannelMeta {
        name: "rocketchat", display_name: "Rocket.Chat", icon: "RC",
        description: "Rocket.Chat REST adapter",
        category: "developer", difficulty: "Easy", setup_time: "~2 min",
        quick_setup: "Paste your server URL, user ID, and token",
        setup_type: "form",
        fields: &[
            ChannelField { key: "server_url", label: "Server URL", field_type: FieldType::Text, env_var: None, required: true, placeholder: "https://rocket.example.com", advanced: false },
            ChannelField { key: "user_id", label: "Bot User ID", field_type: FieldType::Text, env_var: None, required: true, placeholder: "abc123", advanced: false },
            ChannelField { key: "token_env", label: "Auth Token", field_type: FieldType::Secret, env_var: Some("ROCKETCHAT_TOKEN"), required: true, placeholder: "abc123...", advanced: false },
            ChannelField { key: "allowed_channels", label: "Channel IDs", field_type: FieldType::List, env_var: None, required: false, placeholder: "GENERAL", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
        ],
        setup_steps: &["Create a bot in Admin > Users", "Generate a personal access token", "Enter URL, user ID, and token below"],
        config_template: "[channels.rocketchat]\nserver_url = \"\"\ntoken_env = \"ROCKETCHAT_TOKEN\"\nuser_id = \"\"",
    },
    ChannelMeta {
        name: "twitch", display_name: "Twitch", icon: "TV",
        description: "Twitch IRC gateway adapter",
        category: "developer", difficulty: "Easy", setup_time: "~2 min",
        quick_setup: "Paste your OAuth token and enter channel name",
        setup_type: "form",
        fields: &[
            ChannelField { key: "oauth_token_env", label: "OAuth Token", field_type: FieldType::Secret, env_var: Some("TWITCH_OAUTH_TOKEN"), required: true, placeholder: "oauth:abc123...", advanced: false },
            ChannelField { key: "nick", label: "Bot Nickname", field_type: FieldType::Text, env_var: None, required: true, placeholder: "openfang", advanced: false },
            ChannelField { key: "channels", label: "Channels (no #)", field_type: FieldType::List, env_var: None, required: true, placeholder: "mychannel", advanced: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
        ],
        setup_steps: &["Generate an OAuth token at twitchapps.com/tmi", "Enter token, nick, and channel below"],
        config_template: "[channels.twitch]\noauth_token_env = \"TWITCH_OAUTH_TOKEN\"\nnick = \"openfang\"",
    },
    // ── Notifications (4) ───────────────────────────────────────────
    ChannelMeta {
        name: "ntfy", display_name: "ntfy", icon: "NF",
        description: "ntfy.sh pub/sub notification adapter",
        category: "notifications", difficulty: "Easy", setup_time: "~1 min",
        quick_setup: "Just enter a topic name",
        setup_type: "form",
        fields: &[
            ChannelField { key: "topic", label: "Topic", field_type: FieldType::Text, env_var: None, required: true, placeholder: "openfang-alerts", advanced: false },
            ChannelField { key: "server_url", label: "Server URL", field_type: FieldType::Text, env_var: None, required: false, placeholder: "https://ntfy.sh", advanced: true },
            ChannelField { key: "token_env", label: "Auth Token", field_type: FieldType::Secret, env_var: Some("NTFY_TOKEN"), required: false, placeholder: "tk_abc123...", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
        ],
        setup_steps: &["Pick a topic name", "Enter it below — that's it!"],
        config_template: "[channels.ntfy]\ntopic = \"\"",
    },
    ChannelMeta {
        name: "gotify", display_name: "Gotify", icon: "GF",
        description: "Gotify WebSocket notification adapter",
        category: "notifications", difficulty: "Easy", setup_time: "~2 min",
        quick_setup: "Paste your server URL and tokens",
        setup_type: "form",
        fields: &[
            ChannelField { key: "server_url", label: "Server URL", field_type: FieldType::Text, env_var: None, required: true, placeholder: "https://gotify.example.com", advanced: false },
            ChannelField { key: "app_token_env", label: "App Token (send)", field_type: FieldType::Secret, env_var: Some("GOTIFY_APP_TOKEN"), required: true, placeholder: "abc123...", advanced: false },
            ChannelField { key: "client_token_env", label: "Client Token (receive)", field_type: FieldType::Secret, env_var: Some("GOTIFY_CLIENT_TOKEN"), required: true, placeholder: "def456...", advanced: false },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
        ],
        setup_steps: &["Create an app and a client in Gotify", "Copy both tokens", "Enter URL and tokens below"],
        config_template: "[channels.gotify]\nserver_url = \"\"\napp_token_env = \"GOTIFY_APP_TOKEN\"\nclient_token_env = \"GOTIFY_CLIENT_TOKEN\"",
    },
    ChannelMeta {
        name: "webhook", display_name: "Webhook", icon: "WH",
        description: "Generic HMAC-signed webhook adapter",
        category: "notifications", difficulty: "Easy", setup_time: "~1 min",
        quick_setup: "Optionally set an HMAC secret",
        setup_type: "form",
        fields: &[
            ChannelField { key: "secret_env", label: "HMAC Secret", field_type: FieldType::Secret, env_var: Some("WEBHOOK_SECRET"), required: false, placeholder: "my-secret", advanced: false },
            ChannelField { key: "listen_port", label: "Listen Port", field_type: FieldType::Number, env_var: None, required: false, placeholder: "8460", advanced: true },
            ChannelField { key: "callback_url", label: "Callback URL", field_type: FieldType::Text, env_var: None, required: false, placeholder: "https://example.com/webhook", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
        ],
        setup_steps: &["Enter an HMAC secret (or leave blank)", "Click Save — that's it!"],
        config_template: "[channels.webhook]\nsecret_env = \"WEBHOOK_SECRET\"",
    },
    ChannelMeta {
        name: "mumble", display_name: "Mumble", icon: "MB",
        description: "Mumble text chat adapter",
        category: "notifications", difficulty: "Easy", setup_time: "~2 min",
        quick_setup: "Enter server host and username",
        setup_type: "form",
        fields: &[
            ChannelField { key: "host", label: "Host", field_type: FieldType::Text, env_var: None, required: true, placeholder: "mumble.example.com", advanced: false },
            ChannelField { key: "username", label: "Username", field_type: FieldType::Text, env_var: None, required: true, placeholder: "openfang", advanced: false },
            ChannelField { key: "password_env", label: "Server Password", field_type: FieldType::Secret, env_var: Some("MUMBLE_PASSWORD"), required: false, placeholder: "password", advanced: true },
            ChannelField { key: "port", label: "Port", field_type: FieldType::Number, env_var: None, required: false, placeholder: "64738", advanced: true },
            ChannelField { key: "channel", label: "Channel", field_type: FieldType::Text, env_var: None, required: false, placeholder: "Root", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
        ],
        setup_steps: &["Enter host and username below", "Optionally add a password"],
        config_template: "[channels.mumble]\nhost = \"\"\nusername = \"openfang\"",
    },
    ChannelMeta {
        name: "wecom", display_name: "WeCom", icon: "WC",
        description: "WeCom (WeChat Work) adapter",
        category: "messaging", difficulty: "Easy", setup_time: "~3 min",
        quick_setup: "Enter your Corp ID, Agent ID, and Secret",
        setup_type: "form",
        fields: &[
            ChannelField { key: "corp_id", label: "Corp ID", field_type: FieldType::Text, env_var: None, required: true, placeholder: "wwxxxxx", advanced: false },
            ChannelField { key: "agent_id", label: "Agent ID", field_type: FieldType::Text, env_var: None, required: true, placeholder: "wwxxxxx", advanced: false },
            ChannelField { key: "secret_env", label: "Secret", field_type: FieldType::Secret, env_var: Some("WECOM_SECRET"), required: true, placeholder: "secret", advanced: false },
            ChannelField { key: "token", label: "Callback Token", field_type: FieldType::Text, env_var: None, required: false, placeholder: "callback_token", advanced: true },
            ChannelField { key: "encoding_aes_key", label: "Encoding AES Key", field_type: FieldType::Text, env_var: None, required: false, placeholder: "encoding_aes_key", advanced: true },
            ChannelField { key: "webhook_port", label: "Webhook Port", field_type: FieldType::Number, env_var: None, required: false, placeholder: "8454", advanced: true },
            ChannelField { key: "default_agent", label: "Default Agent", field_type: FieldType::Text, env_var: None, required: false, placeholder: "assistant", advanced: true },
        ],
        setup_steps: &["Create a WeCom application at work.weixin.qq.com", "Get Corp ID, Agent ID, and Secret", "Configure callback URL to your webhook endpoint"],
        config_template: "[channels.wecom]\ncorp_id = \"\"\nagent_id = \"\"\nsecret_env = \"WECOM_SECRET\"",
    },
];

/// Check if a channel is configured (has a `[channels.xxx]` section in config).
fn is_channel_configured(config: &openfang_types::config::ChannelsConfig, name: &str) -> bool {
    match name {
        "telegram" => config.telegram.is_some(),
        "discord" => config.discord.is_some(),
        "slack" => config.slack.is_some(),
        "whatsapp" => config.whatsapp.is_some(),
        "signal" => config.signal.is_some(),
        "matrix" => config.matrix.is_some(),
        "email" => config.email.is_some(),
        "line" => config.line.is_some(),
        "viber" => config.viber.is_some(),
        "messenger" => config.messenger.is_some(),
        "threema" => config.threema.is_some(),
        "keybase" => config.keybase.is_some(),
        "reddit" => config.reddit.is_some(),
        "mastodon" => config.mastodon.is_some(),
        "bluesky" => config.bluesky.is_some(),
        "linkedin" => config.linkedin.is_some(),
        "nostr" => config.nostr.is_some(),
        "teams" => config.teams.is_some(),
        "mattermost" => config.mattermost.is_some(),
        "google_chat" => config.google_chat.is_some(),
        "webex" => config.webex.is_some(),
        "feishu" => config.feishu.is_some(),
        "dingtalk" => config.dingtalk.is_some(),
        "dingtalk_stream" => config.dingtalk_stream.is_some(),
        "pumble" => config.pumble.is_some(),
        "flock" => config.flock.is_some(),
        "twist" => config.twist.is_some(),
        "zulip" => config.zulip.is_some(),
        "irc" => config.irc.is_some(),
        "xmpp" => config.xmpp.is_some(),
        "gitter" => config.gitter.is_some(),
        "discourse" => config.discourse.is_some(),
        "revolt" => config.revolt.is_some(),
        "guilded" => config.guilded.is_some(),
        "nextcloud" => config.nextcloud.is_some(),
        "rocketchat" => config.rocketchat.is_some(),
        "twitch" => config.twitch.is_some(),
        "ntfy" => config.ntfy.is_some(),
        "gotify" => config.gotify.is_some(),
        "webhook" => config.webhook.is_some(),
        "mumble" => config.mumble.is_some(),
        "wecom" => config.wecom.is_some(),
        _ => false,
    }
}

/// Build a JSON field descriptor, checking env var presence but never exposing secrets.
/// For non-secret fields, includes the actual config value from `config_values` if available.
fn build_field_json(
    f: &ChannelField,
    config_values: Option<&serde_json::Value>,
) -> serde_json::Value {
    let has_value = f
        .env_var
        .map(|ev| std::env::var(ev).map(|v| !v.is_empty()).unwrap_or(false))
        .unwrap_or(false);
    let mut field = serde_json::json!({
        "key": f.key,
        "label": f.label,
        "type": f.field_type.as_str(),
        "env_var": f.env_var,
        "required": f.required,
        "has_value": has_value,
        "placeholder": f.placeholder,
        "advanced": f.advanced,
    });
    // For non-secret fields, include the actual saved config value so the
    // dashboard can pre-populate forms when editing existing configs.
    if f.env_var.is_none() {
        if let Some(obj) = config_values.and_then(|v| v.as_object()) {
            if let Some(val) = obj.get(f.key) {
                // Convert arrays to comma-separated string for list fields
                let display_val = if f.field_type == FieldType::List {
                    if let Some(arr) = val.as_array() {
                        serde_json::Value::String(
                            arr.iter()
                                .filter_map(|v| {
                                    v.as_str()
                                        .map(|s| s.to_string())
                                        .or_else(|| Some(v.to_string()))
                                })
                                .collect::<Vec<_>>()
                                .join(", "),
                        )
                    } else {
                        val.clone()
                    }
                } else {
                    val.clone()
                };
                field["value"] = display_val;
                if !val.is_null() && val.as_str().map(|s| !s.is_empty()).unwrap_or(true) {
                    field["has_value"] = serde_json::Value::Bool(true);
                }
            }
        }
    }
    field
}

/// Find a channel definition by name.
fn find_channel_meta(name: &str) -> Option<&'static ChannelMeta> {
    CHANNEL_REGISTRY.iter().find(|c| c.name == name)
}

#[cfg(test)]
mod channel_meta_tests {
    use super::*;

    #[test]
    fn feishu_channel_meta_includes_mode_field() {
        let meta = find_channel_meta("feishu").expect("feishu channel meta should exist");
        assert!(meta.fields.iter().any(|f| f.key == "mode"));
    }

    #[test]
    fn feishu_channel_meta_template_includes_websocket_mode_default() {
        let meta = find_channel_meta("feishu").expect("feishu channel meta should exist");
        assert!(meta.config_template.contains("mode = \"websocket\""));
    }
}

/// Serialize a channel's config to a JSON Value for pre-populating dashboard forms.
fn channel_config_values(
    config: &openfang_types::config::ChannelsConfig,
    name: &str,
) -> Option<serde_json::Value> {
    match name {
        "telegram" => config
            .telegram
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "discord" => config
            .discord
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "slack" => config
            .slack
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "whatsapp" => config
            .whatsapp
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "signal" => config
            .signal
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "matrix" => config
            .matrix
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "email" => config
            .email
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "teams" => config
            .teams
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "mattermost" => config
            .mattermost
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "irc" => config
            .irc
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "google_chat" => config
            .google_chat
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "twitch" => config
            .twitch
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "rocketchat" => config
            .rocketchat
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "zulip" => config
            .zulip
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "xmpp" => config
            .xmpp
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "line" => config
            .line
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "viber" => config
            .viber
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "messenger" => config
            .messenger
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "reddit" => config
            .reddit
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "mastodon" => config
            .mastodon
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "bluesky" => config
            .bluesky
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "feishu" => config
            .feishu
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "revolt" => config
            .revolt
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "nextcloud" => config
            .nextcloud
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "guilded" => config
            .guilded
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "keybase" => config
            .keybase
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "threema" => config
            .threema
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "nostr" => config
            .nostr
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "webex" => config
            .webex
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "pumble" => config
            .pumble
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "flock" => config
            .flock
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "twist" => config
            .twist
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "mumble" => config
            .mumble
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "dingtalk" => config
            .dingtalk
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "dingtalk_stream" => config
            .dingtalk_stream
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "discourse" => config
            .discourse
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "gitter" => config
            .gitter
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "ntfy" => config
            .ntfy
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "gotify" => config
            .gotify
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "webhook" => config
            .webhook
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "linkedin" => config
            .linkedin
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        "wecom" => config
            .wecom
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        _ => None,
    }
}

/// GET /api/channels — List all 40 channel adapters with status and field metadata.
pub async fn list_channels(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // Read the live channels config (updated on every hot-reload) instead of the
    // stale boot-time kernel.config, so newly configured channels show correctly.
    let live_channels = state.channels_config.read().await;
    let mut channels = Vec::new();
    let mut configured_count = 0u32;

    for meta in CHANNEL_REGISTRY {
        let configured = is_channel_configured(&live_channels, meta.name);
        if configured {
            configured_count += 1;
        }

        // Check if all required secret env vars are set
        let has_token = meta
            .fields
            .iter()
            .filter(|f| f.required && f.env_var.is_some())
            .all(|f| {
                f.env_var
                    .map(|ev| std::env::var(ev).map(|v| !v.is_empty()).unwrap_or(false))
                    .unwrap_or(true)
            });

        let config_vals = channel_config_values(&live_channels, meta.name);
        let fields: Vec<serde_json::Value> = meta
            .fields
            .iter()
            .map(|f| build_field_json(f, config_vals.as_ref()))
            .collect();

        channels.push(serde_json::json!({
            "name": meta.name,
            "display_name": meta.display_name,
            "icon": meta.icon,
            "description": meta.description,
            "category": meta.category,
            "difficulty": meta.difficulty,
            "setup_time": meta.setup_time,
            "quick_setup": meta.quick_setup,
            "setup_type": meta.setup_type,
            "configured": configured,
            "has_token": has_token,
            "fields": fields,
            "setup_steps": meta.setup_steps,
            "config_template": meta.config_template,
        }));
    }

    Json(serde_json::json!({
        "channels": channels,
        "total": channels.len(),
        "configured_count": configured_count,
    }))
}

/// POST /api/channels/{name}/configure — Save channel secrets + config fields.
pub async fn configure_channel(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    ext: Option<Extension<RequestId>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/channels/:name/configure";
    let meta = match find_channel_meta(&name) {
        Some(m) => m,
        None => {
            return api_json_error(
                StatusCode::NOT_FOUND,
                &rid,
                PATH,
                "Unknown channel",
                format!("No channel definition named '{name}'."),
                Some("Use GET /api/channels to list supported channels."),
            )
        }
    };

    let fields = match body.get("fields").and_then(|v| v.as_object()) {
        Some(f) => f,
        None => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Missing fields",
                "JSON body must include a 'fields' object.".to_string(),
                Some("Map field keys to string values (tokens, webhooks, etc.)."),
            )
        }
    };

    let home = openfang_kernel::config::openfang_home();
    let secrets_path = home.join("secrets.env");
    let config_path = home.join("config.toml");
    let mut config_fields: HashMap<String, (String, FieldType)> = HashMap::new();

    for field_def in meta.fields {
        let value = fields
            .get(field_def.key)
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if value.is_empty() {
            continue;
        }

        if let Some(env_var) = field_def.env_var {
            // Secret field — write to secrets.env and set in process
            if let Err(e) = write_secret_env(&secrets_path, env_var, value) {
                return api_json_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &rid,
                    PATH,
                    "Failed to write secret",
                    format!("{e}"),
                    Some("Check filesystem permissions on secrets.env and disk space."),
                );
            }
            // SAFETY: We are the only writer; this is a single-threaded config operation
            unsafe {
                std::env::set_var(env_var, value);
            }
            // Also write the env var NAME to config.toml so the channel section
            // is not empty and the kernel knows which env var to read.
            config_fields.insert(
                field_def.key.to_string(),
                (env_var.to_string(), FieldType::Text),
            );
        } else {
            // Config field — collect for TOML write with type info
            config_fields.insert(
                field_def.key.to_string(),
                (value.to_string(), field_def.field_type),
            );
        }
    }

    // Write config.toml section
    if let Err(e) = upsert_channel_config(&config_path, &name, &config_fields) {
        return api_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &rid,
            PATH,
            "Failed to write config",
            format!("{e}"),
            Some("Check config.toml permissions and TOML syntax."),
        );
    }

    // Hot-reload: activate the channel immediately
    match crate::channel_bridge::reload_channels_from_disk(&state).await {
        Ok((started, ch_errors)) => {
            let activated = started.iter().any(|s| s.eq_ignore_ascii_case(&name));
            let note = if activated {
                format!("{} activated successfully.", name)
            } else {
                // Surface the specific error from this channel's startup attempt
                // (e.g. "Telegram getMe failed: Unauthorized — Check that the bot token is correct")
                let ch_error = ch_errors
                    .iter()
                    .find(|(n, _)| n.eq_ignore_ascii_case(&name))
                    .map(|(_, e)| e.as_str())
                    .unwrap_or("check credentials and restart if the issue persists");
                format!("Channel configured but could not activate: {ch_error}")
            };
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "configured",
                    "channel": name,
                    "activated": activated,
                    "started_channels": started,
                    "note": note,
                })),
            )
        }
        Err(e) => {
            tracing::warn!(error = %e, "Channel hot-reload failed after configure");
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "configured",
                    "channel": name,
                    "activated": false,
                    "note": format!("Configured, but hot-reload failed: {e}. Restart daemon to activate.")
                })),
            )
        }
    }
}

/// DELETE /api/channels/{name}/configure — Remove channel secrets + config section.
pub async fn remove_channel(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/channels/:name/configure";
    let meta = match find_channel_meta(&name) {
        Some(m) => m,
        None => {
            return api_json_error(
                StatusCode::NOT_FOUND,
                &rid,
                PATH,
                "Unknown channel",
                format!("No channel definition named '{name}'."),
                Some("Use GET /api/channels to list supported channels."),
            )
        }
    };

    let home = openfang_kernel::config::openfang_home();
    let secrets_path = home.join("secrets.env");
    let config_path = home.join("config.toml");

    // Remove all secret env vars for this channel
    for field_def in meta.fields {
        if let Some(env_var) = field_def.env_var {
            let _ = remove_secret_env(&secrets_path, env_var);
            // SAFETY: Single-threaded config operation
            unsafe {
                std::env::remove_var(env_var);
            }
        }
    }

    // Remove config section
    if let Err(e) = remove_channel_config(&config_path, &name) {
        return api_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &rid,
            PATH,
            "Failed to remove config",
            format!("{e}"),
            Some("Check config.toml permissions."),
        );
    }

    // Hot-reload: deactivate the channel immediately
    match crate::channel_bridge::reload_channels_from_disk(&state).await {
        Ok((started, _)) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "removed",
                "channel": name,
                "remaining_channels": started,
                "note": format!("{} deactivated.", name)
            })),
        ),
        Err(e) => {
            tracing::warn!(error = %e, "Channel hot-reload failed after remove");
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "removed",
                    "channel": name,
                    "note": format!("Removed, but hot-reload failed: {e}. Restart daemon to fully deactivate.")
                })),
            )
        }
    }
}

/// POST /api/channels/{name}/test — Connectivity check + optional live test message.
///
/// Accepts an optional JSON body with `channel_id` (for Discord/Slack) or `chat_id`
/// (for Telegram). When provided, sends a real test message to verify the bot can
/// post to that channel.
pub async fn test_channel(
    Path(name): Path<String>,
    raw_body: axum::body::Bytes,
) -> impl IntoResponse {
    let meta = match find_channel_meta(&name) {
        Some(m) => m,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"status": "error", "message": "Unknown channel"})),
            )
        }
    };

    // Check all required env vars are set
    let mut missing = Vec::new();
    for field_def in meta.fields {
        if field_def.required {
            if let Some(env_var) = field_def.env_var {
                if std::env::var(env_var).map(|v| v.is_empty()).unwrap_or(true) {
                    missing.push(env_var);
                }
            }
        }
    }

    if !missing.is_empty() {
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "error",
                "message": format!("Missing required env vars: {}", missing.join(", "))
            })),
        );
    }

    // If a target channel/chat ID is provided, send a real test message
    let body: serde_json::Value = if raw_body.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&raw_body).unwrap_or(serde_json::Value::Null)
    };
    let target = body
        .get("channel_id")
        .or_else(|| body.get("chat_id"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    if let Some(target_id) = target {
        match send_channel_test_message(&name, &target_id).await {
            Ok(()) => {
                return (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "status": "ok",
                        "message": format!("Test message sent to {} channel {}.", meta.display_name, target_id)
                    })),
                );
            }
            Err(e) => {
                return (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "status": "error",
                        "message": format!("Credentials valid but failed to send test message: {e}")
                    })),
                );
            }
        }
    }

    // No target given — for Telegram, go ahead and call getMe to validate the token.
    // For other channels an env-var-exists check is the best we can do without a target.
    if name == "telegram" {
        let token = std::env::var("TELEGRAM_BOT_TOKEN").unwrap_or_default();
        if token.is_empty() {
            return (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "error",
                    "message": "TELEGRAM_BOT_TOKEN is not set. Save your bot token first."
                })),
            );
        }
        let url = format!("https://api.telegram.org/bot{token}/getMe");
        match reqwest::Client::new()
            .get(&url)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
        {
            Err(e) => {
                return (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "status": "error",
                        "message": format!("Could not reach Telegram API: {e}. Check your network or firewall.")
                    })),
                );
            }
            Ok(resp) => {
                let json: serde_json::Value = resp.json().await.unwrap_or_default();
                if json["ok"].as_bool() == Some(true) {
                    let bot_username = json["result"]["username"].as_str().unwrap_or("unknown");
                    return (
                        StatusCode::OK,
                        Json(serde_json::json!({
                            "status": "ok",
                            "message": format!("Bot token valid — connected as @{}. Enter your chat_id above to send a test message.", bot_username)
                        })),
                    );
                } else {
                    let desc = json["description"].as_str().unwrap_or("unknown error");
                    return (
                        StatusCode::OK,
                        Json(serde_json::json!({
                            "status": "error",
                            "message": format!("Telegram rejected the bot token: {desc}. Get a valid token from @BotFather.")
                        })),
                    );
                }
            }
        }
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "message": format!("All required credentials for {} are set. Provide channel_id or chat_id to send a test message.", meta.display_name)
        })),
    )
}

/// Send a real test message to a specific channel/chat on the given platform.
async fn send_channel_test_message(channel_name: &str, target_id: &str) -> Result<(), String> {
    let client = reqwest::Client::new();
    let test_msg = "ArmaraOS test message — your channel is connected!";

    match channel_name {
        "discord" => {
            let token = std::env::var("DISCORD_BOT_TOKEN")
                .map_err(|_| "DISCORD_BOT_TOKEN not set".to_string())?;
            let url = format!("https://discord.com/api/v10/channels/{target_id}/messages");
            let resp = client
                .post(&url)
                .header("Authorization", format!("Bot {token}"))
                .json(&serde_json::json!({ "content": test_msg }))
                .send()
                .await
                .map_err(|e| format!("HTTP request failed: {e}"))?;
            if !resp.status().is_success() {
                let body = resp.text().await.unwrap_or_default();
                return Err(format!("Discord API error: {body}"));
            }
        }
        "telegram" => {
            let token = std::env::var("TELEGRAM_BOT_TOKEN")
                .map_err(|_| "TELEGRAM_BOT_TOKEN not set".to_string())?;
            let url = format!("https://api.telegram.org/bot{token}/sendMessage");
            let resp = client
                .post(&url)
                .json(&serde_json::json!({ "chat_id": target_id, "text": test_msg }))
                .send()
                .await
                .map_err(|e| format!("HTTP request failed: {e}"))?;
            if !resp.status().is_success() {
                let body = resp.text().await.unwrap_or_default();
                return Err(format!("Telegram API error: {body}"));
            }
        }
        "slack" => {
            let token = std::env::var("SLACK_BOT_TOKEN")
                .map_err(|_| "SLACK_BOT_TOKEN not set".to_string())?;
            let url = "https://slack.com/api/chat.postMessage";
            let resp = client
                .post(url)
                .header("Authorization", format!("Bearer {token}"))
                .json(&serde_json::json!({ "channel": target_id, "text": test_msg }))
                .send()
                .await
                .map_err(|e| format!("HTTP request failed: {e}"))?;
            if !resp.status().is_success() {
                let body = resp.text().await.unwrap_or_default();
                return Err(format!("Slack API error: {body}"));
            }
        }
        _ => {
            return Err(format!(
                "Live test messaging not supported for {channel_name}. Credentials are valid."
            ));
        }
    }
    Ok(())
}

/// POST /api/channels/reload — Manually trigger a channel hot-reload from disk config.
pub async fn reload_channels(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match crate::channel_bridge::reload_channels_from_disk(&state).await {
        Ok((started, ch_errors)) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "ok",
                "started": started,
                "errors": ch_errors.into_iter().map(|(n, e)| serde_json::json!({"channel": n, "error": e})).collect::<Vec<_>>(),
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "status": "error",
                "error": e,
            })),
        ),
    }
}

// ---------------------------------------------------------------------------
// WhatsApp QR login flow (OpenClaw-style)
// ---------------------------------------------------------------------------

/// POST /api/channels/whatsapp/qr/start — Start a WhatsApp Web QR login session.
///
/// If a WhatsApp Web gateway is available (e.g. a Baileys-based bridge process),
/// this proxies the request and returns a base64 QR code data URL. If no gateway
/// is running, it returns instructions to set one up.
pub async fn whatsapp_qr_start() -> impl IntoResponse {
    // Check for WhatsApp Web gateway URL in config or env
    let gateway_url = std::env::var("WHATSAPP_WEB_GATEWAY_URL").unwrap_or_default();

    if gateway_url.is_empty() {
        return Json(serde_json::json!({
            "available": false,
            "message": "WhatsApp Web gateway not running. Start the gateway or use Business API mode.",
            "help": "The WhatsApp Web gateway auto-starts with the daemon when configured. Ensure Node.js >= 18 is installed and WhatsApp is configured in config.toml. Set WHATSAPP_WEB_GATEWAY_URL to use an external gateway."
        }));
    }

    // Try to reach the gateway and start a QR session.
    // Uses a raw HTTP request via tokio TcpStream to avoid adding reqwest as a runtime dep.
    let start_url = format!("{}/login/start", gateway_url.trim_end_matches('/'));
    match gateway_http_post(&start_url).await {
        Ok(body) => {
            let qr_url = body
                .get("qr_data_url")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            let sid = body
                .get("session_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            let msg = body
                .get("message")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("Scan this QR code with WhatsApp → Linked Devices");
            let connected = body
                .get("connected")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            Json(serde_json::json!({
                "available": true,
                "qr_data_url": qr_url,
                "session_id": sid,
                "message": msg,
                "connected": connected,
            }))
        }
        Err(e) => Json(serde_json::json!({
            "available": false,
            "message": format!("Could not reach WhatsApp Web gateway: {e}"),
            "help": "Make sure the gateway is running at the configured URL"
        })),
    }
}

/// GET /api/channels/whatsapp/qr/status — Poll for QR scan completion.
///
/// After calling `/qr/start`, the frontend polls this to check if the user
/// has scanned the QR code and the WhatsApp Web session is connected.
pub async fn whatsapp_qr_status(
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let gateway_url = std::env::var("WHATSAPP_WEB_GATEWAY_URL").unwrap_or_default();

    if gateway_url.is_empty() {
        return Json(serde_json::json!({
            "connected": false,
            "message": "Gateway not available"
        }));
    }

    let session_id = params.get("session_id").cloned().unwrap_or_default();
    let status_url = format!(
        "{}/login/status?session_id={}",
        gateway_url.trim_end_matches('/'),
        session_id
    );

    match gateway_http_get(&status_url).await {
        Ok(body) => {
            let connected = body
                .get("connected")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let msg = body
                .get("message")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("Waiting for scan...");
            let expired = body
                .get("expired")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            Json(serde_json::json!({
                "connected": connected,
                "message": msg,
                "expired": expired,
            }))
        }
        Err(_) => Json(serde_json::json!({ "connected": false, "message": "Gateway unreachable" })),
    }
}

/// Lightweight HTTP POST to a gateway URL. Returns parsed JSON body.
async fn gateway_http_post(url_with_path: &str) -> Result<serde_json::Value, String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    // Split into base URL + path from the full URL like "http://127.0.0.1:3009/login/start"
    let without_scheme = url_with_path
        .strip_prefix("http://")
        .or_else(|| url_with_path.strip_prefix("https://"))
        .unwrap_or(url_with_path);
    let (host_port, path) = if let Some(idx) = without_scheme.find('/') {
        (&without_scheme[..idx], &without_scheme[idx..])
    } else {
        (without_scheme, "/")
    };
    let (host, port) = if let Some((h, p)) = host_port.rsplit_once(':') {
        (h, p.parse().unwrap_or(3009u16))
    } else {
        (host_port, 3009u16)
    };

    let mut stream = tokio::net::TcpStream::connect(format!("{host}:{port}"))
        .await
        .map_err(|e| format!("Connect failed: {e}"))?;

    let req = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}:{port}\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
    );
    stream
        .write_all(req.as_bytes())
        .await
        .map_err(|e| format!("Write failed: {e}"))?;

    let mut buf = Vec::new();
    stream
        .read_to_end(&mut buf)
        .await
        .map_err(|e| format!("Read failed: {e}"))?;
    let response = String::from_utf8_lossy(&buf);

    // Find the JSON body after the blank line separating headers from body
    if let Some(idx) = response.find("\r\n\r\n") {
        let body_str = &response[idx + 4..];
        serde_json::from_str(body_str.trim()).map_err(|e| format!("Parse failed: {e}"))
    } else {
        Err("No HTTP body in response".to_string())
    }
}

/// Lightweight HTTP GET to a gateway URL. Returns parsed JSON body.
async fn gateway_http_get(url_with_path: &str) -> Result<serde_json::Value, String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let without_scheme = url_with_path
        .strip_prefix("http://")
        .or_else(|| url_with_path.strip_prefix("https://"))
        .unwrap_or(url_with_path);
    let (host_port, path_and_query) = if let Some(idx) = without_scheme.find('/') {
        (&without_scheme[..idx], &without_scheme[idx..])
    } else {
        (without_scheme, "/")
    };
    let (host, port) = if let Some((h, p)) = host_port.rsplit_once(':') {
        (h, p.parse().unwrap_or(3009u16))
    } else {
        (host_port, 3009u16)
    };

    let mut stream = tokio::net::TcpStream::connect(format!("{host}:{port}"))
        .await
        .map_err(|e| format!("Connect failed: {e}"))?;

    let req = format!(
        "GET {path_and_query} HTTP/1.1\r\nHost: {host}:{port}\r\nConnection: close\r\n\r\n"
    );
    stream
        .write_all(req.as_bytes())
        .await
        .map_err(|e| format!("Write failed: {e}"))?;

    let mut buf = Vec::new();
    stream
        .read_to_end(&mut buf)
        .await
        .map_err(|e| format!("Read failed: {e}"))?;
    let response = String::from_utf8_lossy(&buf);

    if let Some(idx) = response.find("\r\n\r\n") {
        let body_str = &response[idx + 4..];
        serde_json::from_str(body_str.trim()).map_err(|e| format!("Parse failed: {e}"))
    } else {
        Err("No HTTP body in response".to_string())
    }
}

// ---------------------------------------------------------------------------
// Template endpoints
// ---------------------------------------------------------------------------

/// GET /api/templates — List available agent templates.
pub async fn list_templates() -> impl IntoResponse {
    let agents_dir = openfang_kernel::config::openfang_home().join("agents");
    let mut templates = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&agents_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let manifest_path = path.join("agent.toml");
                if manifest_path.exists() {
                    let name = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();

                    let manifest_content = std::fs::read_to_string(&manifest_path).ok();
                    let description = manifest_content
                        .as_ref()
                        .and_then(|content| toml::from_str::<AgentManifest>(content).ok())
                        .map(|m| m.description)
                        .unwrap_or_default();

                    // Add category based on template name
                    let category = get_template_category(&name);

                    templates.push(serde_json::json!({
                        "name": name,
                        "description": description,
                        "category": category,
                        "manifest_toml": manifest_content.unwrap_or_default(),
                    }));
                }
            }
        }
    }

    Json(serde_json::json!({
        "templates": templates,
        "total": templates.len(),
    }))
}

fn get_template_category(name: &str) -> &str {
    match name {
        "hello-world" | "assistant" => "General",
        "researcher" | "analyst" => "Research",
        "coder" | "debugger" | "devops-lead" => "Development",
        "writer" | "doc-writer" => "Writing",
        "ops" | "planner" => "Operations",
        "architect" | "security-auditor" => "Development",
        "code-reviewer" | "data-scientist" | "test-engineer" => "Development",
        "legal-assistant" | "email-assistant" | "social-media" => "Business",
        "customer-support" | "sales-assistant" | "recruiter" => "Business",
        "meeting-assistant" => "Business",
        "translator" | "tutor" | "health-tracker" => "General",
        "personal-finance" | "travel-planner" | "home-automation" => "General",
        _ => "General",
    }
}

/// GET /api/templates/:name — Get template details.
pub async fn get_template(
    Path(name): Path<String>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/templates/:name";
    let agents_dir = openfang_kernel::config::openfang_home().join("agents");
    let manifest_path = agents_dir.join(&name).join("agent.toml");

    if !manifest_path.exists() {
        return api_json_error(
            StatusCode::NOT_FOUND,
            &rid,
            PATH,
            "Template not found",
            format!("No template directory for '{name}' (missing agent.toml)."),
            Some("Use GET /api/templates to list available templates."),
        );
    }

    match std::fs::read_to_string(&manifest_path) {
        Ok(content) => match toml::from_str::<AgentManifest>(&content) {
            Ok(manifest) => (
                StatusCode::OK,
                Json(serde_json::json!({
                    "name": name,
                    "manifest": {
                        "name": manifest.name,
                        "description": manifest.description,
                        "module": manifest.module,
                        "tags": manifest.tags,
                        "model": {
                            "provider": manifest.model.provider,
                            "model": manifest.model.model,
                        },
                        "capabilities": {
                            "tools": manifest.capabilities.tools,
                            "network": manifest.capabilities.network,
                        },
                    },
                    "manifest_toml": content,
                })),
            ),
            Err(e) => {
                tracing::warn!("Invalid template manifest for '{name}': {e}");
                api_json_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &rid,
                    PATH,
                    "Invalid template manifest",
                    format!("{e}"),
                    Some("Fix agent.toml syntax or choose another template."),
                )
            }
        },
        Err(e) => {
            tracing::warn!("Failed to read template '{name}': {e}");
            api_json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &rid,
                PATH,
                "Failed to read template",
                format!("{e}"),
                Some("Check file permissions on the template path."),
            )
        }
    }
}

// ---------------------------------------------------------------------------
// Memory endpoints
// ---------------------------------------------------------------------------

/// GET /api/memory/agents/:id/kv — List KV pairs for an agent.
///
/// Note: memory_store tool writes to a shared namespace, so we read from that
/// same namespace regardless of which agent ID is in the URL.
pub async fn get_agent_kv(
    State(state): State<Arc<AppState>>,
    Path(_id): Path<String>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/memory/agents/:id/kv";
    let agent_id = openfang_kernel::kernel::shared_memory_agent_id();

    match state.kernel.memory.list_kv(agent_id) {
        Ok(pairs) => {
            let kv: Vec<serde_json::Value> = pairs
                .into_iter()
                .map(|(k, v)| serde_json::json!({"key": k, "value": v}))
                .collect();
            (StatusCode::OK, Json(serde_json::json!({"kv_pairs": kv})))
        }
        Err(e) => {
            tracing::warn!("Memory list_kv failed: {e}");
            api_json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &rid,
                PATH,
                "Memory operation failed",
                format!("{e}"),
                Some("Check SQLite / memory store health."),
            )
        }
    }
}

/// GET /api/memory/agents/:id/kv/:key — Get a specific KV value.
pub async fn get_agent_kv_key(
    State(state): State<Arc<AppState>>,
    Path((_id, key)): Path<(String, String)>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/memory/agents/:id/kv/:key";
    let agent_id = openfang_kernel::kernel::shared_memory_agent_id();

    match state.kernel.memory.structured_get(agent_id, &key) {
        Ok(Some(val)) => (
            StatusCode::OK,
            Json(serde_json::json!({"key": key, "value": val})),
        ),
        Ok(None) => api_json_error(
            StatusCode::NOT_FOUND,
            &rid,
            PATH,
            "Key not found",
            format!("No value stored for key '{key}'."),
            Some("Use PUT to create the key or list keys with GET /api/memory/agents/:id/kv."),
        ),
        Err(e) => {
            tracing::warn!("Memory get failed for key '{key}': {e}");
            api_json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &rid,
                PATH,
                "Memory operation failed",
                format!("{e}"),
                Some("Check SQLite / memory store health."),
            )
        }
    }
}

/// PUT /api/memory/agents/:id/kv/:key — Set a KV value.
pub async fn set_agent_kv_key(
    State(state): State<Arc<AppState>>,
    Path((_id, key)): Path<(String, String)>,
    ext: Option<Extension<RequestId>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/memory/agents/:id/kv/:key";
    let agent_id = openfang_kernel::kernel::shared_memory_agent_id();

    let value = body.get("value").cloned().unwrap_or(body);

    match state.kernel.memory.structured_set(agent_id, &key, value) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "stored", "key": key})),
        ),
        Err(e) => {
            tracing::warn!("Memory set failed for key '{key}': {e}");
            api_json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &rid,
                PATH,
                "Memory operation failed",
                format!("{e}"),
                Some("Check SQLite / memory store health."),
            )
        }
    }
}

/// DELETE /api/memory/agents/:id/kv/:key — Delete a KV value.
pub async fn delete_agent_kv_key(
    State(state): State<Arc<AppState>>,
    Path((_id, key)): Path<(String, String)>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/memory/agents/:id/kv/:key";
    let agent_id = openfang_kernel::kernel::shared_memory_agent_id();

    match state.kernel.memory.structured_delete(agent_id, &key) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "deleted", "key": key})),
        ),
        Err(e) => {
            tracing::warn!("Memory delete failed for key '{key}': {e}");
            api_json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &rid,
                PATH,
                "Memory operation failed",
                format!("{e}"),
                Some("Check SQLite / memory store health."),
            )
        }
    }
}

/// GET /api/health — Minimal liveness probe (public, no auth required).
/// Returns only status and version to prevent information leakage.
/// Use GET /api/health/detail for full diagnostics (requires auth).
pub async fn health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // Run the database check on a blocking thread so we never hold the
    // std::sync::Mutex<Connection> on a tokio worker thread.  This prevents
    // the health probe from starving the async runtime when the agent loop
    // is holding the database lock for session saves.
    let memory = state.kernel.memory.clone();
    let db_ok = tokio::task::spawn_blocking(move || memory.sqlite_liveness_probe())
        .await
        .unwrap_or(false);

    let status = if db_ok { "ok" } else { "degraded" };

    Json(serde_json::json!({
        "status": status,
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

/// GET /api/health/detail — Full health diagnostics (requires auth).
pub async fn health_detail(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let health = state.kernel.supervisor.health();

    let memory = state.kernel.memory.clone();
    let db_ok = tokio::task::spawn_blocking(move || memory.sqlite_liveness_probe())
        .await
        .unwrap_or(false);

    let config_warnings = state.kernel.config.validate();
    let status = if db_ok { "ok" } else { "degraded" };

    Json(serde_json::json!({
        "status": status,
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_seconds": state.started_at.elapsed().as_secs(),
        "panic_count": health.panic_count,
        "restart_count": health.restart_count,
        "agent_count": state.kernel.registry.count(),
        "database": if db_ok { "connected" } else { "error" },
        "config_warnings": config_warnings,
    }))
}

// ---------------------------------------------------------------------------
// Prometheus metrics endpoint
// ---------------------------------------------------------------------------

/// GET /api/metrics — Prometheus text-format metrics.
///
/// Returns counters and gauges for monitoring OpenFang in production:
/// - `openfang_agents_active` — number of active agents
/// - `openfang_uptime_seconds` — seconds since daemon started
/// - `openfang_tokens_total` — total tokens consumed (per agent)
/// - `openfang_tool_calls_total` — total tool calls (per agent)
/// - `openfang_panics_total` — supervisor panic count
/// - `openfang_restarts_total` — supervisor restart count
pub async fn prometheus_metrics(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let mut out = String::with_capacity(2048);

    // Uptime
    let uptime = state.started_at.elapsed().as_secs();
    out.push_str("# HELP openfang_uptime_seconds Time since daemon started.\n");
    out.push_str("# TYPE openfang_uptime_seconds gauge\n");
    out.push_str(&format!("openfang_uptime_seconds {uptime}\n\n"));

    // Active agents
    let agents = state.kernel.registry.list();
    let active = agents
        .iter()
        .filter(|a| matches!(a.state, openfang_types::agent::AgentState::Running))
        .count();
    out.push_str("# HELP openfang_agents_active Number of active agents.\n");
    out.push_str("# TYPE openfang_agents_active gauge\n");
    out.push_str(&format!("openfang_agents_active {active}\n"));
    out.push_str("# HELP openfang_agents_total Total number of registered agents.\n");
    out.push_str("# TYPE openfang_agents_total gauge\n");
    out.push_str(&format!("openfang_agents_total {}\n\n", agents.len()));

    // Per-agent token and tool usage
    out.push_str("# HELP openfang_tokens_total Total tokens consumed (rolling hourly window).\n");
    out.push_str("# TYPE openfang_tokens_total gauge\n");
    out.push_str("# HELP openfang_tool_calls_total Total tool calls (rolling hourly window).\n");
    out.push_str("# TYPE openfang_tool_calls_total gauge\n");
    for agent in &agents {
        let name = &agent.name;
        let provider = &agent.manifest.model.provider;
        let model = &agent.manifest.model.model;
        if let Some((tokens, tools)) = state.kernel.scheduler.get_usage(agent.id) {
            out.push_str(&format!(
                "openfang_tokens_total{{agent=\"{name}\",provider=\"{provider}\",model=\"{model}\"}} {tokens}\n"
            ));
            out.push_str(&format!(
                "openfang_tool_calls_total{{agent=\"{name}\"}} {tools}\n"
            ));
        }
    }
    out.push('\n');

    // Supervisor health
    let health = state.kernel.supervisor.health();
    out.push_str("# HELP openfang_panics_total Total supervisor panics since start.\n");
    out.push_str("# TYPE openfang_panics_total counter\n");
    out.push_str(&format!("openfang_panics_total {}\n", health.panic_count));
    out.push_str("# HELP openfang_restarts_total Total supervisor restarts since start.\n");
    out.push_str("# TYPE openfang_restarts_total counter\n");
    out.push_str(&format!(
        "openfang_restarts_total {}\n\n",
        health.restart_count
    ));

    out.push_str(&state.kernel.llm_factory.prometheus_snippet());

    // AINL runtime bridge cache behavior (only increments when ainl-runtime-engine path is used).
    let arm = openfang_runtime::ainl_runtime_bridge_metrics();
    let cache_hits = arm
        .get("cache_hits")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let cache_misses = arm
        .get("cache_misses")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let construct_failures = arm
        .get("construct_failures")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let run_failures = arm
        .get("run_failures")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    out.push_str("# HELP openfang_ainl_runtime_bridge_cache_hits_total Total ainl-runtime bridge cache hits.\n");
    out.push_str("# TYPE openfang_ainl_runtime_bridge_cache_hits_total counter\n");
    out.push_str(&format!(
        "openfang_ainl_runtime_bridge_cache_hits_total {cache_hits}\n"
    ));
    out.push_str("# HELP openfang_ainl_runtime_bridge_cache_misses_total Total ainl-runtime bridge cache misses.\n");
    out.push_str("# TYPE openfang_ainl_runtime_bridge_cache_misses_total counter\n");
    out.push_str(&format!(
        "openfang_ainl_runtime_bridge_cache_misses_total {cache_misses}\n"
    ));
    out.push_str("# HELP openfang_ainl_runtime_bridge_construct_failures_total Total ainl-runtime bridge construction failures.\n");
    out.push_str("# TYPE openfang_ainl_runtime_bridge_construct_failures_total counter\n");
    out.push_str(&format!(
        "openfang_ainl_runtime_bridge_construct_failures_total {construct_failures}\n"
    ));
    out.push_str("# HELP openfang_ainl_runtime_bridge_run_failures_total Total ainl-runtime bridge run failures.\n");
    out.push_str("# TYPE openfang_ainl_runtime_bridge_run_failures_total counter\n");
    out.push_str(&format!(
        "openfang_ainl_runtime_bridge_run_failures_total {run_failures}\n\n"
    ));

    // Version info
    out.push_str("# HELP openfang_info ArmaraOS version and build info.\n");
    out.push_str("# TYPE openfang_info gauge\n");
    out.push_str(&format!(
        "openfang_info{{version=\"{}\"}} 1\n",
        env!("CARGO_PKG_VERSION")
    ));

    (
        StatusCode::OK,
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        out,
    )
}

// ---------------------------------------------------------------------------
// Skills endpoints
// ---------------------------------------------------------------------------

/// GET /api/skills — List installed skills.
pub async fn list_skills(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let skills_dir = state.kernel.config.home_dir.join("skills");
    let mut registry = openfang_skills::registry::SkillRegistry::new(skills_dir);
    let _ = registry.load_all();

    let skills: Vec<serde_json::Value> = registry
        .list()
        .iter()
        .map(|s| {
            let source = match &s.manifest.source {
                Some(openfang_skills::SkillSource::ClawHub { slug, version }) => {
                    serde_json::json!({"type": "clawhub", "slug": slug, "version": version})
                }
                Some(openfang_skills::SkillSource::OpenClaw) => {
                    serde_json::json!({"type": "openclaw"})
                }
                Some(openfang_skills::SkillSource::Bundled) => {
                    serde_json::json!({"type": "bundled"})
                }
                Some(openfang_skills::SkillSource::Native) | None => {
                    serde_json::json!({"type": "local"})
                }
            };
            let runtime_str = format!("{:?}", s.manifest.runtime.runtime_type);
            let runtime_supported =
                s.manifest.runtime.runtime_type != openfang_skills::SkillRuntime::Wasm;
            serde_json::json!({
                "name": s.manifest.skill.name,
                "description": s.manifest.skill.description,
                "version": s.manifest.skill.version,
                "author": s.manifest.skill.author,
                "runtime": runtime_str,
                "runtime_supported": runtime_supported,
                "tools_count": s.manifest.tools.provided.len(),
                "tags": s.manifest.skill.tags,
                "enabled": s.enabled,
                "source": source,
                "has_prompt_context": s.manifest.prompt_context.is_some(),
            })
        })
        .collect();

    Json(serde_json::json!({ "skills": skills, "total": skills.len() }))
}

/// POST /api/skills/install — Install a skill from FangHub (GitHub).
pub async fn install_skill(
    State(state): State<Arc<AppState>>,
    ext: Option<Extension<RequestId>>,
    Json(req): Json<SkillInstallRequest>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/skills/install";
    let skills_dir = state.kernel.config.home_dir.join("skills");
    let config = openfang_skills::marketplace::MarketplaceConfig::default();
    let client = openfang_skills::marketplace::MarketplaceClient::new(config);

    match client.install(&req.name, &skills_dir).await {
        Ok(version) => {
            // Hot-reload so agents see the new skill immediately
            state.kernel.reload_skills();
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "installed",
                    "name": req.name,
                    "version": version,
                })),
            )
        }
        Err(e) => {
            tracing::warn!("Skill install failed: {e}");
            api_json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &rid,
                PATH,
                "Skill install failed",
                format!("{e}"),
                Some("Verify skill name, network access to the marketplace, and ~/.armaraos/skills permissions."),
            )
        }
    }
}

/// POST /api/skills/uninstall — Uninstall a skill.
pub async fn uninstall_skill(
    State(state): State<Arc<AppState>>,
    ext: Option<Extension<RequestId>>,
    Json(req): Json<SkillUninstallRequest>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/skills/uninstall";
    let skills_dir = state.kernel.config.home_dir.join("skills");
    let mut registry = openfang_skills::registry::SkillRegistry::new(skills_dir);
    let _ = registry.load_all();

    match registry.remove(&req.name) {
        Ok(()) => {
            // Hot-reload so agents stop seeing the removed skill
            state.kernel.reload_skills();
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "uninstalled", "name": req.name})),
            )
        }
        Err(e) => api_json_error(
            StatusCode::NOT_FOUND,
            &rid,
            PATH,
            "Skill not found",
            format!("{e}"),
            Some("List installed skills via GET /api/skills."),
        ),
    }
}

/// POST /api/skills/reload — Hot-reload the skill registry from disk.
///
/// Called by the CLI after `openfang skill install` to notify the running
/// daemon that new skill files were added to the skills directory (#752).
pub async fn reload_skills(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    state.kernel.reload_skills();
    Json(serde_json::json!({"status": "reloaded"}))
}

/// GET /api/marketplace/search — Search the FangHub marketplace.
pub async fn marketplace_search(
    ext: Option<Extension<RequestId>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/marketplace/search";
    let query = params.get("q").cloned().unwrap_or_default();
    if query.is_empty() {
        return (
            StatusCode::OK,
            Json(serde_json::json!({"results": [], "total": 0})),
        );
    }

    let config = openfang_skills::marketplace::MarketplaceConfig::default();
    let client = openfang_skills::marketplace::MarketplaceClient::new(config);

    match client.search(&query).await {
        Ok(results) => {
            let items: Vec<serde_json::Value> = results
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "name": r.name,
                        "description": r.description,
                        "stars": r.stars,
                        "url": r.url,
                    })
                })
                .collect();
            (
                StatusCode::OK,
                Json(serde_json::json!({"results": items, "total": items.len()})),
            )
        }
        Err(e) => {
            tracing::warn!("Marketplace search failed: {e}");
            api_json_error(
                StatusCode::BAD_GATEWAY,
                &rid,
                PATH,
                "Marketplace search failed",
                format!("{e}"),
                Some("Check network connectivity to the marketplace API."),
            )
        }
    }
}

// ---------------------------------------------------------------------------
// ClawHub (OpenClaw ecosystem) endpoints
// ---------------------------------------------------------------------------

/// GET /api/clawhub/search — Search ClawHub skills using vector/semantic search.
///
/// Query parameters:
/// - `q` — search query (required)
/// - `limit` — max results (default: 20, max: 50)
pub async fn clawhub_search(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let query = params.get("q").cloned().unwrap_or_default();
    if query.is_empty() {
        return (
            StatusCode::OK,
            Json(serde_json::json!({"items": [], "next_cursor": null})),
        );
    }

    let limit: u32 = params
        .get("limit")
        .and_then(|v| v.parse().ok())
        .unwrap_or(20);

    // Check cache (120s TTL)
    let cache_key = format!("search:{}:{}", query, limit);
    if let Some(entry) = state.clawhub_cache.get(&cache_key) {
        if entry.0.elapsed().as_secs() < 120 {
            return (StatusCode::OK, Json(entry.1.clone()));
        }
    }

    let cache_dir = state.kernel.config.home_dir.join(".cache").join("clawhub");
    let client = openfang_skills::clawhub::ClawHubClient::new(cache_dir);

    let skills_dir = state.kernel.config.home_dir.join("skills");
    match client.search(&query, limit).await {
        Ok(results) => {
            let items: Vec<serde_json::Value> = results
                .results
                .iter()
                .map(|e| {
                    let installed = skills_dir.join(&e.slug).exists();
                    serde_json::json!({
                        "slug": e.slug,
                        "name": e.display_name,
                        "description": e.summary,
                        "version": e.version,
                        "score": e.score,
                        "updated_at": e.updated_at,
                        "installed": installed,
                    })
                })
                .collect();
            let resp = serde_json::json!({
                "items": items,
                "next_cursor": null,
            });
            state
                .clawhub_cache
                .insert(cache_key, (Instant::now(), resp.clone()));
            (StatusCode::OK, Json(resp))
        }
        Err(e) => {
            let msg = format!("{e}");
            tracing::warn!("ClawHub search failed: {msg}");
            let status = if is_clawhub_rate_limit(&e) {
                StatusCode::TOO_MANY_REQUESTS
            } else {
                StatusCode::OK
            };
            (
                status,
                Json(serde_json::json!({"items": [], "next_cursor": null, "error": msg})),
            )
        }
    }
}

/// GET /api/clawhub/browse — Browse ClawHub skills by sort order.
///
/// Query parameters:
/// - `sort` — sort order: "trending", "downloads", "stars", "updated", "rating" (default: "trending")
/// - `limit` — max results (default: 20, max: 50)
/// - `cursor` — pagination cursor from previous response
pub async fn clawhub_browse(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let sort = match params.get("sort").map(|s| s.as_str()) {
        Some("downloads") => openfang_skills::clawhub::ClawHubSort::Downloads,
        Some("stars") => openfang_skills::clawhub::ClawHubSort::Stars,
        Some("updated") => openfang_skills::clawhub::ClawHubSort::Updated,
        Some("rating") => openfang_skills::clawhub::ClawHubSort::Rating,
        _ => openfang_skills::clawhub::ClawHubSort::Trending,
    };

    let limit: u32 = params
        .get("limit")
        .and_then(|v| v.parse().ok())
        .unwrap_or(20);

    let cursor = params.get("cursor").map(|s| s.as_str());

    // Check cache (120s TTL)
    let cache_key = format!("browse:{:?}:{}:{}", sort, limit, cursor.unwrap_or(""));
    if let Some(entry) = state.clawhub_cache.get(&cache_key) {
        if entry.0.elapsed().as_secs() < 120 {
            return (StatusCode::OK, Json(entry.1.clone()));
        }
    }

    let cache_dir = state.kernel.config.home_dir.join(".cache").join("clawhub");
    let client = openfang_skills::clawhub::ClawHubClient::new(cache_dir);

    let skills_dir = state.kernel.config.home_dir.join("skills");
    match client.browse(sort, limit, cursor).await {
        Ok(results) => {
            let items: Vec<serde_json::Value> = results
                .items
                .iter()
                .map(|entry| {
                    let mut json = clawhub_browse_entry_to_json(entry);
                    let installed = skills_dir.join(&entry.slug).exists();
                    json["installed"] = serde_json::json!(installed);
                    json
                })
                .collect();
            let resp = serde_json::json!({
                "items": items,
                "next_cursor": results.next_cursor,
            });
            state
                .clawhub_cache
                .insert(cache_key, (Instant::now(), resp.clone()));
            (StatusCode::OK, Json(resp))
        }
        Err(e) => {
            let msg = format!("{e}");
            tracing::warn!("ClawHub browse failed: {msg}");
            let status = if is_clawhub_rate_limit(&e) {
                StatusCode::TOO_MANY_REQUESTS
            } else {
                StatusCode::OK
            };
            (
                status,
                Json(serde_json::json!({"items": [], "next_cursor": null, "error": msg})),
            )
        }
    }
}

/// GET /api/clawhub/skill/{slug} — Get detailed info about a ClawHub skill.
pub async fn clawhub_skill_detail(
    State(state): State<Arc<AppState>>,
    Path(slug): Path<String>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/clawhub/skill/:slug";
    let cache_dir = state.kernel.config.home_dir.join(".cache").join("clawhub");
    let client = openfang_skills::clawhub::ClawHubClient::new(cache_dir);

    let skills_dir = state.kernel.config.home_dir.join("skills");
    let is_installed = client.is_installed(&slug, &skills_dir);

    match client.get_skill(&slug).await {
        Ok(detail) => {
            let version = detail
                .latest_version
                .as_ref()
                .map(|v| v.version.as_str())
                .unwrap_or("");
            let author = detail
                .owner
                .as_ref()
                .map(|o| o.handle.as_str())
                .unwrap_or("");
            let author_name = detail
                .owner
                .as_ref()
                .map(|o| o.display_name.as_str())
                .unwrap_or("");
            let author_image = detail
                .owner
                .as_ref()
                .map(|o| o.image.as_str())
                .unwrap_or("");

            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "slug": detail.skill.slug,
                    "name": detail.skill.display_name,
                    "description": detail.skill.summary,
                    "version": version,
                    "downloads": detail.skill.stats.downloads,
                    "stars": detail.skill.stats.stars,
                    "author": author,
                    "author_name": author_name,
                    "author_image": author_image,
                    "tags": detail.skill.tags,
                    "updated_at": detail.skill.updated_at,
                    "created_at": detail.skill.created_at,
                    "installed": is_installed,
                })),
            )
        }
        Err(e) => {
            let status = if is_clawhub_rate_limit(&e) {
                StatusCode::TOO_MANY_REQUESTS
            } else {
                StatusCode::NOT_FOUND
            };
            let hint = if status == StatusCode::TOO_MANY_REQUESTS {
                Some("ClawHub rate limit — wait and retry, or use cached browse results.")
            } else {
                Some("Verify the slug exists on ClawHub or check network connectivity.")
            };
            api_json_error(
                status,
                &rid,
                PATH,
                "ClawHub skill fetch failed",
                format!("{e}"),
                hint,
            )
        }
    }
}

/// GET /api/clawhub/skill/{slug}/code — Fetch the source code (SKILL.md) of a ClawHub skill.
pub async fn clawhub_skill_code(
    State(state): State<Arc<AppState>>,
    Path(slug): Path<String>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/clawhub/skill/:slug/code";
    let cache_dir = state.kernel.config.home_dir.join(".cache").join("clawhub");
    let client = openfang_skills::clawhub::ClawHubClient::new(cache_dir);

    // Try to fetch SKILL.md first, then fallback to package.json
    let mut code = String::new();
    let mut filename = String::new();

    if let Ok(content) = client.get_file(&slug, "SKILL.md").await {
        code = content;
        filename = "SKILL.md".to_string();
    } else if let Ok(content) = client.get_file(&slug, "package.json").await {
        code = content;
        filename = "package.json".to_string();
    } else if let Ok(content) = client.get_file(&slug, "skill.toml").await {
        code = content;
        filename = "skill.toml".to_string();
    }

    if code.is_empty() {
        return api_json_error(
            StatusCode::NOT_FOUND,
            &rid,
            PATH,
            "No source code found for this skill",
            format!("Could not fetch SKILL.md, package.json, or skill.toml for '{slug}'."),
            Some("Confirm the slug on ClawHub and network access."),
        );
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "slug": slug,
            "filename": filename,
            "code": code,
        })),
    )
}

/// POST /api/clawhub/install — Install a skill from ClawHub.
///
/// Runs the full security pipeline: SHA256 verification, format detection,
/// manifest security scan, prompt injection scan, and binary dependency check.
pub async fn clawhub_install(
    State(state): State<Arc<AppState>>,
    ext: Option<Extension<RequestId>>,
    Json(req): Json<crate::types::ClawHubInstallRequest>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/clawhub/install";
    let skills_dir = state.kernel.config.home_dir.join("skills");
    let cache_dir = state.kernel.config.home_dir.join(".cache").join("clawhub");
    let client = openfang_skills::clawhub::ClawHubClient::new(cache_dir);

    // Check if already installed
    if client.is_installed(&req.slug, &skills_dir) {
        return api_json_error(
            StatusCode::CONFLICT,
            &rid,
            PATH,
            "Skill already installed",
            format!("Skill '{}' is already installed.", req.slug),
            Some("Uninstall first or pick a different skill."),
        );
    }

    match client.install(&req.slug, &skills_dir).await {
        Ok(result) => {
            // Hot-reload so agents see the new skill immediately (#752)
            state.kernel.reload_skills();

            let warnings: Vec<serde_json::Value> = result
                .warnings
                .iter()
                .map(|w| {
                    serde_json::json!({
                        "severity": format!("{:?}", w.severity),
                        "message": w.message,
                    })
                })
                .collect();

            let translations: Vec<serde_json::Value> = result
                .tool_translations
                .iter()
                .map(|(from, to)| serde_json::json!({"from": from, "to": to}))
                .collect();

            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "installed",
                    "name": result.skill_name,
                    "version": result.version,
                    "slug": result.slug,
                    "is_prompt_only": result.is_prompt_only,
                    "warnings": warnings,
                    "tool_translations": translations,
                })),
            )
        }
        Err(e) => {
            let msg = format!("{e}");
            let status = if matches!(e, openfang_skills::SkillError::SecurityBlocked(_)) {
                StatusCode::FORBIDDEN
            } else if is_clawhub_rate_limit(&e) {
                StatusCode::TOO_MANY_REQUESTS
            } else if matches!(e, openfang_skills::SkillError::Network(_)) {
                StatusCode::BAD_GATEWAY
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            tracing::warn!("ClawHub install failed: {msg}");
            let hint = if status == StatusCode::FORBIDDEN {
                Some("Security scan blocked this package — inspect warnings in logs.")
            } else if status == StatusCode::TOO_MANY_REQUESTS {
                Some("ClawHub rate limit — wait and retry.")
            } else if status == StatusCode::BAD_GATEWAY {
                Some("Network error reaching ClawHub — check connectivity.")
            } else {
                Some("See daemon logs for this request_id.")
            };
            api_json_error(status, &rid, PATH, "ClawHub install failed", msg, hint)
        }
    }
}

/// Check whether a SkillError represents a ClawHub rate-limit (429).
fn is_clawhub_rate_limit(err: &openfang_skills::SkillError) -> bool {
    matches!(err, openfang_skills::SkillError::RateLimited(_))
}

/// Convert a browse entry (nested stats/tags) to a flat JSON object for the frontend.
fn clawhub_browse_entry_to_json(
    entry: &openfang_skills::clawhub::ClawHubBrowseEntry,
) -> serde_json::Value {
    let version = openfang_skills::clawhub::ClawHubClient::entry_version(entry);
    serde_json::json!({
        "slug": entry.slug,
        "name": entry.display_name,
        "description": entry.summary,
        "version": version,
        "downloads": entry.stats.downloads,
        "stars": entry.stats.stars,
        "updated_at": entry.updated_at,
    })
}

// ---------------------------------------------------------------------------
// Hands endpoints
// ---------------------------------------------------------------------------

/// Detect the server platform for install command selection.
fn server_platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    }
}

/// GET /api/hands — List all hand definitions (marketplace).
pub async fn list_hands(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let defs = state.kernel.hand_registry.list_definitions();
    let hands: Vec<serde_json::Value> = defs
        .iter()
        .map(|d| {
            let reqs = state
                .kernel
                .hand_registry
                .check_requirements(&d.id)
                .unwrap_or_default();
            let readiness = state.kernel.hand_registry.readiness(&d.id);
            let requirements_met = readiness
                .as_ref()
                .map(|r| r.requirements_met)
                .unwrap_or(false);
            let active = readiness.as_ref().map(|r| r.active).unwrap_or(false);
            let degraded = readiness.as_ref().map(|r| r.degraded).unwrap_or(false);
            let schema_warning: Option<&str> = match d.schema_version.as_deref() {
                None => Some("legacy"),
                Some(v) if v != openfang_hands::HAND_SCHEMA_VERSION => Some("mismatch"),
                _ => None,
            };
            serde_json::json!({
                "id": d.id,
                "name": d.name,
                "description": d.description,
                "category": d.category,
                "icon": d.icon,
                "tools": d.tools,
                "requirements_met": requirements_met,
                "active": active,
                "degraded": degraded,
                "requirements": reqs.iter().map(|(r, ok)| serde_json::json!({
                    "key": r.key,
                    "label": r.label,
                    "satisfied": ok,
                    "optional": r.optional,
                })).collect::<Vec<_>>(),
                "dashboard_metrics": d.dashboard.metrics.len(),
                "has_settings": !d.settings.is_empty(),
                "settings_count": d.settings.len(),
                "schema_version": d.schema_version,
                "schema_warning": schema_warning,
            })
        })
        .collect();

    Json(serde_json::json!({ "hands": hands, "total": hands.len() }))
}

/// GET /api/hands/active — List active hand instances.
pub async fn list_active_hands(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let instances = state.kernel.hand_registry.list_instances();
    let items: Vec<serde_json::Value> = instances
        .iter()
        .map(|i| {
            serde_json::json!({
                "instance_id": i.instance_id,
                "hand_id": i.hand_id,
                "status": format!("{}", i.status),
                "agent_id": i.agent_id.map(|a| a.to_string()),
                "agent_name": i.agent_name,
                "activated_at": i.activated_at.to_rfc3339(),
                "updated_at": i.updated_at.to_rfc3339(),
            })
        })
        .collect();

    Json(serde_json::json!({ "instances": items, "total": items.len() }))
}

/// GET /api/hands/{hand_id} — Get a single hand definition with requirements check.
pub async fn get_hand(
    State(state): State<Arc<AppState>>,
    Path(hand_id): Path<String>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/hands/:hand_id";
    match state.kernel.hand_registry.get_definition(&hand_id) {
        Some(def) => {
            let reqs = state
                .kernel
                .hand_registry
                .check_requirements(&hand_id)
                .unwrap_or_default();
            let readiness = state.kernel.hand_registry.readiness(&hand_id);
            let requirements_met = readiness
                .as_ref()
                .map(|r| r.requirements_met)
                .unwrap_or(false);
            let active = readiness.as_ref().map(|r| r.active).unwrap_or(false);
            let degraded = readiness.as_ref().map(|r| r.degraded).unwrap_or(false);
            let settings_status = state
                .kernel
                .hand_registry
                .check_settings_availability(&hand_id)
                .unwrap_or_default();
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "id": def.id,
                    "name": def.name,
                    "description": def.description,
                    "category": def.category,
                    "icon": def.icon,
                    "tools": def.tools,
                    "requirements_met": requirements_met,
                    "active": active,
                    "degraded": degraded,
                    "requirements": reqs.iter().map(|(r, ok)| {
                        let mut req_json = serde_json::json!({
                            "key": r.key,
                            "label": r.label,
                            "type": format!("{:?}", r.requirement_type),
                            "check_value": r.check_value,
                            "satisfied": ok,
                            "optional": r.optional,
                        });
                        if let Some(ref desc) = r.description {
                            req_json["description"] = serde_json::json!(desc);
                        }
                        if let Some(ref install) = r.install {
                            req_json["install"] = serde_json::to_value(install).unwrap_or_default();
                        }
                        req_json
                    }).collect::<Vec<_>>(),
                    "server_platform": server_platform(),
                    "agent": {
                        "name": def.agent.name,
                        "description": def.agent.description,
                        "provider": if def.agent.provider == "default" {
                            &state.kernel.config.default_model.provider
                        } else { &def.agent.provider },
                        "model": if def.agent.model == "default" {
                            &state.kernel.config.default_model.model
                        } else { &def.agent.model },
                    },
                    "dashboard": def.dashboard.metrics.iter().map(|m| serde_json::json!({
                        "label": m.label,
                        "memory_key": m.memory_key,
                        "format": m.format,
                    })).collect::<Vec<_>>(),
                    "settings": settings_status,
                })),
            )
        }
        None => api_json_error(
            StatusCode::NOT_FOUND,
            &rid,
            PATH,
            "Hand not found",
            format!("No hand definition for id '{hand_id}'."),
            Some("Use GET /api/hands to list available hands."),
        ),
    }
}

/// POST /api/hands/{hand_id}/check-deps — Re-check dependency status for a hand.
pub async fn check_hand_deps(
    State(state): State<Arc<AppState>>,
    Path(hand_id): Path<String>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/hands/:hand_id/check-deps";
    match state.kernel.hand_registry.get_definition(&hand_id) {
        Some(def) => {
            let reqs = state
                .kernel
                .hand_registry
                .check_requirements(&hand_id)
                .unwrap_or_default();
            let readiness = state.kernel.hand_registry.readiness(&hand_id);
            let requirements_met = readiness
                .as_ref()
                .map(|r| r.requirements_met)
                .unwrap_or(false);
            let active = readiness.as_ref().map(|r| r.active).unwrap_or(false);
            let degraded = readiness.as_ref().map(|r| r.degraded).unwrap_or(false);
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "hand_id": def.id,
                    "requirements_met": requirements_met,
                    "active": active,
                    "degraded": degraded,
                    "server_platform": server_platform(),
                    "requirements": reqs.iter().map(|(r, ok)| {
                        let mut req_json = serde_json::json!({
                            "key": r.key,
                            "label": r.label,
                            "type": format!("{:?}", r.requirement_type),
                            "check_value": r.check_value,
                            "satisfied": ok,
                            "optional": r.optional,
                        });
                        if let Some(ref desc) = r.description {
                            req_json["description"] = serde_json::json!(desc);
                        }
                        if let Some(ref install) = r.install {
                            req_json["install"] = serde_json::to_value(install).unwrap_or_default();
                        }
                        req_json
                    }).collect::<Vec<_>>(),
                })),
            )
        }
        None => api_json_error(
            StatusCode::NOT_FOUND,
            &rid,
            PATH,
            "Hand not found",
            format!("No hand definition for id '{hand_id}'."),
            Some("Use GET /api/hands to list available hands."),
        ),
    }
}

/// POST /api/hands/{hand_id}/install-deps — Auto-install missing dependencies for a hand.
pub async fn install_hand_deps(
    State(state): State<Arc<AppState>>,
    Path(hand_id): Path<String>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/hands/:hand_id/install-deps";
    let def = match state.kernel.hand_registry.get_definition(&hand_id) {
        Some(d) => d.clone(),
        None => {
            return api_json_error(
                StatusCode::NOT_FOUND,
                &rid,
                PATH,
                "Hand not found",
                format!("No hand definition for id '{hand_id}'."),
                Some("Use GET /api/hands to list available hands."),
            );
        }
    };

    let reqs = state
        .kernel
        .hand_registry
        .check_requirements(&hand_id)
        .unwrap_or_default();

    let platform = server_platform();
    let mut results = Vec::new();

    for (req, already_satisfied) in &reqs {
        if *already_satisfied {
            results.push(serde_json::json!({
                "key": req.key,
                "status": "already_installed",
                "message": format!("{} is already available", req.label),
            }));
            continue;
        }

        let install = match &req.install {
            Some(i) => i,
            None => {
                results.push(serde_json::json!({
                    "key": req.key,
                    "status": "skipped",
                    "message": "No install instructions available",
                }));
                continue;
            }
        };

        // Pick the best install command for this platform
        let cmd = match platform {
            "windows" => install.windows.as_deref().or(install.pip.as_deref()),
            "macos" => install.macos.as_deref().or(install.pip.as_deref()),
            _ => install
                .linux_apt
                .as_deref()
                .or(install.linux_dnf.as_deref())
                .or(install.linux_pacman.as_deref())
                .or(install.pip.as_deref()),
        };

        let cmd = match cmd {
            Some(c) => c,
            None => {
                results.push(serde_json::json!({
                    "key": req.key,
                    "status": "no_command",
                    "message": format!("No install command for platform: {platform}"),
                }));
                continue;
            }
        };

        // Execute the install command
        let (shell, flag) = if cfg!(windows) {
            ("cmd", "/C")
        } else {
            ("sh", "-c")
        };

        // For winget on Windows, add --accept flags to avoid interactive prompts
        let final_cmd = if cfg!(windows) && cmd.starts_with("winget ") {
            format!("{cmd} --accept-source-agreements --accept-package-agreements")
        } else {
            cmd.to_string()
        };

        tracing::info!(hand = %hand_id, dep = %req.key, cmd = %final_cmd, "Auto-installing dependency");

        let output = match tokio::time::timeout(
            std::time::Duration::from_secs(300),
            tokio::process::Command::new(shell)
                .arg(flag)
                .arg(&final_cmd)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .stdin(std::process::Stdio::null())
                .output(),
        )
        .await
        {
            Ok(Ok(out)) => out,
            Ok(Err(e)) => {
                results.push(serde_json::json!({
                    "key": req.key,
                    "status": "error",
                    "command": final_cmd,
                    "message": format!("Failed to execute: {e}"),
                }));
                continue;
            }
            Err(_) => {
                results.push(serde_json::json!({
                    "key": req.key,
                    "status": "timeout",
                    "command": final_cmd,
                    "message": "Installation timed out after 5 minutes",
                }));
                continue;
            }
        };

        let exit_code = output.status.code().unwrap_or(-1);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        if exit_code == 0 {
            results.push(serde_json::json!({
                "key": req.key,
                "status": "installed",
                "command": final_cmd,
                "message": format!("{} installed successfully", req.label),
            }));
        } else {
            // On Windows, winget may return non-zero even on success (e.g., already installed)
            let combined = format!("{stdout}{stderr}");
            let likely_ok = combined.contains("already installed")
                || combined.contains("No applicable update")
                || combined.contains("No available upgrade")
                || combined.contains("already an App at")
                || combined.contains("is already installed");
            results.push(serde_json::json!({
                "key": req.key,
                "status": if likely_ok { "installed" } else { "error" },
                "command": final_cmd,
                "exit_code": exit_code,
                "message": if likely_ok {
                    format!("{} is already installed", req.label)
                } else {
                    let msg = stderr.chars().take(500).collect::<String>();
                    format!("Install failed (exit {}): {}", exit_code, msg.trim())
                },
            }));
        }
    }

    // On Windows, refresh PATH to pick up newly installed binaries from winget/pip
    #[cfg(windows)]
    {
        let home = std::env::var("USERPROFILE").unwrap_or_default();
        if !home.is_empty() {
            let winget_pkgs =
                std::path::Path::new(&home).join("AppData\\Local\\Microsoft\\WinGet\\Packages");
            if winget_pkgs.is_dir() {
                let mut extra_paths = Vec::new();
                if let Ok(entries) = std::fs::read_dir(&winget_pkgs) {
                    for entry in entries.flatten() {
                        let pkg_dir = entry.path();
                        // Look for bin/ subdirectory (ffmpeg style)
                        if let Ok(sub_entries) = std::fs::read_dir(&pkg_dir) {
                            for sub in sub_entries.flatten() {
                                let bin_dir = sub.path().join("bin");
                                if bin_dir.is_dir() {
                                    extra_paths.push(bin_dir.to_string_lossy().to_string());
                                }
                            }
                        }
                        // Direct exe in package dir (yt-dlp style)
                        if std::fs::read_dir(&pkg_dir)
                            .map(|rd| {
                                rd.flatten().any(|e| {
                                    e.path().extension().map(|x| x == "exe").unwrap_or(false)
                                })
                            })
                            .unwrap_or(false)
                        {
                            extra_paths.push(pkg_dir.to_string_lossy().to_string());
                        }
                    }
                }
                // Also add pip Scripts dir
                let pip_scripts =
                    std::path::Path::new(&home).join("AppData\\Local\\Programs\\Python");
                if pip_scripts.is_dir() {
                    if let Ok(entries) = std::fs::read_dir(&pip_scripts) {
                        for entry in entries.flatten() {
                            let scripts = entry.path().join("Scripts");
                            if scripts.is_dir() {
                                extra_paths.push(scripts.to_string_lossy().to_string());
                            }
                        }
                    }
                }
                if !extra_paths.is_empty() {
                    let current_path = std::env::var("PATH").unwrap_or_default();
                    let new_path = format!("{};{}", extra_paths.join(";"), current_path);
                    std::env::set_var("PATH", &new_path);
                    tracing::info!(
                        added = extra_paths.len(),
                        "Refreshed PATH with winget/pip directories"
                    );
                }
            }
        }
    }

    // Re-check requirements after installation
    let reqs_after = state
        .kernel
        .hand_registry
        .check_requirements(&hand_id)
        .unwrap_or_default();
    let all_satisfied = reqs_after.iter().all(|(_, ok)| *ok);

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "hand_id": def.id,
            "results": results,
            "requirements_met": all_satisfied,
            "requirements": reqs_after.iter().map(|(r, ok)| {
                serde_json::json!({
                    "key": r.key,
                    "label": r.label,
                    "satisfied": ok,
                })
            }).collect::<Vec<_>>(),
        })),
    )
}

/// POST /api/hands/install — Install a hand from TOML content.
pub async fn install_hand(
    State(state): State<Arc<AppState>>,
    ext: Option<Extension<RequestId>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/hands/install";
    let toml_content = body["toml_content"].as_str().unwrap_or("");
    let skill_content = body["skill_content"].as_str().unwrap_or("");

    if toml_content.is_empty() {
        return api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Missing toml_content",
            "JSON body must include non-empty 'toml_content'.".to_string(),
            Some("Optional 'skill_content' can accompany the hand TOML."),
        );
    }

    match state
        .kernel
        .hand_registry
        .install_from_content(toml_content, skill_content)
    {
        Ok(def) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "id": def.id,
                "name": def.name,
                "description": def.description,
                "category": format!("{:?}", def.category),
            })),
        ),
        Err(e) => api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Hand install failed",
            format!("{e}"),
            Some("Validate hand TOML against the hand schema."),
        ),
    }
}

/// POST /api/hands/upsert — Install or update a hand definition.
///
/// Like `install_hand` but overwrites an existing definition with the same ID.
/// Active instances are NOT automatically restarted — deactivate + reactivate
/// to pick up the new definition.
pub async fn upsert_hand(
    State(state): State<Arc<AppState>>,
    ext: Option<Extension<RequestId>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/hands/upsert";
    let toml_content = body["toml_content"].as_str().unwrap_or("");
    let skill_content = body["skill_content"].as_str().unwrap_or("");

    if toml_content.is_empty() {
        return api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Missing toml_content",
            "JSON body must include non-empty 'toml_content'.".to_string(),
            Some("Optional 'skill_content' can accompany the hand TOML."),
        );
    }

    match state
        .kernel
        .hand_registry
        .upsert_from_content(toml_content, skill_content)
    {
        Ok(def) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "id": def.id,
                "name": def.name,
                "description": def.description,
                "category": format!("{:?}", def.category),
            })),
        ),
        Err(e) => api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Hand upsert failed",
            format!("{e}"),
            Some("Validate hand TOML against the hand schema."),
        ),
    }
}

/// POST /api/hands/{hand_id}/activate — Activate a hand (spawns agent).
pub async fn activate_hand(
    State(state): State<Arc<AppState>>,
    Path(hand_id): Path<String>,
    ext: Option<Extension<RequestId>>,
    body: Option<Json<openfang_hands::ActivateHandRequest>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/hands/:hand_id/activate";
    let config = body.map(|b| b.0.config).unwrap_or_default();

    match state.kernel.activate_hand(&hand_id, config) {
        Ok(instance) => {
            // If the hand agent has a non-reactive schedule (autonomous hands),
            // start its background loop so it begins running immediately.
            if let Some(agent_id) = instance.agent_id {
                let entry = state
                    .kernel
                    .registry
                    .list()
                    .into_iter()
                    .find(|e| e.id == agent_id);
                if let Some(entry) = entry {
                    if !matches!(
                        entry.manifest.schedule,
                        openfang_types::agent::ScheduleMode::Reactive
                    ) {
                        state.kernel.start_background_for_agent(
                            agent_id,
                            &entry.name,
                            &entry.manifest.schedule,
                        );
                    }
                }
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "instance_id": instance.instance_id,
                    "hand_id": instance.hand_id,
                    "status": format!("{}", instance.status),
                    "agent_id": instance.agent_id.map(|a| a.to_string()),
                    "agent_name": instance.agent_name,
                    "activated_at": instance.activated_at.to_rfc3339(),
                })),
            )
        }
        Err(e) => api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Activate hand failed",
            format!("{e}"),
            Some("Check hand requirements and agent spawn limits."),
        ),
    }
}

/// POST /api/hands/instances/{id}/pause — Pause a hand instance.
pub async fn pause_hand(
    State(state): State<Arc<AppState>>,
    Path(id): Path<uuid::Uuid>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/hands/instances/:id/pause";
    match state.kernel.pause_hand(id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "paused", "instance_id": id})),
        ),
        Err(e) => api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Pause hand failed",
            format!("{e}"),
            Some("Verify the instance id from GET /api/hands/instances."),
        ),
    }
}

/// POST /api/hands/instances/{id}/resume — Resume a paused hand instance.
pub async fn resume_hand(
    State(state): State<Arc<AppState>>,
    Path(id): Path<uuid::Uuid>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/hands/instances/:id/resume";
    match state.kernel.resume_hand(id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "resumed", "instance_id": id})),
        ),
        Err(e) => api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Resume hand failed",
            format!("{e}"),
            Some("Verify the instance id from GET /api/hands/instances."),
        ),
    }
}

/// DELETE /api/hands/instances/{id} — Deactivate a hand (kills agent).
pub async fn deactivate_hand(
    State(state): State<Arc<AppState>>,
    Path(id): Path<uuid::Uuid>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/hands/instances/:id";
    match state.kernel.deactivate_hand(id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "deactivated", "instance_id": id})),
        ),
        Err(e) => api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Deactivate hand failed",
            format!("{e}"),
            Some("Verify the instance id from GET /api/hands/instances."),
        ),
    }
}

/// GET /api/hands/{hand_id}/settings — Get settings schema and current values for a hand.
pub async fn get_hand_settings(
    State(state): State<Arc<AppState>>,
    Path(hand_id): Path<String>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/hands/:hand_id/settings";
    let settings_status = match state
        .kernel
        .hand_registry
        .check_settings_availability(&hand_id)
    {
        Ok(s) => s,
        Err(_) => {
            return api_json_error(
                StatusCode::NOT_FOUND,
                &rid,
                PATH,
                "Hand not found",
                format!("No hand definition for id '{hand_id}'."),
                Some("Use GET /api/hands to list available hands."),
            );
        }
    };

    // Find active instance config values (if any)
    let instance_config: std::collections::HashMap<String, serde_json::Value> = state
        .kernel
        .hand_registry
        .list_instances()
        .iter()
        .find(|i| i.hand_id == hand_id)
        .map(|i| i.config.clone())
        .unwrap_or_default();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "hand_id": hand_id,
            "settings": settings_status,
            "current_values": instance_config,
        })),
    )
}

/// PUT /api/hands/{hand_id}/settings — Update settings for a hand instance.
pub async fn update_hand_settings(
    State(state): State<Arc<AppState>>,
    Path(hand_id): Path<String>,
    ext: Option<Extension<RequestId>>,
    Json(config): Json<std::collections::HashMap<String, serde_json::Value>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/hands/:hand_id/settings";
    // Find active instance for this hand
    let instance_id = state
        .kernel
        .hand_registry
        .list_instances()
        .iter()
        .find(|i| i.hand_id == hand_id)
        .map(|i| i.instance_id);

    match instance_id {
        Some(id) => match state.kernel.hand_registry.update_config(id, config.clone()) {
            Ok(()) => (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "ok",
                    "hand_id": hand_id,
                    "instance_id": id,
                    "config": config,
                })),
            ),
            Err(e) => api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Hand settings update failed",
                format!("{e}"),
                Some("Validate values against the hand settings schema."),
            ),
        },
        None => api_json_error(
            StatusCode::NOT_FOUND,
            &rid,
            PATH,
            "No active hand instance",
            format!("No active instance for hand '{hand_id}'. Activate the hand first."),
            Some("POST /api/hands/:hand_id/activate to create an instance."),
        ),
    }
}

/// GET /api/hands/instances/{id}/stats — Get dashboard stats for a hand instance.
pub async fn hand_stats(
    State(state): State<Arc<AppState>>,
    Path(id): Path<uuid::Uuid>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/hands/instances/:id/stats";
    let instance = match state.kernel.hand_registry.get_instance(id) {
        Some(i) => i,
        None => {
            return api_json_error(
                StatusCode::NOT_FOUND,
                &rid,
                PATH,
                "Instance not found",
                format!("No hand instance for id {id}."),
                Some("List instances via GET /api/hands/instances."),
            );
        }
    };

    let def = match state.kernel.hand_registry.get_definition(&instance.hand_id) {
        Some(d) => d,
        None => {
            return api_json_error(
                StatusCode::NOT_FOUND,
                &rid,
                PATH,
                "Hand definition not found",
                format!("Missing definition for hand '{}'.", instance.hand_id),
                Some("Reinstall or repair the hand package."),
            );
        }
    };

    let agent_id = match instance.agent_id {
        Some(aid) => aid,
        None => {
            return (
                StatusCode::OK,
                Json(serde_json::json!({
                    "instance_id": id,
                    "hand_id": instance.hand_id,
                    "metrics": {},
                })),
            );
        }
    };

    // Read dashboard metrics from shared structured memory (memory_store uses shared namespace)
    let shared_id = openfang_kernel::kernel::shared_memory_agent_id();
    let mut metrics = serde_json::Map::new();
    for metric in &def.dashboard.metrics {
        // Try shared memory first (where memory_store tool writes), fall back to agent-specific
        let value = state
            .kernel
            .memory
            .structured_get(shared_id, &metric.memory_key)
            .ok()
            .flatten()
            .or_else(|| {
                state
                    .kernel
                    .memory
                    .structured_get(agent_id, &metric.memory_key)
                    .ok()
                    .flatten()
            })
            .unwrap_or(serde_json::Value::Null);
        metrics.insert(
            metric.label.clone(),
            serde_json::json!({
                "value": value,
                "format": metric.format,
            }),
        );
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "instance_id": id,
            "hand_id": instance.hand_id,
            "status": format!("{}", instance.status),
            "agent_id": agent_id.to_string(),
            "metrics": metrics,
        })),
    )
}

/// GET /api/hands/instances/{id}/browser — Get live browser state for a hand instance.
pub async fn hand_instance_browser(
    State(state): State<Arc<AppState>>,
    Path(id): Path<uuid::Uuid>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/hands/instances/:id/browser";
    // 1. Look up instance
    let instance = match state.kernel.hand_registry.get_instance(id) {
        Some(i) => i,
        None => {
            return api_json_error(
                StatusCode::NOT_FOUND,
                &rid,
                PATH,
                "Instance not found",
                format!("No hand instance for id {id}."),
                Some("List instances via GET /api/hands/instances."),
            );
        }
    };

    // 2. Get agent_id
    let agent_id = match instance.agent_id {
        Some(aid) => aid,
        None => {
            return (StatusCode::OK, Json(serde_json::json!({"active": false})));
        }
    };

    let agent_id_str = agent_id.to_string();

    // 3. Check if a browser session exists (without creating one)
    if !state.kernel.browser_ctx.has_session(&agent_id_str) {
        return (StatusCode::OK, Json(serde_json::json!({"active": false})));
    }

    // 4. Send ReadPage command to get page info
    let mut url = String::new();
    let mut title = String::new();
    let mut content = String::new();

    match state
        .kernel
        .browser_ctx
        .send_command(
            &agent_id_str,
            openfang_runtime::browser::BrowserCommand::ReadPage,
        )
        .await
    {
        Ok(resp) if resp.success => {
            if let Some(data) = &resp.data {
                url = data["url"].as_str().unwrap_or("").to_string();
                title = data["title"].as_str().unwrap_or("").to_string();
                content = data["content"].as_str().unwrap_or("").to_string();
                // Truncate content to avoid huge payloads (UTF-8 safe)
                if content.len() > 2000 {
                    content = format!(
                        "{}... (truncated)",
                        openfang_types::truncate_str(&content, 2000)
                    );
                }
            }
        }
        Ok(_) => {}  // Non-success: leave defaults
        Err(_) => {} // Error: leave defaults
    }

    // 5. Send Screenshot command to get visual state
    let mut screenshot_base64 = String::new();

    match state
        .kernel
        .browser_ctx
        .send_command(
            &agent_id_str,
            openfang_runtime::browser::BrowserCommand::Screenshot,
        )
        .await
    {
        Ok(resp) if resp.success => {
            if let Some(data) = &resp.data {
                screenshot_base64 = data["image_base64"].as_str().unwrap_or("").to_string();
            }
        }
        Ok(_) => {}
        Err(_) => {}
    }

    // 6. Return combined state
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "active": true,
            "url": url,
            "title": title,
            "content": content,
            "screenshot_base64": screenshot_base64,
        })),
    )
}

// ---------------------------------------------------------------------------
// MCP server endpoints
// ---------------------------------------------------------------------------

/// GET /api/mcp/servers — List configured MCP servers and their tools.
pub async fn list_mcp_servers(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // Get configured servers from the effective merged list (manual config.toml + integrations)
    let effective = state
        .kernel
        .effective_mcp_servers
        .read()
        .map(|s| s.clone())
        .unwrap_or_default();
    let config_servers: Vec<serde_json::Value> = effective
        .iter()
        .map(|s| {
            let transport = match &s.transport {
                openfang_types::config::McpTransportEntry::Stdio { command, args } => {
                    serde_json::json!({
                        "type": "stdio",
                        "command": command,
                        "args": args,
                    })
                }
                openfang_types::config::McpTransportEntry::Sse { url } => {
                    serde_json::json!({
                        "type": "sse",
                        "url": url,
                    })
                }
                openfang_types::config::McpTransportEntry::Http { url } => {
                    serde_json::json!({
                        "type": "http",
                        "url": url,
                    })
                }
            };
            serde_json::json!({
                "name": s.name,
                "transport": transport,
                "timeout_secs": s.timeout_secs,
                "env": s.env,
            })
        })
        .collect();

    // Get connected servers and their tools from the live MCP connections
    let connections = state.kernel.mcp_connections.lock().await;
    let evaluated = openfang_runtime::mcp_readiness::evaluate_from_connections(&connections);
    let readiness_json = serde_json::to_value(&evaluated.report).unwrap_or_else(|_| {
        serde_json::json!({ "version": openfang_runtime::mcp_readiness::READINESS_SCHEMA_VERSION, "checks": {} })
    });
    let calendar_readiness_json =
        serde_json::to_value(&evaluated.calendar_readiness).unwrap_or(serde_json::Value::Null);

    let connected: Vec<serde_json::Value> = connections
        .iter()
        .map(|conn| {
            let tools: Vec<serde_json::Value> = conn
                .tools()
                .iter()
                .map(|t| {
                    let flags = openfang_runtime::mcp_readiness::flags_for_tool(
                        conn.name(),
                        &t.name,
                        &t.description,
                    );
                    let is_calendar_like = flags
                        .check_ids
                        .contains(openfang_runtime::mcp_readiness::CHECK_ID_CALENDAR);
                    let mut readiness_map = serde_json::Map::new();
                    for id in &flags.check_ids {
                        readiness_map.insert(id.clone(), serde_json::json!(true));
                    }
                    serde_json::json!({
                        "name": t.name,
                        "description": t.description,
                        "calendar_like": is_calendar_like,
                        "readiness": serde_json::Value::Object(readiness_map),
                    })
                })
                .collect();
            let has_calendar_tools = tools
                .iter()
                .any(|t| t["calendar_like"].as_bool().unwrap_or(false));
            serde_json::json!({
                "name": conn.name(),
                "tools_count": tools.len(),
                "tools": tools,
                "connected": true,
                "calendar_capable": has_calendar_tools,
            })
        })
        .collect();

    Json(serde_json::json!({
        "configured": config_servers,
        "connected": connected,
        "total_configured": config_servers.len(),
        "total_connected": connected.len(),
        "readiness": readiness_json,
        "calendar_readiness": calendar_readiness_json,
    }))
}

// ---------------------------------------------------------------------------
// Audit endpoints
// ---------------------------------------------------------------------------

/// GET /api/audit/recent — Get recent audit log entries.
pub async fn audit_recent(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let n: usize = params
        .get("n")
        .and_then(|v| v.parse().ok())
        .unwrap_or(50)
        .min(1000); // Cap at 1000

    let entries = state.kernel.audit_log.recent(n);
    let tip = state.kernel.audit_log.tip_hash();
    let q = params
        .get("q")
        .map(|s| s.to_lowercase())
        .filter(|s| !s.is_empty());

    let items: Vec<serde_json::Value> = entries
        .iter()
        .filter(|e| {
            q.as_ref().is_none_or(|needle| {
                let hay = format!("{:?} {} {} {}", e.action, e.detail, e.outcome, e.agent_id)
                    .to_lowercase();
                hay.contains(needle.as_str())
            })
        })
        .map(|e| {
            serde_json::json!({
                "seq": e.seq,
                "timestamp": e.timestamp,
                "agent_id": e.agent_id,
                "action": format!("{:?}", e.action),
                "detail": e.detail,
                "outcome": e.outcome,
                "hash": e.hash,
            })
        })
        .collect();

    Json(serde_json::json!({
        "entries": items,
        "total": state.kernel.audit_log.len(),
        "tip_hash": tip,
    }))
}

/// GET /api/audit/verify — Verify the audit chain integrity.
pub async fn audit_verify(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let entry_count = state.kernel.audit_log.len();
    match state.kernel.audit_log.verify_integrity() {
        Ok(()) => {
            if entry_count == 0 {
                // SECURITY: Warn that an empty audit log has no forensic value
                Json(serde_json::json!({
                    "valid": true,
                    "entries": 0,
                    "warning": "Audit log is empty — no events have been recorded yet",
                    "tip_hash": state.kernel.audit_log.tip_hash(),
                }))
            } else {
                Json(serde_json::json!({
                    "valid": true,
                    "entries": entry_count,
                    "tip_hash": state.kernel.audit_log.tip_hash(),
                }))
            }
        }
        Err(msg) => Json(serde_json::json!({
            "valid": false,
            "error": msg,
            "entries": entry_count,
        })),
    }
}

/// GET /api/cron/runs — Recent scheduler audit rows (CronJobRun / Output / Failure).
pub async fn cron_runs(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    use openfang_runtime::audit::AuditAction;
    let n: usize = params
        .get("n")
        .and_then(|v| v.parse().ok())
        .unwrap_or(200)
        .min(2000);
    let entries = state.kernel.audit_log.recent(n);
    let runs: Vec<serde_json::Value> = entries
        .into_iter()
        .filter(|e| {
            matches!(
                e.action,
                AuditAction::CronJobRun | AuditAction::CronJobOutput | AuditAction::CronJobFailure
            )
        })
        .map(|e| {
            serde_json::json!({
                "seq": e.seq,
                "timestamp": e.timestamp,
                "agent_id": e.agent_id,
                "action": format!("{:?}", e.action),
                "detail": e.detail,
                "outcome": e.outcome,
            })
        })
        .collect();
    let count = runs.len();
    Json(serde_json::json!({ "runs": runs, "count": count }))
}

/// GET /api/observability/snapshot — Compact JSON for dashboards (queue depth, agents, channels, cron tick).
pub async fn observability_snapshot(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    use openfang_types::agent::AgentState;

    let agents = state.kernel.registry.list();
    let agent_count = agents.len();
    let running_agents = agents
        .iter()
        .filter(|a| matches!(a.state, AgentState::Running))
        .count();

    let live_channels = state.channels_config.read().await;
    let mut channels_configured = 0u32;
    let mut channels_ready = 0u32;
    for meta in CHANNEL_REGISTRY {
        let configured = is_channel_configured(&live_channels, meta.name);
        if configured {
            channels_configured += 1;
        }
        let has_token = meta
            .fields
            .iter()
            .filter(|f| f.required && f.env_var.is_some())
            .all(|f| {
                f.env_var
                    .map(|ev| std::env::var(ev).map(|v| !v.is_empty()).unwrap_or(false))
                    .unwrap_or(true)
            });
        if configured && has_token {
            channels_ready += 1;
        }
    }

    Json(serde_json::json!({
        "agent_count": agent_count,
        "running_agents": running_agents,
        "pending_approvals": state.kernel.approval_manager.pending_count(),
        "cron_jobs_registered": state.kernel.cron_scheduler.total_jobs(),
        "last_cron_scheduler_tick": state.kernel.last_cron_scheduler_tick_rfc3339(),
        "channels_configured": channels_configured,
        "channels_ready": channels_ready,
        "uptime_secs": state.started_at.elapsed().as_secs(),
    }))
}

/// GET /api/logs/stream — SSE endpoint for real-time audit log streaming.
///
/// Streams new audit entries as Server-Sent Events. Accepts optional query
/// parameters for filtering:
///   - `level`  — filter by classified level (info, warn, error)
///   - `filter` — text substring filter across action/detail/agent_id
///   - `token`  — auth token (for EventSource clients that cannot set headers)
///
/// A heartbeat ping is sent every 15 seconds to keep the connection alive.
/// The endpoint polls the audit log every second and sends only new entries
/// (tracked by sequence number). On first connect, existing entries are sent
/// as a backfill so the client has immediate context.
pub async fn logs_stream(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> axum::response::Response {
    use axum::response::sse::{Event, KeepAlive, Sse};

    let level_filter = params.get("level").cloned().unwrap_or_default();
    let text_filter = params
        .get("filter")
        .cloned()
        .unwrap_or_default()
        .to_lowercase();

    let (tx, rx) = tokio::sync::mpsc::channel::<
        Result<axum::response::sse::Event, std::convert::Infallible>,
    >(256);

    tokio::spawn(async move {
        let mut last_seq: u64 = 0;
        let mut first_poll = true;

        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;

            let entries = state.kernel.audit_log.recent(200);

            for entry in &entries {
                // On first poll, send all existing entries as backfill.
                // After that, only send entries newer than last_seq.
                if !first_poll && entry.seq <= last_seq {
                    continue;
                }

                let action_str = format!("{:?}", entry.action);

                // Apply level filter
                if !level_filter.is_empty() {
                    let classified = classify_audit_level(&action_str);
                    if classified != level_filter {
                        continue;
                    }
                }

                // Apply text filter
                if !text_filter.is_empty() {
                    let haystack = format!("{} {} {}", action_str, entry.detail, entry.agent_id)
                        .to_lowercase();
                    if !haystack.contains(&text_filter) {
                        continue;
                    }
                }

                let json = serde_json::json!({
                    "seq": entry.seq,
                    "timestamp": entry.timestamp,
                    "agent_id": entry.agent_id,
                    "action": action_str,
                    "detail": entry.detail,
                    "outcome": entry.outcome,
                    "hash": entry.hash,
                });
                let data = serde_json::to_string(&json).unwrap_or_default();
                if tx.send(Ok(Event::default().data(data))).await.is_err() {
                    return; // Client disconnected
                }
            }

            // Update tracking state
            if let Some(last) = entries.last() {
                last_seq = last.seq;
            }
            first_poll = false;
        }
    });

    let rx_stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    Sse::new(rx_stream)
        .keep_alive(
            KeepAlive::new()
                .interval(std::time::Duration::from_secs(15))
                .text("ping"),
        )
        .into_response()
}

fn resolve_daemon_tracing_log_path(home: &std::path::Path) -> Option<std::path::PathBuf> {
    let daemon = home.join("logs").join("daemon.log");
    if daemon.is_file() {
        return Some(daemon);
    }
    let tui = home.join("tui.log");
    if tui.is_file() {
        return Some(tui);
    }
    None
}

/// Rank of a `tracing-subscriber` default-format line for minimum-level filtering (higher = more severe).
fn tracing_line_level_rank(line: &str) -> u8 {
    if line.contains(" ERROR ") {
        return 5;
    }
    if line.contains(" WARN ") {
        return 4;
    }
    if line.contains(" INFO ") {
        return 3;
    }
    if line.contains("DEBUG") {
        return 2;
    }
    if line.contains("TRACE") {
        return 1;
    }
    3
}

fn tracing_min_level_floor(min: &str) -> u8 {
    match min {
        "error" => 5,
        "warn" => 4,
        "info" => 3,
        "debug" => 2,
        "trace" => 1,
        _ => 1,
    }
}

fn daemon_line_matches(line: &str, min_level: &str, text_sub: &str) -> bool {
    if !text_sub.is_empty() && !line.to_lowercase().contains(text_sub) {
        return false;
    }
    if min_level.is_empty() {
        return true;
    }
    tracing_line_level_rank(line) >= tracing_min_level_floor(min_level)
}

fn read_daemon_log_tail(path: &std::path::Path, max_lines: usize) -> Vec<String> {
    use std::io::{Read, Seek, SeekFrom};
    const MAX_READ: u64 = 512 * 1024;
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };
    let len = match f.metadata() {
        Ok(m) => m.len(),
        Err(_) => return Vec::new(),
    };
    let read_start = len.saturating_sub(MAX_READ);
    if f.seek(SeekFrom::Start(read_start)).is_err() {
        return Vec::new();
    }
    let take = len - read_start;
    let mut buf = Vec::new();
    if f.take(take).read_to_end(&mut buf).is_err() {
        return Vec::new();
    }
    let mut s = String::from_utf8_lossy(&buf).into_owned();
    if read_start > 0 {
        if let Some(idx) = s.find('\n') {
            s = s[idx + 1..].to_string();
        }
    }
    let mut lines: Vec<String> = s.lines().map(std::string::ToString::to_string).collect();
    if lines.len() > max_lines {
        let skip = lines.len() - max_lines;
        lines.drain(..skip);
    }
    lines
}

/// GET /api/logs/daemon/recent — last lines from `~/.armaraos/logs/daemon.log` (or `tui.log` fallback).
///
/// Query: `lines` (1–2000, default 200), `level` (trace|debug|info|warn|error), `filter` (substring).
pub async fn daemon_logs_recent(
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let home = openfang_kernel::config::openfang_home();
    let path_opt = resolve_daemon_tracing_log_path(&home);
    let rel_display = path_opt
        .as_ref()
        .and_then(|p| p.strip_prefix(&home).ok().map(|s| s.display().to_string()));

    let n: usize = params
        .get("lines")
        .and_then(|s| s.parse().ok())
        .unwrap_or(200)
        .clamp(1, 2000);

    let min_level = params
        .get("level")
        .cloned()
        .unwrap_or_default()
        .to_lowercase();
    let text = params
        .get("filter")
        .cloned()
        .unwrap_or_default()
        .to_lowercase();

    let lines: Vec<serde_json::Value> = match &path_opt {
        None => Vec::new(),
        Some(p) => {
            let raw = read_daemon_log_tail(p, 8000);
            let filtered: Vec<String> = raw
                .into_iter()
                .filter(|l| daemon_line_matches(l, &min_level, &text))
                .collect();
            let skip = filtered.len().saturating_sub(n);
            filtered
                .into_iter()
                .skip(skip)
                .enumerate()
                .map(|(i, line)| {
                    serde_json::json!({
                        "seq": (skip + i + 1) as u64,
                        "line": line,
                    })
                })
                .collect()
        }
    };

    Json(serde_json::json!({
        "path": rel_display,
        "lines": lines,
    }))
}

/// GET /api/logs/daemon/stream — SSE tail of daemon tracing log (same files as [`daemon_logs_recent`]).
///
/// Query: `level`, `filter`, optional `token=` (remote clients when `api_key` is set).
pub async fn daemon_logs_stream(
    Query(params): Query<HashMap<String, String>>,
) -> axum::response::Response {
    use axum::response::sse::{Event, KeepAlive, Sse};
    use std::convert::Infallible;
    use std::io::{Read, Seek, SeekFrom};
    use std::time::Duration;

    let home = openfang_kernel::config::openfang_home();
    let min_level = params
        .get("level")
        .cloned()
        .unwrap_or_default()
        .to_lowercase();
    let text_filter = params
        .get("filter")
        .cloned()
        .unwrap_or_default()
        .to_lowercase();

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(256);

    tokio::spawn(async move {
        let mut offset: u64 = 0;
        let mut initialized = false;
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let Some(pth) = resolve_daemon_tracing_log_path(&home) else {
                continue;
            };
            if !initialized {
                initialized = true;
                let backfill = read_daemon_log_tail(&pth, 300);
                for line in backfill {
                    if !daemon_line_matches(&line, &min_level, &text_filter) {
                        continue;
                    }
                    let data = serde_json::to_string(&serde_json::json!({ "line": line }))
                        .unwrap_or_default();
                    if tx.send(Ok(Event::default().data(data))).await.is_err() {
                        return;
                    }
                }
                offset = std::fs::metadata(&pth).map(|m| m.len()).unwrap_or(0);
                continue;
            }
            let meta = match std::fs::metadata(&pth) {
                Ok(m) => m,
                Err(_) => continue,
            };
            let len = meta.len();
            if len < offset {
                offset = 0;
            }
            if len <= offset {
                continue;
            }
            let mut f = match std::fs::File::open(&pth) {
                Ok(f) => f,
                Err(_) => continue,
            };
            if f.seek(SeekFrom::Start(offset)).is_err() {
                continue;
            }
            let mut chunk = String::new();
            let take = len - offset;
            if f.take(take).read_to_string(&mut chunk).is_err() {
                continue;
            }
            offset = len;
            for line in chunk.lines() {
                if !daemon_line_matches(line, &min_level, &text_filter) {
                    continue;
                }
                let data =
                    serde_json::to_string(&serde_json::json!({ "line": line })).unwrap_or_default();
                if tx.send(Ok(Event::default().data(data))).await.is_err() {
                    return;
                }
            }
        }
    });

    let rx_stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    Sse::new(rx_stream)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("ping"),
        )
        .into_response()
}

/// GET /api/events/stream — SSE stream of kernel [`openfang_types::event::Event`] values (JSON per message).
///
/// On connect, sends up to 100 recent events from the bus history (oldest first), then live events.
/// Optional query: `token=` for clients that cannot set `Authorization` (same as [`logs_stream`]).
/// Heartbeat every 15s.
pub async fn kernel_events_stream(State(state): State<Arc<AppState>>) -> axum::response::Response {
    use axum::response::sse::{Event as SseWireEvent, KeepAlive, Sse};
    use std::convert::Infallible;
    use tokio::sync::broadcast::error::RecvError;

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<SseWireEvent, Infallible>>(256);

    let mut historical = state.kernel.event_bus.history(100).await;
    historical.reverse(); // chronological (oldest first)

    let mut rx_bus = state.kernel.event_bus.subscribe_all();
    tokio::spawn(async move {
        for ev in historical {
            if let Ok(json) = serde_json::to_string(&ev) {
                if tx
                    .send(Ok(SseWireEvent::default().data(json)))
                    .await
                    .is_err()
                {
                    return;
                }
            }
        }

        loop {
            match rx_bus.recv().await {
                Ok(ev) => {
                    if let Ok(json) = serde_json::to_string(&ev) {
                        if tx
                            .send(Ok(SseWireEvent::default().data(json)))
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                }
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            }
        }
    });

    let rx_stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    Sse::new(rx_stream)
        .keep_alive(
            KeepAlive::new()
                .interval(std::time::Duration::from_secs(15))
                .text("ping"),
        )
        .into_response()
}

/// Classify an audit action string into a level (info, warn, error).
fn classify_audit_level(action: &str) -> &'static str {
    let a = action.to_lowercase();
    if a.contains("error") || a.contains("fail") || a.contains("crash") || a.contains("denied") {
        "error"
    } else if a.contains("warn") || a.contains("block") || a.contains("kill") {
        "warn"
    } else {
        "info"
    }
}

// ---------------------------------------------------------------------------
// Peer endpoints
// ---------------------------------------------------------------------------

/// GET /api/peers — List known OFP peers.
pub async fn list_peers(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // Peers are tracked in the wire module's PeerRegistry.
    // The kernel doesn't directly hold a PeerRegistry, so we return an empty list
    // unless one is available. The API server can be extended to inject a registry.
    if let Some(ref peer_registry) = state.peer_registry {
        let peers: Vec<serde_json::Value> = peer_registry
            .all_peers()
            .iter()
            .map(|p| {
                serde_json::json!({
                    "node_id": p.node_id,
                    "node_name": p.node_name,
                    "address": p.address.to_string(),
                    "state": format!("{:?}", p.state),
                    "agents": p.agents.iter().map(|a| serde_json::json!({
                        "id": a.id,
                        "name": a.name,
                    })).collect::<Vec<_>>(),
                    "connected_at": p.connected_at.to_rfc3339(),
                    "protocol_version": p.protocol_version,
                })
            })
            .collect();
        Json(serde_json::json!({"peers": peers, "total": peers.len()}))
    } else {
        Json(serde_json::json!({"peers": [], "total": 0}))
    }
}

/// GET /api/network/status — OFP network status summary.
pub async fn network_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let enabled = state.kernel.config.network_enabled
        && !state.kernel.config.network.shared_secret.is_empty();

    let (node_id, listen_address, connected_peers, total_peers) =
        if let Some(peer_node) = state.kernel.peer_node.get() {
            let registry = peer_node.registry();
            (
                peer_node.node_id().to_string(),
                peer_node.local_addr().to_string(),
                registry.connected_count(),
                registry.total_count(),
            )
        } else {
            (String::new(), String::new(), 0, 0)
        };

    Json(serde_json::json!({
        "enabled": enabled,
        "node_id": node_id,
        "listen_address": listen_address,
        "connected_peers": connected_peers,
        "total_peers": total_peers,
    }))
}

/// GET /api/system/daemon-resources — Live CPU + RSS memory for the daemon process.
pub async fn daemon_resources(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(state.daemon_resources.snapshot())
}

/// GET /api/system/network-hints — VPN/proxy/tunnel hints from host network interfaces.
pub async fn system_network_hints() -> impl IntoResponse {
    Json(crate::network_hints::collect())
}

// ---------------------------------------------------------------------------
// Tools endpoint
// ---------------------------------------------------------------------------

/// GET /api/tools — List all tool definitions (built-in + MCP).
pub async fn list_tools(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let mut tools: Vec<serde_json::Value> = builtin_tool_definitions()
        .iter()
        .map(|t| {
            serde_json::json!({
                "name": t.name,
                "description": t.description,
                "input_schema": t.input_schema,
            })
        })
        .collect();

    // Include MCP tools so they're visible in Settings -> Tools
    if let Ok(mcp_tools) = state.kernel.mcp_tools.lock() {
        for t in mcp_tools.iter() {
            tools.push(serde_json::json!({
                "name": t.name,
                "description": t.description,
                "input_schema": t.input_schema,
                "source": "mcp",
            }));
        }
    }

    Json(serde_json::json!({"tools": tools, "total": tools.len()}))
}

// ---------------------------------------------------------------------------
// ArmaraOS home directory browser (read-only, confined to config.home_dir)
// ---------------------------------------------------------------------------

/// Max entries returned per list call (sorted; remainder omitted with `truncated: true`).
const ARMARAOS_HOME_BROWSER_MAX_ENTRIES: usize = 4000;
/// Max bytes returned for a single file read (prevents huge memory use).
const ARMARAOS_HOME_BROWSER_MAX_READ_BYTES: u64 = 512 * 1024;
/// Max bytes for `GET /api/armaraos-home/download` (diagnostics zips may include DB).
const ARMARAOS_HOME_DOWNLOAD_MAX_BYTES: u64 = 256 * 1024 * 1024;

#[derive(serde::Deserialize)]
pub struct ArmaraosHomeBrowserQuery {
    #[serde(default)]
    pub path: String,
}

fn normalize_armaraos_home_rel(raw: &str) -> String {
    raw.trim().trim_start_matches(['/', '\\']).to_string()
}

fn armaraos_home_rel_join(listing_dir: &str, name: &str) -> String {
    let d = listing_dir.trim();
    if d.is_empty() {
        name.to_string()
    } else {
        format!("{d}/{name}")
    }
}

/// Paths that are never writable from the dashboard, even if matched by `dashboard.home_editable_globs`.
fn armaraos_home_edit_path_blocked(rel: &str) -> bool {
    let n = rel.replace('\\', "/");
    if n.starts_with("data/") {
        return true;
    }
    matches!(
        n.as_str(),
        ".env" | "secrets.env" | "vault.enc" | "config.toml" | "daemon.json"
    ) || n.starts_with(".env/")
        || n.ends_with("/.env")
}

fn armaraos_home_build_edit_globset(
    dashboard: &openfang_types::config::DashboardConfig,
) -> Result<Option<GlobSet>, String> {
    let mut b = GlobSetBuilder::new();
    let mut any = false;
    for p in &dashboard.home_editable_globs {
        let t = p.trim();
        if t.is_empty() {
            continue;
        }
        let g = Glob::new(t).map_err(|e| format!("Invalid glob {t:?}: {e}"))?;
        b.add(g);
        any = true;
    }
    if !any {
        return Ok(None);
    }
    b.build().map_err(|e| e.to_string()).map(Some)
}

fn armaraos_home_rel_matches_edit_globs(rel: &str, set: &GlobSet) -> bool {
    let n = rel.replace('\\', "/");
    set.is_match(&n)
}

/// GET /api/armaraos-home/list?path= — List a directory under the ArmaraOS home folder (~/.armaraos by default).
pub async fn armaraos_home_list(
    State(state): State<Arc<AppState>>,
    ext: Option<Extension<RequestId>>,
    Query(q): Query<ArmaraosHomeBrowserQuery>,
) -> axum::response::Response {
    use std::cmp::Ordering;

    let rid = resolve_request_id(ext);
    let home_dir = &state.kernel.config.home_dir;
    let rel = normalize_armaraos_home_rel(&q.path);

    let resolved = match resolve_sandbox_path(&rel, home_dir) {
        Ok(p) => p,
        Err(e) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                "/api/armaraos-home/list",
                "Invalid path",
                e,
                Some("Use paths relative to your ArmaraOS home directory. Path traversal (..) is not allowed."),
            )
            .into_response();
        }
    };

    let meta = match std::fs::metadata(&resolved) {
        Ok(m) => m,
        Err(e) => {
            return api_json_error(
                StatusCode::NOT_FOUND,
                &rid,
                "/api/armaraos-home/list",
                "Path not found",
                e.to_string(),
                None,
            )
            .into_response();
        }
    };

    if !meta.is_dir() {
        return api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            "/api/armaraos-home/list",
            "Not a directory",
            format!("{} is not a folder.", rel),
            Some("Open a directory path, not a file."),
        )
        .into_response();
    }

    let read = match std::fs::read_dir(&resolved) {
        Ok(r) => r,
        Err(e) => {
            return api_json_error(
                StatusCode::FORBIDDEN,
                &rid,
                "/api/armaraos-home/list",
                "Cannot read directory",
                e.to_string(),
                None,
            )
            .into_response();
        }
    };

    #[derive(Debug)]
    struct Row {
        name: String,
        kind: &'static str,
        size: Option<u64>,
        mtime_ms: Option<i64>,
        sort_dir: bool,
    }

    let mut rows: Vec<Row> = Vec::new();
    for item in read.flatten() {
        let name = item.file_name().to_string_lossy().to_string();
        let path = resolved.join(&name);
        let sm = match std::fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let is_link = sm.file_type().is_symlink();
        let kind: &'static str = if is_link {
            "symlink"
        } else if sm.is_dir() {
            "dir"
        } else {
            "file"
        };
        let sort_dir = !is_link && sm.is_dir();
        let size = if kind == "file" { Some(sm.len()) } else { None };
        let mtime_ms = sm.modified().ok().and_then(|t| {
            t.duration_since(std::time::UNIX_EPOCH)
                .ok()
                .map(|d| d.as_millis() as i64)
        });
        rows.push(Row {
            name,
            kind,
            size,
            mtime_ms,
            sort_dir,
        });
    }

    rows.sort_by(|a, b| match (a.sort_dir, b.sort_dir) {
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        _ => a
            .name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.name.cmp(&b.name)),
    });

    let truncated = rows.len() > ARMARAOS_HOME_BROWSER_MAX_ENTRIES;
    if truncated {
        rows.truncate(ARMARAOS_HOME_BROWSER_MAX_ENTRIES);
    }

    let dash = &state.kernel.config.dashboard;
    let glob_result = armaraos_home_build_edit_globset(dash);
    let allowlist_error = glob_result.as_ref().err().cloned();
    let edit_set = match &glob_result {
        Ok(Some(gs)) => Some(gs),
        _ => None,
    };

    let entries: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|r| {
            let child_rel = armaraos_home_rel_join(&rel, &r.name);
            let editable = match &edit_set {
                Some(gs) => {
                    (r.kind == "file" || r.kind == "symlink")
                        && !armaraos_home_edit_path_blocked(&child_rel)
                        && armaraos_home_rel_matches_edit_globs(&child_rel, gs)
                }
                None => false,
            };
            serde_json::json!({
                "name": r.name,
                "kind": r.kind,
                "size": r.size,
                "mtime_ms": r.mtime_ms,
                "editable": editable,
            })
        })
        .collect();

    Json(serde_json::json!({
        "path": rel,
        "root": home_dir.to_string_lossy(),
        "entries": entries,
        "truncated": truncated,
        "home_edit": {
            "allowlist_enabled": edit_set.is_some(),
            "allowlist_error": allowlist_error,
            "max_bytes": dash.home_edit_max_bytes,
            "backup": dash.home_edit_backup,
        },
    }))
    .into_response()
}

/// GET /api/armaraos-home/read?path= — Read a file under the ArmaraOS home folder (size-capped).
pub async fn armaraos_home_read(
    State(state): State<Arc<AppState>>,
    ext: Option<Extension<RequestId>>,
    Query(q): Query<ArmaraosHomeBrowserQuery>,
) -> axum::response::Response {
    use base64::Engine as _;

    let rid = resolve_request_id(ext);
    let home_dir = &state.kernel.config.home_dir;
    let rel = normalize_armaraos_home_rel(&q.path);

    if rel.is_empty() {
        return api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            "/api/armaraos-home/read",
            "Missing path",
            "Query parameter `path` must name a file relative to the ArmaraOS home directory."
                .to_string(),
            Some("Example: path=config.toml"),
        )
        .into_response();
    }

    let resolved = match resolve_sandbox_path(&rel, home_dir) {
        Ok(p) => p,
        Err(e) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                "/api/armaraos-home/read",
                "Invalid path",
                e,
                Some("Use paths relative to your ArmaraOS home directory."),
            )
            .into_response();
        }
    };

    if !resolved.is_file() {
        return api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            "/api/armaraos-home/read",
            "Not a file",
            format!("{} is not a regular file.", rel),
            Some("Pick a file path, not a directory."),
        )
        .into_response();
    }

    let len = match std::fs::metadata(&resolved) {
        Ok(m) => m.len(),
        Err(e) => {
            return api_json_error(
                StatusCode::NOT_FOUND,
                &rid,
                "/api/armaraos-home/read",
                "File not found",
                e.to_string(),
                None,
            )
            .into_response();
        }
    };

    if len > ARMARAOS_HOME_BROWSER_MAX_READ_BYTES {
        return api_json_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            &rid,
            "/api/armaraos-home/read",
            "File too large",
            format!(
                "File is {} bytes; maximum allowed is {} bytes.",
                len, ARMARAOS_HOME_BROWSER_MAX_READ_BYTES
            ),
            Some("Open the file in an external editor, or increase limits in a future release."),
        )
        .into_response();
    }

    let data = match std::fs::read(&resolved) {
        Ok(d) => d,
        Err(e) => {
            return api_json_error(
                StatusCode::FORBIDDEN,
                &rid,
                "/api/armaraos-home/read",
                "Cannot read file",
                e.to_string(),
                None,
            )
            .into_response();
        }
    };

    let (encoding, content) = match String::from_utf8(data) {
        Ok(s) => ("utf8", serde_json::Value::String(s)),
        Err(e) => (
            "base64",
            serde_json::Value::String(
                base64::engine::general_purpose::STANDARD.encode(e.as_bytes()),
            ),
        ),
    };

    let dash = &state.kernel.config.dashboard;
    let gs_result = armaraos_home_build_edit_globset(dash);
    let allowlist_error = gs_result.as_ref().err().cloned();
    let editable = encoding == "utf8"
        && match &gs_result {
            Ok(Some(gs)) => {
                !armaraos_home_edit_path_blocked(&rel)
                    && armaraos_home_rel_matches_edit_globs(&rel, gs)
            }
            _ => false,
        };

    Json(serde_json::json!({
        "path": rel,
        "encoding": encoding,
        "content": content,
        "size": len,
        "editable": editable,
        "allowlist_error": allowlist_error,
        "home_edit_max_bytes": dash.home_edit_max_bytes,
        "home_edit_backup": dash.home_edit_backup,
    }))
    .into_response()
}

/// GET /api/armaraos-home/download?path= — Stream a file from the ArmaraOS home tree (larger cap than read).
pub async fn armaraos_home_download(
    State(state): State<Arc<AppState>>,
    ext: Option<Extension<RequestId>>,
    Query(q): Query<ArmaraosHomeBrowserQuery>,
) -> axum::response::Response {
    let rid = resolve_request_id(ext);
    let home_dir = &state.kernel.config.home_dir;
    let rel = normalize_armaraos_home_rel(&q.path);
    const PATH: &str = "/api/armaraos-home/download";

    if rel.is_empty() {
        return api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Missing path",
            "Query parameter `path` must name a file relative to the ArmaraOS home directory."
                .to_string(),
            Some("Example: path=support/armaraos-diagnostics-20260101-120000.zip"),
        )
        .into_response();
    }

    let resolved = match resolve_sandbox_path(&rel, home_dir) {
        Ok(p) => p,
        Err(e) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid path",
                e,
                Some("Use paths relative to your ArmaraOS home directory."),
            )
            .into_response();
        }
    };

    if !resolved.is_file() {
        return api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Not a file",
            format!("{rel} is not a regular file."),
            Some("Pick a file path, not a directory."),
        )
        .into_response();
    }

    let len = match std::fs::metadata(&resolved) {
        Ok(m) => m.len(),
        Err(e) => {
            return api_json_error(
                StatusCode::NOT_FOUND,
                &rid,
                PATH,
                "File not found",
                e.to_string(),
                None,
            )
            .into_response();
        }
    };

    if len > ARMARAOS_HOME_DOWNLOAD_MAX_BYTES {
        return api_json_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            &rid,
            PATH,
            "File too large",
            format!(
                "File is {} bytes; maximum download is {} bytes. Open the folder in Finder / Explorer instead.",
                len, ARMARAOS_HOME_DOWNLOAD_MAX_BYTES
            ),
            None,
        )
        .into_response();
    }

    let path_for_blocking = resolved.clone();
    let bytes = match tokio::task::spawn_blocking(move || std::fs::read(&path_for_blocking)).await {
        Ok(Ok(b)) => b,
        Ok(Err(e)) => {
            return api_json_error(
                StatusCode::FORBIDDEN,
                &rid,
                PATH,
                "Cannot read file",
                e.to_string(),
                None,
            )
            .into_response();
        }
        Err(e) => {
            return api_json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &rid,
                PATH,
                "Read task failed",
                e.to_string(),
                None,
            )
            .into_response();
        }
    };

    let basename = std::path::Path::new(&rel)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("download");
    let safe_name: String = basename
        .chars()
        .filter(|c| !matches!(c, '"' | '\r' | '\n' | '\\'))
        .collect();
    let safe_name = if safe_name.is_empty() {
        "download".to_string()
    } else {
        safe_name
    };
    let cd = format!("attachment; filename=\"{safe_name}\"");
    use axum::body::Body;
    use axum::http::{header, HeaderValue, Response};
    let disp = match HeaderValue::from_str(&cd) {
        Ok(h) => h,
        Err(_) => {
            return api_json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &rid,
                PATH,
                "Header build failed",
                "Invalid filename for Content-Disposition.".to_string(),
                None,
            )
            .into_response();
        }
    };
    match Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_DISPOSITION, disp)
        .body(Body::from(bytes))
    {
        Ok(resp) => resp.into_response(),
        Err(_) => api_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &rid,
            PATH,
            "Response build failed",
            "Could not build download response.".to_string(),
            None,
        )
        .into_response(),
    }
}

#[derive(serde::Deserialize)]
pub struct ArmaraosHomeWriteBody {
    pub path: String,
    pub content: String,
}

/// POST /api/armaraos-home/write — Write a UTF-8 file under the ArmaraOS home directory when allowed by `[dashboard] home_editable_globs`.
pub async fn armaraos_home_write(
    State(state): State<Arc<AppState>>,
    ext: Option<Extension<RequestId>>,
    Json(body): Json<ArmaraosHomeWriteBody>,
) -> axum::response::Response {
    let rid = resolve_request_id(ext);
    let home_dir = &state.kernel.config.home_dir;
    let rel = normalize_armaraos_home_rel(&body.path);

    if rel.is_empty() {
        return api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            "/api/armaraos-home/write",
            "Missing path",
            "Field `path` must name a file relative to the ArmaraOS home directory.".to_string(),
            Some("Set dashboard.home_editable_globs in config.toml to enable editing."),
        )
        .into_response();
    }

    if armaraos_home_edit_path_blocked(&rel) {
        return api_json_error(
            StatusCode::FORBIDDEN,
            &rid,
            "/api/armaraos-home/write",
            "Path not editable",
            "This path is blocked from dashboard writes (secrets, config, or data directory)."
                .to_string(),
            None,
        )
        .into_response();
    }

    let dash = &state.kernel.config.dashboard;
    let max_bytes = dash.home_edit_max_bytes;
    let content_len = body.content.len();
    if content_len > max_bytes as usize {
        return api_json_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            &rid,
            "/api/armaraos-home/write",
            "Content too large",
            format!("Body is {content_len} bytes; maximum is {max_bytes}."),
            Some("Raise [dashboard] home_edit_max_bytes if you need larger files."),
        )
        .into_response();
    }

    let gs = match armaraos_home_build_edit_globset(dash) {
        Ok(Some(gs)) => gs,
        Ok(None) => {
            return api_json_error(
                StatusCode::FORBIDDEN,
                &rid,
                "/api/armaraos-home/write",
                "Editing disabled",
                "dashboard.home_editable_globs is empty — add glob patterns in config.toml to allow writes.".to_string(),
                Some(r#"Example: home_editable_globs = ["notes/**"]"#),
            )
            .into_response();
        }
        Err(e) => {
            return api_json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &rid,
                "/api/armaraos-home/write",
                "Invalid allowlist",
                e,
                Some("Fix dashboard.home_editable_globs in config.toml (glob syntax error)."),
            )
            .into_response();
        }
    };

    if !armaraos_home_rel_matches_edit_globs(&rel, &gs) {
        return api_json_error(
            StatusCode::FORBIDDEN,
            &rid,
            "/api/armaraos-home/write",
            "Path not allowed",
            format!("Path `{rel}` does not match any entry in dashboard.home_editable_globs."),
            None,
        )
        .into_response();
    }

    let resolved = match resolve_sandbox_path(&rel, home_dir) {
        Ok(p) => p,
        Err(e) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                "/api/armaraos-home/write",
                "Invalid path",
                e,
                None,
            )
            .into_response();
        }
    };

    if resolved.is_dir() {
        return api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            "/api/armaraos-home/write",
            "Not a file",
            format!("{rel} is a directory."),
            None,
        )
        .into_response();
    }

    let parent = match resolved.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                "/api/armaraos-home/write",
                "Invalid path",
                "Missing parent directory.".to_string(),
                None,
            )
            .into_response();
        }
    };

    if !parent.is_dir() {
        return api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            "/api/armaraos-home/write",
            "Parent missing",
            format!("Parent directory does not exist: {}", parent.display()),
            Some("Create the folder first, then save the file."),
        )
        .into_response();
    }

    if resolved.exists() && dash.home_edit_backup {
        let bak = format!("{}.bak", resolved.display());
        if let Err(e) = std::fs::copy(&resolved, &bak) {
            tracing::warn!(path = %resolved.display(), error = %e, "armaraos-home write: backup copy failed");
        }
    }

    let fname = match resolved.file_name().and_then(|s| s.to_str()) {
        Some(s) => s,
        None => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                "/api/armaraos-home/write",
                "Invalid filename",
                "Non-UTF8 file name.".to_string(),
                None,
            )
            .into_response();
        }
    };

    let tmp = parent.join(format!(".{fname}.armaraos_write.{}", std::process::id()));

    let bytes = body.content.into_bytes();
    if let Err(e) = std::fs::write(&tmp, &bytes) {
        return api_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &rid,
            "/api/armaraos-home/write",
            "Write failed",
            e.to_string(),
            None,
        )
        .into_response();
    }

    if cfg!(windows) && resolved.exists() {
        if let Err(e) = std::fs::remove_file(&resolved) {
            let _ = std::fs::remove_file(&tmp);
            return api_json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &rid,
                "/api/armaraos-home/write",
                "Replace failed",
                e.to_string(),
                None,
            )
            .into_response();
        }
    }

    if let Err(e) = std::fs::rename(&tmp, &resolved) {
        let _ = std::fs::remove_file(&tmp);
        return api_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &rid,
            "/api/armaraos-home/write",
            "Commit failed",
            e.to_string(),
            None,
        )
        .into_response();
    }

    Json(serde_json::json!({
        "ok": true,
        "path": rel,
        "bytes_written": content_len,
    }))
    .into_response()
}

// ---------------------------------------------------------------------------
// Config endpoint
// ---------------------------------------------------------------------------

/// GET /api/config — Get kernel configuration (secrets redacted).
pub async fn get_config(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // Return a redacted view of the kernel config
    let config = &state.kernel.config;
    Json(serde_json::json!({
        "home_dir": config.home_dir.to_string_lossy(),
        "data_dir": config.data_dir.to_string_lossy(),
        "config_schema_version": config.config_schema_version,
        "api_key": if config.api_key.is_empty() { "not set" } else { "***" },
        "default_model": {
            "provider": config.default_model.provider,
            "model": config.default_model.model,
            "api_key_env": config.default_model.api_key_env,
        },
        "memory": {
            "decay_rate": config.memory.decay_rate,
        },
    }))
}

// ---------------------------------------------------------------------------
// Usage endpoint
// ---------------------------------------------------------------------------

/// GET /api/usage — Get per-agent usage statistics.
pub async fn usage_stats(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let agents: Vec<serde_json::Value> = state
        .kernel
        .registry
        .list()
        .iter()
        .map(|e| {
            // Prefer persistent SQLite-backed usage totals so values survive daemon
            // restarts and desktop upgrades/reinstalls. Fall back to in-memory
            // scheduler counters only if usage summary lookup fails.
            let (input_tokens, output_tokens, tool_calls, cost_usd, source) =
                match state.kernel.metering.get_summary(Some(e.id)) {
                    Ok(s) => (
                        s.total_input_tokens,
                        s.total_output_tokens,
                        s.total_tool_calls,
                        s.total_cost_usd,
                        "persistent",
                    ),
                    Err(_) => {
                        let (tokens, tc) = state.kernel.scheduler.get_usage(e.id).unwrap_or((0, 0));
                        (tokens, 0, tc, 0.0, "scheduler_fallback")
                    }
                };
            let total_tokens = input_tokens + output_tokens;
            serde_json::json!({
                "agent_id": e.id.to_string(),
                "name": e.name,
                "total_tokens": total_tokens,
                "input_tokens": input_tokens,
                "output_tokens": output_tokens,
                "tool_calls": tool_calls,
                "cost_usd": cost_usd,
                "source": source,
            })
        })
        .collect();

    Json(serde_json::json!({"agents": agents}))
}

// ---------------------------------------------------------------------------
// Usage summary endpoints
// ---------------------------------------------------------------------------

/// GET /api/usage/summary — Get overall usage summary from UsageStore.
pub async fn usage_summary(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match state.kernel.memory.usage().query_summary(None) {
        Ok(s) => Json(serde_json::json!({
            "total_input_tokens": s.total_input_tokens,
            "total_output_tokens": s.total_output_tokens,
            "total_cost_usd": s.total_cost_usd,
            "call_count": s.call_count,
            "total_tool_calls": s.total_tool_calls,
        })),
        Err(_) => Json(serde_json::json!({
            "total_input_tokens": 0,
            "total_output_tokens": 0,
            "total_cost_usd": 0.0,
            "call_count": 0,
            "total_tool_calls": 0,
        })),
    }
}

/// GET /api/usage/by-model — Get usage grouped by model.
pub async fn usage_by_model(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match state.kernel.memory.usage().query_by_model() {
        Ok(models) => {
            let list: Vec<serde_json::Value> = models
                .iter()
                .map(|m| {
                    serde_json::json!({
                        "model": m.model,
                        "total_cost_usd": m.total_cost_usd,
                        "total_input_tokens": m.total_input_tokens,
                        "total_output_tokens": m.total_output_tokens,
                        "call_count": m.call_count,
                    })
                })
                .collect();
            Json(serde_json::json!({"models": list}))
        }
        Err(_) => Json(serde_json::json!({"models": []})),
    }
}

/// GET /api/usage/daily — Get daily usage breakdown for the last 7 days.
pub async fn usage_daily(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let days = state.kernel.memory.usage().query_daily_breakdown(7);
    let today_cost = state.kernel.memory.usage().query_today_cost();
    let first_event = state.kernel.memory.usage().query_first_event_date();

    let days_list = match days {
        Ok(d) => d
            .iter()
            .map(|day| {
                serde_json::json!({
                    "date": day.date,
                    "cost_usd": day.cost_usd,
                    "tokens": day.tokens,
                    "calls": day.calls,
                })
            })
            .collect::<Vec<_>>(),
        Err(_) => vec![],
    };

    Json(serde_json::json!({
        "days": days_list,
        "today_cost_usd": today_cost.unwrap_or(0.0),
        "first_event_date": first_event.unwrap_or(None),
    }))
}

/// GET /api/usage/compression — Durable eco-mode prompt compression effectiveness.
///
/// Returns SQLite-backed aggregates with p50/p95 savings and estimated token reduction
/// by mode and by agent. Query `?window=7d`, `?window=30d`, or `?window=all`.
pub async fn usage_compression(
    State(state): State<Arc<AppState>>,
    Query(query): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let window_days = query
        .get("window")
        .map(|w| w.trim().to_ascii_lowercase())
        .and_then(|w| {
            if w.is_empty() || w == "all" {
                return None;
            }
            if let Some(stripped) = w.strip_suffix('d') {
                stripped.parse::<u32>().ok()
            } else {
                w.parse::<u32>().ok()
            }
        });
    match state
        .kernel
        .memory
        .usage()
        .query_compression_summary(window_days)
    {
        Ok(summary) => Json(serde_json::to_value(summary).unwrap_or_default()),
        Err(_) => {
            // Compression rollups failed — still try adaptive-eco aggregates so Budget isn't blind.
            let adaptive_eco = match (
                state.kernel.metering.get_adaptive_eco_summary(window_days),
                state
                    .kernel
                    .metering
                    .get_adaptive_eco_replay_report(window_days),
            ) {
                (Ok(ref summary), Ok(ref replay)) => Some(serde_json::json!({
                    "summary": serde_json::to_value(summary).unwrap_or_default(),
                    "replay": serde_json::to_value(replay).unwrap_or_default(),
                })),
                _ => None,
            };
            Json(serde_json::json!({
                "window": query.get("window").cloned().unwrap_or_else(|| "all".to_string()),
                "modes": {},
                "agents": [],
                "estimated_compression_tokens_saved": 0,
                "cache_read_input_tokens": 0,
                "estimated_total_input_tokens_saved": 0,
                "estimated_cache_cost_saved_usd": 0.0,
                "estimated_compression_cost_saved_usd": 0.0,
                "estimated_total_cost_saved_usd": 0.0,
                "adaptive_eco": adaptive_eco,
                "compression_summary_error": true,
                "adaptive_eco_filled_from_fallback": adaptive_eco.is_some(),
            }))
        }
    }
}

/// GET /api/usage/adaptive-eco — Durable adaptive eco telemetry (shadow mismatches, breaker trips).
pub async fn usage_adaptive_eco(
    State(state): State<Arc<AppState>>,
    Query(query): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let window_days = query
        .get("window")
        .map(|w| w.trim().to_ascii_lowercase())
        .and_then(|w| {
            if w.is_empty() || w == "all" {
                return None;
            }
            if let Some(stripped) = w.strip_suffix('d') {
                stripped.parse::<u32>().ok()
            } else {
                w.parse::<u32>().ok()
            }
        });
    match state.kernel.metering.get_adaptive_eco_summary(window_days) {
        Ok(summary) => Json(serde_json::to_value(summary).unwrap_or_default()),
        Err(_) => Json(serde_json::json!({
            "window": query.get("window").cloned().unwrap_or_else(|| "all".to_string()),
            "events": 0,
            "shadow_mismatch_turns": 0,
            "circuit_breaker_trips": 0,
            "hysteresis_blocks": 0
        })),
    }
}

/// GET /api/usage/adaptive-eco/replay — Aggregated replay report (shadow mismatch rate, semantic percentiles).
pub async fn usage_adaptive_eco_replay(
    State(state): State<Arc<AppState>>,
    Query(query): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let window_days = query
        .get("window")
        .map(|w| w.trim().to_ascii_lowercase())
        .and_then(|w| {
            if w.is_empty() || w == "all" {
                return None;
            }
            if let Some(stripped) = w.strip_suffix('d') {
                stripped.parse::<u32>().ok()
            } else {
                w.parse::<u32>().ok()
            }
        });
    match state
        .kernel
        .metering
        .get_adaptive_eco_replay_report(window_days)
    {
        Ok(report) => Json(serde_json::to_value(report).unwrap_or_default()),
        Err(_) => Json(serde_json::json!({
            "window": query.get("window").cloned().unwrap_or_else(|| "all".to_string()),
            "adaptive_eco_events": 0,
            "shadow_mismatch_turns": 0,
            "shadow_mismatch_rate": 0.0,
            "circuit_breaker_trips": 0,
            "hysteresis_blocks": 0,
            "eco_compression_turns": 0,
            "compression_semantic_p50": null,
            "compression_semantic_p95": null,
            "compression_semantic_mean": null,
            "effective_mode_flip_turns": 0,
            "effective_mode_transition_slots": 0,
            "effective_mode_flip_rate": 0.0,
            "adaptive_confidence_samples": 0,
            "adaptive_confidence_p50": null,
            "adaptive_confidence_p95": null,
            "adaptive_confidence_mean": null,
            "adaptive_confidence_bucket_low": 0,
            "adaptive_confidence_bucket_mid": 0,
            "adaptive_confidence_bucket_high": 0
        })),
    }
}

// ---------------------------------------------------------------------------
// Budget endpoints
// ---------------------------------------------------------------------------

/// GET /api/budget — Current budget status (limits, spend, % used).
pub async fn budget_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let budget = state.budget_config.read().await;
    let status = state.kernel.metering.budget_status(&budget);
    Json(serde_json::to_value(&status).unwrap_or_default())
}

/// PUT /api/budget — Update global budget limits (in-memory only, not persisted to config.toml).
pub async fn update_budget(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    {
        let mut budget = state.budget_config.write().await;
        if let Some(v) = body["max_hourly_usd"].as_f64() {
            budget.max_hourly_usd = v;
        }
        if let Some(v) = body["max_daily_usd"].as_f64() {
            budget.max_daily_usd = v;
        }
        if let Some(v) = body["max_monthly_usd"].as_f64() {
            budget.max_monthly_usd = v;
        }
        if let Some(v) = body["alert_threshold"].as_f64() {
            budget.alert_threshold = v.clamp(0.0, 1.0);
        }
        if let Some(v) = body["default_max_llm_tokens_per_hour"].as_u64() {
            budget.default_max_llm_tokens_per_hour = v;
        }
    }
    let budget = state.budget_config.read().await;
    let status = state.kernel.metering.budget_status(&budget);
    Json(serde_json::to_value(&status).unwrap_or_default())
}

/// GET /api/budget/agents/{id} — Per-agent budget/quota status.
pub async fn agent_budget_status(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/budget/agents/:id";
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid agent ID",
                "Agent id must be a valid UUID.".to_string(),
                Some("Use GET /api/agents to list ids."),
            )
        }
    };

    let entry = match state.kernel.registry.get(agent_id) {
        Some(e) => e,
        None => {
            return api_json_error(
                StatusCode::NOT_FOUND,
                &rid,
                PATH,
                "Agent not found",
                format!("No agent registered for id {id}."),
                Some("Spawn an agent or pick a valid id from GET /api/agents."),
            )
        }
    };

    let quota = &entry.manifest.resources;
    let usage_store = openfang_memory::usage::UsageStore::new(state.kernel.memory.usage_conn());
    let hourly = usage_store.query_hourly(agent_id).unwrap_or(0.0);
    let daily = usage_store.query_daily(agent_id).unwrap_or(0.0);
    let monthly = usage_store.query_monthly(agent_id).unwrap_or(0.0);

    // Token usage from scheduler
    let token_usage = state.kernel.scheduler.get_usage(agent_id);
    let tokens_used = token_usage.map(|(t, _)| t).unwrap_or(0);

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "agent_id": agent_id.to_string(),
            "agent_name": entry.name,
            "hourly": {
                "spend": hourly,
                "limit": quota.max_cost_per_hour_usd,
                "pct": if quota.max_cost_per_hour_usd > 0.0 { hourly / quota.max_cost_per_hour_usd } else { 0.0 },
            },
            "daily": {
                "spend": daily,
                "limit": quota.max_cost_per_day_usd,
                "pct": if quota.max_cost_per_day_usd > 0.0 { daily / quota.max_cost_per_day_usd } else { 0.0 },
            },
            "monthly": {
                "spend": monthly,
                "limit": quota.max_cost_per_month_usd,
                "pct": if quota.max_cost_per_month_usd > 0.0 { monthly / quota.max_cost_per_month_usd } else { 0.0 },
            },
            "tokens": {
                "used": tokens_used,
                "limit": quota.max_llm_tokens_per_hour,
                "pct": if quota.max_llm_tokens_per_hour > 0 { tokens_used as f64 / quota.max_llm_tokens_per_hour as f64 } else { 0.0 },
            },
            "turn_stats": {
                "last_latency_ms": entry.turn_stats.last_latency_ms,
                "last_fallback_note": entry.turn_stats.last_fallback_note,
                "last_success_at": entry.turn_stats.last_success_at.map(|t| t.to_rfc3339()),
                "last_error_at": entry.turn_stats.last_error_at.map(|t| t.to_rfc3339()),
                "last_error_summary": entry.turn_stats.last_error_summary,
                "turns_ok": entry.turn_stats.turns_ok,
                "turns_err": entry.turn_stats.turns_err,
                "last_input_tokens": entry.turn_stats.last_input_tokens,
                "last_output_tokens": entry.turn_stats.last_output_tokens,
            },
        })),
    )
}

/// GET /api/budget/agents — Per-agent cost ranking (top spenders).
pub async fn agent_budget_ranking(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let usage_store = openfang_memory::usage::UsageStore::new(state.kernel.memory.usage_conn());
    let mut agents: Vec<serde_json::Value> = state
        .kernel
        .registry
        .list()
        .iter()
        .map(|entry| {
            let daily = usage_store.query_daily(entry.id).unwrap_or(0.0);
            let ts = &entry.turn_stats;
            serde_json::json!({
                "agent_id": entry.id.to_string(),
                "name": entry.name,
                "daily_cost_usd": daily,
                "hourly_limit": entry.manifest.resources.max_cost_per_hour_usd,
                "daily_limit": entry.manifest.resources.max_cost_per_day_usd,
                "monthly_limit": entry.manifest.resources.max_cost_per_month_usd,
                "max_llm_tokens_per_hour": entry.manifest.resources.max_llm_tokens_per_hour,
                "turn_stats": {
                    "last_latency_ms": ts.last_latency_ms,
                    "last_fallback_note": ts.last_fallback_note,
                    "last_success_at": ts.last_success_at.map(|t| t.to_rfc3339()),
                    "last_error_at": ts.last_error_at.map(|t| t.to_rfc3339()),
                    "turns_ok": ts.turns_ok,
                    "turns_err": ts.turns_err,
                    "last_input_tokens": ts.last_input_tokens,
                    "last_output_tokens": ts.last_output_tokens,
                },
            })
        })
        .collect();
    agents.sort_by(|a, b| {
        let da = a["daily_cost_usd"].as_f64().unwrap_or(0.0);
        let db = b["daily_cost_usd"].as_f64().unwrap_or(0.0);
        db.partial_cmp(&da).unwrap_or(std::cmp::Ordering::Equal)
    });

    Json(serde_json::json!({"agents": agents, "total": agents.len()}))
}

/// PUT /api/budget/agents/{id} — Update per-agent budget limits at runtime.
pub async fn update_agent_budget(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/budget/agents/:id";
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid agent ID",
                "Agent id must be a valid UUID.".to_string(),
                Some("Use GET /api/agents to list ids."),
            )
        }
    };

    let hourly = body["max_cost_per_hour_usd"].as_f64();
    let daily = body["max_cost_per_day_usd"].as_f64();
    let monthly = body["max_cost_per_month_usd"].as_f64();
    let tokens = body["max_llm_tokens_per_hour"].as_u64();

    if hourly.is_none() && daily.is_none() && monthly.is_none() && tokens.is_none() {
        return api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Missing budget fields",
            "Provide at least one of: max_cost_per_hour_usd, max_cost_per_day_usd, max_cost_per_month_usd, max_llm_tokens_per_hour.".to_string(),
            Some("Send a JSON object with one or more limit fields."),
        );
    }

    match state
        .kernel
        .registry
        .update_resources(agent_id, hourly, daily, monthly, tokens)
    {
        Ok(()) => {
            // Persist updated entry
            if let Some(entry) = state.kernel.registry.get(agent_id) {
                let _ = state.kernel.memory.save_agent(&entry);
                sync_agent_toml_for_kernel(state.kernel.as_ref(), &entry);
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "ok", "message": "Agent budget updated"})),
            )
        }
        Err(e) => api_json_error(
            StatusCode::NOT_FOUND,
            &rid,
            PATH,
            "Agent budget update failed",
            format!("{e}"),
            Some("Verify the agent exists and manifest resources are valid."),
        ),
    }
}

// ---------------------------------------------------------------------------
// Session listing endpoints
// ---------------------------------------------------------------------------

/// GET /api/sessions — List all sessions with metadata.
pub async fn list_sessions(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match state.kernel.memory.list_sessions() {
        Ok(sessions) => Json(serde_json::json!({"sessions": sessions})),
        Err(_) => Json(serde_json::json!({"sessions": []})),
    }
}

/// DELETE /api/sessions/:id — Delete a session.
pub async fn delete_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/sessions/:id";
    let session_id = match id.parse::<uuid::Uuid>() {
        Ok(u) => openfang_types::agent::SessionId(u),
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid session ID",
                "Session id must be a valid UUID.".to_string(),
                Some("Use GET /api/sessions to list session ids."),
            );
        }
    };

    match state.kernel.memory.delete_session(session_id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "deleted", "session_id": id})),
        ),
        Err(e) => api_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &rid,
            PATH,
            "Session delete failed",
            e.to_string(),
            Some("Check database health and daemon logs."),
        ),
    }
}

/// PUT /api/sessions/:id/label — Set a session label.
pub async fn set_session_label(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/sessions/:id/label";
    let session_id = match id.parse::<uuid::Uuid>() {
        Ok(u) => openfang_types::agent::SessionId(u),
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid session ID",
                "Session id must be a valid UUID.".to_string(),
                Some("Use GET /api/sessions to list session ids."),
            );
        }
    };

    let label = req.get("label").and_then(|v| v.as_str());

    // Validate label if present
    if let Some(lbl) = label {
        if let Err(e) = openfang_types::agent::SessionLabel::new(lbl) {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid session label",
                e.to_string(),
                Some("Labels must meet length and character rules."),
            );
        }
    }

    match state.kernel.memory.set_session_label(session_id, label) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "updated",
                "session_id": id,
                "label": label,
            })),
        ),
        Err(e) => api_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &rid,
            PATH,
            "Session label update failed",
            e.to_string(),
            Some("Check database health and whether the session exists."),
        ),
    }
}

/// GET /api/sessions/by-label/:label — Find session by label (scoped to agent).
pub async fn find_session_by_label(
    State(state): State<Arc<AppState>>,
    Path((agent_id_str, label)): Path<(String, String)>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/agents/:id/sessions/by-label/:label";
    let agent_id = match agent_id_str.parse::<uuid::Uuid>() {
        Ok(u) => openfang_types::agent::AgentId(u),
        Err(_) => {
            // Try name lookup
            match state.kernel.registry.find_by_name(&agent_id_str) {
                Some(entry) => entry.id,
                None => {
                    return api_json_error(
                        StatusCode::NOT_FOUND,
                        &rid,
                        PATH,
                        "Agent not found",
                        format!("No agent matches UUID or name '{agent_id_str}'."),
                        Some("Use GET /api/agents or pass a valid agent id."),
                    );
                }
            }
        }
    };

    match state.kernel.memory.find_session_by_label(agent_id, &label) {
        Ok(Some(session)) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "session_id": session.id.0.to_string(),
                "agent_id": session.agent_id.0.to_string(),
                "label": session.label,
                "message_count": session.messages.len(),
            })),
        ),
        Ok(None) => api_json_error(
            StatusCode::NOT_FOUND,
            &rid,
            PATH,
            "Session not found",
            format!("No session with label '{label}' for this agent."),
            Some("List sessions with GET /api/sessions."),
        ),
        Err(e) => api_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &rid,
            PATH,
            "Session lookup failed",
            e.to_string(),
            Some("Check database health."),
        ),
    }
}

// ---------------------------------------------------------------------------
// Trigger update endpoint
// ---------------------------------------------------------------------------

/// PUT /api/triggers/:id — Update a trigger (enable/disable toggle).
pub async fn update_trigger(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/triggers/:id";
    let trigger_id = TriggerId(match id.parse() {
        Ok(u) => u,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid trigger ID",
                "Trigger id must be a valid UUID.".to_string(),
                Some("Use GET /api/triggers to list ids."),
            );
        }
    });

    if let Some(enabled) = req.get("enabled").and_then(|v| v.as_bool()) {
        if state.kernel.set_trigger_enabled(trigger_id, enabled) {
            (
                StatusCode::OK,
                Json(
                    serde_json::json!({"status": "updated", "trigger_id": id, "enabled": enabled}),
                ),
            )
        } else {
            api_json_error(
                StatusCode::NOT_FOUND,
                &rid,
                PATH,
                "Trigger not found",
                format!("No trigger registered for id {id}."),
                Some("List triggers with GET /api/triggers."),
            )
        }
    } else {
        api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Missing enabled field",
            "JSON body must include boolean 'enabled'.".to_string(),
            Some("Send {\"enabled\": true} or {\"enabled\": false}."),
        )
    }
}

// ---------------------------------------------------------------------------
// Agent update endpoint
// ---------------------------------------------------------------------------

/// PUT /api/agents/{id}/update — Replace a running agent's manifest from validated TOML.
pub async fn update_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
    Json(req): Json<AgentUpdateRequest>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/agents/:id/update";
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid agent ID",
                "Agent id must be a valid UUID.".to_string(),
                Some("Use GET /api/agents to list ids."),
            );
        }
    };

    if state.kernel.registry.get(agent_id).is_none() {
        return api_json_error(
            StatusCode::NOT_FOUND,
            &rid,
            PATH,
            "Agent not found",
            format!("No agent registered for id {id}."),
            Some("Spawn an agent or pick a valid id from GET /api/agents."),
        );
    }

    let manifest: AgentManifest = match toml::from_str(&req.manifest_toml) {
        Ok(m) => m,
        Err(e) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid manifest",
                format!("{e}"),
                Some("Fix agent.toml syntax and retry."),
            );
        }
    };

    match state.kernel.apply_agent_manifest_update(agent_id, manifest) {
        Ok(entry) => {
            sync_agent_toml_for_kernel(state.kernel.as_ref(), &entry);
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "ok",
                    "agent_id": id,
                    "name": entry.name,
                    "note": "Manifest applied to the running kernel and persisted. Autonomous schedule loops (continuous / periodic / proactive triggers) were reloaded without a daemon restart. Session memory was cleared for safety.",
                })),
            )
        }
        Err(e) => match e {
            openfang_kernel::error::KernelError::OpenFang(
                openfang_types::error::OpenFangError::Config(msg),
            ) => api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Manifest rejected",
                msg,
                Some(
                    "The manifest name must match the agent. Use PATCH /api/agents/:id to rename.",
                ),
            ),
            openfang_kernel::error::KernelError::OpenFang(
                openfang_types::error::OpenFangError::AgentNotFound(s),
            ) => api_json_error(
                StatusCode::NOT_FOUND,
                &rid,
                PATH,
                "Agent not found",
                s,
                Some("Use GET /api/agents to list ids."),
            ),
            other => api_json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &rid,
                PATH,
                "Manifest update failed",
                format!("{other}"),
                Some("Check daemon logs and retry."),
            ),
        },
    }
}

/// PATCH /api/agents/{id} — Partial update of agent fields (name, description, model, system_prompt).
pub async fn patch_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/agents/:id";
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid agent ID",
                "Agent id must be a valid UUID.".to_string(),
                Some("Use GET /api/agents to list ids."),
            );
        }
    };

    if state.kernel.registry.get(agent_id).is_none() {
        return api_json_error(
            StatusCode::NOT_FOUND,
            &rid,
            PATH,
            "Agent not found",
            format!("No agent registered for id {id}."),
            Some("Spawn an agent or pick a valid id from GET /api/agents."),
        );
    }

    // Apply partial updates using dedicated registry methods
    if let Some(name) = body.get("name").and_then(|v| v.as_str()) {
        if let Err(e) = state
            .kernel
            .registry
            .update_name(agent_id, name.to_string())
        {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Agent name update failed",
                format!("{e}"),
                Some("Check naming rules and uniqueness."),
            );
        }
    }
    if let Some(desc) = body.get("description").and_then(|v| v.as_str()) {
        if let Err(e) = state
            .kernel
            .registry
            .update_description(agent_id, desc.to_string())
        {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Agent description update failed",
                format!("{e}"),
                None,
            );
        }
    }
    if let Some(model) = body.get("model").and_then(|v| v.as_str()) {
        let explicit_provider = body.get("provider").and_then(|v| v.as_str());
        if let Err(e) = state
            .kernel
            .set_agent_model(agent_id, model, explicit_provider)
        {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Agent model update failed",
                format!("{e}"),
                Some("Verify provider/model in config and GET /api/providers."),
            );
        }
    }
    if let Some(system_prompt) = body.get("system_prompt").and_then(|v| v.as_str()) {
        if let Err(e) = state
            .kernel
            .registry
            .update_system_prompt(agent_id, system_prompt.to_string())
        {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "System prompt update failed",
                format!("{e}"),
                None,
            );
        }
    }

    // Persist updated entry to SQLite
    if let Some(entry) = state.kernel.registry.get(agent_id) {
        let _ = state.kernel.memory.save_agent(&entry);
        sync_agent_toml_for_kernel(state.kernel.as_ref(), &entry);
        (
            StatusCode::OK,
            Json(
                serde_json::json!({"status": "ok", "agent_id": entry.id.to_string(), "name": entry.name}),
            ),
        )
    } else {
        api_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &rid,
            PATH,
            "Agent vanished during update",
            "Registry lost the agent while updating — inconsistent state.".to_string(),
            Some("Retry the request or restart the daemon if this persists."),
        )
    }
}

// ---------------------------------------------------------------------------
// Migration endpoint
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Security dashboard endpoint
// ---------------------------------------------------------------------------

/// GET /api/security — Security feature status for the dashboard.
pub async fn security_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let auth_mode = if state.kernel.config.api_key.is_empty() {
        "localhost_only"
    } else {
        "bearer_token"
    };

    let audit_count = state.kernel.audit_log.len();

    Json(serde_json::json!({
        "core_protections": {
            "path_traversal": true,
            "ssrf_protection": true,
            "capability_system": true,
            "privilege_escalation_prevention": true,
            "subprocess_isolation": true,
            "security_headers": true,
            "wire_hmac_auth": true,
            "request_id_tracking": true
        },
        "configurable": {
            "rate_limiter": {
                "enabled": true,
                "tokens_per_minute": 500,
                "algorithm": "GCRA"
            },
            "websocket_limits": {
                "max_per_ip": 5,
                "idle_timeout_secs": 1800,
                "max_message_size": 65536,
                "max_messages_per_minute": 10
            },
            "wasm_sandbox": {
                "fuel_metering": true,
                "epoch_interruption": true,
                "default_timeout_secs": 30,
                "default_fuel_limit": 1_000_000u64
            },
            "auth": {
                "mode": auth_mode,
                "api_key_set": !state.kernel.config.api_key.is_empty()
            }
        },
        "monitoring": {
            "audit_trail": {
                "enabled": true,
                "algorithm": "SHA-256 Merkle Chain",
                "entry_count": audit_count
            },
            "taint_tracking": {
                "enabled": true,
                "tracked_labels": [
                    "ExternalNetwork",
                    "UserInput",
                    "PII",
                    "Secret",
                    "UntrustedAgent"
                ]
            },
            "manifest_signing": {
                "algorithm": "Ed25519",
                "available": true
            }
        },
        "secret_zeroization": true,
        "total_features": 15
    }))
}

/// GET /api/migrate/detect — Auto-detect OpenClaw installation.
pub async fn migrate_detect() -> impl IntoResponse {
    match openfang_migrate::openclaw::detect_openclaw_home() {
        Some(path) => {
            let scan = openfang_migrate::openclaw::scan_openclaw_workspace(&path);
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "detected": true,
                    "path": path.display().to_string(),
                    "scan": scan,
                })),
            )
        }
        None => (
            StatusCode::OK,
            Json(serde_json::json!({
                "detected": false,
                "path": null,
                "scan": null,
            })),
        ),
    }
}

/// POST /api/migrate/scan — Scan a specific directory for OpenClaw workspace.
pub async fn migrate_scan(
    ext: Option<Extension<RequestId>>,
    Json(req): Json<MigrateScanRequest>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/migrate/scan";
    let path = std::path::PathBuf::from(&req.path);
    if !path.exists() {
        return api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Directory not found",
            format!("Path does not exist: {}", req.path),
            Some("Pass an absolute or relative path to an existing directory."),
        );
    }
    let scan = openfang_migrate::openclaw::scan_openclaw_workspace(&path);
    (StatusCode::OK, Json(serde_json::json!(scan)))
}

/// POST /api/migrate — Run migration from another agent framework.
pub async fn run_migrate(
    ext: Option<Extension<RequestId>>,
    Json(req): Json<MigrateRequest>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/migrate";
    let source = match req.source.as_str() {
        "openclaw" => openfang_migrate::MigrateSource::OpenClaw,
        "langchain" => openfang_migrate::MigrateSource::LangChain,
        "autogpt" => openfang_migrate::MigrateSource::AutoGpt,
        other => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Unknown migration source",
                format!("Unknown source '{other}'. Use 'openclaw', 'langchain', or 'autogpt'."),
                None,
            );
        }
    };

    let options = openfang_migrate::MigrateOptions {
        source,
        source_dir: std::path::PathBuf::from(&req.source_dir),
        target_dir: std::path::PathBuf::from(&req.target_dir),
        dry_run: req.dry_run,
    };

    match openfang_migrate::run_migration(&options) {
        Ok(report) => {
            let imported: Vec<serde_json::Value> = report
                .imported
                .iter()
                .map(|i| {
                    serde_json::json!({
                        "kind": format!("{}", i.kind),
                        "name": i.name,
                        "destination": i.destination,
                    })
                })
                .collect();

            let skipped: Vec<serde_json::Value> = report
                .skipped
                .iter()
                .map(|s| {
                    serde_json::json!({
                        "kind": format!("{}", s.kind),
                        "name": s.name,
                        "reason": s.reason,
                    })
                })
                .collect();

            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "completed",
                    "dry_run": req.dry_run,
                    "imported": imported,
                    "imported_count": imported.len(),
                    "skipped": skipped,
                    "skipped_count": skipped.len(),
                    "warnings": report.warnings,
                    "report_markdown": report.to_markdown(),
                })),
            )
        }
        Err(e) => api_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &rid,
            PATH,
            "Migration failed",
            format!("{e}"),
            Some("Check source_dir/target_dir permissions and logs."),
        ),
    }
}

// ── Model Catalog Endpoints ─────────────────────────────────────────

/// GET /api/models — List all models in the catalog.
///
/// Query parameters:
/// - `provider` — filter by provider (e.g. `?provider=anthropic`)
/// - `tier` — filter by tier (e.g. `?tier=smart`)
/// - `available` — only show models from configured providers (`?available=true`)
pub async fn list_models(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let catalog = state
        .kernel
        .model_catalog
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let provider_filter = params.get("provider").map(|s| s.to_lowercase());
    let tier_filter = params.get("tier").map(|s| s.to_lowercase());
    let available_only = params
        .get("available")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);

    let models: Vec<serde_json::Value> = catalog
        .list_models()
        .iter()
        .filter(|m| {
            if let Some(ref p) = provider_filter {
                if m.provider.to_lowercase() != *p {
                    return false;
                }
            }
            if let Some(ref t) = tier_filter {
                if m.tier.to_string() != *t {
                    return false;
                }
            }
            if available_only {
                let provider = catalog.get_provider(&m.provider);
                if let Some(p) = provider {
                    if p.auth_status == openfang_types::model_catalog::AuthStatus::Missing {
                        return false;
                    }
                }
            }
            true
        })
        .map(|m| {
            // Custom models from unknown providers are assumed available
            let available = catalog
                .get_provider(&m.provider)
                .map(|p| p.auth_status != openfang_types::model_catalog::AuthStatus::Missing)
                .unwrap_or(m.tier == openfang_types::model_catalog::ModelTier::Custom);
            serde_json::json!({
                "id": m.id,
                "display_name": m.display_name,
                "provider": m.provider,
                "tier": m.tier,
                "context_window": m.context_window,
                "max_output_tokens": m.max_output_tokens,
                "input_cost_per_m": m.input_cost_per_m,
                "output_cost_per_m": m.output_cost_per_m,
                "supports_tools": m.supports_tools,
                "supports_vision": m.supports_vision,
                "supports_streaming": m.supports_streaming,
                "available": available,
            })
        })
        .collect();

    let total = catalog.list_models().len();
    let available_count = catalog.available_models().len();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "models": models,
            "total": total,
            "available": available_count,
        })),
    )
}

/// GET /api/models/aliases — List all alias-to-model mappings.
pub async fn list_aliases(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let aliases = state
        .kernel
        .model_catalog
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .list_aliases()
        .clone();
    let entries: Vec<serde_json::Value> = aliases
        .iter()
        .map(|(alias, model_id)| {
            serde_json::json!({
                "alias": alias,
                "model_id": model_id,
            })
        })
        .collect();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "aliases": entries,
            "total": entries.len(),
        })),
    )
}

/// GET /api/models/{id} — Get a single model by ID or alias.
pub async fn get_model(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/models/:id";
    let catalog = state
        .kernel
        .model_catalog
        .read()
        .unwrap_or_else(|e| e.into_inner());
    match catalog.find_model(&id) {
        Some(m) => {
            let available = catalog
                .get_provider(&m.provider)
                .map(|p| p.auth_status != openfang_types::model_catalog::AuthStatus::Missing)
                .unwrap_or(m.tier == openfang_types::model_catalog::ModelTier::Custom);
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "id": m.id,
                    "display_name": m.display_name,
                    "provider": m.provider,
                    "tier": m.tier,
                    "context_window": m.context_window,
                    "max_output_tokens": m.max_output_tokens,
                    "input_cost_per_m": m.input_cost_per_m,
                    "output_cost_per_m": m.output_cost_per_m,
                    "supports_tools": m.supports_tools,
                    "supports_vision": m.supports_vision,
                    "supports_streaming": m.supports_streaming,
                    "aliases": m.aliases,
                    "available": available,
                })),
            )
        }
        None => api_json_error(
            StatusCode::NOT_FOUND,
            &rid,
            PATH,
            "Model not found",
            format!("No model or alias matching '{id}'."),
            Some("Use GET /api/models to list catalog entries."),
        ),
    }
}

/// GET /api/providers — List all providers with auth status.
///
/// For local providers (ollama, vllm, lmstudio), also probes reachability and
/// discovers available models via their health endpoints.
///
/// Probes run **concurrently** and results are **cached for 60 seconds** so the
/// endpoint responds instantly on repeated dashboard loads even when local
/// providers are unreachable (fixes #474).
pub async fn list_providers(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let provider_list: Vec<openfang_types::model_catalog::ProviderInfo> = {
        let catalog = state
            .kernel
            .model_catalog
            .read()
            .unwrap_or_else(|e| e.into_inner());
        catalog.list_providers().to_vec()
    };

    // Collect local providers that need probing
    let local_providers: Vec<(usize, String, String)> = provider_list
        .iter()
        .enumerate()
        .filter(|(_, p)| !p.key_required && !p.base_url.is_empty())
        .map(|(i, p)| (i, p.id.clone(), p.base_url.clone()))
        .collect();

    // Fire all probes concurrently (cached results return instantly)
    let cache = &state.provider_probe_cache;
    let probe_futures: Vec<_> = local_providers
        .iter()
        .map(|(_, id, url)| {
            openfang_runtime::provider_health::probe_provider_cached(id, url, cache)
        })
        .collect();
    let probe_results = futures::future::join_all(probe_futures).await;

    // Index probe results by provider list position for O(1) lookup
    let mut probe_map: HashMap<usize, openfang_runtime::provider_health::ProbeResult> =
        HashMap::with_capacity(local_providers.len());
    for ((idx, _, _), result) in local_providers.iter().zip(probe_results.into_iter()) {
        probe_map.insert(*idx, result);
    }

    let mut providers: Vec<serde_json::Value> = Vec::with_capacity(provider_list.len());

    for (i, p) in provider_list.iter().enumerate() {
        let mut entry = serde_json::json!({
            "id": p.id,
            "display_name": p.display_name,
            "auth_status": p.auth_status,
            "model_count": p.model_count,
            "key_required": p.key_required,
            "api_key_env": p.api_key_env,
            "base_url": p.base_url,
        });

        // For local providers, attach the probe result
        if let Some(probe) = probe_map.remove(&i) {
            entry["is_local"] = serde_json::json!(true);
            entry["reachable"] = serde_json::json!(probe.reachable);
            entry["latency_ms"] = serde_json::json!(probe.latency_ms);
            if !probe.discovered_models.is_empty() {
                entry["discovered_models"] = serde_json::json!(probe.discovered_models);
                // Merge discovered models into the catalog so agents can use them
                if let Ok(mut catalog) = state.kernel.model_catalog.write() {
                    catalog.merge_discovered_models(&p.id, &probe.discovered_models);
                }
            }
            if let Some(err) = &probe.error {
                entry["error"] = serde_json::json!(err);
            }
        } else if !p.key_required {
            // Local provider with empty base_url (e.g. claude-code) — skip probing
            entry["is_local"] = serde_json::json!(true);
        }

        providers.push(entry);
    }

    let total = providers.len();
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "providers": providers,
            "total": total,
        })),
    )
}

/// POST /api/models/custom — Add a custom model to the catalog.
///
/// Persists to `~/.openfang/custom_models.json` and makes the model immediately
/// available for agent assignment.
pub async fn add_custom_model(
    State(state): State<Arc<AppState>>,
    ext: Option<Extension<RequestId>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/models/custom";
    let id = body
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let provider = body
        .get("provider")
        .and_then(|v| v.as_str())
        .unwrap_or("openrouter")
        .to_string();
    let context_window = body
        .get("context_window")
        .and_then(|v| v.as_u64())
        .unwrap_or(128_000);
    let max_output = body
        .get("max_output_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(8_192);

    if id.is_empty() {
        return api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Missing model id",
            "JSON body must include non-empty 'id'.".to_string(),
            Some("Optional: display_name, provider, context_window, pricing fields."),
        );
    }

    let display = body
        .get("display_name")
        .and_then(|v| v.as_str())
        .unwrap_or(&id)
        .to_string();

    let entry = openfang_types::model_catalog::ModelCatalogEntry {
        id: id.clone(),
        display_name: display,
        provider: provider.clone(),
        tier: openfang_types::model_catalog::ModelTier::Custom,
        context_window,
        max_output_tokens: max_output,
        input_cost_per_m: body
            .get("input_cost_per_m")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
        output_cost_per_m: body
            .get("output_cost_per_m")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
        supports_tools: body
            .get("supports_tools")
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        supports_vision: body
            .get("supports_vision")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        supports_streaming: body
            .get("supports_streaming")
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        aliases: vec![],
    };

    let mut catalog = state
        .kernel
        .model_catalog
        .write()
        .unwrap_or_else(|e| e.into_inner());

    if !catalog.add_custom_model(entry) {
        return api_json_error(
            StatusCode::CONFLICT,
            &rid,
            PATH,
            "Model already exists",
            format!("Model '{id}' already exists for provider '{provider}'."),
            Some("Remove the existing entry or choose a different id."),
        );
    }

    // Persist to disk
    let custom_path = state.kernel.config.home_dir.join("custom_models.json");
    if let Err(e) = catalog.save_custom_models(&custom_path) {
        tracing::warn!("Failed to persist custom models: {e}");
    }

    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "id": id,
            "provider": provider,
            "status": "added"
        })),
    )
}

/// DELETE /api/models/custom/{id} — Remove a custom model.
pub async fn remove_custom_model(
    State(state): State<Arc<AppState>>,
    ext: Option<Extension<RequestId>>,
    axum::extract::Path(model_id): axum::extract::Path<String>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/models/custom/:id";
    let mut catalog = state
        .kernel
        .model_catalog
        .write()
        .unwrap_or_else(|e| e.into_inner());

    if !catalog.remove_custom_model(&model_id) {
        return api_json_error(
            StatusCode::NOT_FOUND,
            &rid,
            PATH,
            "Custom model not found",
            format!("No custom model '{model_id}' in the catalog."),
            Some("List models with GET /api/models?available=false."),
        );
    }

    let custom_path = state.kernel.config.home_dir.join("custom_models.json");
    if let Err(e) = catalog.save_custom_models(&custom_path) {
        tracing::warn!("Failed to persist custom models: {e}");
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "removed"})),
    )
}

// ── A2A (Agent-to-Agent) Protocol Endpoints ─────────────────────────

/// GET /.well-known/agent.json — A2A Agent Card for the default agent.
pub async fn a2a_agent_card(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let agents = state.kernel.registry.list();
    let base_url = format!("http://{}", state.kernel.config.api_listen);

    if let Some(first) = agents.first() {
        let card = openfang_runtime::a2a::build_agent_card(&first.manifest, &base_url);
        (
            StatusCode::OK,
            Json(serde_json::to_value(&card).unwrap_or_default()),
        )
    } else {
        let card = serde_json::json!({
            "name": "openfang",
            "description": "ArmaraOS Agent OS — no agents spawned yet",
            "url": format!("{base_url}/a2a"),
            "version": "0.1.0",
            "capabilities": { "streaming": true },
            "skills": [],
            "defaultInputModes": ["text"],
            "defaultOutputModes": ["text"],
        });
        (StatusCode::OK, Json(card))
    }
}

/// GET /a2a/agents — List all A2A agent cards.
pub async fn a2a_list_agents(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let agents = state.kernel.registry.list();
    let base_url = format!("http://{}", state.kernel.config.api_listen);

    let cards: Vec<serde_json::Value> = agents
        .iter()
        .map(|entry| {
            let card = openfang_runtime::a2a::build_agent_card(&entry.manifest, &base_url);
            serde_json::to_value(&card).unwrap_or_default()
        })
        .collect();

    let total = cards.len();
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "agents": cards,
            "total": total,
        })),
    )
}

/// POST /a2a/tasks/send — Submit a task to an agent via A2A.
pub async fn a2a_send_task(
    State(state): State<Arc<AppState>>,
    ext: Option<Extension<RequestId>>,
    Json(request): Json<serde_json::Value>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/a2a/tasks/send";
    // Extract message text from A2A format
    let message_text = request["params"]["message"]["parts"]
        .as_array()
        .and_then(|parts| {
            parts.iter().find_map(|p| {
                if p["type"].as_str() == Some("text") {
                    p["text"].as_str().map(String::from)
                } else {
                    None
                }
            })
        })
        .unwrap_or_else(|| "No message provided".to_string());

    // Find target agent (use first available or specified)
    let agents = state.kernel.registry.list();
    if agents.is_empty() {
        return api_json_error(
            StatusCode::NOT_FOUND,
            &rid,
            PATH,
            "No agents available",
            "Spawn at least one agent before using A2A task send.".to_string(),
            Some("POST /api/agents to spawn an agent."),
        );
    }

    let agent = &agents[0];
    let task_id = uuid::Uuid::new_v4().to_string();
    let session_id = request["params"]["sessionId"].as_str().map(String::from);

    // Create the task in the store as Working
    let task = openfang_runtime::a2a::A2aTask {
        id: task_id.clone(),
        session_id: session_id.clone(),
        status: openfang_runtime::a2a::A2aTaskStatus::Working.into(),
        messages: vec![openfang_runtime::a2a::A2aMessage {
            role: "user".to_string(),
            parts: vec![openfang_runtime::a2a::A2aPart::Text {
                text: message_text.clone(),
            }],
        }],
        artifacts: vec![],
    };
    state.kernel.a2a_task_store.insert(task);

    // Send message to agent
    match state.kernel.send_message(agent.id, &message_text).await {
        Ok(result) => {
            let response_msg = openfang_runtime::a2a::A2aMessage {
                role: "agent".to_string(),
                parts: vec![openfang_runtime::a2a::A2aPart::Text {
                    text: result.response,
                }],
            };
            state
                .kernel
                .a2a_task_store
                .complete(&task_id, response_msg, vec![]);
            match state.kernel.a2a_task_store.get(&task_id) {
                Some(completed_task) => (
                    StatusCode::OK,
                    Json(serde_json::to_value(&completed_task).unwrap_or_default()),
                ),
                None => api_json_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &rid,
                    PATH,
                    "Task state lost",
                    "Task disappeared after completion.".to_string(),
                    Some("This is an internal consistency error — retry the request."),
                ),
            }
        }
        Err(e) => {
            let error_msg = openfang_runtime::a2a::A2aMessage {
                role: "agent".to_string(),
                parts: vec![openfang_runtime::a2a::A2aPart::Text {
                    text: format!("Error: {e}"),
                }],
            };
            state.kernel.a2a_task_store.fail(&task_id, error_msg);
            match state.kernel.a2a_task_store.get(&task_id) {
                Some(failed_task) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::to_value(&failed_task).unwrap_or_default()),
                ),
                None => api_json_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &rid,
                    PATH,
                    "Agent error",
                    format!("{e}"),
                    Some("Check LLM provider, agent logs, and /api/budget."),
                ),
            }
        }
    }
}

/// GET /a2a/tasks/{id} — Get task status from the task store.
pub async fn a2a_get_task(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/a2a/tasks/:id";
    match state.kernel.a2a_task_store.get(&task_id) {
        Some(task) => (
            StatusCode::OK,
            Json(serde_json::to_value(&task).unwrap_or_default()),
        ),
        None => api_json_error(
            StatusCode::NOT_FOUND,
            &rid,
            PATH,
            "Task not found",
            format!("No task with id '{task_id}'."),
            Some("Tasks are ephemeral — use the id returned from POST /a2a/tasks/send."),
        ),
    }
}

/// POST /a2a/tasks/{id}/cancel — Cancel a tracked task.
pub async fn a2a_cancel_task(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/a2a/tasks/:id/cancel";
    if state.kernel.a2a_task_store.cancel(&task_id) {
        match state.kernel.a2a_task_store.get(&task_id) {
            Some(task) => (
                StatusCode::OK,
                Json(serde_json::to_value(&task).unwrap_or_default()),
            ),
            None => api_json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &rid,
                PATH,
                "Task state lost",
                "Task disappeared after cancellation.".to_string(),
                Some("Retry or submit a new task."),
            ),
        }
    } else {
        api_json_error(
            StatusCode::NOT_FOUND,
            &rid,
            PATH,
            "Task not found",
            format!("No task with id '{task_id}'."),
            None,
        )
    }
}

// ── A2A Management Endpoints (outbound) ─────────────────────────────────

/// GET /api/a2a/agents — List discovered external A2A agents.
pub async fn a2a_list_external_agents(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let agents = state
        .kernel
        .a2a_external_agents
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let items: Vec<serde_json::Value> = agents
        .iter()
        .map(|(_, card)| {
            serde_json::json!({
                "name": card.name,
                "url": card.url,
                "description": card.description,
                "skills": card.skills,
                "version": card.version,
            })
        })
        .collect();
    Json(serde_json::json!({"agents": items, "total": items.len()}))
}

/// POST /api/a2a/discover — Discover a new external A2A agent by URL.
pub async fn a2a_discover_external(
    State(state): State<Arc<AppState>>,
    ext: Option<Extension<RequestId>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/a2a/discover";
    let url = match body["url"].as_str() {
        Some(u) => u.to_string(),
        None => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Missing url",
                "JSON body must include string 'url'.".to_string(),
                Some("Pass the agent card base URL for discovery."),
            )
        }
    };

    let client = openfang_runtime::a2a::A2aClient::new();
    match client.discover(&url).await {
        Ok(card) => {
            let card_json = serde_json::to_value(&card).unwrap_or_default();
            // Store in kernel's external agents list
            {
                let mut agents = state
                    .kernel
                    .a2a_external_agents
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                // Update or add
                if let Some(existing) = agents.iter_mut().find(|(u, _)| u == &url) {
                    existing.1 = card;
                } else {
                    agents.push((url.clone(), card));
                }
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "url": url,
                    "agent": card_json,
                })),
            )
        }
        Err(e) => api_json_error(
            StatusCode::BAD_GATEWAY,
            &rid,
            PATH,
            "A2A discover failed",
            e,
            Some("Verify the URL serves a valid agent card."),
        ),
    }
}

/// POST /api/a2a/send — Send a task to an external A2A agent.
pub async fn a2a_send_external(
    State(_state): State<Arc<AppState>>,
    ext: Option<Extension<RequestId>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/a2a/send";
    let url = match body["url"].as_str() {
        Some(u) => u.to_string(),
        None => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Missing url",
                "JSON body must include string 'url'.".to_string(),
                None,
            )
        }
    };
    let message = match body["message"].as_str() {
        Some(m) => m.to_string(),
        None => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Missing message",
                "JSON body must include string 'message'.".to_string(),
                None,
            )
        }
    };
    let session_id = body["session_id"].as_str();

    let client = openfang_runtime::a2a::A2aClient::new();
    match client.send_task(&url, &message, session_id).await {
        Ok(task) => (
            StatusCode::OK,
            Json(serde_json::to_value(&task).unwrap_or_default()),
        ),
        Err(e) => api_json_error(
            StatusCode::BAD_GATEWAY,
            &rid,
            PATH,
            "A2A send failed",
            e,
            Some("Verify the remote agent URL and protocol compatibility."),
        ),
    }
}

/// GET /api/a2a/tasks/{id}/status — Get task status from an external A2A agent.
pub async fn a2a_external_task_status(
    State(_state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
    ext: Option<Extension<RequestId>>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/a2a/tasks/:id/status";
    let url = match params.get("url") {
        Some(u) => u.clone(),
        None => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Missing url parameter",
                "Query string must include 'url' for the remote agent.".to_string(),
                None,
            )
        }
    };

    let client = openfang_runtime::a2a::A2aClient::new();
    match client.get_task(&url, &task_id).await {
        Ok(task) => (
            StatusCode::OK,
            Json(serde_json::to_value(&task).unwrap_or_default()),
        ),
        Err(e) => api_json_error(
            StatusCode::BAD_GATEWAY,
            &rid,
            PATH,
            "External task status failed",
            e,
            Some("Verify task_id and remote agent availability."),
        ),
    }
}

// ── MCP HTTP Endpoint ───────────────────────────────────────────────────

/// POST /mcp — Handle MCP JSON-RPC requests over HTTP.
///
/// Exposes the same MCP protocol normally served via stdio, allowing
/// external MCP clients to connect over HTTP instead.
pub async fn mcp_http(
    State(state): State<Arc<AppState>>,
    Json(request): Json<serde_json::Value>,
) -> impl IntoResponse {
    // Gather all available tools (builtin + skills + MCP)
    let mut tools = builtin_tool_definitions();
    {
        let registry = state
            .kernel
            .skill_registry
            .read()
            .unwrap_or_else(|e| e.into_inner());
        for skill_tool in registry.all_tool_definitions() {
            tools.push(openfang_types::tool::ToolDefinition {
                name: skill_tool.name.clone(),
                description: skill_tool.description.clone(),
                input_schema: skill_tool.input_schema.clone(),
            });
        }
    }
    if let Ok(mcp_tools) = state.kernel.mcp_tools.lock() {
        tools.extend(mcp_tools.iter().cloned());
    }

    // Check if this is a tools/call that needs real execution
    let method = request["method"].as_str().unwrap_or("");
    if method == "tools/call" {
        let tool_name = request["params"]["name"].as_str().unwrap_or("");
        let arguments = request["params"]
            .get("arguments")
            .cloned()
            .unwrap_or(serde_json::json!({}));

        // Verify the tool exists
        if !tools.iter().any(|t| t.name == tool_name) {
            return Json(serde_json::json!({
                "jsonrpc": "2.0",
                "id": request.get("id").cloned(),
                "error": {"code": -32602, "message": format!("Unknown tool: {tool_name}")}
            }));
        }

        // Snapshot skill registry before async call (RwLockReadGuard is !Send)
        let skill_snapshot = state
            .kernel
            .skill_registry
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .snapshot();

        // Execute the tool via the kernel's tool runner
        let kernel_handle: Arc<dyn openfang_runtime::kernel_handle::KernelHandle> =
            state.kernel.clone() as Arc<dyn openfang_runtime::kernel_handle::KernelHandle>;
        let ainl_library_root = state.kernel.config.home_dir.join("ainl-library");
        let result = openfang_runtime::tool_runner::execute_tool(
            "mcp-http",
            tool_name,
            &arguments,
            Some(&kernel_handle),
            None,
            None,
            Some(&skill_snapshot),
            Some(&state.kernel.mcp_connections),
            Some(&state.kernel.web_ctx),
            Some(&state.kernel.browser_ctx),
            None,
            None,
            Some(ainl_library_root.as_path()),
            Some(&state.kernel.media_engine),
            None, // exec_policy
            if state.kernel.config.tts.enabled {
                Some(&state.kernel.tts_engine)
            } else {
                None
            },
            if state.kernel.config.docker.enabled {
                Some(&state.kernel.config.docker)
            } else {
                None
            },
            Some(&*state.kernel.process_manager),
            None,
        )
        .await;

        return Json(serde_json::json!({
            "jsonrpc": "2.0",
            "id": request.get("id").cloned(),
            "result": {
                "content": [{"type": "text", "text": result.content}],
                "isError": result.is_error,
            }
        }));
    }

    // For non-tools/call methods (initialize, tools/list, etc.), delegate to the handler
    let response = openfang_runtime::mcp_server::handle_mcp_request(&request, &tools).await;
    Json(response)
}

// ── Multi-Session Endpoints ─────────────────────────────────────────────

/// GET /api/agents/{id}/sessions — List all sessions for an agent.
pub async fn list_agent_sessions(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/agents/:id/sessions";
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid agent ID",
                "Agent id must be a valid UUID.".to_string(),
                Some("Use GET /api/agents to list ids."),
            )
        }
    };
    match state.kernel.list_agent_sessions(agent_id) {
        Ok(sessions) => (
            StatusCode::OK,
            Json(serde_json::json!({"sessions": sessions})),
        ),
        Err(e) => api_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &rid,
            PATH,
            "List sessions failed",
            format!("{e}"),
            Some("Check database health."),
        ),
    }
}

/// POST /api/agents/{id}/sessions — Create a new session for an agent.
pub async fn create_agent_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/agents/:id/sessions";
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid agent ID",
                "Agent id must be a valid UUID.".to_string(),
                Some("Use GET /api/agents to list ids."),
            )
        }
    };
    let label = req.get("label").and_then(|v| v.as_str());
    match state.kernel.create_agent_session(agent_id, label) {
        Ok(session) => (StatusCode::OK, Json(session)),
        Err(e) => api_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &rid,
            PATH,
            "Create session failed",
            format!("{e}"),
            Some("Check database health and agent state."),
        ),
    }
}

/// POST /api/agents/{id}/sessions/{session_id}/switch — Switch to an existing session.
pub async fn switch_agent_session(
    State(state): State<Arc<AppState>>,
    Path((id, session_id_str)): Path<(String, String)>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/agents/:id/sessions/:session_id/switch";
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid agent ID",
                "Agent id must be a valid UUID.".to_string(),
                Some("Use GET /api/agents to list ids."),
            )
        }
    };
    let session_id = match session_id_str.parse::<uuid::Uuid>() {
        Ok(uuid) => openfang_types::agent::SessionId(uuid),
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid session ID",
                "session_id must be a valid UUID.".to_string(),
                Some("Use GET /api/agents/:id/sessions to list sessions."),
            )
        }
    };
    match state.kernel.switch_agent_session(agent_id, session_id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "message": "Session switched"})),
        ),
        Err(e) => api_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &rid,
            PATH,
            "Switch session failed",
            format!("{e}"),
            Some("Verify the session belongs to this agent."),
        ),
    }
}

// ── Extended Chat Command API Endpoints ─────────────────────────────────

/// POST /api/agents/{id}/session/reset — Reset an agent's session.
pub async fn reset_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/agents/:id/session/reset";
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid agent ID",
                "Agent id must be a valid UUID.".to_string(),
                Some("Use GET /api/agents to list ids."),
            )
        }
    };
    match state.kernel.reset_session(agent_id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "message": "Session reset"})),
        ),
        Err(e) => api_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &rid,
            PATH,
            "Session reset failed",
            format!("{e}"),
            None,
        ),
    }
}

/// DELETE /api/agents/{id}/history — Clear ALL conversation history for an agent.
pub async fn clear_agent_history(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/agents/:id/history";
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid agent ID",
                "Agent id must be a valid UUID.".to_string(),
                Some("Use GET /api/agents to list ids."),
            )
        }
    };
    if state.kernel.registry.get(agent_id).is_none() {
        return api_json_error(
            StatusCode::NOT_FOUND,
            &rid,
            PATH,
            "Agent not found",
            format!("No agent registered for id {id}."),
            None,
        );
    }
    match state.kernel.clear_agent_history(agent_id) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "message": "All history cleared"})),
        ),
        Err(e) => api_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &rid,
            PATH,
            "Clear history failed",
            format!("{e}"),
            None,
        ),
    }
}

/// POST /api/agents/{id}/session/compact — Trigger LLM session compaction.
pub async fn compact_session(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/agents/:id/session/compact";
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid agent ID",
                "Agent id must be a valid UUID.".to_string(),
                Some("Use GET /api/agents to list ids."),
            )
        }
    };
    match state.kernel.compact_agent_session(agent_id).await {
        Ok(msg) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "message": msg})),
        ),
        Err(e) => api_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &rid,
            PATH,
            "Session compaction failed",
            format!("{e}"),
            Some("Check LLM provider and session size."),
        ),
    }
}

// ── Slash Templates ──────────────────────────────────────────────────────────

const SLASH_TEMPLATES_FILE: &str = "slash-templates.json";

/// GET /api/slash-templates — Load saved slash templates from disk.
///
/// Returns `{"templates": [...]}` where each entry is `{"name":"...","text":"..."}`.
/// Returns an empty array (not 404) when the file doesn't exist yet.
pub async fn get_slash_templates(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let path = state.kernel.config.home_dir.join(SLASH_TEMPLATES_FILE);
    let body = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(ref e) if e.kind() == std::io::ErrorKind::NotFound => {
            return (StatusCode::OK, Json(serde_json::json!({ "templates": [] }))).into_response();
        }
        Err(e) => {
            tracing::warn!("Failed to read slash-templates.json: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };
    match serde_json::from_str::<serde_json::Value>(&body) {
        Ok(v) => (StatusCode::OK, Json(v)).into_response(),
        Err(e) => {
            tracing::warn!("slash-templates.json is invalid JSON: {e}");
            (StatusCode::OK, Json(serde_json::json!({ "templates": [] }))).into_response()
        }
    }
}

/// PUT /api/slash-templates — Persist the full template list to disk.
///
/// Body: `{"templates": [{"name":"...","text":"..."},...]}`.
/// Writes atomically via a temp file to avoid corruption on crash.
pub async fn put_slash_templates(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let path = state.kernel.config.home_dir.join(SLASH_TEMPLATES_FILE);

    // Validate — must have a "templates" array
    if !body.get("templates").map(|v| v.is_array()).unwrap_or(false) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "body must be {\"templates\":[...]}" })),
        )
            .into_response();
    }

    let json = match serde_json::to_string_pretty(&body) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response()
        }
    };

    // Atomic write: write to .tmp then rename
    let tmp_path = path.with_extension("json.tmp");
    if let Err(e) = std::fs::write(&tmp_path, &json) {
        tracing::warn!("Failed to write slash-templates.json.tmp: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response();
    }
    if let Err(e) = std::fs::rename(&tmp_path, &path) {
        tracing::warn!("Failed to rename slash-templates.json.tmp: {e}");
        let _ = std::fs::remove_file(&tmp_path);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response();
    }

    (StatusCode::OK, Json(serde_json::json!({ "status": "ok" }))).into_response()
}

const UI_PREFS_FILE: &str = "ui-prefs.json";

/// GET /api/ui-prefs — Load persisted dashboard UI preferences from disk.
///
/// Returns a JSON object of arbitrary key/value pairs (e.g. `{"pinned_agents":["id1","id2"]}`).
/// Returns `{}` (not 404) when the file doesn't exist yet.
pub async fn get_ui_prefs(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let path = state.kernel.config.home_dir.join(UI_PREFS_FILE);
    let body = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(ref e) if e.kind() == std::io::ErrorKind::NotFound => {
            return (StatusCode::OK, Json(serde_json::json!({}))).into_response();
        }
        Err(e) => {
            tracing::warn!("Failed to read ui-prefs.json: {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };
    match serde_json::from_str::<serde_json::Value>(&body) {
        Ok(v) => (StatusCode::OK, Json(v)).into_response(),
        Err(e) => {
            tracing::warn!("ui-prefs.json is invalid JSON: {e}");
            (StatusCode::OK, Json(serde_json::json!({}))).into_response()
        }
    }
}

/// PUT /api/ui-prefs — Persist dashboard UI preferences to disk.
///
/// Body: any JSON object. Existing keys are replaced wholesale (full overwrite).
/// Writes atomically via a temp file to avoid corruption on crash.
pub async fn put_ui_prefs(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    if !body.is_object() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "body must be a JSON object" })),
        )
            .into_response();
    }

    let path = state.kernel.config.home_dir.join(UI_PREFS_FILE);
    let json = match serde_json::to_string_pretty(&body) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response()
        }
    };

    let tmp_path = path.with_extension("json.tmp");
    if let Err(e) = std::fs::write(&tmp_path, &json) {
        tracing::warn!("Failed to write ui-prefs.json.tmp: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response();
    }
    if let Err(e) = std::fs::rename(&tmp_path, &path) {
        tracing::warn!("Failed to rename ui-prefs.json.tmp: {e}");
        let _ = std::fs::remove_file(&tmp_path);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response();
    }

    (StatusCode::OK, Json(serde_json::json!({ "status": "ok" }))).into_response()
}

// ─────────────────────────────────────────────────────────────────────────────

/// Request body for `POST /api/agents/{id}/btw`.
#[derive(serde::Deserialize)]
pub struct BtwRequest {
    pub text: String,
}

/// POST /api/agents/{id}/btw — Inject context into a currently-running agent loop.
///
/// Works only while the agent is mid-run (streaming or blocking). Returns 409
/// Conflict if the agent is not currently executing. The injected text is added
/// as a `[btw] …` user message at the start of the next loop iteration.
pub async fn inject_btw(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
    Json(req): Json<BtwRequest>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/agents/:id/btw";

    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid agent ID",
                "Agent id must be a valid UUID.".to_string(),
                Some("Use GET /api/agents to list ids."),
            )
            .into_response()
        }
    };

    let text = req.text.trim().to_string();
    if text.is_empty() {
        return api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Empty context",
            "text must not be empty.".to_string(),
            None,
        )
        .into_response();
    }

    if state.kernel.inject_btw(agent_id, text) {
        (
            StatusCode::OK,
            Json(serde_json::json!({ "status": "injected" })),
        )
            .into_response()
    } else {
        api_json_error(
            StatusCode::CONFLICT,
            &rid,
            PATH,
            "Agent not running",
            "No agent loop is currently active for this agent. Send a normal message first, then use /btw while it is running.".to_string(),
            None,
        )
        .into_response()
    }
}

/// Request body for `POST /api/agents/{id}/redirect`.
#[derive(serde::Deserialize)]
pub struct RedirectRequest {
    pub text: String,
}

/// POST /api/agents/{id}/redirect — Override the running agent loop with a new directive.
///
/// Stronger than `/btw`: injects a high-priority system message and prunes recent
/// assistant messages to break the agent's current momentum. Works only while the
/// agent is mid-run. Returns 409 Conflict if no loop is active.
pub async fn inject_redirect(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
    Json(req): Json<RedirectRequest>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/agents/:id/redirect";

    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid agent ID",
                "Agent id must be a valid UUID.".to_string(),
                Some("Use GET /api/agents to list ids."),
            )
            .into_response()
        }
    };

    let text = req.text.trim().to_string();
    if text.is_empty() {
        return api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Empty directive",
            "text must not be empty.".to_string(),
            None,
        )
        .into_response();
    }

    if state.kernel.inject_redirect(agent_id, text) {
        (
            StatusCode::OK,
            Json(serde_json::json!({ "status": "redirected" })),
        )
            .into_response()
    } else {
        api_json_error(
            StatusCode::CONFLICT,
            &rid,
            PATH,
            "Agent not running",
            "No agent loop is currently active for this agent. Send a normal message first, then use /redirect while it is running.".to_string(),
            None,
        )
        .into_response()
    }
}

/// POST /api/agents/{id}/stop — Cancel an agent's current LLM run.
pub async fn stop_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/agents/:id/stop";
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid agent ID",
                "Agent id must be a valid UUID.".to_string(),
                Some("Use GET /api/agents to list ids."),
            )
        }
    };
    match state.kernel.stop_agent_run(agent_id) {
        Ok(true) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "message": "Run cancelled"})),
        ),
        Ok(false) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "ok", "message": "No active run"})),
        ),
        Err(e) => api_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &rid,
            PATH,
            "Stop run failed",
            format!("{e}"),
            None,
        ),
    }
}

/// PUT /api/agents/{id}/model — Switch an agent's model.
pub async fn set_model(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/agents/:id/model";
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid agent ID",
                "Agent id must be a valid UUID.".to_string(),
                Some("Use GET /api/agents to list ids."),
            )
        }
    };
    let model = match body["model"].as_str() {
        Some(m) if !m.is_empty() => m,
        _ => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Missing model",
                "JSON body must include non-empty 'model'.".to_string(),
                Some("Optional 'provider' overrides resolution."),
            )
        }
    };
    let explicit_provider = body["provider"].as_str();
    match state
        .kernel
        .set_agent_model(agent_id, model, explicit_provider)
    {
        Ok(()) => {
            // Return the resolved model+provider so frontend stays in sync.
            // The model name may have been normalized (provider prefix stripped),
            // so we read it back from the registry instead of echoing the raw input.
            let (resolved_model, resolved_provider) = state
                .kernel
                .registry
                .get(agent_id)
                .map(|e| {
                    sync_agent_toml_for_kernel(state.kernel.as_ref(), &e);
                    (
                        e.manifest.model.model.clone(),
                        e.manifest.model.provider.clone(),
                    )
                })
                .unwrap_or_else(|| (model.to_string(), String::new()));
            (
                StatusCode::OK,
                Json(
                    serde_json::json!({"status": "ok", "model": resolved_model, "provider": resolved_provider}),
                ),
            )
        }
        Err(e) => api_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &rid,
            PATH,
            "Set model failed",
            format!("{e}"),
            Some("Verify the model exists in GET /api/models."),
        ),
    }
}

/// GET /api/agents/{id}/tools — Get an agent's tool allowlist/blocklist.
pub async fn get_agent_tools(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/agents/:id/tools";
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid agent ID",
                "Agent id must be a valid UUID.".to_string(),
                Some("Use GET /api/agents to list ids."),
            )
        }
    };
    let entry = match state.kernel.registry.get(agent_id) {
        Some(e) => e,
        None => {
            return api_json_error(
                StatusCode::NOT_FOUND,
                &rid,
                PATH,
                "Agent not found",
                format!("No agent registered for id {id}."),
                None,
            )
        }
    };
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "tool_allowlist": entry.manifest.tool_allowlist,
            "tool_blocklist": entry.manifest.tool_blocklist,
        })),
    )
}

/// PUT /api/agents/{id}/tools — Update an agent's tool allowlist/blocklist.
pub async fn set_agent_tools(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/agents/:id/tools";
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid agent ID",
                "Agent id must be a valid UUID.".to_string(),
                Some("Use GET /api/agents to list ids."),
            )
        }
    };
    let allowlist = body
        .get("tool_allowlist")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect::<Vec<_>>()
        });
    let blocklist = body
        .get("tool_blocklist")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect::<Vec<_>>()
        });

    if allowlist.is_none() && blocklist.is_none() {
        return api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Missing tool filters",
            "Provide 'tool_allowlist' and/or 'tool_blocklist'.".to_string(),
            None,
        );
    }

    match state
        .kernel
        .set_agent_tool_filters(agent_id, allowlist, blocklist)
    {
        Ok(()) => {
            if let Some(entry) = state.kernel.registry.get(agent_id) {
                sync_agent_toml_for_kernel(state.kernel.as_ref(), &entry);
            }
            (StatusCode::OK, Json(serde_json::json!({"status": "ok"})))
        }
        Err(e) => api_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &rid,
            PATH,
            "Update tool filters failed",
            format!("{e}"),
            None,
        ),
    }
}

// ── Per-Agent Skill & MCP Endpoints ────────────────────────────────────

/// GET /api/agents/{id}/skills — Get an agent's skill assignment info.
pub async fn get_agent_skills(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/agents/:id/skills";
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid agent ID",
                "Agent id must be a valid UUID.".to_string(),
                Some("Use GET /api/agents to list ids."),
            )
        }
    };
    let entry = match state.kernel.registry.get(agent_id) {
        Some(e) => e,
        None => {
            return api_json_error(
                StatusCode::NOT_FOUND,
                &rid,
                PATH,
                "Agent not found",
                format!("No agent registered for id {id}."),
                None,
            )
        }
    };
    let available = state
        .kernel
        .skill_registry
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .skill_names();
    let mode = if entry.manifest.skills.is_empty() {
        "all"
    } else {
        "allowlist"
    };
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "assigned": entry.manifest.skills,
            "available": available,
            "mode": mode,
        })),
    )
}

/// PUT /api/agents/{id}/skills — Update an agent's skill allowlist.
pub async fn set_agent_skills(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/agents/:id/skills";
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid agent ID",
                "Agent id must be a valid UUID.".to_string(),
                Some("Use GET /api/agents to list ids."),
            )
        }
    };
    let skills: Vec<String> = body["skills"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    match state.kernel.set_agent_skills(agent_id, skills.clone()) {
        Ok(()) => {
            if let Some(entry) = state.kernel.registry.get(agent_id) {
                sync_agent_toml_for_kernel(state.kernel.as_ref(), &entry);
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "ok", "skills": skills})),
            )
        }
        Err(e) => api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Set agent skills failed",
            format!("{e}"),
            Some("Use names from GET /api/agents/:id/skills available list."),
        ),
    }
}

/// GET /api/agents/{id}/mcp_servers — Get an agent's MCP server assignment info.
pub async fn get_agent_mcp_servers(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/agents/:id/mcp_servers";
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid agent ID",
                "Agent id must be a valid UUID.".to_string(),
                Some("Use GET /api/agents to list ids."),
            )
        }
    };
    let entry = match state.kernel.registry.get(agent_id) {
        Some(e) => e,
        None => {
            return api_json_error(
                StatusCode::NOT_FOUND,
                &rid,
                PATH,
                "Agent not found",
                format!("No agent registered for id {id}."),
                None,
            )
        }
    };
    // Collect known MCP server names from connected tools
    let mut available: Vec<String> = Vec::new();
    if let Ok(mcp_tools) = state.kernel.mcp_tools.lock() {
        let mut seen = std::collections::HashSet::new();
        for tool in mcp_tools.iter() {
            if let Some(server) = openfang_runtime::mcp::extract_mcp_server(&tool.name) {
                if seen.insert(server.to_string()) {
                    available.push(server.to_string());
                }
            }
        }
    }
    let mode = if entry.manifest.mcp_servers.is_empty() {
        "all"
    } else {
        "allowlist"
    };
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "assigned": entry.manifest.mcp_servers,
            "available": available,
            "mode": mode,
        })),
    )
}

/// PUT /api/agents/{id}/mcp_servers — Update an agent's MCP server allowlist.
pub async fn set_agent_mcp_servers(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/agents/:id/mcp_servers";
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid agent ID",
                "Agent id must be a valid UUID.".to_string(),
                Some("Use GET /api/agents to list ids."),
            )
        }
    };
    let servers: Vec<String> = body["mcp_servers"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    match state
        .kernel
        .set_agent_mcp_servers(agent_id, servers.clone())
    {
        Ok(()) => {
            if let Some(entry) = state.kernel.registry.get(agent_id) {
                sync_agent_toml_for_kernel(state.kernel.as_ref(), &entry);
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "ok", "mcp_servers": servers})),
            )
        }
        Err(e) => api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Set MCP servers failed",
            format!("{e}"),
            Some("Use server names from connected MCP tools."),
        ),
    }
}

// ── Provider Key Management Endpoints ──────────────────────────────────

/// POST /api/providers/{name}/key — Save an API key for a provider.
///
/// SECURITY: Writes to `~/.openfang/secrets.env`, sets env var in process,
/// and refreshes auth detection. Key is zeroized after use.
pub async fn set_provider_key(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    ext: Option<Extension<RequestId>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/providers/:name/key";
    let key = match body["key"].as_str() {
        Some(k) if !k.trim().is_empty() => k.trim().to_string(),
        _ => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Missing API key",
                "JSON body must include non-empty 'key'.".to_string(),
                Some("Keys are written to secrets.env and the process environment."),
            );
        }
    };

    // Look up env var from catalog; for unknown/custom providers derive one.
    let env_var = {
        let catalog = state
            .kernel
            .model_catalog
            .read()
            .unwrap_or_else(|e| e.into_inner());
        catalog
            .get_provider(&name)
            .map(|p| p.api_key_env.clone())
            .unwrap_or_else(|| {
                // Custom provider — derive env var: MY_PROVIDER → MY_PROVIDER_API_KEY
                format!("{}_API_KEY", name.to_uppercase().replace('-', "_"))
            })
    };

    // Store in vault (best-effort — no-op if vault not initialized)
    state.kernel.store_credential(&env_var, &key);

    // Write to secrets.env file (dual-write for backward compat / vault corruption recovery)
    let secrets_path = state.kernel.config.home_dir.join("secrets.env");
    if let Err(e) = write_secret_env(&secrets_path, &env_var, &key) {
        return api_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &rid,
            PATH,
            "Failed to write secrets.env",
            format!("{e}"),
            Some("Check permissions on ~/.openfang/secrets.env."),
        );
    }

    // Set env var in current process so detect_auth picks it up
    std::env::set_var(&env_var, &key);

    // Refresh auth detection
    state
        .kernel
        .model_catalog
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .detect_auth();

    // Auto-switch default provider if current default has no working key.
    // This fixes the common case where a user adds e.g. a Gemini key via dashboard
    // but their agent still tries to use the previous provider (which has no key).
    //
    // Read the effective default from the hot-reload override (if set) rather than
    // the stale boot-time config — a previous set_provider_key call may have already
    // switched the default.
    let (current_provider, current_key_env) = {
        let guard = state
            .kernel
            .default_model_override
            .read()
            .unwrap_or_else(|e| e.into_inner());
        match guard.as_ref() {
            Some(dm) => (dm.provider.clone(), dm.api_key_env.clone()),
            None => (
                state.kernel.config.default_model.provider.clone(),
                state.kernel.config.default_model.api_key_env.clone(),
            ),
        }
    };
    let current_has_key = if current_key_env.is_empty() {
        false
    } else {
        std::env::var(&current_key_env)
            .ok()
            .filter(|v| !v.is_empty())
            .is_some()
    };
    let switched = if !current_has_key && current_provider != name {
        // Find a default model for the newly-keyed provider
        let default_model = {
            let catalog = state
                .kernel
                .model_catalog
                .read()
                .unwrap_or_else(|e| e.into_inner());
            catalog.default_model_for_provider(&name)
        };
        if let Some(model_id) = default_model {
            // Update config.toml to persist the switch
            let config_path = state.kernel.config.home_dir.join("config.toml");
            let update_toml = format!(
                "\n[default_model]\nprovider = \"{}\"\nmodel = \"{}\"\napi_key_env = \"{}\"\n",
                name, model_id, env_var
            );
            backup_config(&config_path);
            let new_content = if let Ok(existing) = std::fs::read_to_string(&config_path) {
                let cleaned = remove_toml_section(&existing, "default_model");
                format!("{}\n{}", cleaned.trim(), update_toml)
            } else {
                update_toml
            };
            let tmp = config_path.with_extension("toml.tmp");
            if std::fs::write(&tmp, &new_content).is_ok() {
                let _ = std::fs::rename(&tmp, &config_path);
            }

            // Hot-update the in-memory default model override so resolve_driver()
            // immediately creates drivers for the new provider — no restart needed.
            {
                let new_dm = openfang_types::config::DefaultModelConfig {
                    provider: name.clone(),
                    model: model_id,
                    api_key_env: env_var.clone(),
                    base_url: None,
                };
                let mut guard = state
                    .kernel
                    .default_model_override
                    .write()
                    .unwrap_or_else(|e| e.into_inner());
                *guard = Some(new_dm);
            }
            true
        } else {
            false
        }
    } else if current_provider == name {
        // User is saving a key for the CURRENT default provider. The env var is
        // already set (set_var above), but we must ensure default_model_override
        // has the correct api_key_env so resolve_driver reads the right variable.
        let needs_update = {
            let guard = state
                .kernel
                .default_model_override
                .read()
                .unwrap_or_else(|e| e.into_inner());
            match guard.as_ref() {
                Some(dm) => dm.api_key_env != env_var,
                None => state.kernel.config.default_model.api_key_env != env_var,
            }
        };
        if needs_update {
            let mut guard = state
                .kernel
                .default_model_override
                .write()
                .unwrap_or_else(|e| e.into_inner());
            let base = guard
                .clone()
                .unwrap_or_else(|| state.kernel.config.default_model.clone());
            *guard = Some(openfang_types::config::DefaultModelConfig {
                api_key_env: env_var.clone(),
                ..base
            });
        }
        false
    } else {
        false
    };

    let mut resp = serde_json::json!({"status": "saved", "provider": name});
    if switched {
        resp["switched_default"] = serde_json::json!(true);
        resp["message"] = serde_json::json!(format!(
            "API key saved and default provider switched to '{}'.",
            name
        ));
    }

    (StatusCode::OK, Json(resp))
}

/// DELETE /api/providers/{name}/key — Remove an API key for a provider.
pub async fn delete_provider_key(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/providers/:name/key";
    let env_var = {
        let catalog = state
            .kernel
            .model_catalog
            .read()
            .unwrap_or_else(|e| e.into_inner());
        catalog
            .get_provider(&name)
            .map(|p| p.api_key_env.clone())
            .unwrap_or_else(|| {
                // Custom/unknown provider — derive env var from convention
                format!("{}_API_KEY", name.to_uppercase().replace('-', "_"))
            })
    };

    if env_var.is_empty() {
        return api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "No API key for provider",
            "This provider does not use a configurable API key.".to_string(),
            Some("Local providers may not need keys."),
        );
    }

    // Remove from vault (best-effort)
    state.kernel.remove_credential(&env_var);

    // Remove from secrets.env
    let secrets_path = state.kernel.config.home_dir.join("secrets.env");
    if let Err(e) = remove_secret_env(&secrets_path, &env_var) {
        return api_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &rid,
            PATH,
            "Failed to update secrets.env",
            format!("{e}"),
            Some("Check file permissions."),
        );
    }

    // Remove from process environment
    std::env::remove_var(&env_var);

    // Refresh auth detection
    state
        .kernel
        .model_catalog
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .detect_auth();

    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "removed", "provider": name})),
    )
}

/// POST /api/providers/{name}/test — Test a provider's connectivity.
pub async fn test_provider(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/providers/:name/test";
    let (env_var, base_url, key_required, default_model) = {
        let catalog = state
            .kernel
            .model_catalog
            .read()
            .unwrap_or_else(|e| e.into_inner());
        match catalog.get_provider(&name) {
            Some(p) => {
                // Find a default model for this provider to use in the test request
                let model_id = catalog
                    .default_model_for_provider(&name)
                    .unwrap_or_default();
                (
                    p.api_key_env.clone(),
                    p.base_url.clone(),
                    p.key_required,
                    model_id,
                )
            }
            None => {
                return api_json_error(
                    StatusCode::NOT_FOUND,
                    &rid,
                    PATH,
                    "Unknown provider",
                    format!("No provider '{name}' in the catalog."),
                    Some("Use GET /api/providers for valid ids."),
                );
            }
        }
    };

    let api_key = std::env::var(&env_var).ok();
    // Only require API key for providers that need one (skip local providers like ollama/vllm/lmstudio)
    if key_required && api_key.is_none() && !env_var.is_empty() {
        return api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Provider API key not configured",
            format!("Set {} via POST /api/providers/:name/key.", env_var),
            Some("Or export the variable before starting the daemon."),
        );
    }

    // Attempt a lightweight connectivity test
    let start = std::time::Instant::now();
    let driver_config = openfang_runtime::llm_driver::DriverConfig {
        provider: name.clone(),
        api_key,
        base_url: if base_url.is_empty() {
            None
        } else {
            Some(base_url)
        },
        skip_permissions: true,
        ..Default::default()
    };

    let network_hints = crate::network_hints::collect();
    match openfang_runtime::drivers::create_driver(&driver_config) {
        Ok(driver) => {
            // Send a minimal completion request to test connectivity
            let test_req = openfang_runtime::llm_driver::CompletionRequest {
                model: default_model.clone(),
                messages: vec![openfang_types::message::Message::user("Hi")],
                tools: vec![],
                max_tokens: 1,
                temperature: 0.0,
                system: None,
                thinking: None,
            };
            match driver.complete(test_req).await {
                Ok(_) => {
                    let latency_ms = start.elapsed().as_millis();
                    (
                        StatusCode::OK,
                        Json(serde_json::json!({
                            "status": "ok",
                            "provider": name,
                            "latency_ms": latency_ms,
                            "network_hints": network_hints,
                        })),
                    )
                }
                Err(e) => (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "status": "error",
                        "provider": name,
                        "error": format!("{e}"),
                        "network_hints": network_hints,
                    })),
                ),
            }
        }
        Err(e) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "error",
                "provider": name,
                "error": format!("Failed to create driver: {e}"),
                "network_hints": network_hints,
            })),
        ),
    }
}

/// PUT /api/providers/{name}/url — Set a custom base URL for a provider.
pub async fn set_provider_url(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    ext: Option<Extension<RequestId>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/providers/:name/url";
    // Accept any provider name — custom providers are supported via OpenAI-compatible format.
    let base_url = match body["base_url"].as_str() {
        Some(u) if !u.trim().is_empty() => u.trim().to_string(),
        _ => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Missing base_url",
                "JSON body must include non-empty 'base_url'.".to_string(),
                None,
            );
        }
    };

    // Validate URL scheme
    if !base_url.starts_with("http://") && !base_url.starts_with("https://") {
        return api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Invalid base_url",
            "base_url must start with http:// or https://.".to_string(),
            None,
        );
    }

    // Update catalog in memory
    {
        let mut catalog = state
            .kernel
            .model_catalog
            .write()
            .unwrap_or_else(|e| e.into_inner());
        catalog.set_provider_url(&name, &base_url);
    }

    // Persist to config.toml [provider_urls] section
    let config_path = state.kernel.config.home_dir.join("config.toml");
    if let Err(e) = upsert_provider_url(&config_path, &name, &base_url) {
        return api_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &rid,
            PATH,
            "Failed to save config",
            format!("{e}"),
            Some("Check config.toml permissions."),
        );
    }

    // Probe reachability at the new URL
    let probe = openfang_runtime::provider_health::probe_provider(&name, &base_url).await;

    // Merge discovered models into catalog
    if !probe.discovered_models.is_empty() {
        if let Ok(mut catalog) = state.kernel.model_catalog.write() {
            catalog.merge_discovered_models(&name, &probe.discovered_models);
        }
    }

    let mut resp = serde_json::json!({
        "status": "saved",
        "provider": name,
        "base_url": base_url,
        "reachable": probe.reachable,
        "latency_ms": probe.latency_ms,
    });
    if !probe.discovered_models.is_empty() {
        resp["discovered_models"] = serde_json::json!(probe.discovered_models);
    }

    (StatusCode::OK, Json(resp))
}

/// Upsert a provider URL in the `[provider_urls]` section of config.toml.
fn upsert_provider_url(
    config_path: &std::path::Path,
    provider: &str,
    url: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let content = if config_path.exists() {
        std::fs::read_to_string(config_path)?
    } else {
        String::new()
    };

    let mut doc: toml::Value = if content.trim().is_empty() {
        toml::Value::Table(toml::map::Map::new())
    } else {
        toml::from_str(&content)?
    };

    let root = doc.as_table_mut().ok_or("Config is not a TOML table")?;

    if !root.contains_key("provider_urls") {
        root.insert(
            "provider_urls".to_string(),
            toml::Value::Table(toml::map::Map::new()),
        );
    }
    let urls_table = root
        .get_mut("provider_urls")
        .and_then(|v| v.as_table_mut())
        .ok_or("provider_urls is not a table")?;

    urls_table.insert(provider.to_string(), toml::Value::String(url.to_string()));

    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let tmp = config_path.with_extension("toml.tmp");
    std::fs::write(&tmp, toml::to_string_pretty(&doc)?)?;
    std::fs::rename(&tmp, config_path)?;
    Ok(())
}

/// POST /api/skills/create — Create a local prompt-only skill.
pub async fn create_skill(
    State(state): State<Arc<AppState>>,
    ext: Option<Extension<RequestId>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/skills/create";
    let name = match body["name"].as_str() {
        Some(n) if !n.trim().is_empty() => n.trim().to_string(),
        _ => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Missing skill name",
                "JSON body must include non-empty 'name'.".to_string(),
                None,
            );
        }
    };

    // Validate name (alphanumeric + hyphens only)
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Invalid skill name",
            "Skill name must contain only letters, numbers, hyphens, and underscores.".to_string(),
            None,
        );
    }

    let description = body["description"].as_str().unwrap_or("").to_string();
    let runtime = body["runtime"].as_str().unwrap_or("prompt_only");
    let prompt_context = body["prompt_context"].as_str().unwrap_or("").to_string();

    // Only allow prompt_only skills from the web UI for safety
    if runtime != "prompt_only" {
        return api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Unsupported runtime",
            "Only prompt_only skills can be created from the web UI.".to_string(),
            None,
        );
    }

    // Write skill.toml to ~/.openfang/skills/{name}/
    let skill_dir = state.kernel.config.home_dir.join("skills").join(&name);
    if skill_dir.exists() {
        return api_json_error(
            StatusCode::CONFLICT,
            &rid,
            PATH,
            "Skill already exists",
            format!("Skill '{name}' already exists."),
            Some("Choose a different name or remove the existing directory."),
        );
    }

    if let Err(e) = std::fs::create_dir_all(&skill_dir) {
        return api_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &rid,
            PATH,
            "Failed to create skill directory",
            format!("{e}"),
            Some("Check disk space and permissions on the skills folder."),
        );
    }

    let toml_content = format!(
        "[skill]\nname = \"{}\"\ndescription = \"{}\"\nruntime = \"prompt_only\"\n\n[prompt]\ncontext = \"\"\"\n{}\n\"\"\"\n",
        name,
        description.replace('"', "\\\""),
        prompt_context
    );

    let toml_path = skill_dir.join("skill.toml");
    let toml_tmp = skill_dir.join("skill.toml.tmp");
    if let Err(e) = std::fs::write(&toml_tmp, &toml_content)
        .and_then(|_| std::fs::rename(&toml_tmp, &toml_path))
    {
        return api_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &rid,
            PATH,
            "Failed to write skill.toml",
            format!("{e}"),
            None,
        );
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "created",
            "name": name,
            "note": "Restart the daemon to load the new skill, or it will be available on next boot."
        })),
    )
}

// ── Helper functions for secrets.env management ────────────────────────

/// Write or update a key in the secrets.env file.
/// File format: one `KEY=value` per line. Existing keys are overwritten.
///
/// Uses a write-to-tempfile-then-rename pattern for atomic, Windows-safe updates.
/// On Windows, `std::fs::write` can fail with "Access denied" if any other process
/// (antivirus, the daemon itself during a hot-reload) has the file open. Writing to
/// a sibling temp file and renaming avoids the lock window.
fn write_secret_env(path: &std::path::Path, key: &str, value: &str) -> Result<(), std::io::Error> {
    // Avoid corrupting the KEY=value line format if the user accidentally pasted
    // with trailing newlines/spaces (common on Windows).
    let value = value.trim();
    if value.contains('\n') || value.contains('\r') {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "secret value contains newline characters — paste the token without surrounding text or line breaks",
        ));
    }
    let mut lines: Vec<String> = if path.exists() {
        std::fs::read_to_string(path)?
            .lines()
            .map(|l| l.to_string())
            .collect()
    } else {
        Vec::new()
    };

    // Remove existing line for this key
    lines.retain(|l| !l.starts_with(&format!("{key}=")));

    // Add new line
    lines.push(format!("{key}={value}"));

    // Ensure parent directory exists
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "secrets.env has no parent directory",
        )
    })?;
    std::fs::create_dir_all(parent)?;

    // Write to a sibling temp file first, then rename for an atomic update.
    // This prevents partial-write corruption and avoids Windows file-lock errors.
    let tmp_path = path.with_extension("env.tmp");
    std::fs::write(&tmp_path, lines.join("\n") + "\n")?;

    // SECURITY: Restrict file permissions on Unix before the rename so the
    // final file never has world-readable permissions even briefly.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o600));
    }

    std::fs::rename(&tmp_path, path)?;

    Ok(())
}

/// Remove a key from the secrets.env file.
fn remove_secret_env(path: &std::path::Path, key: &str) -> Result<(), std::io::Error> {
    if !path.exists() {
        return Ok(());
    }

    let lines: Vec<String> = std::fs::read_to_string(path)?
        .lines()
        .filter(|l| !l.starts_with(&format!("{key}=")))
        .map(|l| l.to_string())
        .collect();

    let tmp_path = path.with_extension("env.tmp");
    std::fs::write(&tmp_path, lines.join("\n") + "\n")?;
    std::fs::rename(&tmp_path, path)?;

    Ok(())
}

// ── Config.toml channel management helpers ──────────────────────────

/// Upsert a `[channels.<name>]` section in config.toml with the given non-secret fields.
fn upsert_channel_config(
    config_path: &std::path::Path,
    channel_name: &str,
    fields: &HashMap<String, (String, FieldType)>,
) -> Result<(), Box<dyn std::error::Error>> {
    let content = if config_path.exists() {
        std::fs::read_to_string(config_path)?
    } else {
        String::new()
    };

    let mut doc: toml::Value = if content.trim().is_empty() {
        toml::Value::Table(toml::map::Map::new())
    } else {
        toml::from_str(&content)?
    };

    let root = doc.as_table_mut().ok_or("Config is not a TOML table")?;

    // Ensure [channels] table exists
    if !root.contains_key("channels") {
        root.insert(
            "channels".to_string(),
            toml::Value::Table(toml::map::Map::new()),
        );
    }
    let channels_table = root
        .get_mut("channels")
        .and_then(|v| v.as_table_mut())
        .ok_or("channels is not a table")?;

    // Build channel sub-table with correct TOML types
    let mut ch_table = toml::map::Map::new();
    for (k, (v, ft)) in fields {
        let toml_val = match ft {
            FieldType::Number => {
                if let Ok(n) = v.parse::<i64>() {
                    toml::Value::Integer(n)
                } else {
                    toml::Value::String(v.clone())
                }
            }
            FieldType::List => {
                // Always store list items as strings so that numeric IDs
                // (e.g. Discord guild snowflakes, Telegram user IDs) are
                // deserialized correctly into Vec<String> config fields.
                // Accept comma-separated values, whitespace/newline-separated values,
                // or mixed (users often paste IDs one-per-line).
                let items: Vec<toml::Value> = v
                    .split(|c: char| c == ',' || c.is_whitespace())
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .map(|s| toml::Value::String(s.to_string()))
                    .collect();
                toml::Value::Array(items)
            }
            _ => toml::Value::String(v.clone()),
        };
        ch_table.insert(k.clone(), toml_val);
    }
    channels_table.insert(channel_name.to_string(), toml::Value::Table(ch_table));

    // Ensure parent directory exists
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Write to a sibling temp file first, then rename for an atomic update.
    // This prevents partial-write corruption and avoids Windows file-lock errors
    // when the daemon or another process has config.toml open.
    let toml_str = toml::to_string_pretty(&doc)?;
    let tmp_path = config_path.with_extension("toml.tmp");
    std::fs::write(&tmp_path, &toml_str)?;
    std::fs::rename(&tmp_path, config_path)?;
    Ok(())
}

/// Remove a `[channels.<name>]` section from config.toml.
fn remove_channel_config(
    config_path: &std::path::Path,
    channel_name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if !config_path.exists() {
        return Ok(());
    }

    let content = std::fs::read_to_string(config_path)?;
    if content.trim().is_empty() {
        return Ok(());
    }

    let mut doc: toml::Value = toml::from_str(&content)?;

    if let Some(channels) = doc
        .as_table_mut()
        .and_then(|r| r.get_mut("channels"))
        .and_then(|c| c.as_table_mut())
    {
        channels.remove(channel_name);
    }

    let tmp_path = config_path.with_extension("toml.tmp");
    std::fs::write(&tmp_path, toml::to_string_pretty(&doc)?)?;
    std::fs::rename(&tmp_path, config_path)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Integration management endpoints
// ---------------------------------------------------------------------------

fn json_string_map(
    value: Option<&serde_json::Value>,
) -> Result<std::collections::HashMap<String, String>, String> {
    let Some(v) = value else {
        return Ok(std::collections::HashMap::new());
    };
    let obj = v
        .as_object()
        .ok_or_else(|| "Expected JSON object".to_string())?;
    let mut out = std::collections::HashMap::new();
    for (k, val) in obj {
        if val.is_null() {
            out.insert(k.clone(), String::new());
            continue;
        }
        let s = val
            .as_str()
            .ok_or_else(|| format!("Field '{k}' must be a string"))?;
        out.insert(k.clone(), s.to_string());
    }
    Ok(out)
}

fn split_integration_payload(
    template: &openfang_extensions::IntegrationTemplate,
    env_map: &std::collections::HashMap<String, String>,
    config_map: &std::collections::HashMap<String, String>,
) -> (
    std::collections::HashMap<String, String>,
    std::collections::HashMap<String, String>,
) {
    let mut secrets = std::collections::HashMap::new();
    let mut cfg = config_map.clone();
    for (k, v) in env_map {
        if k == "allowed_paths" {
            cfg.insert(k.clone(), v.clone());
            continue;
        }
        if let Some(meta) = template.required_env.iter().find(|e| e.name == *k) {
            if meta.is_secret {
                secrets.insert(k.clone(), v.clone());
            } else {
                cfg.insert(k.clone(), v.clone());
            }
        }
    }
    (secrets, cfg)
}

/// GET /api/integrations/mcp-presets — Curated MCP presets for the dashboard installer.
pub async fn mcp_integration_presets() -> impl IntoResponse {
    Json(serde_json::json!({
        "presets": [
            {
                "preset_id": "filesystem",
                "integration_id": "filesystem",
                "title": "Filesystem",
                "subtitle": "Local files (allowed directories)",
                "order": 10
            },
            {
                "preset_id": "github",
                "integration_id": "github",
                "title": "GitHub",
                "subtitle": "Repos, issues, and pull requests",
                "order": 20
            },
            {
                "preset_id": "postgres",
                "integration_id": "postgresql",
                "title": "PostgreSQL",
                "subtitle": "Query Postgres via MCP",
                "order": 30
            },
            {
                "preset_id": "google-calendar",
                "integration_id": "google-calendar",
                "title": "Google Calendar",
                "subtitle": "OAuth — use Connect flow when available",
                "order": 40
            },
            {
                "preset_id": "apple-caldav",
                "integration_id": "apple-caldav",
                "title": "Apple / CalDAV",
                "subtitle": "iCloud calendars via CalDAV",
                "order": 50
            }
        ]
    }))
}

/// POST /api/integrations/validate — Validate a configured install payload (no side effects).
pub async fn validate_integration_config(
    State(state): State<Arc<AppState>>,
    ext: Option<Extension<RequestId>>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/integrations/validate";
    let id = match req.get("id").and_then(|v| v.as_str()) {
        Some(id) => id.to_string(),
        None => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Missing integration id",
                "JSON body must include string 'id'.".to_string(),
                Some("Use GET /api/integrations/mcp-presets for recommended IDs."),
            );
        }
    };

    let env_map = match json_string_map(req.get("env")) {
        Ok(m) => m,
        Err(e) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid env map",
                e,
                None,
            );
        }
    };
    let config_map = match json_string_map(req.get("config")) {
        Ok(m) => m,
        Err(e) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid config map",
                e,
                None,
            );
        }
    };

    let template = {
        let registry = state
            .kernel
            .extension_registry
            .read()
            .unwrap_or_else(|e| e.into_inner());
        match registry.get_template(&id) {
            Some(t) => t.clone(),
            None => {
                return api_json_error(
                    StatusCode::NOT_FOUND,
                    &rid,
                    PATH,
                    "Unknown integration",
                    format!("Unknown integration: '{id}'"),
                    Some("Use GET /api/integrations/available to list templates."),
                );
            }
        }
    };

    if let Err(e) = openfang_extensions::installer::validate_user_supplied_keys(
        &template,
        &id,
        &env_map,
        &config_map,
    ) {
        return api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Invalid integration fields",
            e.to_string(),
            None,
        );
    }

    let (secrets, cfg) = split_integration_payload(&template, &env_map, &config_map);
    let field_errors = openfang_extensions::installer::integration_payload_field_errors(
        &template, &id, &secrets, &cfg,
    );

    let ok = field_errors.is_empty();
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "id": id,
            "ok": ok,
            "field_errors": field_errors,
        })),
    )
}

/// POST /api/integrations/custom/validate — Validate a custom MCP definition (no side effects).
pub async fn validate_custom_mcp_config(Json(req): Json<serde_json::Value>) -> impl IntoResponse {
    match openfang_extensions::custom_mcp::parse_custom_mcp_payload(&req) {
        Err(field_errors) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "ok": false,
                "field_errors": field_errors,
            })),
        ),
        Ok((template, secrets, cfg)) => {
            let id = template.id.clone();
            let field_errors = openfang_extensions::custom_mcp::custom_mcp_field_errors(
                &template, &id, &secrets, &cfg,
            );
            let ok = field_errors.is_empty();
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "id": id,
                    "ok": ok,
                    "field_errors": field_errors,
                })),
            )
        }
    }
}

/// POST /api/integrations/custom/add — Install a user-defined MCP server (registry + vault).
pub async fn add_custom_mcp(
    State(state): State<Arc<AppState>>,
    ext: Option<Extension<RequestId>>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/integrations/custom/add";

    let (template, secrets, cfg) =
        match openfang_extensions::custom_mcp::parse_custom_mcp_payload(&req) {
            Ok(x) => x,
            Err(field_errors) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "request_id": rid.0,
                        "path": PATH,
                        "error": "Invalid custom MCP payload",
                        "ok": false,
                        "field_errors": field_errors,
                    })),
                );
            }
        };

    let id = template.id.clone();
    let field_errors =
        openfang_extensions::custom_mcp::custom_mcp_field_errors(&template, &id, &secrets, &cfg);
    if !field_errors.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "request_id": rid.0,
                "path": PATH,
                "error": "Invalid custom MCP fields",
                "ok": false,
                "field_errors": field_errors,
            })),
        );
    }

    let install_result = {
        let mut resolver = state
            .kernel
            .credential_resolver
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut registry = state
            .kernel
            .extension_registry
            .write()
            .unwrap_or_else(|e| e.into_inner());

        match openfang_extensions::installer::install_custom_mcp(
            &mut registry,
            &mut resolver,
            template,
            &secrets,
            &cfg,
        ) {
            Ok(result) => Ok(result),
            Err(openfang_extensions::ExtensionError::AlreadyInstalled(id)) => Err((
                StatusCode::CONFLICT,
                format!("Integration '{id}' already installed"),
            )),
            Err(openfang_extensions::ExtensionError::InvalidIntegrationId(msg)) => {
                Err((StatusCode::BAD_REQUEST, msg))
            }
            Err(e) => Err((StatusCode::BAD_REQUEST, e.to_string())),
        }
    };

    let result = match install_result {
        Ok(r) => r,
        Err((status, msg)) => {
            return api_json_error(status, &rid, PATH, "Custom MCP install failed", msg, None);
        }
    };

    state.kernel.extension_health.register(&id);

    let connected = state.kernel.reload_extension_mcps().await.unwrap_or(0);

    let integration_status = match &result.status {
        openfang_extensions::IntegrationStatus::Ready => "ready",
        openfang_extensions::IntegrationStatus::Setup => "setup",
        openfang_extensions::IntegrationStatus::Disabled => "disabled",
        openfang_extensions::IntegrationStatus::Available => "available",
        openfang_extensions::IntegrationStatus::Error(_) => "error",
    };

    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "id": id,
            "status": "installed",
            "mode": "custom",
            "integration_status": integration_status,
            "connected": connected > 0,
            "message": result.message,
            "ok": true,
        })),
    )
}

/// GET /api/integrations — List installed integrations with status.
pub async fn list_integrations(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let registry = state
        .kernel
        .extension_registry
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let health = &state.kernel.extension_health;

    let mut entries = Vec::new();
    for info in registry.list_all_info() {
        let h = health.get_health(&info.template.id);
        let status = match &info.installed {
            Some(inst) if !inst.enabled => "disabled",
            Some(_) => match h.as_ref().map(|h| &h.status) {
                Some(openfang_extensions::IntegrationStatus::Ready) => "ready",
                Some(openfang_extensions::IntegrationStatus::Error(_)) => "error",
                _ => "installed",
            },
            None => continue, // Only show installed
        };
        entries.push(serde_json::json!({
            "id": info.template.id,
            "name": info.template.name,
            "icon": info.template.icon,
            "category": info.template.category.to_string(),
            "status": status,
            "tool_count": h.as_ref().map(|h| h.tool_count).unwrap_or(0),
            "installed_at": info.installed.as_ref().map(|i| i.installed_at.to_rfc3339()),
        }));
    }

    Json(serde_json::json!({
        "installed": entries,
        "count": entries.len(),
    }))
}

/// GET /api/integrations/available — List all available templates.
pub async fn list_available_integrations(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let registry = state
        .kernel
        .extension_registry
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let templates: Vec<serde_json::Value> = registry
        .list_templates()
        .iter()
        .map(|t| {
            let installed = registry.is_installed(&t.id);
            serde_json::json!({
                "id": t.id,
                "name": t.name,
                "description": t.description,
                "icon": t.icon,
                "category": t.category.to_string(),
                "installed": installed,
                "tags": t.tags,
                "required_env": t.required_env.iter().map(|e| serde_json::json!({
                    "name": e.name,
                    "label": e.label,
                    "help": e.help,
                    "is_secret": e.is_secret,
                    "get_url": e.get_url,
                })).collect::<Vec<_>>(),
                "has_oauth": t.oauth.is_some(),
                "setup_instructions": t.setup_instructions,
            })
        })
        .collect();

    Json(serde_json::json!({
        "integrations": templates,
        "count": templates.len(),
    }))
}

/// POST /api/integrations/add — Install an integration.
///
/// Back-compat: `{ "id": "github" }` registers the integration without running the
/// credential-aware installer (legacy behavior).
///
/// Configured install: include `env` and/or `config` objects. Values are split into
/// vault secrets vs non-secret config using template metadata (`required_env.is_secret`).
pub async fn add_integration(
    State(state): State<Arc<AppState>>,
    ext: Option<Extension<RequestId>>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/integrations/add";
    let id = match req.get("id").and_then(|v| v.as_str()) {
        Some(id) => id.to_string(),
        None => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Missing integration id",
                "JSON body must include string 'id'.".to_string(),
                Some("Use GET /api/integrations/available to list templates."),
            );
        }
    };

    let use_configured = req.get("env").is_some() || req.get("config").is_some();

    if !use_configured {
        // Scope the write lock so it's dropped before any .await
        let install_err = {
            let mut registry = state
                .kernel
                .extension_registry
                .write()
                .unwrap_or_else(|e| e.into_inner());

            if registry.is_installed(&id) {
                Some((
                    StatusCode::CONFLICT,
                    format!("Integration '{}' already installed", id),
                ))
            } else if registry.get_template(&id).is_none() {
                Some((
                    StatusCode::NOT_FOUND,
                    format!("Unknown integration: '{}'", id),
                ))
            } else {
                let entry = openfang_extensions::InstalledIntegration {
                    id: id.clone(),
                    installed_at: chrono::Utc::now(),
                    enabled: true,
                    oauth_provider: None,
                    config: std::collections::HashMap::new(),
                };
                match registry.install(entry) {
                    Ok(_) => None,
                    Err(e) => Some((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
                }
            }
        }; // write lock dropped here

        if let Some((status, error)) = install_err {
            return api_json_error(
                status,
                &rid,
                PATH,
                "Integration install failed",
                error,
                None,
            );
        }

        state.kernel.extension_health.register(&id);

        let connected = state.kernel.reload_extension_mcps().await.unwrap_or(0);

        return (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "id": id,
                "status": "installed",
                "mode": "legacy",
                "connected": connected > 0,
                "message": format!("Integration '{}' installed", id),
            })),
        );
    }

    let env_map = match json_string_map(req.get("env")) {
        Ok(m) => m,
        Err(e) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid env map",
                e,
                None,
            );
        }
    };
    let config_map = match json_string_map(req.get("config")) {
        Ok(m) => m,
        Err(e) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid config map",
                e,
                None,
            );
        }
    };

    let template = {
        let registry = state
            .kernel
            .extension_registry
            .read()
            .unwrap_or_else(|e| e.into_inner());
        match registry.get_template(&id) {
            Some(t) => t.clone(),
            None => {
                return api_json_error(
                    StatusCode::NOT_FOUND,
                    &rid,
                    PATH,
                    "Unknown integration",
                    format!("Unknown integration: '{id}'"),
                    Some("Use GET /api/integrations/available to list templates."),
                );
            }
        }
    };

    if let Err(e) = openfang_extensions::installer::validate_user_supplied_keys(
        &template,
        &id,
        &env_map,
        &config_map,
    ) {
        return api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Invalid integration fields",
            e.to_string(),
            None,
        );
    }

    let (secrets, merged_config) = split_integration_payload(&template, &env_map, &config_map);

    let install_result = {
        let mut resolver = state
            .kernel
            .credential_resolver
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut registry = state
            .kernel
            .extension_registry
            .write()
            .unwrap_or_else(|e| e.into_inner());

        match openfang_extensions::installer::install_integration(
            &mut registry,
            &mut resolver,
            &id,
            &secrets,
            &merged_config,
        ) {
            Ok(result) => Ok(result),
            Err(openfang_extensions::ExtensionError::AlreadyInstalled(id)) => Err((
                StatusCode::CONFLICT,
                format!("Integration '{id}' already installed"),
            )),
            Err(openfang_extensions::ExtensionError::NotFound(id)) => Err((
                StatusCode::NOT_FOUND,
                format!("Unknown integration: '{id}'"),
            )),
            Err(e) => Err((StatusCode::BAD_REQUEST, e.to_string())),
        }
    };

    let result = match install_result {
        Ok(r) => r,
        Err((status, msg)) => {
            return api_json_error(
                status,
                &rid,
                PATH,
                "Integration install failed",
                msg,
                Some("Check field names and required values."),
            );
        }
    };

    state.kernel.extension_health.register(&id);

    let connected = state.kernel.reload_extension_mcps().await.unwrap_or(0);

    let integration_status = match &result.status {
        openfang_extensions::IntegrationStatus::Ready => "ready",
        openfang_extensions::IntegrationStatus::Setup => "setup",
        openfang_extensions::IntegrationStatus::Disabled => "disabled",
        openfang_extensions::IntegrationStatus::Available => "available",
        openfang_extensions::IntegrationStatus::Error(_) => "error",
    };

    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "id": id,
            "status": "installed",
            "mode": "configured",
            "integration_status": integration_status,
            "connected": connected > 0,
            "message": result.message,
        })),
    )
}

/// DELETE /api/integrations/:id — Remove an integration.
pub async fn remove_integration(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/integrations/:id";
    // Scope the write lock
    let uninstall_err = {
        let mut registry = state
            .kernel
            .extension_registry
            .write()
            .unwrap_or_else(|e| e.into_inner());
        registry.uninstall(&id).err()
    };

    if let Some(e) = uninstall_err {
        return api_json_error(
            StatusCode::NOT_FOUND,
            &rid,
            PATH,
            "Integration remove failed",
            e.to_string(),
            Some("The integration may not be installed."),
        );
    }

    state.kernel.extension_health.unregister(&id);

    // Hot-disconnect the removed MCP server
    let _ = state.kernel.reload_extension_mcps().await;

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "id": id,
            "status": "removed",
        })),
    )
}

/// POST /api/integrations/:id/reconnect — Reconnect an MCP server.
pub async fn reconnect_integration(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/integrations/:id/reconnect";
    let is_installed = {
        let registry = state
            .kernel
            .extension_registry
            .read()
            .unwrap_or_else(|e| e.into_inner());
        registry.is_installed(&id)
    };

    if !is_installed {
        return api_json_error(
            StatusCode::NOT_FOUND,
            &rid,
            PATH,
            "Integration not installed",
            format!("Integration '{id}' is not installed."),
            Some("POST /api/integrations/add first."),
        );
    }

    match state.kernel.reconnect_extension_mcp(&id).await {
        Ok(tool_count) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "id": id,
                "status": "connected",
                "tool_count": tool_count,
            })),
        ),
        Err(e) => api_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &rid,
            PATH,
            "Integration reconnect failed",
            e,
            Some("Check MCP server logs and required environment variables."),
        ),
    }
}

/// GET /api/integrations/health — Health status for all integrations.
pub async fn integrations_health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let health_entries = state.kernel.extension_health.all_health();
    let entries: Vec<serde_json::Value> = health_entries
        .iter()
        .map(|h| {
            serde_json::json!({
                "id": h.id,
                "status": h.status.to_string(),
                "tool_count": h.tool_count,
                "last_ok": h.last_ok.map(|t| t.to_rfc3339()),
                "last_error": h.last_error,
                "consecutive_failures": h.consecutive_failures,
                "reconnecting": h.reconnecting,
                "reconnect_attempts": h.reconnect_attempts,
                "connected_since": h.connected_since.map(|t| t.to_rfc3339()),
            })
        })
        .collect();

    Json(serde_json::json!({
        "health": entries,
        "count": entries.len(),
    }))
}

/// POST /api/integrations/reload — Hot-reload integration configs and reconnect MCP.
pub async fn reload_integrations(
    State(state): State<Arc<AppState>>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/integrations/reload";
    match state.kernel.reload_extension_mcps().await {
        Ok(connected) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "reloaded",
                "new_connections": connected,
            })),
        ),
        Err(e) => api_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &rid,
            PATH,
            "Integration reload failed",
            e,
            Some("Check extension configs and MCP connectivity."),
        ),
    }
}

// ---------------------------------------------------------------------------
// Scheduled Jobs (cron) endpoints — /api/schedules aliases the kernel scheduler
// (same persistence as /api/cron/jobs — ~/.armaraos/cron_jobs.json).
// ---------------------------------------------------------------------------

fn sanitize_schedule_api_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let mut out = cleaned
        .split_whitespace()
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if out.is_empty() {
        out = "schedule".to_string();
    }
    if out.len() > 120 {
        out.truncate(120);
    }
    out
}

/// Resolve `agent_id` (UUID or registered name) to an [`AgentId`].
fn resolve_schedule_agent_id(state: &AppState, agent_id_str: &str) -> Result<AgentId, String> {
    if let Ok(aid) = agent_id_str.parse::<AgentId>() {
        if state.kernel.registry.get(aid).is_some() {
            return Ok(aid);
        }
        return Err(format!("No agent with id {agent_id_str}"));
    }
    state
        .kernel
        .registry
        .list()
        .into_iter()
        .find(|a| a.name == agent_id_str)
        .map(|a| a.id)
        .ok_or_else(|| format!("No agent named {agent_id_str}"))
}

/// GET /api/schedules — List all kernel cron jobs (same backing store as GET /api/cron/jobs).
pub async fn list_schedules(
    State(state): State<Arc<AppState>>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let _rid = resolve_request_id(ext);
    let jobs = state.kernel.cron_scheduler.list_all_jobs();
    let total = jobs.len();
    let schedules: Vec<serde_json::Value> = jobs
        .into_iter()
        .map(|j| serde_json::to_value(&j).unwrap_or_default())
        .collect();
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "schedules": schedules,
            "total": total,
            "source": "kernel_cron"
        })),
    )
}

/// POST /api/schedules — Create a new cron-based scheduled job.
pub async fn create_schedule(
    State(state): State<Arc<AppState>>,
    ext: Option<Extension<RequestId>>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/schedules";
    let name = match req["name"].as_str() {
        Some(n) if !n.is_empty() => n.to_string(),
        _ => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Missing schedule name",
                "JSON body must include a non-empty 'name' field.".to_string(),
                Some("Pass name, cron, agent_id, and optional message for the scheduler."),
            );
        }
    };

    let cron = match req["cron"].as_str() {
        Some(c) if !c.is_empty() => c.to_string(),
        _ => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Missing cron expression",
                "JSON body must include a non-empty 'cron' field.".to_string(),
                Some("Use five fields: minute hour day-of-month month day-of-week."),
            );
        }
    };

    // Validate cron expression: must be 5 space-separated fields
    let cron_parts: Vec<&str> = cron.split_whitespace().collect();
    if cron_parts.len() != 5 {
        return api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Invalid cron expression",
            "Cron must have exactly 5 fields (minute hour dom mon dow), space-separated."
                .to_string(),
            Some("Example: \"0 9 * * *\" for daily 9:00."),
        );
    }

    let agent_id_str = req["agent_id"].as_str().unwrap_or("").to_string();
    if agent_id_str.is_empty() {
        return api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Missing agent_id",
            "JSON body must include 'agent_id' (UUID or agent name).".to_string(),
            Some("Use an id from GET /api/agents."),
        );
    }
    let message = req["message"].as_str().unwrap_or("").to_string();
    let enabled = req.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true);

    let target_agent = match resolve_schedule_agent_id(&state, &agent_id_str) {
        Ok(id) => id,
        Err(e) => {
            return api_json_error(
                StatusCode::NOT_FOUND,
                &rid,
                PATH,
                "Agent not found",
                e,
                Some("Use an id from GET /api/agents."),
            );
        }
    };

    let job_name = sanitize_schedule_api_name(&name);
    let body = serde_json::json!({
        "name": job_name,
        "agent_id": target_agent.to_string(),
        "schedule": { "kind": "cron", "expr": cron },
        "action": {
            "kind": "agent_turn",
            "message": if message.is_empty() {
                "[Scheduled task]".to_string()
            } else {
                message.clone()
            },
            "timeout_secs": 300u64
        },
        "delivery": { "kind": "none" },
        "enabled": enabled,
    });

    match state
        .kernel
        .cron_create(&target_agent.to_string(), body)
        .await
    {
        Ok(result) => {
            let parsed: serde_json::Value = serde_json::from_str(&result)
                .unwrap_or_else(|_| serde_json::json!({ "raw": result }));
            let job_id = parsed
                .get("job_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let mut body = serde_json::json!({
                "result": parsed,
                "source": "kernel_cron",
                "name": job_name,
            });
            // Back-compat: wizard.js and older clients expect top-level `id` (cron job UUID).
            if let Some(ref jid) = job_id {
                body["id"] = serde_json::json!(jid);
            }
            (StatusCode::CREATED, Json(body))
        }
        Err(e) => api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Schedule creation failed",
            e,
            Some("Same shape as POST /api/cron/jobs — validate name, cron expr, and agent_id."),
        ),
    }
}

/// PUT /api/schedules/:id — Update a kernel cron job (same id as GET /api/cron/jobs).
pub async fn update_schedule(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/schedules/:id";
    let uuid = match uuid::Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid cron job id",
                "Expected a UUID (same as kernel cron job id).".to_string(),
                Some("Use GET /api/schedules or GET /api/cron/jobs."),
            );
        }
    };
    let job_id = CronJobId(uuid);
    let mut job = match state.kernel.cron_scheduler.get_job(job_id) {
        Some(j) => j,
        None => {
            return api_json_error(
                StatusCode::NOT_FOUND,
                &rid,
                PATH,
                "Schedule not found",
                format!("No cron job with id {id}."),
                Some("Use GET /api/schedules to list ids."),
            );
        }
    };

    if let Some(enabled) = req.get("enabled").and_then(|v| v.as_bool()) {
        job.enabled = enabled;
    }
    if let Some(name) = req.get("name").and_then(|v| v.as_str()) {
        job.name = sanitize_schedule_api_name(name);
    }
    if let Some(cron) = req.get("cron").and_then(|v| v.as_str()) {
        let cron_parts: Vec<&str> = cron.split_whitespace().collect();
        if cron_parts.len() != 5 {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid cron expression",
                "Cron must have exactly 5 fields (minute hour dom mon dow).".to_string(),
                None,
            );
        }
        job.schedule = CronSchedule::Cron {
            expr: cron.to_string(),
            tz: None,
        };
    }
    if let Some(aid_str) = req.get("agent_id").and_then(|v| v.as_str()) {
        match resolve_schedule_agent_id(&state, aid_str) {
            Ok(aid) => job.agent_id = aid,
            Err(e) => {
                return api_json_error(
                    StatusCode::NOT_FOUND,
                    &rid,
                    PATH,
                    "Agent not found",
                    e,
                    None,
                );
            }
        }
    }
    if let Some(msg) = req.get("message").and_then(|v| v.as_str()) {
        if let CronAction::AgentTurn {
            ref mut message, ..
        } = job.action
        {
            *message = msg.to_string();
        }
    }

    match state.kernel.cron_scheduler.update_job(job_id, job) {
        Ok(()) => {
            let _ = state.kernel.cron_scheduler.persist();
            (
                StatusCode::OK,
                Json(
                    serde_json::json!({"status": "updated", "job_id": id, "source": "kernel_cron"}),
                ),
            )
        }
        Err(e) => api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Schedule update failed",
            format!("{e}"),
            Some("Validate merged fields against cron job rules."),
        ),
    }
}

/// DELETE /api/schedules/:id — Remove a kernel cron job (alias of DELETE /api/cron/jobs/:id).
pub async fn delete_schedule(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    delete_cron_job(State(state), Path(id), ext).await
}

/// POST /api/schedules/:id/run — Manually run a kernel cron job now (alias of POST /api/cron/jobs/:id/run).
pub async fn run_schedule(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    run_cron_job(State(state), Path(id), ext).await
}

// ---------------------------------------------------------------------------
// Support endpoints
// ---------------------------------------------------------------------------

/// POST /api/support/diagnostics — Generate a redacted diagnostics bundle.
///
/// Produces a `.zip` under `~/.armaraos/support/` containing:
/// - `README.txt` — what each file is for (start here if unsure)
/// - `diagnostics_snapshot.json` — structured support snapshot (config schema, paths, runtime, SQLite user_version)
/// - `meta.json` — compact metadata (overlaps snapshot; kept for compatibility)
/// - `config.toml` (as-is)
/// - `secrets.env` (redacted values)
/// - `audit.json` (recent audit trail entries)
/// - the SQLite DB (openfang.db + -wal/-shm when present)
/// - recent logs (best-effort)
pub async fn create_diagnostics_bundle(
    State(state): State<Arc<AppState>>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/support/diagnostics";
    let home = openfang_kernel::config::openfang_home();
    let support_dir = home.join("support");
    if let Err(e) = std::fs::create_dir_all(&support_dir) {
        return api_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &rid,
            PATH,
            "Failed to create support directory",
            format!("{e}"),
            Some("Check permissions on the ArmaraOS home directory."),
        );
    }

    let ts = chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string();
    let out_path = support_dir.join(format!("armaraos-diagnostics-{ts}.zip"));
    let tmp_redacted = support_dir.join(format!("secrets.redacted-{ts}.env"));
    let tmp_meta = support_dir.join(format!("meta-{ts}.json"));
    let tmp_audit = support_dir.join(format!("audit-{ts}.json"));
    let tmp_snapshot = support_dir.join(format!("diagnostics-snapshot-{ts}.json"));
    let tmp_readme = support_dir.join(format!("README-support-{ts}.txt"));

    // 1) Create redacted secrets.env copy
    let secrets_path = home.join("secrets.env");
    if secrets_path.exists() {
        match std::fs::read_to_string(&secrets_path) {
            Ok(s) => {
                let mut out = String::new();
                for line in s.lines() {
                    let t = line.trim();
                    if t.is_empty() || t.starts_with('#') || !t.contains('=') {
                        out.push_str(line);
                        out.push('\n');
                        continue;
                    }
                    let mut parts = t.splitn(2, '=');
                    let k = parts.next().unwrap_or("").trim();
                    let _v = parts.next().unwrap_or("");
                    out.push_str(k);
                    out.push('=');
                    out.push_str("***REDACTED***");
                    out.push('\n');
                }
                if let Err(e) = std::fs::write(&tmp_redacted, out) {
                    return api_json_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        &rid,
                        PATH,
                        "Failed to write redacted secrets",
                        format!("{e}"),
                        Some("Check disk space and permissions in the support folder."),
                    );
                }
            }
            Err(e) => {
                return api_json_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &rid,
                    PATH,
                    "Failed to read secrets.env",
                    format!("{e}"),
                    Some("Check file permissions on secrets.env."),
                )
            }
        }
    }

    // 2) Write metadata
    let config_path = home.join("config.toml");
    let data_dir = state.kernel.config.data_dir.clone();
    let db_path = state
        .kernel
        .config
        .memory
        .sqlite_path
        .clone()
        .unwrap_or_else(|| data_dir.join("openfang.db"));
    let logs_dir = home.join("logs");
    let agent_count = state.kernel.registry.list().len();
    let uptime_secs = state.started_at.elapsed().as_secs();
    let sqlite_user_ver = openfang_memory::migration::read_sqlite_user_version(&db_path);
    let expected_mem = openfang_memory::migration::memory_substrate_schema_expected();
    let cfg = &state.kernel.config;
    let ts_rfc = chrono::Utc::now().to_rfc3339();

    let snapshot = serde_json::json!({
        "generated_at": ts_rfc,
        "daemon": {
            "package_version": env!("CARGO_PKG_VERSION"),
            "config_schema_version_effective": cfg.config_schema_version,
            "config_schema_version_binary": openfang_types::config::CONFIG_SCHEMA_VERSION,
            "uptime_seconds": uptime_secs,
            "agent_count": agent_count,
        },
        "paths": {
            "home_dir": home.display().to_string(),
            "data_dir": data_dir.display().to_string(),
            "db_path": db_path.display().to_string(),
            "logs_dir": logs_dir.display().to_string(),
            "config_path": config_path.display().to_string(),
        },
        "runtime": {
            "api_listen": cfg.api_listen,
            "log_level": cfg.log_level,
            "network_enabled": cfg.network_enabled,
            "default_provider": cfg.default_model.provider,
            "default_model": cfg.default_model.model,
        },
        "memory_sqlite": {
            "user_version": sqlite_user_ver,
            "expected_schema_version": expected_mem,
            "user_version_matches_expected": sqlite_user_ver.map(|u| u == expected_mem),
        },
        "platform": {
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
        },
        "env": {
            "ARMARAOS_HOME_set": std::env::var_os("ARMARAOS_HOME").is_some(),
            "OPENFANG_HOME_set": std::env::var_os("OPENFANG_HOME").is_some(),
        },
    });
    if let Err(e) = std::fs::write(
        &tmp_snapshot,
        serde_json::to_vec_pretty(&snapshot).unwrap_or_default(),
    ) {
        return api_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &rid,
            PATH,
            "Failed to write diagnostics snapshot",
            format!("{e}"),
            None,
        );
    }

    let readme_text = format!(
        "ArmaraOS diagnostics bundle\n\
---------------------------\n\
Generated (UTC): {ts_rfc}\n\
\n\
Start with diagnostics_snapshot.json for a structured overview (config schema, paths, runtime, memory DB version).\n\
\n\
Files:\n\
- diagnostics_snapshot.json — Full support snapshot (recommended first read)\n\
- meta.json — Compact metadata (legacy fields; overlap with snapshot)\n\
- README.txt — This file\n\
- config.toml — On-disk configuration (as-is)\n\
- secrets.env — Redacted (values replaced; keys preserved)\n\
- audit.json — Recent audit trail export\n\
- data/openfang.db* — SQLite memory substrate (+ WAL/SHM if present)\n\
- home/logs/... — Recent log files from the home folder\n\
"
    );
    if let Err(e) = std::fs::write(&tmp_readme, readme_text) {
        return api_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &rid,
            PATH,
            "Failed to write README",
            format!("{e}"),
            None,
        );
    }

    let meta = serde_json::json!({
        "app": "ArmaraOS",
        "timestamp": ts_rfc,
        "version": env!("CARGO_PKG_VERSION"),
        "home_dir": home.display().to_string(),
        "data_dir": data_dir.display().to_string(),
        "db_path": db_path.display().to_string(),
        "logs_dir": logs_dir.display().to_string(),
        "platform": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "config_schema_version": cfg.config_schema_version,
        "config_schema_version_binary": openfang_types::config::CONFIG_SCHEMA_VERSION,
        "uptime_seconds": uptime_secs,
        "agent_count": agent_count,
        "api_listen": cfg.api_listen,
        "log_level": cfg.log_level,
        "network_enabled": cfg.network_enabled,
        "default_provider": cfg.default_model.provider,
        "default_model": cfg.default_model.model,
        "memory_sqlite_user_version": sqlite_user_ver,
        "memory_substrate_schema_expected": expected_mem,
    });
    if let Err(e) = std::fs::write(
        &tmp_meta,
        serde_json::to_vec_pretty(&meta).unwrap_or_default(),
    ) {
        return api_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &rid,
            PATH,
            "Failed to write metadata",
            format!("{e}"),
            None,
        );
    }

    // 3) Write audit export (explicit file, even though the backing store may be SQLite)
    let audit_entries = state.kernel.audit_log.recent(500);
    let audit_export = serde_json::json!({
        "tip_hash": state.kernel.audit_log.tip_hash(),
        "total": state.kernel.audit_log.len(),
        "exported": audit_entries.len(),
        "entries": audit_entries,
    });
    if let Err(e) = std::fs::write(
        &tmp_audit,
        serde_json::to_vec_pretty(&audit_export).unwrap_or_default(),
    ) {
        return api_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &rid,
            PATH,
            "Failed to write audit export",
            format!("{e}"),
            None,
        );
    }

    // 4) Build archive
    let file = match std::fs::File::create(&out_path) {
        Ok(f) => f,
        Err(e) => {
            return api_json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &rid,
                PATH,
                "Failed to create archive",
                format!("{e}"),
                Some("Check disk space and permissions."),
            )
        }
    };
    let mut zip = ZipWriter::new(file);
    let opts = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o644);

    let mut add_path = |src: &FsPath, dest: &str| -> Result<(), String> {
        if !src.exists() {
            return Ok(());
        }
        if src.is_dir() {
            return Ok(());
        }
        zip.start_file(dest, opts)
            .map_err(|e| format!("zip start {dest}: {e}"))?;
        let mut f = std::fs::File::open(src).map_err(|e| format!("open {dest}: {e}"))?;
        std::io::copy(&mut f, &mut zip).map_err(|e| format!("zip write {dest}: {e}"))?;
        Ok(())
    };

    if let Err(e) = add_path(&tmp_readme, "README.txt") {
        let _ = std::fs::remove_file(&out_path);
        return api_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &rid,
            PATH,
            "Diagnostics bundle failed",
            e,
            Some("Could not add README.txt to the archive."),
        );
    }
    if let Err(e) = add_path(&tmp_snapshot, "diagnostics_snapshot.json") {
        let _ = std::fs::remove_file(&out_path);
        return api_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &rid,
            PATH,
            "Diagnostics bundle failed",
            e,
            Some("Could not add diagnostics_snapshot.json to the archive."),
        );
    }
    if let Err(e) = add_path(&config_path, "config.toml") {
        let _ = std::fs::remove_file(&out_path);
        return api_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &rid,
            PATH,
            "Diagnostics bundle failed",
            e,
            Some("Could not add config.toml to the archive."),
        );
    }
    if tmp_redacted.exists() {
        if let Err(e) = add_path(&tmp_redacted, "secrets.env") {
            let _ = std::fs::remove_file(&out_path);
            return api_json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &rid,
                PATH,
                "Diagnostics bundle failed",
                e,
                Some("Could not add redacted secrets to the archive."),
            );
        }
    }
    if let Err(e) = add_path(&tmp_meta, "meta.json") {
        let _ = std::fs::remove_file(&out_path);
        return api_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &rid,
            PATH,
            "Diagnostics bundle failed",
            e,
            None,
        );
    }
    if let Err(e) = add_path(&tmp_audit, "audit.json") {
        let _ = std::fs::remove_file(&out_path);
        return api_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &rid,
            PATH,
            "Diagnostics bundle failed",
            e,
            None,
        );
    }

    // Include DB + WAL/SHM if present
    let db_base = db_path.clone();
    for (src, name) in [
        (db_base.clone(), "data/openfang.db".to_string()),
        (
            PathBuf::from(format!("{}-wal", db_base.display())),
            "data/openfang.db-wal".to_string(),
        ),
        (
            PathBuf::from(format!("{}-shm", db_base.display())),
            "data/openfang.db-shm".to_string(),
        ),
    ] {
        let _ = add_path(&src, &name);
    }

    // Logs (best-effort, cap to a reasonable number of files)
    if logs_dir.is_dir() {
        let mut added = 0usize;
        for entry in WalkDir::new(&logs_dir)
            .max_depth(2)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if added >= 25 {
                break;
            }
            if !entry.file_type().is_file() {
                continue;
            }
            let p = entry.path();
            let rel = p.strip_prefix(&home).unwrap_or(p);
            let name = format!("home/{}", rel.display());
            if add_path(p, &name).is_ok() {
                added += 1;
            }
        }
    }

    if let Err(e) = zip.finish() {
        let _ = std::fs::remove_file(&out_path);
        return api_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &rid,
            PATH,
            "Failed to finalize archive",
            format!("{e}"),
            None,
        );
    }

    // Cleanup temp files (best-effort)
    let _ = std::fs::remove_file(&tmp_redacted);
    let _ = std::fs::remove_file(&tmp_readme);
    let _ = std::fs::remove_file(&tmp_snapshot);
    let _ = std::fs::remove_file(&tmp_meta);
    let _ = std::fs::remove_file(&tmp_audit);

    let bundle_filename = out_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("armaraos-diagnostics.zip")
        .to_string();
    let relative_path = format!("support/{bundle_filename}");

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "bundle_path": out_path.display().to_string(),
            "bundle_filename": bundle_filename,
            "relative_path": relative_path,
        })),
    )
}

/// True when `name` is exactly `armaraos-diagnostics-YYYYMMDD-HHMMSS.zip` (no path segments).
fn is_allowed_diagnostics_zip_name(name: &str) -> bool {
    const PREFIX: &str = "armaraos-diagnostics-";
    const SUFFIX: &str = ".zip";
    if !name.starts_with(PREFIX) || !name.ends_with(SUFFIX) {
        return false;
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return false;
    }
    let inner = &name[PREFIX.len()..name.len() - SUFFIX.len()];
    let parts: Vec<&str> = inner.splitn(2, '-').collect();
    if parts.len() != 2 {
        return false;
    }
    if parts[0].len() != 8 || parts[1].len() != 6 {
        return false;
    }
    parts[0].chars().all(|c| c.is_ascii_digit()) && parts[1].chars().all(|c| c.is_ascii_digit())
}

/// GET /api/support/diagnostics/download?name=armaraos-diagnostics-YYYYMMDD-HHMMSS.zip
///
/// Streams a diagnostics zip from `~/…/support/` after validating the filename (no path traversal).
pub async fn download_diagnostics_bundle(
    ext: Option<Extension<RequestId>>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/support/diagnostics/download";
    let name = match q.get("name") {
        Some(n) if !n.trim().is_empty() => n.trim(),
        _ => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Missing name",
                "Query parameter `name` is required.".to_string(),
                None,
            )
            .into_response();
        }
    };
    if !is_allowed_diagnostics_zip_name(name) {
        return api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Invalid bundle name",
            "Expected armaraos-diagnostics-YYYYMMDD-HHMMSS.zip.".to_string(),
            None,
        )
        .into_response();
    }
    let home = openfang_kernel::config::openfang_home();
    let full = home.join("support").join(name);
    if !full.is_file() {
        return api_json_error(
            StatusCode::NOT_FOUND,
            &rid,
            PATH,
            "Bundle not found",
            "Generate a new bundle from Settings → System.".to_string(),
            None,
        )
        .into_response();
    }
    let bytes = match std::fs::read(&full) {
        Ok(b) => b,
        Err(e) => {
            return api_json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &rid,
                PATH,
                "Read failed",
                format!("{e}"),
                None,
            )
            .into_response();
        }
    };
    use axum::body::Body;
    use axum::http::{header, HeaderValue, Response};
    let cd = format!("attachment; filename=\"{name}\"");
    let disp = match HeaderValue::from_str(&cd) {
        Ok(h) => h,
        Err(_) => {
            return api_json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &rid,
                PATH,
                "Header build failed",
                "Invalid filename for Content-Disposition.".to_string(),
                None,
            )
            .into_response();
        }
    };
    match Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/zip")
        .header(header::CONTENT_DISPOSITION, disp)
        .body(Body::from(bytes))
    {
        Ok(resp) => resp.into_response(),
        Err(_) => api_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &rid,
            PATH,
            "Response build failed",
            "Could not build download response.".to_string(),
            None,
        )
        .into_response(),
    }
}

// ---------------------------------------------------------------------------
// Agent Identity endpoint
// ---------------------------------------------------------------------------

/// Request body for updating agent visual identity.
#[derive(serde::Deserialize)]
pub struct UpdateIdentityRequest {
    pub emoji: Option<String>,
    pub avatar_url: Option<String>,
    pub color: Option<String>,
    #[serde(default)]
    pub archetype: Option<String>,
    #[serde(default)]
    pub vibe: Option<String>,
    #[serde(default)]
    pub greeting_style: Option<String>,
}

/// PATCH /api/agents/{id}/identity — Update an agent's visual identity.
pub async fn update_agent_identity(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
    Json(req): Json<UpdateIdentityRequest>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/agents/:id/identity";
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid agent ID",
                "Agent id must be a valid UUID.".to_string(),
                None,
            );
        }
    };

    // Validate color format if provided
    if let Some(ref color) = req.color {
        if !color.is_empty() && !color.starts_with('#') {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid color",
                "Color must be a hex code starting with '#'.".to_string(),
                None,
            );
        }
    }

    // Validate avatar_url if provided
    if let Some(ref url) = req.avatar_url {
        if !url.is_empty()
            && !url.starts_with("http://")
            && !url.starts_with("https://")
            && !url.starts_with("data:")
        {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid avatar URL",
                "Avatar URL must be http(s) or a data URI.".to_string(),
                None,
            );
        }
    }

    let current = state
        .kernel
        .registry
        .get(agent_id)
        .map(|e| e.identity)
        .unwrap_or_default();
    let identity = AgentIdentity {
        emoji: patch_merge_identity_opt(req.emoji, current.emoji),
        avatar_url: patch_merge_identity_opt(req.avatar_url, current.avatar_url),
        color: patch_merge_color_opt(req.color, current.color),
        archetype: patch_merge_identity_opt(req.archetype, current.archetype),
        vibe: patch_merge_identity_opt(req.vibe, current.vibe),
        greeting_style: patch_merge_identity_opt(req.greeting_style, current.greeting_style),
    };

    match state.kernel.registry.update_identity(agent_id, identity) {
        Ok(()) => {
            // Persist identity to SQLite
            if let Some(entry) = state.kernel.registry.get(agent_id) {
                let _ = state.kernel.memory.save_agent(&entry);
                sync_agent_toml_for_kernel(state.kernel.as_ref(), &entry);
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "ok", "agent_id": id})),
            )
        }
        Err(_) => api_json_error(
            StatusCode::NOT_FOUND,
            &rid,
            PATH,
            "Agent not found",
            format!("No agent registered for id {id}."),
            Some("Use GET /api/agents to list agents."),
        ),
    }
}

// ---------------------------------------------------------------------------
// Agent Config Hot-Update
// ---------------------------------------------------------------------------

/// Request body for patching agent config (name, description, prompt, identity, model,
/// optional `api_key_env` / `base_url`, fallback chain, `max_iterations` for autonomous).
#[derive(serde::Deserialize)]
pub struct PatchAgentConfigRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub system_prompt: Option<String>,
    pub emoji: Option<String>,
    pub avatar_url: Option<String>,
    pub color: Option<String>,
    pub archetype: Option<String>,
    pub vibe: Option<String>,
    pub greeting_style: Option<String>,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub api_key_env: Option<String>,
    pub base_url: Option<String>,
    pub fallback_models: Option<Vec<openfang_types::agent::FallbackModel>>,
    /// Override the agent's autonomous loop step limit (maps to [autonomous] max_iterations).
    pub max_iterations: Option<u32>,
    /// Toggle the embedded ainl-runtime-engine path. Maps to manifest `ainl_runtime_engine`.
    pub ainl_runtime_engine: Option<bool>,
}

/// PATCH /api/agents/{id}/config — Hot-update agent name, description, system prompt, and identity.
pub async fn patch_agent_config(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
    Json(req): Json<PatchAgentConfigRequest>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/agents/:id/config";
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid agent ID",
                "Agent id must be a valid UUID.".to_string(),
                None,
            );
        }
    };

    // Input length limits
    const MAX_NAME_LEN: usize = 256;
    const MAX_DESC_LEN: usize = 4096;
    const MAX_PROMPT_LEN: usize = 65_536;

    if let Some(ref name) = req.name {
        if name.len() > MAX_NAME_LEN {
            return api_json_error(
                StatusCode::PAYLOAD_TOO_LARGE,
                &rid,
                PATH,
                "Name too long",
                format!("Name exceeds max length ({MAX_NAME_LEN} chars)."),
                None,
            );
        }
    }
    if let Some(ref desc) = req.description {
        if desc.len() > MAX_DESC_LEN {
            return api_json_error(
                StatusCode::PAYLOAD_TOO_LARGE,
                &rid,
                PATH,
                "Description too long",
                format!("Description exceeds max length ({MAX_DESC_LEN} chars)."),
                None,
            );
        }
    }
    if let Some(ref prompt) = req.system_prompt {
        if prompt.len() > MAX_PROMPT_LEN {
            return api_json_error(
                StatusCode::PAYLOAD_TOO_LARGE,
                &rid,
                PATH,
                "System prompt too long",
                format!("System prompt exceeds max length ({MAX_PROMPT_LEN} chars)."),
                None,
            );
        }
    }

    // Validate color format if provided
    if let Some(ref color) = req.color {
        if !color.is_empty() && !color.starts_with('#') {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid color",
                "Color must be a hex code starting with '#'.".to_string(),
                None,
            );
        }
    }

    // Validate avatar_url if provided
    if let Some(ref url) = req.avatar_url {
        if !url.is_empty()
            && !url.starts_with("http://")
            && !url.starts_with("https://")
            && !url.starts_with("data:")
        {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid avatar URL",
                "Avatar URL must be http(s) or a data URI.".to_string(),
                None,
            );
        }
    }

    // Update name
    if let Some(ref new_name) = req.name {
        if !new_name.is_empty() {
            if let Err(e) = state
                .kernel
                .registry
                .update_name(agent_id, new_name.clone())
            {
                return api_json_error(
                    StatusCode::CONFLICT,
                    &rid,
                    PATH,
                    "Name update failed",
                    format!("{e}"),
                    None,
                );
            }
        }
    }

    // Update description (ignore empty strings — dashboard clients used to send "" when the field was unknown)
    if let Some(ref new_desc) = req.description {
        if !new_desc.is_empty()
            && state
                .kernel
                .registry
                .update_description(agent_id, new_desc.clone())
                .is_err()
        {
            return api_json_error(
                StatusCode::NOT_FOUND,
                &rid,
                PATH,
                "Agent not found",
                format!("No agent registered for id {id}."),
                Some("Use GET /api/agents to list agents."),
            );
        }
    }

    // Update system prompt (hot-swap — takes effect on next message; skip empty — same client bug)
    if let Some(ref new_prompt) = req.system_prompt {
        if !new_prompt.is_empty()
            && state
                .kernel
                .registry
                .update_system_prompt(agent_id, new_prompt.clone())
                .is_err()
        {
            return api_json_error(
                StatusCode::NOT_FOUND,
                &rid,
                PATH,
                "Agent not found",
                format!("No agent registered for id {id}."),
                Some("Use GET /api/agents to list agents."),
            );
        }
    }

    // Update identity fields (merge — only overwrite provided fields)
    let has_identity_field = req.emoji.is_some()
        || req.avatar_url.is_some()
        || req.color.is_some()
        || req.archetype.is_some()
        || req.vibe.is_some()
        || req.greeting_style.is_some();

    if has_identity_field {
        // Read current identity, merge with provided fields
        let current = state
            .kernel
            .registry
            .get(agent_id)
            .map(|e| e.identity)
            .unwrap_or_default();
        let merged = AgentIdentity {
            emoji: patch_merge_identity_opt(req.emoji, current.emoji),
            avatar_url: patch_merge_identity_opt(req.avatar_url, current.avatar_url),
            color: patch_merge_color_opt(req.color, current.color),
            archetype: patch_merge_identity_opt(req.archetype, current.archetype),
            vibe: patch_merge_identity_opt(req.vibe, current.vibe),
            greeting_style: patch_merge_identity_opt(req.greeting_style, current.greeting_style),
        };
        if state
            .kernel
            .registry
            .update_identity(agent_id, merged)
            .is_err()
        {
            return api_json_error(
                StatusCode::NOT_FOUND,
                &rid,
                PATH,
                "Agent not found",
                format!("No agent registered for id {id}."),
                Some("Use GET /api/agents to list agents."),
            );
        }
    }

    // Update model/provider — use set_agent_model for catalog-based provider
    // resolution when provider is not explicitly provided (fixes #387/#466:
    // changing model from another provider without specifying provider now
    // auto-resolves the correct provider from the model catalog).
    if let Some(ref new_model) = req.model {
        if !new_model.is_empty() {
            if let Some(ref new_provider) = req.provider {
                if !new_provider.is_empty() {
                    // Explicit provider given — still route through set_agent_model
                    // so provider-specific auth/env hints stay in sync.
                    if let Err(e) =
                        state
                            .kernel
                            .set_agent_model(agent_id, new_model, Some(new_provider))
                    {
                        return api_json_error(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            &rid,
                            PATH,
                            "Model update failed",
                            format!("{e}"),
                            None,
                        );
                    }
                } else {
                    // Provider is empty string — resolve from catalog
                    if let Err(e) = state.kernel.set_agent_model(agent_id, new_model, None) {
                        return api_json_error(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            &rid,
                            PATH,
                            "Model update failed",
                            format!("{e}"),
                            None,
                        );
                    }
                }
            } else {
                // No provider field at all — resolve from catalog
                if let Err(e) = state.kernel.set_agent_model(agent_id, new_model, None) {
                    return api_json_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        &rid,
                        PATH,
                        "Model update failed",
                        format!("{e}"),
                        None,
                    );
                }
            }
        }
    }

    // Update fallback model chain
    if let Some(fallbacks) = req.fallback_models {
        if state
            .kernel
            .registry
            .update_fallback_models(agent_id, fallbacks)
            .is_err()
        {
            return api_json_error(
                StatusCode::NOT_FOUND,
                &rid,
                PATH,
                "Agent not found",
                format!("No agent registered for id {id}."),
                Some("Use GET /api/agents to list agents."),
            );
        }
    }

    // Update autonomous loop step limit
    if let Some(max_iter) = req.max_iterations {
        if max_iter == 0 || max_iter > 10_000 {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid max_iterations",
                "max_iterations must be between 1 and 10000.".to_string(),
                None,
            );
        }
        if state
            .kernel
            .registry
            .update_max_iterations(agent_id, max_iter)
            .is_err()
        {
            return api_json_error(
                StatusCode::NOT_FOUND,
                &rid,
                PATH,
                "Agent not found",
                format!("No agent registered for id {id}."),
                Some("Use GET /api/agents to list agents."),
            );
        }
    }

    // Toggle ainl-runtime-engine shim
    if let Some(enabled) = req.ainl_runtime_engine {
        if state
            .kernel
            .registry
            .update_ainl_runtime_engine(agent_id, enabled)
            .is_err()
        {
            return api_json_error(
                StatusCode::NOT_FOUND,
                &rid,
                PATH,
                "Agent not found",
                format!("No agent registered for id {id}."),
                Some("Use GET /api/agents to list agents."),
            );
        }
    }

    // Persist updated manifest to database so changes survive restart,
    // and also write user-editable fields back to the on-disk agent.toml
    // so the kernel's disk-vs-DB comparison never clobbers them on reload.
    if let Some(entry) = state.kernel.registry.get(agent_id) {
        if let Err(e) = state.kernel.memory.save_agent(&entry) {
            tracing::warn!("Failed to persist agent config update: {e}");
        }

        sync_agent_toml_for_kernel(state.kernel.as_ref(), &entry);
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "ok", "agent_id": id})),
    )
}

/// Persist dashboard-held fields to `agent.toml`, creating the file and parent directory if missing.
fn sync_agent_toml_for_kernel(kernel: &OpenFangKernel, entry: &openfang_types::agent::AgentEntry) {
    let path = kernel.agent_toml_path(&entry.name);
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                tracing::warn!(
                    path = %parent.display(),
                    error = %e,
                    "Failed to create agent directory for agent.toml sync"
                );
                return;
            }
        }
    }
    if !path.exists() {
        match toml::to_string_pretty(&entry.manifest) {
            Ok(body) => {
                if let Err(e) = std::fs::write(&path, body) {
                    tracing::warn!(
                        path = %path.display(),
                        error = %e,
                        "Failed to materialize agent.toml"
                    );
                    return;
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "Failed to serialize manifest for agent.toml");
                return;
            }
        }
    }
    patch_agent_toml_on_disk(&path, entry);
}

#[derive(serde::Serialize)]
struct AgentTomlResourcesPatch<'a> {
    resources: &'a openfang_types::agent::ResourceQuota,
}

#[derive(serde::Serialize)]
struct AgentTomlFallbacksPatch<'a> {
    fallback_models: &'a [openfang_types::agent::FallbackModel],
}

#[derive(serde::Serialize)]
struct AgentTomlRoutingPatch<'a> {
    routing: &'a openfang_types::agent::ModelRoutingConfig,
}

#[derive(serde::Serialize)]
struct AgentTomlProfilePatch<'a> {
    profile: &'a openfang_types::agent::ToolProfile,
}

#[derive(serde::Serialize)]
struct AgentTomlPriorityPatch {
    priority: openfang_types::agent::Priority,
}

/// Read the existing agent.toml, update dashboard-held fields in-place, and write it back.
///
/// Keeps structural template keys (capabilities, exec_policy, module, schedule, …) from
/// the file while syncing name, description, tags, priority, workspace,
/// `generate_identity_files`, metadata, model, identity, budgets/resources, routing,
/// fallbacks, skill/MCP/tool filters, etc. from the live [`AgentEntry`].
fn patch_agent_toml_on_disk(path: &std::path::Path, entry: &openfang_types::agent::AgentEntry) {
    use toml::Value;

    let Ok(raw) = std::fs::read_to_string(path) else {
        return;
    };
    let Ok(mut doc) = raw.parse::<Value>() else {
        return;
    };

    let Some(root) = doc.as_table_mut() else {
        return;
    };

    root.insert(
        "name".to_string(),
        Value::String(entry.manifest.name.clone()),
    );
    root.insert(
        "description".to_string(),
        Value::String(entry.manifest.description.clone()),
    );

    match &entry.manifest.profile {
        Some(p) => {
            if let Ok(s) = toml::to_string(&AgentTomlProfilePatch { profile: p }) {
                if let Ok(Value::Table(t)) = s.parse::<Value>() {
                    if let Some(pv) = t.get("profile") {
                        root.insert("profile".to_string(), pv.clone());
                    }
                }
            }
        }
        None => {
            root.remove("profile");
        }
    }

    match &entry.manifest.pinned_model {
        Some(p) if !p.is_empty() => {
            root.insert("pinned_model".to_string(), Value::String(p.clone()));
        }
        _ => {
            root.remove("pinned_model");
        }
    }

    // [model] — full primary model row from the registry
    {
        let m = &entry.manifest.model;
        let model_tbl = root
            .entry("model".to_string())
            .or_insert_with(|| Value::Table(toml::map::Map::new()));
        if let Some(tbl) = model_tbl.as_table_mut() {
            tbl.insert("provider".to_string(), Value::String(m.provider.clone()));
            tbl.insert("model".to_string(), Value::String(m.model.clone()));
            tbl.insert(
                "system_prompt".to_string(),
                Value::String(m.system_prompt.clone()),
            );
            tbl.insert(
                "max_tokens".to_string(),
                Value::Integer(i64::from(m.max_tokens)),
            );
            tbl.insert(
                "temperature".to_string(),
                Value::Float(m.temperature as f64),
            );
            match &m.api_key_env {
                Some(s) if !s.is_empty() => {
                    tbl.insert("api_key_env".to_string(), Value::String(s.clone()));
                }
                _ => {
                    tbl.remove("api_key_env");
                }
            }
            match &m.base_url {
                Some(s) if !s.is_empty() => {
                    tbl.insert("base_url".to_string(), Value::String(s.clone()));
                }
                _ => {
                    tbl.remove("base_url");
                }
            }
        }
    }

    // [identity]
    {
        let id = &entry.identity;
        let identity_tbl = root
            .entry("identity".to_string())
            .or_insert_with(|| Value::Table(toml::map::Map::new()));
        if let Some(tbl) = identity_tbl.as_table_mut() {
            set_toml_opt_value(tbl, "emoji", id.emoji.as_deref());
            set_toml_opt_value(tbl, "vibe", id.vibe.as_deref());
            set_toml_opt_value(tbl, "archetype", id.archetype.as_deref());
            set_toml_opt_value(tbl, "color", id.color.as_deref());
            set_toml_opt_value(tbl, "avatar_url", id.avatar_url.as_deref());
            set_toml_opt_value(tbl, "greeting_style", id.greeting_style.as_deref());
        }
    }

    if let Some(autonomous) = &entry.manifest.autonomous {
        let autonomous_tbl = root
            .entry("autonomous".to_string())
            .or_insert_with(|| Value::Table(toml::map::Map::new()));
        if let Some(tbl) = autonomous_tbl.as_table_mut() {
            tbl.insert(
                "max_iterations".to_string(),
                Value::Integer(autonomous.max_iterations as i64),
            );
        }
    }

    set_string_array_root(root, "skills", &entry.manifest.skills);
    set_string_array_root(root, "mcp_servers", &entry.manifest.mcp_servers);
    set_string_array_root(root, "tool_allowlist", &entry.manifest.tool_allowlist);
    set_string_array_root(root, "tool_blocklist", &entry.manifest.tool_blocklist);

    if let Ok(s) = toml::to_string(&AgentTomlResourcesPatch {
        resources: &entry.manifest.resources,
    }) {
        if let Ok(v) = s.parse::<Value>() {
            if let Some(Value::Table(inner)) = v.get("resources") {
                root.insert("resources".to_string(), Value::Table(inner.clone()));
            }
        }
    }

    if entry.manifest.fallback_models.is_empty() {
        root.remove("fallback_models");
    } else if let Ok(s) = toml::to_string(&AgentTomlFallbacksPatch {
        fallback_models: &entry.manifest.fallback_models,
    }) {
        if let Ok(v) = s.parse::<Value>() {
            if let Some(fbv) = v.get("fallback_models") {
                root.insert("fallback_models".to_string(), fbv.clone());
            }
        }
    }

    match &entry.manifest.routing {
        Some(r) => {
            if let Ok(s) = toml::to_string(&AgentTomlRoutingPatch { routing: r }) {
                if let Ok(v) = s.parse::<Value>() {
                    if let Some(Value::Table(rt)) = v.get("routing") {
                        root.insert("routing".to_string(), Value::Table(rt.clone()));
                    }
                }
            }
        }
        None => {
            root.remove("routing");
        }
    }

    set_string_array_root(root, "tags", &entry.manifest.tags);

    if let Ok(s) = toml::to_string(&AgentTomlPriorityPatch {
        priority: entry.manifest.priority,
    }) {
        if let Ok(Value::Table(t)) = s.parse::<Value>() {
            if let Some(pv) = t.get("priority") {
                root.insert("priority".to_string(), pv.clone());
            }
        }
    }

    root.insert(
        "generate_identity_files".to_string(),
        Value::Boolean(entry.manifest.generate_identity_files),
    );

    // Dashboard / API toggle — must stay in sync with SQLite so restarts and
    // disk-vs-DB merge logic do not drop the experimental ainl-runtime shim flag.
    root.insert(
        "ainl_runtime_engine".to_string(),
        Value::Boolean(entry.manifest.ainl_runtime_engine),
    );

    match &entry.manifest.workspace {
        Some(p) => {
            let s = p.to_string_lossy();
            if !s.is_empty() {
                root.insert("workspace".to_string(), Value::String(s.into_owned()));
            } else {
                root.remove("workspace");
            }
        }
        None => {
            root.remove("workspace");
        }
    }

    if entry.manifest.metadata.is_empty() {
        root.remove("metadata");
    } else if let Ok(jv) = serde_json::to_value(&entry.manifest.metadata) {
        if let Some(tv) = json_value_to_toml(&jv) {
            root.insert("metadata".to_string(), tv);
        }
    }

    let Ok(updated) = toml::to_string_pretty(&doc) else {
        return;
    };
    if updated != raw {
        if let Err(e) = std::fs::write(path, &updated) {
            tracing::warn!(path = %path.display(), "Failed to write agent.toml after config patch: {e}");
        } else {
            tracing::debug!(path = %path.display(), "Wrote user edits back to agent.toml");
        }
    }
}

fn set_string_array_root(
    root: &mut toml::map::Map<String, toml::Value>,
    key: &str,
    list: &[String],
) {
    use toml::Value;
    if list.is_empty() {
        root.remove(key);
    } else {
        root.insert(
            key.to_string(),
            Value::Array(list.iter().map(|s| Value::String(s.clone())).collect()),
        );
    }
}

fn set_toml_opt_value(tbl: &mut toml::map::Map<String, toml::Value>, key: &str, val: Option<&str>) {
    match val {
        Some(v) if !v.is_empty() => {
            tbl.insert(key.to_string(), toml::Value::String(v.to_string()));
        }
        _ => {
            tbl.remove(key);
        }
    }
}

/// Convert a [`serde_json::Value`] tree into `toml::Value` for persisting `manifest.metadata`.
fn json_value_to_toml(v: &serde_json::Value) -> Option<toml::Value> {
    use serde_json::Value as J;
    match v {
        J::Null => None,
        J::Bool(b) => Some(toml::Value::Boolean(*b)),
        J::Number(n) => {
            if let Some(i) = n.as_i64() {
                Some(toml::Value::Integer(i))
            } else if let Some(u) = n.as_u64() {
                Some(toml::Value::Integer(i64::try_from(u).unwrap_or(i64::MAX)))
            } else {
                Some(toml::Value::Float(n.as_f64().unwrap_or(0.0)))
            }
        }
        J::String(s) => Some(toml::Value::String(s.clone())),
        J::Array(a) => {
            let mut out = Vec::new();
            for x in a {
                if let Some(tv) = json_value_to_toml(x) {
                    out.push(tv);
                }
            }
            Some(toml::Value::Array(out))
        }
        J::Object(m) => {
            let mut tbl = toml::map::Map::new();
            for (k, x) in m {
                if let Some(tv) = json_value_to_toml(x) {
                    tbl.insert(k.clone(), tv);
                }
            }
            Some(toml::Value::Table(tbl))
        }
    }
}

// ---------------------------------------------------------------------------
// Agent Cloning
// ---------------------------------------------------------------------------

/// Request body for cloning an agent.
#[derive(serde::Deserialize)]
pub struct CloneAgentRequest {
    pub new_name: String,
}

/// POST /api/agents/{id}/clone — Clone an agent with its workspace files.
pub async fn clone_agent(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
    Json(req): Json<CloneAgentRequest>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/agents/:id/clone";
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid agent ID",
                "Agent id must be a valid UUID.".to_string(),
                None,
            );
        }
    };

    if req.new_name.len() > 256 {
        return api_json_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            &rid,
            PATH,
            "Name too long",
            "Name exceeds max length (256 chars).".to_string(),
            None,
        );
    }

    if req.new_name.trim().is_empty() {
        return api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Invalid new_name",
            "new_name cannot be empty.".to_string(),
            None,
        );
    }

    let source = match state.kernel.registry.get(agent_id) {
        Some(e) => e,
        None => {
            return api_json_error(
                StatusCode::NOT_FOUND,
                &rid,
                PATH,
                "Agent not found",
                format!("No agent registered for id {id}."),
                Some("Use GET /api/agents to list agents."),
            );
        }
    };

    // Deep-clone manifest with new name
    let mut cloned_manifest = source.manifest.clone();
    cloned_manifest.name = req.new_name.clone();
    cloned_manifest.workspace = None; // Let kernel assign a new workspace

    // Spawn the cloned agent
    let new_id = match state.kernel.spawn_agent(cloned_manifest) {
        Ok(id) => id,
        Err(e) => {
            return api_json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &rid,
                PATH,
                "Clone failed",
                format!("Clone spawn failed: {e}"),
                None,
            );
        }
    };

    // Copy workspace files from source to destination
    let new_entry = state.kernel.registry.get(new_id);
    if let (Some(ref src_ws), Some(ref new_entry)) = (source.manifest.workspace, new_entry) {
        if let Some(ref dst_ws) = new_entry.manifest.workspace {
            // Security: canonicalize both paths
            if let (Ok(src_can), Ok(dst_can)) = (src_ws.canonicalize(), dst_ws.canonicalize()) {
                for &fname in KNOWN_IDENTITY_FILES {
                    let src_file = src_can.join(fname);
                    let dst_file = dst_can.join(fname);
                    if src_file.exists() {
                        let _ = std::fs::copy(&src_file, &dst_file);
                    }
                }
            }
        }
    }

    // Copy identity from source
    let _ = state
        .kernel
        .registry
        .update_identity(new_id, source.identity.clone());

    // Register in channel router so binding resolution finds the cloned agent
    if let Some(ref mgr) = *state.bridge_manager.lock().await {
        mgr.router().register_agent(req.new_name.clone(), new_id);
    }

    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "agent_id": new_id.to_string(),
            "name": req.new_name,
        })),
    )
}

// ---------------------------------------------------------------------------
// Workspace File Editor endpoints
// ---------------------------------------------------------------------------

/// Whitelisted workspace identity files that can be read/written via API.
const KNOWN_IDENTITY_FILES: &[&str] = &[
    "SOUL.md",
    "IDENTITY.md",
    "USER.md",
    "TOOLS.md",
    "MEMORY.md",
    "AGENTS.md",
    "BOOTSTRAP.md",
    "HEARTBEAT.md",
];

/// GET /api/agents/{id}/files — List workspace identity files.
pub async fn list_agent_files(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/agents/:id/files";
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid agent ID",
                "Agent id must be a valid UUID.".to_string(),
                None,
            );
        }
    };

    let entry = match state.kernel.registry.get(agent_id) {
        Some(e) => e,
        None => {
            return api_json_error(
                StatusCode::NOT_FOUND,
                &rid,
                PATH,
                "Agent not found",
                format!("No agent registered for id {id}."),
                Some("Use GET /api/agents to list agents."),
            );
        }
    };

    let workspace = match entry.manifest.workspace {
        Some(ref ws) => ws.clone(),
        None => {
            return api_json_error(
                StatusCode::NOT_FOUND,
                &rid,
                PATH,
                "No workspace",
                "This agent has no workspace directory configured.".to_string(),
                None,
            );
        }
    };

    let mut files = Vec::new();
    for &name in KNOWN_IDENTITY_FILES {
        let path = workspace.join(name);
        let (exists, size_bytes) = if path.exists() {
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            (true, size)
        } else {
            (false, 0u64)
        };
        files.push(serde_json::json!({
            "name": name,
            "exists": exists,
            "size_bytes": size_bytes,
        }));
    }

    (StatusCode::OK, Json(serde_json::json!({ "files": files })))
}

/// GET /api/agents/{id}/files/{filename} — Read a workspace identity file.
pub async fn get_agent_file(
    State(state): State<Arc<AppState>>,
    Path((id, filename)): Path<(String, String)>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/agents/:id/files/:filename";
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid agent ID",
                "Agent id must be a valid UUID.".to_string(),
                None,
            );
        }
    };

    // Validate filename whitelist
    if !KNOWN_IDENTITY_FILES.contains(&filename.as_str()) {
        return api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "File not allowed",
            "Filename must be one of the whitelisted identity files.".to_string(),
            None,
        );
    }

    let entry = match state.kernel.registry.get(agent_id) {
        Some(e) => e,
        None => {
            return api_json_error(
                StatusCode::NOT_FOUND,
                &rid,
                PATH,
                "Agent not found",
                format!("No agent registered for id {id}."),
                Some("Use GET /api/agents to list agents."),
            );
        }
    };

    let workspace = match entry.manifest.workspace {
        Some(ref ws) => ws.clone(),
        None => {
            return api_json_error(
                StatusCode::NOT_FOUND,
                &rid,
                PATH,
                "No workspace",
                "This agent has no workspace directory configured.".to_string(),
                None,
            );
        }
    };

    // Security: canonicalize and verify stays inside workspace
    let file_path = workspace.join(&filename);
    let canonical = match file_path.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            return api_json_error(
                StatusCode::NOT_FOUND,
                &rid,
                PATH,
                "File not found",
                "The file does not exist or could not be accessed.".to_string(),
                None,
            );
        }
    };
    let ws_canonical = match workspace.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            return api_json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &rid,
                PATH,
                "Workspace path error",
                "Could not canonicalize the agent workspace path.".to_string(),
                None,
            );
        }
    };
    if !canonical.starts_with(&ws_canonical) {
        return api_json_error(
            StatusCode::FORBIDDEN,
            &rid,
            PATH,
            "Path traversal denied",
            "Resolved path escapes the workspace directory.".to_string(),
            None,
        );
    }

    let content = match std::fs::read_to_string(&canonical) {
        Ok(c) => c,
        Err(_) => {
            return api_json_error(
                StatusCode::NOT_FOUND,
                &rid,
                PATH,
                "File not found",
                "Could not read file contents.".to_string(),
                None,
            );
        }
    };

    let size_bytes = content.len();
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "name": filename,
            "content": content,
            "size_bytes": size_bytes,
        })),
    )
}

/// Request body for writing a workspace identity file.
#[derive(serde::Deserialize)]
pub struct SetAgentFileRequest {
    pub content: String,
}

/// PUT /api/agents/{id}/files/{filename} — Write a workspace identity file.
pub async fn set_agent_file(
    State(state): State<Arc<AppState>>,
    Path((id, filename)): Path<(String, String)>,
    ext: Option<Extension<RequestId>>,
    Json(req): Json<SetAgentFileRequest>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/agents/:id/files/:filename";
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid agent ID",
                "Agent id must be a valid UUID.".to_string(),
                None,
            );
        }
    };

    // Validate filename whitelist
    if !KNOWN_IDENTITY_FILES.contains(&filename.as_str()) {
        return api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "File not allowed",
            "Filename must be one of the whitelisted identity files.".to_string(),
            None,
        );
    }

    // Max 32KB content
    const MAX_FILE_SIZE: usize = 32_768;
    if req.content.len() > MAX_FILE_SIZE {
        return api_json_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            &rid,
            PATH,
            "Payload too large",
            format!("File content exceeds max size ({MAX_FILE_SIZE} bytes)."),
            None,
        );
    }

    let entry = match state.kernel.registry.get(agent_id) {
        Some(e) => e,
        None => {
            return api_json_error(
                StatusCode::NOT_FOUND,
                &rid,
                PATH,
                "Agent not found",
                format!("No agent registered for id {id}."),
                Some("Use GET /api/agents to list agents."),
            );
        }
    };

    let workspace = match entry.manifest.workspace {
        Some(ref ws) => ws.clone(),
        None => {
            return api_json_error(
                StatusCode::NOT_FOUND,
                &rid,
                PATH,
                "No workspace",
                "This agent has no workspace directory configured.".to_string(),
                None,
            );
        }
    };

    // Security: verify workspace path and target stays inside it
    let ws_canonical = match workspace.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            return api_json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &rid,
                PATH,
                "Workspace path error",
                "Could not canonicalize the agent workspace path.".to_string(),
                None,
            );
        }
    };

    let file_path = workspace.join(&filename);
    // For new files, check the parent directory instead
    let check_path = if file_path.exists() {
        file_path
            .canonicalize()
            .unwrap_or_else(|_| file_path.clone())
    } else {
        // Parent must be inside workspace
        file_path
            .parent()
            .and_then(|p| p.canonicalize().ok())
            .map(|p| p.join(&filename))
            .unwrap_or_else(|| file_path.clone())
    };
    if !check_path.starts_with(&ws_canonical) {
        return api_json_error(
            StatusCode::FORBIDDEN,
            &rid,
            PATH,
            "Path traversal denied",
            "Resolved path escapes the workspace directory.".to_string(),
            None,
        );
    }

    // Atomic write: write to .tmp, then rename
    let tmp_path = workspace.join(format!(".{filename}.tmp"));
    if let Err(e) = std::fs::write(&tmp_path, &req.content) {
        return api_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &rid,
            PATH,
            "Write failed",
            format!("{e}"),
            None,
        );
    }
    if let Err(e) = std::fs::rename(&tmp_path, &file_path) {
        let _ = std::fs::remove_file(&tmp_path);
        return api_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &rid,
            PATH,
            "Rename failed",
            format!("{e}"),
            None,
        );
    }

    let size_bytes = req.content.len();
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "name": filename,
            "size_bytes": size_bytes,
        })),
    )
}

// ---------------------------------------------------------------------------
// File Upload endpoints
// ---------------------------------------------------------------------------

/// Response body for file uploads.
#[derive(serde::Serialize)]
struct UploadResponse {
    file_id: String,
    filename: String,
    content_type: String,
    size: usize,
    /// Transcription text for audio uploads (populated via Whisper STT).
    #[serde(skip_serializing_if = "Option::is_none")]
    transcription: Option<String>,
}

/// Metadata stored alongside uploaded files.
struct UploadMeta {
    #[allow(dead_code)]
    filename: String,
    content_type: String,
}

/// In-memory upload metadata registry.
static UPLOAD_REGISTRY: LazyLock<DashMap<String, UploadMeta>> = LazyLock::new(DashMap::new);

/// Maximum upload size: 10 MB.
const MAX_UPLOAD_SIZE: usize = 10 * 1024 * 1024;

fn upload_extension_lower(filename: &str) -> Option<String> {
    FsPath::new(filename)
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_lowercase())
}

/// Blocked extensions (executables / installers). Uploads are never executed, but we reject these anyway.
const BLOCKED_UPLOAD_EXTENSIONS: &[&str] = &[
    "exe", "dll", "com", "scr", "msi", "pif", "cpl", "sys", "drv", "app", "deb", "rpm", "dmg",
    "pkg", "iso", "bat", "cmd",
];

/// Allowlisted extensions when MIME is missing or generic (`application/octet-stream`).
const ALLOWED_UPLOAD_EXTENSIONS: &[&str] = &[
    "png",
    "jpg",
    "jpeg",
    "jpe",
    "gif",
    "webp",
    "bmp",
    "ico",
    "tif",
    "tiff",
    "heic",
    "heif",
    "avif",
    "svg",
    "mp3",
    "wav",
    "ogg",
    "oga",
    "opus",
    "flac",
    "m4a",
    "aac",
    "wma",
    "mp4",
    "webm",
    "mov",
    "mkv",
    "m4v",
    "pdf",
    "csv",
    "tsv",
    "tab",
    "txt",
    "md",
    "markdown",
    "json",
    "jsonl",
    "json5",
    "xml",
    "xsl",
    "xslt",
    "html",
    "htm",
    "xhtml",
    "css",
    "js",
    "jsx",
    "mjs",
    "cjs",
    "ts",
    "tsx",
    "mts",
    "cts",
    "vue",
    "svelte",
    "php",
    "phtml",
    "py",
    "pyw",
    "pyi",
    "rb",
    "erb",
    "java",
    "kt",
    "kts",
    "go",
    "rs",
    "c",
    "h",
    "cpp",
    "hpp",
    "cc",
    "cxx",
    "cs",
    "swift",
    "scala",
    "sc",
    "clj",
    "cljs",
    "edn",
    "sh",
    "bash",
    "zsh",
    "fish",
    "ps1",
    "psm1",
    "psd1",
    "sql",
    "ini",
    "cfg",
    "conf",
    "config",
    "toml",
    "yaml",
    "yml",
    "env",
    "properties",
    "ainl",
    "lang",
    "graphql",
    "gql",
    "r",
    "lua",
    "pl",
    "pm",
    "dart",
    "ex",
    "exs",
    "hs",
    "lhs",
    "ml",
    "mli",
    "nim",
    "zig",
    "v",
    "vh",
    "sv",
    "tex",
    "log",
    "xlsx",
    "xls",
    "xlsm",
    "ods",
    "docx",
    "doc",
    "odt",
    "rtf",
    "pptx",
    "ppt",
    "odp",
    "woff",
    "woff2",
    "ttf",
    "otf",
    "eot",
];

fn upload_extension_blocked(ext: &str) -> bool {
    BLOCKED_UPLOAD_EXTENSIONS.contains(&ext)
}

fn upload_extension_allowlisted(ext: &str) -> bool {
    ALLOWED_UPLOAD_EXTENSIONS.contains(&ext)
}

fn infer_mime_from_extension(ext: &str) -> String {
    match ext {
        "png" => "image/png",
        "jpg" | "jpeg" | "jpe" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "tif" | "tiff" => "image/tiff",
        "heic" | "heif" => "image/heic",
        "avif" => "image/avif",
        "svg" => "image/svg+xml",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" | "oga" | "opus" => "audio/ogg",
        "flac" => "audio/flac",
        "m4a" | "aac" => "audio/mp4",
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "mkv" => "video/x-matroska",
        "pdf" => "application/pdf",
        "json" | "jsonl" | "json5" => "application/json",
        "csv" | "tsv" | "tab" => "text/csv",
        "xlsx" | "xlsm" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "xls" => "application/vnd.ms-excel",
        "ods" => "application/vnd.oasis.opendocument.spreadsheet",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "doc" => "application/msword",
        "odt" => "application/vnd.oasis.opendocument.text",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "ppt" => "application/vnd.ms-powerpoint",
        "odp" => "application/vnd.oasis.opendocument.presentation",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" | "jsx" | "mjs" | "cjs" => "text/javascript",
        "ts" | "tsx" | "mts" | "cts" => "text/typescript",
        "php" | "phtml" => "application/x-httpd-php",
        "py" | "pyw" | "pyi" => "text/x-python",
        "rs" => "text/rust",
        "go" => "text/x-go",
        "java" => "text/x-java",
        "rb" | "erb" => "text/x-ruby",
        "sql" => "application/sql",
        "xml" | "xsl" | "xslt" => "application/xml",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "graphql" | "gql" => "application/graphql",
        "ainl" | "lang" => "text/plain",
        _ => "text/plain",
    }
    .to_string()
}

fn upload_mime_disallowed(ct: &str) -> bool {
    matches!(
        ct,
        "application/wasm"
            | "application/x-msdownload"
            | "application/x-msdos-program"
            | "application/x-executable"
    ) || ct.starts_with("application/x-dosexec")
}

fn upload_mime_allowed(ct: &str) -> bool {
    if upload_mime_disallowed(ct) {
        return false;
    }
    if ct.starts_with("image/")
        || ct.starts_with("audio/")
        || ct.starts_with("video/")
        || ct.starts_with("text/")
        || ct.starts_with("font/")
    {
        return true;
    }
    if ct.starts_with("application/vnd.openxmlformats")
        || ct.starts_with("application/vnd.oasis.opendocument")
        || ct.starts_with("application/vnd.ms-")
    {
        return true;
    }
    matches!(
        ct,
        "application/pdf"
            | "application/json"
            | "application/xml"
            | "application/javascript"
            | "application/x-javascript"
            | "application/ecmascript"
            | "application/typescript"
            | "application/rtf"
            | "application/sql"
            | "application/csv"
            | "application/xhtml+xml"
            | "application/x-httpd-php"
            | "application/x-php"
            | "application/x-yaml"
            | "application/x-sh"
            | "application/x-shellscript"
            | "application/toml"
            | "application/graphql"
            | "application/ld+json"
            | "application/msword"
    )
}

/// Validate reported MIME + filename; normalize storage type (fixes `application/octet-stream` + `.py`, etc.).
fn normalize_upload_content_type(filename: &str, reported: &str) -> Result<String, &'static str> {
    let ext = upload_extension_lower(filename);
    if let Some(ref e) = ext {
        if upload_extension_blocked(e) {
            return Err("This file extension is not allowed for security reasons.");
        }
    }

    let r = reported.trim().to_lowercase();

    if r == "application/octet-stream" || r == "binary/octet-stream" || r.is_empty() {
        let ext = ext
            .as_ref()
            .ok_or("Unknown file type: add a known extension or use a supported MIME type.")?;
        if !upload_extension_allowlisted(ext) {
            return Err("This file type is not allowed for generic binary uploads.");
        }
        return Ok(infer_mime_from_extension(ext));
    }

    if upload_mime_disallowed(&r) {
        return Err("This MIME type is not allowed.");
    }

    if upload_mime_allowed(&r) {
        return Ok(r);
    }

    let ext = ext
        .as_ref()
        .ok_or("This MIME type is not allowed for this file.")?;
    if !upload_extension_allowlisted(ext) {
        return Err("This file type is not allowed.");
    }
    Ok(infer_mime_from_extension(ext))
}

/// POST /api/agents/{id}/upload — Upload a file attachment.
///
/// Accepts multipart/form-data. The client must include a file field with:
/// - `Content-Type` field (e.g., `image/png`, `text/plain`, `application/pdf`)
/// - `filename` attribute (original filename)
pub async fn upload_file(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/agents/:id/upload";
    let _agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid agent ID",
                "Agent id must be a valid UUID.".to_string(),
                None,
            );
        }
    };

    let mut file_data: Option<(Option<String>, String, axum::body::Bytes)> = None;
    let mut filename_from_field: Option<String> = None;

    while let Some(field) = multipart.next_field().await.transpose() {
        let field = match field {
            Ok(f) => f,
            Err(e) => {
                return api_json_error(
                    StatusCode::BAD_REQUEST,
                    &rid,
                    PATH,
                    "Multipart read failed",
                    format!("Failed to read field: {e}"),
                    None,
                );
            }
        };

        let field_name = field.name().unwrap_or("").to_string();

        match field_name.as_str() {
            "file" => {
                let filename_attr = field.file_name().map(|s| s.to_string());
                let content_type = field
                    .content_type()
                    .unwrap_or("application/octet-stream")
                    .to_string();
                let bytes = match field.bytes().await {
                    Ok(b) => b,
                    Err(e) => {
                        return api_json_error(
                            StatusCode::BAD_REQUEST,
                            &rid,
                            PATH,
                            "File read failed",
                            format!("Failed to read file: {e}"),
                            None,
                        );
                    }
                };
                file_data = Some((filename_attr, content_type, bytes));
            }
            "filename" => {
                filename_from_field = field.text().await.ok();
            }
            _ => {}
        }
    }

    let (filename_attr, reported_content_type, body) = match file_data {
        Some(data) => data,
        None => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "No file provided",
                "Multipart request must include a `file` field.".to_string(),
                None,
            );
        }
    };

    let filename = filename_from_field
        .or(filename_attr)
        .unwrap_or_else(|| "upload".to_string());

    let content_type = match normalize_upload_content_type(&filename, &reported_content_type) {
        Ok(ct) => ct,
        Err(msg) => {
            return api_json_error(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                &rid,
                PATH,
                "Unsupported file type",
                msg.to_string(),
                Some("Use a known extension or supported MIME type (documents, code, images, audio, video, fonts, Office, PDF, CSV, etc.). Executables and installers are blocked."),
            );
        }
    };

    // Validate size
    if body.len() > MAX_UPLOAD_SIZE {
        return api_json_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            &rid,
            PATH,
            "File too large",
            format!("Max size is {} MB.", MAX_UPLOAD_SIZE / (1024 * 1024)),
            None,
        );
    }

    if body.is_empty() {
        return api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Empty file body",
            "Upload body was empty.".to_string(),
            None,
        );
    }

    // Generate file ID and save
    let file_id = uuid::Uuid::new_v4().to_string();
    let upload_dir = std::env::temp_dir().join("openfang_uploads");
    if let Err(e) = std::fs::create_dir_all(&upload_dir) {
        tracing::warn!("Failed to create upload dir: {e}");
        return api_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &rid,
            PATH,
            "Failed to create upload directory",
            format!("{e}"),
            None,
        );
    }

    let file_path = upload_dir.join(&file_id);
    if let Err(e) = std::fs::write(&file_path, &body) {
        tracing::warn!("Failed to write upload: {e}");
        return api_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &rid,
            PATH,
            "Failed to save file",
            format!("{e}"),
            None,
        );
    }

    let size = body.len();
    UPLOAD_REGISTRY.insert(
        file_id.clone(),
        UploadMeta {
            filename: filename.clone(),
            content_type: content_type.clone(),
        },
    );

    // Auto-transcribe audio uploads using the media engine
    let transcription = if content_type.starts_with("audio/") {
        let attachment = openfang_types::media::MediaAttachment {
            media_type: openfang_types::media::MediaType::Audio,
            mime_type: content_type.clone(),
            source: openfang_types::media::MediaSource::FilePath {
                path: file_path.to_string_lossy().to_string(),
            },
            size_bytes: size as u64,
        };
        match state
            .kernel
            .media_engine
            .transcribe_audio(&attachment)
            .await
        {
            Ok(result) => {
                tracing::info!(chars = result.description.len(), provider = %result.provider, "Audio transcribed");
                Some(result.description)
            }
            Err(e) => {
                tracing::warn!("Audio transcription failed: {e}");
                None
            }
        }
    } else {
        None
    };

    (
        StatusCode::CREATED,
        Json(serde_json::json!(UploadResponse {
            file_id,
            filename,
            content_type,
            size,
            transcription,
        })),
    )
}

/// GET /api/uploads/{file_id} — Serve an uploaded file.
pub async fn serve_upload(Path(file_id): Path<String>) -> impl IntoResponse {
    // Validate file_id is a UUID to prevent path traversal
    if uuid::Uuid::parse_str(&file_id).is_err() {
        return (
            StatusCode::BAD_REQUEST,
            [(
                axum::http::header::CONTENT_TYPE,
                "application/json".to_string(),
            )],
            b"{\"error\":\"Invalid file ID\"}".to_vec(),
        );
    }

    let file_path = std::env::temp_dir().join("openfang_uploads").join(&file_id);

    // Look up metadata from registry; fall back to disk probe for generated images
    // (image_generate saves files without registering in UPLOAD_REGISTRY).
    let content_type = match UPLOAD_REGISTRY.get(&file_id) {
        Some(m) => m.content_type.clone(),
        None => {
            // Infer content type from file magic bytes
            if !file_path.exists() {
                return (
                    StatusCode::NOT_FOUND,
                    [(
                        axum::http::header::CONTENT_TYPE,
                        "application/json".to_string(),
                    )],
                    b"{\"error\":\"File not found\"}".to_vec(),
                );
            }
            "image/png".to_string()
        }
    };

    match std::fs::read(&file_path) {
        Ok(data) => (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, content_type)],
            data,
        ),
        Err(_) => (
            StatusCode::NOT_FOUND,
            [(
                axum::http::header::CONTENT_TYPE,
                "application/json".to_string(),
            )],
            b"{\"error\":\"File not found on disk\"}".to_vec(),
        ),
    }
}

// ---------------------------------------------------------------------------
// Execution Approval System — backed by kernel.approval_manager
// ---------------------------------------------------------------------------

/// GET /api/approvals — List pending and recent approval requests.
///
/// Transforms field names to match the dashboard template expectations:
/// `action_summary` → `action`, `agent_id` → `agent_name`, `requested_at` → `created_at`.
pub async fn list_approvals(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let pending = state.kernel.approval_manager.list_pending();
    let recent = state.kernel.approval_manager.list_recent(50);

    // Resolve agent names for display
    let registry_agents = state.kernel.registry.list();
    let agent_name_for = |agent_id: &str| {
        registry_agents
            .iter()
            .find(|ag| ag.id.to_string() == agent_id || ag.name == agent_id)
            .map(|ag| ag.name.clone())
            .unwrap_or_else(|| agent_id.to_string())
    };

    let mut approvals: Vec<serde_json::Value> = pending
        .into_iter()
        .map(|a| {
            let agent_name = agent_name_for(&a.agent_id);
            serde_json::json!({
                "id": a.id,
                "agent_id": a.agent_id,
                "agent_name": agent_name,
                "tool_name": a.tool_name,
                "description": a.description,
                "action_summary": a.action_summary,
                "action": a.action_summary,
                "risk_level": a.risk_level,
                "requested_at": a.requested_at,
                "created_at": a.requested_at,
                "timeout_secs": a.timeout_secs,
                "status": "pending"
            })
        })
        .collect();

    approvals.extend(recent.into_iter().map(|record| {
        let request = record.request;
        let agent_name = agent_name_for(&request.agent_id);
        let status = match record.decision {
            openfang_types::approval::ApprovalDecision::Approved => "approved",
            openfang_types::approval::ApprovalDecision::Denied => "rejected",
            openfang_types::approval::ApprovalDecision::TimedOut => "expired",
        };
        serde_json::json!({
            "id": request.id,
            "agent_id": request.agent_id,
            "agent_name": agent_name,
            "tool_name": request.tool_name,
            "description": request.description,
            "action_summary": request.action_summary,
            "action": request.action_summary,
            "risk_level": request.risk_level,
            "requested_at": request.requested_at,
            "created_at": request.requested_at,
            "timeout_secs": request.timeout_secs,
            "status": status,
            "decided_at": record.decided_at,
            "decided_by": record.decided_by,
        })
    }));

    approvals.sort_by(|a, b| {
        let a_pending = a["status"].as_str() == Some("pending");
        let b_pending = b["status"].as_str() == Some("pending");
        b_pending
            .cmp(&a_pending)
            .then_with(|| b["created_at"].as_str().cmp(&a["created_at"].as_str()))
    });

    let total = approvals.len();

    Json(serde_json::json!({"approvals": approvals, "total": total}))
}

/// POST /api/approvals — Create a manual approval request (for external systems).
///
/// Note: Most approval requests are created automatically by the tool_runner
/// when an agent invokes a tool that requires approval. This endpoint exists
/// for external integrations that need to inject approval gates.
#[derive(serde::Deserialize)]
pub struct CreateApprovalRequest {
    pub agent_id: String,
    pub tool_name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub action_summary: String,
}

pub async fn create_approval(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateApprovalRequest>,
) -> impl IntoResponse {
    use openfang_types::approval::{ApprovalRequest, RiskLevel};

    let policy = state.kernel.approval_manager.policy();
    let id = uuid::Uuid::new_v4();
    let approval_req = ApprovalRequest {
        id,
        agent_id: req.agent_id,
        tool_name: req.tool_name.clone(),
        description: if req.description.is_empty() {
            format!("Manual approval request for {}", req.tool_name)
        } else {
            req.description
        },
        action_summary: if req.action_summary.is_empty() {
            req.tool_name.clone()
        } else {
            req.action_summary
        },
        risk_level: RiskLevel::High,
        requested_at: chrono::Utc::now(),
        timeout_secs: policy.timeout_secs,
    };

    // Spawn the request in the background (it will block until resolved or timed out)
    let kernel = Arc::clone(&state.kernel);
    tokio::spawn(async move {
        kernel
            .approval_manager
            .request_approval(approval_req, Some(kernel.event_bus.as_ref()))
            .await;
    });

    (
        StatusCode::CREATED,
        Json(serde_json::json!({"id": id.to_string(), "status": "pending"})),
    )
}

/// POST /api/approvals/{id}/approve — Approve a pending request.
pub async fn approve_request(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/approvals/:id/approve";
    let uuid = match uuid::Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid approval ID",
                "Approval id must be a UUID.".to_string(),
                None,
            );
        }
    };

    match state.kernel.approval_manager.resolve(
        uuid,
        openfang_types::approval::ApprovalDecision::Approved,
        Some("api".to_string()),
    ) {
        Ok(resp) => (
            StatusCode::OK,
            Json(
                serde_json::json!({"id": id, "status": "approved", "decided_at": resp.decided_at.to_rfc3339()}),
            ),
        ),
        Err(e) => api_json_error(
            StatusCode::NOT_FOUND,
            &rid,
            PATH,
            "Approval not found",
            e,
            None,
        ),
    }
}

/// POST /api/approvals/{id}/reject — Reject a pending request.
pub async fn reject_request(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/approvals/:id/reject";
    let uuid = match uuid::Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid approval ID",
                "Approval id must be a UUID.".to_string(),
                None,
            );
        }
    };

    match state.kernel.approval_manager.resolve(
        uuid,
        openfang_types::approval::ApprovalDecision::Denied,
        Some("api".to_string()),
    ) {
        Ok(resp) => (
            StatusCode::OK,
            Json(
                serde_json::json!({"id": id, "status": "rejected", "decided_at": resp.decided_at.to_rfc3339()}),
            ),
        ),
        Err(e) => api_json_error(
            StatusCode::NOT_FOUND,
            &rid,
            PATH,
            "Approval not found",
            e,
            None,
        ),
    }
}

// ---------------------------------------------------------------------------
// Config Reload endpoint
// ---------------------------------------------------------------------------

/// POST /api/config/reload — Reload configuration from disk and apply hot-reloadable changes.
///
/// Reads the config file, diffs against current config, validates the new config,
/// and applies hot-reloadable actions (approval policy, cron limits, etc.).
/// Returns the reload plan showing what changed and what was applied.
pub async fn config_reload(
    State(state): State<Arc<AppState>>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/config/reload";
    // SECURITY: Record config reload in audit trail
    state.kernel.audit_log.record(
        "system",
        openfang_runtime::audit::AuditAction::ConfigChange,
        "config reload requested via API",
        "pending",
    );
    match state.kernel.reload_config() {
        Ok(plan) => {
            let status = if plan.restart_required {
                "partial"
            } else if plan.has_changes() {
                "applied"
            } else {
                "no_changes"
            };

            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": status,
                    "restart_required": plan.restart_required,
                    "restart_reasons": plan.restart_reasons,
                    "hot_actions_applied": plan.hot_actions.iter().map(|a| format!("{a:?}")).collect::<Vec<_>>(),
                    "noop_changes": plan.noop_changes,
                })),
            )
        }
        Err(e) => api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Config reload failed",
            e,
            Some("Fix config errors on disk, then retry."),
        ),
    }
}

// ---------------------------------------------------------------------------
// Config Schema endpoint
// ---------------------------------------------------------------------------

/// GET /api/config/schema — Return a simplified JSON description of the config structure.
pub async fn config_schema(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // Build provider/model options from model catalog for dropdowns
    let catalog = state
        .kernel
        .model_catalog
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let provider_options: Vec<String> = catalog
        .list_providers()
        .iter()
        .map(|p| p.id.clone())
        .collect();
    let model_options: Vec<serde_json::Value> = catalog
        .list_models()
        .iter()
        .map(|m| serde_json::json!({"id": m.id, "name": m.display_name, "provider": m.provider}))
        .collect();
    drop(catalog);

    // Helper: normalize field definitions to objects with {name, type, label}
    // so the frontend template can iterate and render inputs correctly.
    let f = |name: &str, ftype: &str, label: &str| -> serde_json::Value {
        serde_json::json!({"name": name, "type": ftype, "label": label})
    };

    Json(serde_json::json!({
        "sections": {
            "general": {
                "root_level": true,
                "fields": [
                    f("api_listen", "string", "API Listen Address"),
                    f("api_key", "string", "API Key"),
                    f("log_level", "string", "Log Level")
                ]
            },
            "default_model": {
                "hot_reloadable": true,
                "fields": [
                    { "name": "provider", "type": "select", "label": "Provider", "options": provider_options },
                    { "name": "model", "type": "select", "label": "Model", "options": model_options },
                    f("api_key_env", "string", "API Key Env Var"),
                    f("base_url", "string", "Base URL")
                ]
            },
            "memory": {
                "fields": [
                    f("decay_rate", "number", "Decay Rate"),
                    f("vector_dims", "number", "Vector Dimensions")
                ]
            },
            "web": {
                "fields": [
                    f("provider", "string", "Search Provider"),
                    f("timeout_secs", "number", "Timeout (seconds)"),
                    f("max_results", "number", "Max Results")
                ]
            },
            "browser": {
                "fields": [
                    f("headless", "boolean", "Headless Mode"),
                    f("timeout_secs", "number", "Timeout (seconds)"),
                    f("executable_path", "string", "Chrome/Chromium Path")
                ]
            },
            "network": {
                "fields": [
                    f("enabled", "boolean", "Enable OFP Network"),
                    f("listen_addr", "string", "Listen Address"),
                    f("shared_secret", "string", "Shared Secret")
                ]
            },
            "extensions": {
                "fields": [
                    f("auto_connect", "boolean", "Auto Connect"),
                    f("health_check_interval_secs", "number", "Health Check Interval (s)")
                ]
            },
            "vault": {
                "fields": [
                    f("path", "string", "Vault Path")
                ]
            },
            "a2a": {
                "fields": [
                    f("enabled", "boolean", "Enable A2A"),
                    f("name", "string", "Agent Name"),
                    f("description", "string", "Description"),
                    f("url", "string", "URL")
                ]
            },
            "channels": {
                "fields": [
                    f("telegram", "object", "Telegram"),
                    f("discord", "object", "Discord"),
                    f("slack", "object", "Slack"),
                    f("whatsapp", "object", "WhatsApp")
                ]
            }
        }
    }))
}

// ---------------------------------------------------------------------------
// Config Set endpoint
// ---------------------------------------------------------------------------

/// POST /api/config/set — Set a single config value and persist to config.toml.
///
/// Accepts JSON `{ "path": "section.key", "value": "..." }`.
/// Writes the value to the TOML config file and triggers a reload.
pub async fn config_set(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let path = match body.get("path").and_then(|v| v.as_str()) {
        Some(p) => p.to_string(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"status": "error", "error": "missing 'path' field"})),
            );
        }
    };
    let value = match body.get("value") {
        Some(v) => v.clone(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"status": "error", "error": "missing 'value' field"})),
            );
        }
    };

    let config_path = state.kernel.config.home_dir.join("config.toml");

    // Read existing config as a TOML table, or start fresh
    let mut table: toml::value::Table = if config_path.exists() {
        match std::fs::read_to_string(&config_path) {
            Ok(content) => toml::from_str(&content).unwrap_or_default(),
            Err(_) => toml::value::Table::new(),
        }
    } else {
        toml::value::Table::new()
    };

    // Convert JSON value to TOML value
    let toml_val = json_to_toml_value(&value);

    // Parse "section.key" path and set value
    let parts: Vec<&str> = path.split('.').collect();
    match parts.len() {
        1 => {
            table.insert(parts[0].to_string(), toml_val);
        }
        2 => {
            let section = table
                .entry(parts[0].to_string())
                .or_insert_with(|| toml::Value::Table(toml::value::Table::new()));
            if let toml::Value::Table(ref mut t) = section {
                t.insert(parts[1].to_string(), toml_val);
            }
        }
        3 => {
            let section = table
                .entry(parts[0].to_string())
                .or_insert_with(|| toml::Value::Table(toml::value::Table::new()));
            if let toml::Value::Table(ref mut t) = section {
                let sub = t
                    .entry(parts[1].to_string())
                    .or_insert_with(|| toml::Value::Table(toml::value::Table::new()));
                if let toml::Value::Table(ref mut t2) = sub {
                    t2.insert(parts[2].to_string(), toml_val);
                }
            }
        }
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    serde_json::json!({"status": "error", "error": "path too deep (max 3 levels)"}),
                ),
            );
        }
    }

    // Write back
    let toml_string = match toml::to_string_pretty(&table) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(
                    serde_json::json!({"status": "error", "error": format!("serialize failed: {e}")}),
                ),
            );
        }
    };
    if let Err(e) = std::fs::write(&config_path, &toml_string) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"status": "error", "error": format!("write failed: {e}")})),
        );
    }

    // Trigger reload
    let reload_status = match state.kernel.reload_config() {
        Ok(plan) => {
            if plan.restart_required {
                "applied_partial"
            } else {
                "applied"
            }
        }
        Err(_) => "saved_reload_failed",
    };

    state.kernel.audit_log.record(
        "system",
        openfang_runtime::audit::AuditAction::ConfigChange,
        format!("config set: {path}"),
        "completed",
    );

    (
        StatusCode::OK,
        Json(serde_json::json!({"status": reload_status, "path": path})),
    )
}

/// Convert a serde_json::Value to a toml::Value.
fn json_to_toml_value(value: &serde_json::Value) -> toml::Value {
    match value {
        serde_json::Value::String(s) => toml::Value::String(s.clone()),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_u64() {
                toml::Value::Integer(i as i64)
            } else if let Some(i) = n.as_i64() {
                toml::Value::Integer(i)
            } else if let Some(f) = n.as_f64() {
                toml::Value::Float(f)
            } else {
                toml::Value::String(n.to_string())
            }
        }
        serde_json::Value::Bool(b) => toml::Value::Boolean(*b),
        _ => toml::Value::String(value.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Delivery tracking endpoints
// ---------------------------------------------------------------------------

/// GET /api/agents/:id/deliveries — List recent delivery receipts for an agent.
pub async fn get_agent_deliveries(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/agents/:id/deliveries";
    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            // Try name lookup
            match state.kernel.registry.find_by_name(&id) {
                Some(entry) => entry.id,
                None => {
                    return api_json_error(
                        StatusCode::NOT_FOUND,
                        &rid,
                        PATH,
                        "Agent not found",
                        format!("No agent matches id or name `{id}`."),
                        Some("Use GET /api/agents to list agents."),
                    );
                }
            }
        }
    };

    let limit = params
        .get("limit")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(50)
        .min(500);

    let receipts = state.kernel.delivery_tracker.get_receipts(agent_id, limit);
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "agent_id": agent_id.to_string(),
            "count": receipts.len(),
            "receipts": receipts,
        })),
    )
}

// ---------------------------------------------------------------------------
// Cron job management endpoints
// ---------------------------------------------------------------------------

/// GET /api/cron/jobs — List all cron jobs, optionally filtered by agent_id.
pub async fn list_cron_jobs(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/cron/jobs";
    let jobs = if let Some(agent_id_str) = params.get("agent_id") {
        match uuid::Uuid::parse_str(agent_id_str) {
            Ok(uuid) => {
                let aid = AgentId(uuid);
                state.kernel.cron_scheduler.list_jobs(aid)
            }
            Err(_) => {
                return api_json_error(
                    StatusCode::BAD_REQUEST,
                    &rid,
                    PATH,
                    "Invalid agent_id",
                    "The agent_id query parameter must be a UUID.".to_string(),
                    None,
                );
            }
        }
    } else {
        state.kernel.cron_scheduler.list_all_jobs()
    };
    let total = jobs.len();
    let jobs_json: Vec<serde_json::Value> = jobs
        .into_iter()
        .map(|j| serde_json::to_value(&j).unwrap_or_default())
        .collect();
    (
        StatusCode::OK,
        Json(serde_json::json!({"jobs": jobs_json, "total": total})),
    )
}

/// POST /api/cron/jobs — Create a new cron job.
pub async fn create_cron_job(
    State(state): State<Arc<AppState>>,
    ext: Option<Extension<RequestId>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/cron/jobs";
    let agent_id = body["agent_id"].as_str().unwrap_or("");
    match state.kernel.cron_create(agent_id, body.clone()).await {
        Ok(result) => (
            StatusCode::CREATED,
            Json(serde_json::json!({"result": result})),
        ),
        Err(e) => api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Cron job creation failed",
            e,
            Some("Validate agent_id (UUID), name, schedule, action, and optional delivery against the cron schema."),
        ),
    }
}

fn parse_cron_job_body(body: &serde_json::Value, id: CronJobId) -> Result<CronJob, String> {
    let name = body["name"]
        .as_str()
        .ok_or_else(|| "Missing 'name' field".to_string())?
        .to_string();
    let schedule: CronSchedule = serde_json::from_value(body["schedule"].clone())
        .map_err(|e| format!("Invalid schedule: {e}"))?;
    let action: CronAction = serde_json::from_value(body["action"].clone())
        .map_err(|e| format!("Invalid action: {e}"))?;
    let delivery: CronDelivery = if body["delivery"].is_object() {
        serde_json::from_value(body["delivery"].clone())
            .map_err(|e| format!("Invalid delivery: {e}"))?
    } else {
        CronDelivery::None
    };
    let enabled = body["enabled"].as_bool().unwrap_or(true);
    let agent_id = AgentId(
        uuid::Uuid::parse_str(
            body["agent_id"]
                .as_str()
                .ok_or_else(|| "Missing agent_id".to_string())?,
        )
        .map_err(|e| format!("Invalid agent ID: {e}"))?,
    );
    Ok(CronJob {
        id,
        agent_id,
        name,
        schedule,
        action,
        delivery,
        enabled,
        created_at: chrono::Utc::now(),
        last_run: None,
        next_run: None,
    })
}

/// PUT /api/cron/jobs/{id} — Update an existing cron job (same id).
pub async fn update_cron_job(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/cron/jobs/:id";
    let uuid = match uuid::Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid job ID",
                "Job id must be a UUID.".to_string(),
                Some("Use the id from GET /api/cron/jobs."),
            );
        }
    };
    let job_id = CronJobId(uuid);
    let job = match parse_cron_job_body(&body, job_id) {
        Ok(j) => j,
        Err(e) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid cron job body",
                e,
                Some("Use the same JSON shape as POST /api/cron/jobs."),
            );
        }
    };
    match state.kernel.cron_scheduler.update_job(job_id, job) {
        Ok(()) => {
            let _ = state.kernel.cron_scheduler.persist();
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "updated", "job_id": id})),
            )
        }
        Err(e) => api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Cron job update failed",
            format!("{e}"),
            None,
        ),
    }
}

/// DELETE /api/cron/jobs/{id} — Delete a cron job.
pub async fn delete_cron_job(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/cron/jobs/:id";
    match uuid::Uuid::parse_str(&id) {
        Ok(uuid) => {
            let job_id = openfang_types::scheduler::CronJobId(uuid);
            match state.kernel.cron_scheduler.remove_job(job_id) {
                Ok(_) => {
                    let _ = state.kernel.cron_scheduler.persist();
                    (
                        StatusCode::OK,
                        Json(serde_json::json!({"status": "deleted"})),
                    )
                }
                Err(e) => api_json_error(
                    StatusCode::NOT_FOUND,
                    &rid,
                    PATH,
                    "Cron job not found",
                    format!("{e}"),
                    Some("List jobs with GET /api/cron/jobs."),
                ),
            }
        }
        Err(_) => api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Invalid job ID",
            "Job id must be a UUID.".to_string(),
            Some("Use the id from GET /api/cron/jobs."),
        ),
    }
}

/// PUT /api/cron/jobs/{id}/enable — Enable or disable a cron job.
pub async fn toggle_cron_job(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/cron/jobs/:id/enable";
    let enabled = body["enabled"].as_bool().unwrap_or(true);
    match uuid::Uuid::parse_str(&id) {
        Ok(uuid) => {
            let job_id = openfang_types::scheduler::CronJobId(uuid);
            match state.kernel.cron_scheduler.set_enabled(job_id, enabled) {
                Ok(()) => {
                    let _ = state.kernel.cron_scheduler.persist();
                    (
                        StatusCode::OK,
                        Json(serde_json::json!({"id": id, "enabled": enabled})),
                    )
                }
                Err(e) => api_json_error(
                    StatusCode::NOT_FOUND,
                    &rid,
                    PATH,
                    "Cron job not found",
                    format!("{e}"),
                    Some("List jobs with GET /api/cron/jobs."),
                ),
            }
        }
        Err(_) => api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Invalid job ID",
            "Job id must be a UUID.".to_string(),
            Some("Use the id from GET /api/cron/jobs."),
        ),
    }
}

/// GET /api/cron/jobs/{id}/status — Get status of a specific cron job.
pub async fn cron_job_status(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/cron/jobs/:id/status";
    match uuid::Uuid::parse_str(&id) {
        Ok(uuid) => {
            let job_id = openfang_types::scheduler::CronJobId(uuid);
            match state.kernel.cron_scheduler.get_meta(job_id) {
                Some(meta) => (
                    StatusCode::OK,
                    Json(serde_json::to_value(&meta).unwrap_or_default()),
                ),
                None => api_json_error(
                    StatusCode::NOT_FOUND,
                    &rid,
                    PATH,
                    "Job not found",
                    format!("No cron job with id {id}."),
                    Some("List jobs with GET /api/cron/jobs."),
                ),
            }
        }
        Err(_) => api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Invalid job ID",
            "Job id must be a UUID.".to_string(),
            Some("Use the id from GET /api/cron/jobs."),
        ),
    }
}

// ---------------------------------------------------------------------------
// Run cron job on demand
// ---------------------------------------------------------------------------

/// POST /api/cron/jobs/{id}/run — Trigger a cron job immediately.
///
/// Returns `{"status": "triggered", "job_id": "..."}` and spawns the execution
/// in the background.  The job's status can be polled via
/// `GET /api/cron/jobs/{id}/status`.
pub async fn run_cron_job(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/cron/jobs/:id/run";
    let uuid = match uuid::Uuid::parse_str(&id) {
        Ok(u) => u,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid job ID",
                "Job id must be a UUID.".to_string(),
                Some("Use the id from GET /api/cron/jobs."),
            );
        }
    };
    let job_id = openfang_types::scheduler::CronJobId(uuid);

    // Atomically check existence + enabled + reserve next_run in one lock hold.
    let job = match state.kernel.cron_scheduler.try_claim_for_run(job_id) {
        Ok(j) => j,
        Err(openfang_kernel::cron::ClaimError::NotFound) => {
            return api_json_error(
                StatusCode::NOT_FOUND,
                &rid,
                PATH,
                "Job not found",
                format!("No cron job with id {id}."),
                Some("List jobs with GET /api/cron/jobs."),
            );
        }
        Err(openfang_kernel::cron::ClaimError::Disabled) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Job is disabled",
                "Enable the job before running it on demand.".to_string(),
                Some("PUT /api/cron/jobs/:id/enable with {\"enabled\": true}."),
            );
        }
    };

    // Spawn execution in the background so we don't block the HTTP response.
    let kernel = Arc::clone(&state.kernel);
    let job_name = job.name.clone();
    tokio::spawn(async move {
        match kernel.cron_run_job(&job).await {
            Ok(_) => tracing::info!(job = %job_name, "On-demand cron job completed"),
            Err(e) => tracing::warn!(job = %job_name, error = %e, "On-demand cron job failed"),
        }
    });

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "triggered",
            "job_id": id,
        })),
    )
}

// ---------------------------------------------------------------------------
// Webhook trigger endpoints
// ---------------------------------------------------------------------------

/// POST /hooks/wake — Inject a system event via webhook trigger.
///
/// Publishes a custom event through the kernel's event system, which can
/// trigger proactive agents that subscribe to the event type.
pub async fn webhook_wake(
    State(state): State<Arc<AppState>>,
    ext: Option<Extension<RequestId>>,
    headers: axum::http::HeaderMap,
    Json(body): Json<openfang_types::webhook::WakePayload>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/hooks/wake";
    // Check if webhook triggers are enabled
    let wh_config = match &state.kernel.config.webhook_triggers {
        Some(c) if c.enabled => c,
        _ => {
            return api_json_error(
                StatusCode::NOT_FOUND,
                &rid,
                PATH,
                "Webhook triggers not enabled",
                "Enable webhook_triggers in config and restart.".to_string(),
                None,
            );
        }
    };

    // Validate bearer token (constant-time comparison)
    if !validate_webhook_token(&headers, &wh_config.token_env) {
        return api_json_error(
            StatusCode::UNAUTHORIZED,
            &rid,
            PATH,
            "Invalid or missing token",
            "Provide Authorization: Bearer <token> matching the configured env.".to_string(),
            None,
        );
    }

    // Validate payload
    if let Err(e) = body.validate() {
        return api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Invalid payload",
            e,
            None,
        );
    }

    // Publish through the kernel's publish_event (KernelHandle trait), which
    // goes through the full event processing pipeline including trigger evaluation.
    let event_payload = serde_json::json!({
        "source": "webhook",
        "mode": body.mode,
        "text": body.text,
    });
    if let Err(e) =
        KernelHandle::publish_event(state.kernel.as_ref(), "webhook.wake", event_payload).await
    {
        tracing::warn!("Webhook wake event publish failed: {e}");
        return api_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &rid,
            PATH,
            "Event publish failed",
            e.to_string(),
            None,
        );
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "accepted", "mode": body.mode})),
    )
}

/// POST /hooks/agent — Run an isolated agent turn via webhook.
///
/// Sends a message directly to the specified agent and returns the response.
/// This enables external systems (CI/CD, Slack, etc.) to trigger agent work.
pub async fn webhook_agent(
    State(state): State<Arc<AppState>>,
    ext: Option<Extension<RequestId>>,
    headers: axum::http::HeaderMap,
    Json(body): Json<openfang_types::webhook::AgentHookPayload>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/hooks/agent";
    // Check if webhook triggers are enabled
    let wh_config = match &state.kernel.config.webhook_triggers {
        Some(c) if c.enabled => c,
        _ => {
            return api_json_error(
                StatusCode::NOT_FOUND,
                &rid,
                PATH,
                "Webhook triggers not enabled",
                "Enable webhook_triggers in config and restart.".to_string(),
                None,
            );
        }
    };

    // Validate bearer token
    if !validate_webhook_token(&headers, &wh_config.token_env) {
        return api_json_error(
            StatusCode::UNAUTHORIZED,
            &rid,
            PATH,
            "Invalid or missing token",
            "Provide Authorization: Bearer <token> matching the configured env.".to_string(),
            None,
        );
    }

    // Validate payload
    if let Err(e) = body.validate() {
        return api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Invalid payload",
            e,
            None,
        );
    }

    // Resolve the agent by name or ID (if not specified, use the first running agent)
    let agent_id: AgentId = match &body.agent {
        Some(agent_ref) => match agent_ref.parse() {
            Ok(id) => id,
            Err(_) => {
                // Try name lookup
                match state.kernel.registry.find_by_name(agent_ref) {
                    Some(entry) => entry.id,
                    None => {
                        return api_json_error(
                            StatusCode::NOT_FOUND,
                            &rid,
                            PATH,
                            "Agent not found",
                            format!("No agent matches `{agent_ref}`."),
                            Some("Use GET /api/agents to list agents."),
                        );
                    }
                }
            }
        },
        None => {
            // No agent specified — use the first available agent
            match state.kernel.registry.list().first() {
                Some(entry) => entry.id,
                None => {
                    return api_json_error(
                        StatusCode::NOT_FOUND,
                        &rid,
                        PATH,
                        "No agents available",
                        "Spawn an agent before calling this webhook.".to_string(),
                        None,
                    );
                }
            }
        }
    };

    // Actually send the message to the agent and get the response
    match state.kernel.send_message(agent_id, &body.message).await {
        Ok(result) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "completed",
                "agent_id": agent_id.to_string(),
                "response": result.response,
                "usage": {
                    "input_tokens": result.total_usage.input_tokens,
                    "output_tokens": result.total_usage.output_tokens,
                },
            })),
        ),
        Err(e) => api_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &rid,
            PATH,
            "Agent execution failed",
            format!("{e}"),
            None,
        ),
    }
}

// ─── Agent Bindings API ────────────────────────────────────────────────

/// GET /api/bindings — List all agent bindings.
pub async fn list_bindings(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let bindings = state.kernel.list_bindings();
    (
        StatusCode::OK,
        Json(serde_json::json!({ "bindings": bindings })),
    )
}

/// POST /api/bindings — Add a new agent binding.
pub async fn add_binding(
    State(state): State<Arc<AppState>>,
    Json(binding): Json<openfang_types::config::AgentBinding>,
) -> impl IntoResponse {
    // Validate agent exists
    let agents = state.kernel.registry.list();
    let agent_exists = agents.iter().any(|e| e.name == binding.agent)
        || binding.agent.parse::<uuid::Uuid>().is_ok();
    if !agent_exists {
        tracing::warn!(agent = %binding.agent, "Binding references unknown agent");
    }

    state.kernel.add_binding(binding);
    let reload = crate::channel_bridge::reload_channels_from_disk(&state).await;
    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "status": "created",
            "channels_reloaded": reload.is_ok(),
            "channels_started": reload.as_ref().map(|(names, _)| names).unwrap_or(&Vec::new()).clone(),
            "reload_error": reload.err(),
        })),
    )
}

/// DELETE /api/bindings/:index — Remove a binding by index.
pub async fn remove_binding(
    State(state): State<Arc<AppState>>,
    Path(index): Path<usize>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/bindings/:index";
    match state.kernel.remove_binding(index) {
        Some(_) => {
            let reload = crate::channel_bridge::reload_channels_from_disk(&state).await;
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "removed",
                    "channels_reloaded": reload.is_ok(),
                    "channels_started": reload.as_ref().map(|(names, _)| names).unwrap_or(&Vec::new()).clone(),
                    "reload_error": reload.err(),
                })),
            )
        }
        None => api_json_error(
            StatusCode::NOT_FOUND,
            &rid,
            PATH,
            "Binding not found",
            format!("No binding at index {index}."),
            Some("Use GET /api/bindings to list bindings."),
        ),
    }
}

// ─── Device Pairing endpoints ───────────────────────────────────────────

/// POST /api/pairing/request — Create a new pairing request (returns token + QR URI).
pub async fn pairing_request(
    State(state): State<Arc<AppState>>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/pairing/request";
    if !state.kernel.config.pairing.enabled {
        return api_json_error(
            StatusCode::NOT_FOUND,
            &rid,
            PATH,
            "Pairing not enabled",
            "Enable pairing in config and restart.".to_string(),
            None,
        )
        .into_response();
    }
    match state.kernel.pairing.create_pairing_request() {
        Ok(req) => {
            let qr_uri = format!("openfang://pair?token={}", req.token);
            Json(serde_json::json!({
                "token": req.token,
                "qr_uri": qr_uri,
                "expires_at": req.expires_at.to_rfc3339(),
            }))
            .into_response()
        }
        Err(e) => api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Pairing request failed",
            e,
            None,
        )
        .into_response(),
    }
}

/// POST /api/pairing/complete — Complete pairing with token + device info.
pub async fn pairing_complete(
    State(state): State<Arc<AppState>>,
    ext: Option<Extension<RequestId>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/pairing/complete";
    if !state.kernel.config.pairing.enabled {
        return api_json_error(
            StatusCode::NOT_FOUND,
            &rid,
            PATH,
            "Pairing not enabled",
            "Enable pairing in config and restart.".to_string(),
            None,
        )
        .into_response();
    }
    let token = body.get("token").and_then(|v| v.as_str()).unwrap_or("");
    let display_name = body
        .get("display_name")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let platform = body
        .get("platform")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let push_token = body
        .get("push_token")
        .and_then(|v| v.as_str())
        .map(String::from);
    let device_info = openfang_kernel::pairing::PairedDevice {
        device_id: uuid::Uuid::new_v4().to_string(),
        display_name: display_name.to_string(),
        platform: platform.to_string(),
        paired_at: chrono::Utc::now(),
        last_seen: chrono::Utc::now(),
        push_token,
    };
    match state.kernel.pairing.complete_pairing(token, device_info) {
        Ok(device) => Json(serde_json::json!({
            "device_id": device.device_id,
            "display_name": device.display_name,
            "platform": device.platform,
            "paired_at": device.paired_at.to_rfc3339(),
        }))
        .into_response(),
        Err(e) => api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Pairing failed",
            e,
            None,
        )
        .into_response(),
    }
}

/// GET /api/pairing/devices — List paired devices.
pub async fn pairing_devices(
    State(state): State<Arc<AppState>>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/pairing/devices";
    if !state.kernel.config.pairing.enabled {
        return api_json_error(
            StatusCode::NOT_FOUND,
            &rid,
            PATH,
            "Pairing not enabled",
            "Enable pairing in config and restart.".to_string(),
            None,
        )
        .into_response();
    }
    let devices: Vec<_> = state
        .kernel
        .pairing
        .list_devices()
        .into_iter()
        .map(|d| {
            serde_json::json!({
                "device_id": d.device_id,
                "display_name": d.display_name,
                "platform": d.platform,
                "paired_at": d.paired_at.to_rfc3339(),
                "last_seen": d.last_seen.to_rfc3339(),
            })
        })
        .collect();
    Json(serde_json::json!({"devices": devices})).into_response()
}

/// DELETE /api/pairing/devices/{id} — Remove a paired device.
pub async fn pairing_remove_device(
    State(state): State<Arc<AppState>>,
    Path(device_id): Path<String>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/pairing/devices/:id";
    if !state.kernel.config.pairing.enabled {
        return api_json_error(
            StatusCode::NOT_FOUND,
            &rid,
            PATH,
            "Pairing not enabled",
            "Enable pairing in config and restart.".to_string(),
            None,
        )
        .into_response();
    }
    match state.kernel.pairing.remove_device(&device_id) {
        Ok(()) => Json(serde_json::json!({"ok": true})).into_response(),
        Err(e) => api_json_error(
            StatusCode::NOT_FOUND,
            &rid,
            PATH,
            "Device not found",
            e,
            None,
        )
        .into_response(),
    }
}

/// POST /api/pairing/notify — Push a notification to all paired devices.
pub async fn pairing_notify(
    State(state): State<Arc<AppState>>,
    ext: Option<Extension<RequestId>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/pairing/notify";
    if !state.kernel.config.pairing.enabled {
        return api_json_error(
            StatusCode::NOT_FOUND,
            &rid,
            PATH,
            "Pairing not enabled",
            "Enable pairing in config and restart.".to_string(),
            None,
        )
        .into_response();
    }
    let title = body
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("ArmaraOS");
    let message = body.get("message").and_then(|v| v.as_str()).unwrap_or("");
    if message.is_empty() {
        return api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Invalid body",
            "Field `message` is required and must be non-empty.".to_string(),
            None,
        )
        .into_response();
    }
    state.kernel.pairing.notify_devices(title, message).await;
    Json(serde_json::json!({"ok": true, "notified": state.kernel.pairing.list_devices().len()}))
        .into_response()
}

/// GET /api/commands — List available chat commands (for dynamic slash menu).
pub async fn list_commands(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let mut commands: Vec<serde_json::Value> = channel_command_specs()
        .iter()
        .map(|spec| {
            serde_json::json!({
                "cmd": format!("/{}", spec.name),
                "desc": spec.desc,
                "source": "channel",
            })
        })
        .collect();

    commands.extend([
        serde_json::json!({"cmd": "/context", "desc": "Show context window usage & pressure"}),
        serde_json::json!({"cmd": "/verbose", "desc": "Cycle tool detail level (/verbose [off|on|full])"}),
        serde_json::json!({"cmd": "/queue", "desc": "Check if agent is processing"}),
        serde_json::json!({"cmd": "/clear", "desc": "Clear chat display"}),
        serde_json::json!({"cmd": "/exit", "desc": "Disconnect from agent"}),
    ]);

    // Add skill-registered tool names as potential commands
    if let Ok(registry) = state.kernel.skill_registry.read() {
        for skill in registry.list() {
            let desc: String = skill.manifest.skill.description.chars().take(80).collect();
            commands.push(serde_json::json!({
                "cmd": format!("/{}", skill.manifest.skill.name),
                "desc": if desc.is_empty() { format!("Skill: {}", skill.manifest.skill.name) } else { desc },
                "source": "skill",
            }));
        }
    }

    Json(serde_json::json!({"commands": commands}))
}

/// SECURITY: Validate webhook bearer token using constant-time comparison.
fn validate_webhook_token(headers: &axum::http::HeaderMap, token_env: &str) -> bool {
    let expected = match std::env::var(token_env) {
        Ok(t) if t.len() >= 32 => t,
        _ => return false,
    };

    let provided = match headers.get("authorization") {
        Some(v) => match v.to_str() {
            Ok(s) if s.starts_with("Bearer ") => &s[7..],
            _ => return false,
        },
        None => return false,
    };

    use subtle::ConstantTimeEq;
    if provided.len() != expected.len() {
        return false;
    }
    provided.as_bytes().ct_eq(expected.as_bytes()).into()
}

// ══════════════════════════════════════════════════════════════════════
// GitHub Copilot OAuth Device Flow
// ══════════════════════════════════════════════════════════════════════

/// State for an in-progress device flow.
struct CopilotFlowState {
    device_code: String,
    interval: u64,
    expires_at: Instant,
}

/// Active device flows, keyed by poll_id. Auto-expire after the flow's TTL.
static COPILOT_FLOWS: LazyLock<DashMap<String, CopilotFlowState>> = LazyLock::new(DashMap::new);

/// POST /api/providers/github-copilot/oauth/start
///
/// Initiates a GitHub device flow for Copilot authentication.
/// Returns a user code and verification URI that the user visits in their browser.
pub async fn copilot_oauth_start(ext: Option<Extension<RequestId>>) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/providers/github-copilot/oauth/start";
    // Clean up expired flows first
    COPILOT_FLOWS.retain(|_, state| state.expires_at > Instant::now());

    match openfang_runtime::copilot_oauth::start_device_flow().await {
        Ok(resp) => {
            let poll_id = uuid::Uuid::new_v4().to_string();

            COPILOT_FLOWS.insert(
                poll_id.clone(),
                CopilotFlowState {
                    device_code: resp.device_code,
                    interval: resp.interval,
                    expires_at: Instant::now() + std::time::Duration::from_secs(resp.expires_in),
                },
            );

            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "user_code": resp.user_code,
                    "verification_uri": resp.verification_uri,
                    "poll_id": poll_id,
                    "expires_in": resp.expires_in,
                    "interval": resp.interval,
                })),
            )
        }
        Err(e) => api_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &rid,
            PATH,
            "Device flow start failed",
            e,
            None,
        ),
    }
}

/// GET /api/providers/github-copilot/oauth/poll/{poll_id}
///
/// Poll the status of a GitHub device flow.
/// Returns `pending`, `complete`, `expired`, `denied`, or `error`.
/// On `complete`, saves the token to secrets.env and sets GITHUB_TOKEN.
pub async fn copilot_oauth_poll(
    State(state): State<Arc<AppState>>,
    Path(poll_id): Path<String>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/providers/github-copilot/oauth/poll/:poll_id";
    let flow = match COPILOT_FLOWS.get(&poll_id) {
        Some(f) => f,
        None => {
            return api_json_error(
                StatusCode::NOT_FOUND,
                &rid,
                PATH,
                "Unknown poll_id",
                "No OAuth device flow matches this poll_id (expired or invalid).".to_string(),
                Some("Start a new flow via POST /api/providers/github-copilot/oauth/start."),
            );
        }
    };

    if flow.expires_at <= Instant::now() {
        drop(flow);
        COPILOT_FLOWS.remove(&poll_id);
        return (
            StatusCode::OK,
            Json(serde_json::json!({"status": "expired"})),
        );
    }

    let device_code = flow.device_code.clone();
    drop(flow);

    match openfang_runtime::copilot_oauth::poll_device_flow(&device_code).await {
        openfang_runtime::copilot_oauth::DeviceFlowStatus::Pending => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "pending"})),
        ),
        openfang_runtime::copilot_oauth::DeviceFlowStatus::Complete { access_token } => {
            // Store in vault (best-effort)
            state.kernel.store_credential("GITHUB_TOKEN", &access_token);

            // Save to secrets.env (dual-write)
            let secrets_path = state.kernel.config.home_dir.join("secrets.env");
            if let Err(e) = write_secret_env(&secrets_path, "GITHUB_TOKEN", &access_token) {
                return api_json_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &rid,
                    PATH,
                    "Failed to save token",
                    format!("{e}"),
                    Some("Check permissions on secrets.env under the config home directory."),
                );
            }

            // Set in current process
            std::env::set_var("GITHUB_TOKEN", access_token.as_str());

            // Refresh auth detection
            state
                .kernel
                .model_catalog
                .write()
                .unwrap_or_else(|e| e.into_inner())
                .detect_auth();

            // Clean up flow state
            COPILOT_FLOWS.remove(&poll_id);

            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "complete"})),
            )
        }
        openfang_runtime::copilot_oauth::DeviceFlowStatus::SlowDown { new_interval } => {
            // Update interval
            if let Some(mut f) = COPILOT_FLOWS.get_mut(&poll_id) {
                f.interval = new_interval;
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "pending", "interval": new_interval})),
            )
        }
        openfang_runtime::copilot_oauth::DeviceFlowStatus::Expired => {
            COPILOT_FLOWS.remove(&poll_id);
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "expired"})),
            )
        }
        openfang_runtime::copilot_oauth::DeviceFlowStatus::AccessDenied => {
            COPILOT_FLOWS.remove(&poll_id);
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "denied"})),
            )
        }
        openfang_runtime::copilot_oauth::DeviceFlowStatus::Error(e) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "error", "error": e})),
        ),
    }
}

// ---------------------------------------------------------------------------
// Agent Communication (Comms) endpoints
// ---------------------------------------------------------------------------

/// GET /api/comms/topology — Build agent topology graph from registry.
pub async fn comms_topology(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    use openfang_types::comms::{EdgeKind, TopoEdge, TopoNode, Topology};

    let agents = state.kernel.registry.list();

    let nodes: Vec<TopoNode> = agents
        .iter()
        .map(|e| TopoNode {
            id: e.id.to_string(),
            name: e.name.clone(),
            state: format!("{:?}", e.state),
            model: e.manifest.model.model.clone(),
        })
        .collect();

    let mut edges: Vec<TopoEdge> = Vec::new();

    // Parent-child edges from registry
    for agent in &agents {
        for child_id in &agent.children {
            edges.push(TopoEdge {
                from: agent.id.to_string(),
                to: child_id.to_string(),
                kind: EdgeKind::ParentChild,
            });
        }
    }

    // Peer message edges from event bus history
    let events = state.kernel.event_bus.history(500).await;
    let mut peer_pairs = std::collections::HashSet::new();
    for event in &events {
        if let openfang_types::event::EventPayload::Message(_) = &event.payload {
            if let openfang_types::event::EventTarget::Agent(target_id) = &event.target {
                let from = event.source.to_string();
                let to = target_id.to_string();
                // Deduplicate: only one edge per pair, skip self-loops
                if from != to {
                    let key = if from < to {
                        (from.clone(), to.clone())
                    } else {
                        (to.clone(), from.clone())
                    };
                    if peer_pairs.insert(key) {
                        edges.push(TopoEdge {
                            from,
                            to,
                            kind: EdgeKind::Peer,
                        });
                    }
                }
            }
        }
    }

    Json(serde_json::to_value(Topology { nodes, edges }).unwrap_or_default())
}

/// Filter a kernel event into a CommsEvent, if it represents inter-agent communication.
fn filter_to_comms_event(
    event: &openfang_types::event::Event,
    agents: &[openfang_types::agent::AgentEntry],
) -> Option<openfang_types::comms::CommsEvent> {
    use openfang_types::comms::{CommsEvent, CommsEventKind};
    use openfang_types::event::{EventPayload, EventTarget, LifecycleEvent};

    let resolve_name = |id: &str| -> String {
        agents
            .iter()
            .find(|a| a.id.to_string() == id)
            .map(|a| a.name.clone())
            .unwrap_or_else(|| id.to_string())
    };

    match &event.payload {
        EventPayload::Message(msg) => {
            let target_id = match &event.target {
                EventTarget::Agent(id) => id.to_string(),
                _ => String::new(),
            };
            Some(CommsEvent {
                id: event.id.to_string(),
                timestamp: event.timestamp.to_rfc3339(),
                kind: CommsEventKind::AgentMessage,
                source_id: event.source.to_string(),
                source_name: resolve_name(&event.source.to_string()),
                target_id: target_id.clone(),
                target_name: resolve_name(&target_id),
                detail: openfang_types::truncate_str(&msg.content, 200).to_string(),
            })
        }
        EventPayload::Lifecycle(lifecycle) => match lifecycle {
            LifecycleEvent::Spawned { agent_id, name } => Some(CommsEvent {
                id: event.id.to_string(),
                timestamp: event.timestamp.to_rfc3339(),
                kind: CommsEventKind::AgentSpawned,
                source_id: event.source.to_string(),
                source_name: resolve_name(&event.source.to_string()),
                target_id: agent_id.to_string(),
                target_name: name.clone(),
                detail: format!("Agent '{}' spawned", name),
            }),
            LifecycleEvent::Terminated { agent_id, reason } => Some(CommsEvent {
                id: event.id.to_string(),
                timestamp: event.timestamp.to_rfc3339(),
                kind: CommsEventKind::AgentTerminated,
                source_id: event.source.to_string(),
                source_name: resolve_name(&event.source.to_string()),
                target_id: agent_id.to_string(),
                target_name: resolve_name(&agent_id.to_string()),
                detail: format!("Terminated: {}", reason),
            }),
            _ => None,
        },
        EventPayload::System(openfang_types::event::SystemEvent::AgentActivity {
            phase,
            detail,
        }) => {
            let aid = event.source.to_string();
            let detail_str = match phase.as_str() {
                "thinking" => "Thinking…".to_string(),
                "tool_use" => format!("Using tool: {}", detail.as_deref().unwrap_or("tool")),
                "streaming" => "Writing response…".to_string(),
                "done" | "error" => return None,
                _ => {
                    if let Some(d) = detail {
                        format!("{phase}: {d}")
                    } else {
                        phase.clone()
                    }
                }
            };
            Some(CommsEvent {
                id: event.id.to_string(),
                timestamp: event.timestamp.to_rfc3339(),
                kind: CommsEventKind::AgentActivity,
                source_id: aid.clone(),
                source_name: resolve_name(&aid),
                target_id: String::new(),
                target_name: String::new(),
                detail: detail_str,
            })
        }
        _ => None,
    }
}

/// Convert an audit entry into a CommsEvent if it represents inter-agent activity.
fn audit_to_comms_event(
    entry: &openfang_runtime::audit::AuditEntry,
    agents: &[openfang_types::agent::AgentEntry],
) -> Option<openfang_types::comms::CommsEvent> {
    use openfang_types::comms::{CommsEvent, CommsEventKind};

    let resolve_name = |id: &str| -> String {
        agents
            .iter()
            .find(|a| a.id.to_string() == id)
            .map(|a| a.name.clone())
            .unwrap_or_else(|| {
                if id.is_empty() || id == "system" {
                    "system".to_string()
                } else {
                    openfang_types::truncate_str(id, 12).to_string()
                }
            })
    };

    let action_str = format!("{:?}", entry.action);
    let (kind, detail, target_label) = match action_str.as_str() {
        "AgentMessage" => {
            // Format detail: "tokens_in=X, tokens_out=Y" → readable summary
            let detail = if entry.detail.starts_with("tokens_in=") {
                let parts: Vec<&str> = entry.detail.split(", ").collect();
                let in_tok = parts
                    .first()
                    .and_then(|p| p.strip_prefix("tokens_in="))
                    .unwrap_or("?");
                let out_tok = parts
                    .get(1)
                    .and_then(|p| p.strip_prefix("tokens_out="))
                    .unwrap_or("?");
                if entry.outcome == "ok" {
                    format!("{} in / {} out tokens", in_tok, out_tok)
                } else {
                    format!(
                        "{} in / {} out — {}",
                        in_tok,
                        out_tok,
                        openfang_types::truncate_str(&entry.outcome, 80)
                    )
                }
            } else if entry.outcome != "ok" {
                format!(
                    "{} — {}",
                    openfang_types::truncate_str(&entry.detail, 80),
                    openfang_types::truncate_str(&entry.outcome, 80)
                )
            } else {
                openfang_types::truncate_str(&entry.detail, 200).to_string()
            };
            (CommsEventKind::AgentMessage, detail, "user")
        }
        "AgentSpawn" => (
            CommsEventKind::AgentSpawned,
            format!(
                "Agent spawned: {}",
                openfang_types::truncate_str(&entry.detail, 100)
            ),
            "",
        ),
        "AgentKill" => (
            CommsEventKind::AgentTerminated,
            format!(
                "Agent killed: {}",
                openfang_types::truncate_str(&entry.detail, 100)
            ),
            "",
        ),
        _ => return None,
    };

    Some(CommsEvent {
        id: format!("audit-{}", entry.seq),
        timestamp: entry.timestamp.clone(),
        kind,
        source_id: entry.agent_id.clone(),
        source_name: resolve_name(&entry.agent_id),
        target_id: if target_label.is_empty() {
            String::new()
        } else {
            target_label.to_string()
        },
        target_name: if target_label.is_empty() {
            String::new()
        } else {
            target_label.to_string()
        },
        detail,
    })
}

/// GET /api/comms/events — Return recent inter-agent communication events.
///
/// Sources from both the event bus (for lifecycle events with full context)
/// and the audit log (for message/spawn/kill events that are always captured).
pub async fn comms_events(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let limit = params
        .get("limit")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(100)
        .min(500);

    let agents = state.kernel.registry.list();

    // Primary source: event bus (has full source/target context)
    let bus_events = state.kernel.event_bus.history(500).await;
    let mut comms_events: Vec<openfang_types::comms::CommsEvent> = bus_events
        .iter()
        .filter_map(|e| filter_to_comms_event(e, &agents))
        .collect();

    // Secondary source: audit log (always populated, wider coverage)
    let audit_entries = state.kernel.audit_log.recent(500);
    let seen_ids: std::collections::HashSet<String> =
        comms_events.iter().map(|e| e.id.clone()).collect();

    for entry in audit_entries.iter().rev() {
        if let Some(ev) = audit_to_comms_event(entry, &agents) {
            if !seen_ids.contains(&ev.id) {
                comms_events.push(ev);
            }
        }
    }

    // Sort by timestamp descending (newest first)
    comms_events.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    comms_events.truncate(limit);

    Json(comms_events)
}

/// GET /api/comms/events/stream — SSE stream of inter-agent communication events.
///
/// Polls the audit log every 500ms for new inter-agent events.
pub async fn comms_events_stream(State(state): State<Arc<AppState>>) -> axum::response::Response {
    use axum::response::sse::{Event, KeepAlive, Sse};

    let (tx, rx) = tokio::sync::mpsc::channel::<
        Result<axum::response::sse::Event, std::convert::Infallible>,
    >(256);

    tokio::spawn(async move {
        let mut last_seq: u64 = {
            let entries = state.kernel.audit_log.recent(1);
            entries.last().map(|e| e.seq).unwrap_or(0)
        };

        loop {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;

            let agents = state.kernel.registry.list();
            let entries = state.kernel.audit_log.recent(50);

            for entry in &entries {
                if entry.seq <= last_seq {
                    continue;
                }
                if let Some(comms_event) = audit_to_comms_event(entry, &agents) {
                    let data = serde_json::to_string(&comms_event).unwrap_or_default();
                    if tx.send(Ok(Event::default().data(data))).await.is_err() {
                        return; // Client disconnected
                    }
                }
            }

            if let Some(last) = entries.last() {
                last_seq = last.seq;
            }
        }
    });

    let rx_stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    Sse::new(rx_stream)
        .keep_alive(
            KeepAlive::new()
                .interval(std::time::Duration::from_secs(15))
                .text("ping"),
        )
        .into_response()
}

/// POST /api/comms/send — Send a message from one agent to another.
pub async fn comms_send(
    State(state): State<Arc<AppState>>,
    ext: Option<Extension<RequestId>>,
    Json(req): Json<openfang_types::comms::CommsSendRequest>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/comms/send";
    // Validate from agent exists
    let from_id: openfang_types::agent::AgentId = match req.from_agent_id.parse() {
        Ok(id) => id,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid from_agent_id",
                "from_agent_id must be a valid agent UUID.".to_string(),
                None,
            );
        }
    };
    if state.kernel.registry.get(from_id).is_none() {
        return api_json_error(
            StatusCode::NOT_FOUND,
            &rid,
            PATH,
            "Source agent not found",
            format!(
                "No agent registered for from_agent_id {}.",
                req.from_agent_id
            ),
            Some("Use GET /api/agents to list agents."),
        );
    }

    // Validate to agent exists
    let to_id: openfang_types::agent::AgentId = match req.to_agent_id.parse() {
        Ok(id) => id,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid to_agent_id",
                "to_agent_id must be a valid agent UUID.".to_string(),
                None,
            );
        }
    };
    if state.kernel.registry.get(to_id).is_none() {
        return api_json_error(
            StatusCode::NOT_FOUND,
            &rid,
            PATH,
            "Target agent not found",
            format!("No agent registered for to_agent_id {}.", req.to_agent_id),
            Some("Use GET /api/agents to list agents."),
        );
    }

    // SECURITY: Limit message size
    if req.message.len() > 64 * 1024 {
        return api_json_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            &rid,
            PATH,
            "Message too large",
            "Message body exceeds max size (64KB).".to_string(),
            None,
        );
    }

    match state.kernel.send_message(to_id, &req.message).await {
        Ok(result) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "ok": true,
                "response": result.response,
                "input_tokens": result.total_usage.input_tokens,
                "output_tokens": result.total_usage.output_tokens,
            })),
        ),
        Err(e) => api_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &rid,
            PATH,
            "Message delivery failed",
            format!("{e}"),
            None,
        ),
    }
}

/// POST /api/comms/task — Post a task to the agent task queue.
pub async fn comms_task(
    State(state): State<Arc<AppState>>,
    ext: Option<Extension<RequestId>>,
    Json(req): Json<openfang_types::comms::CommsTaskRequest>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/comms/task";
    if req.title.is_empty() {
        return api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Title required",
            "Field `title` must be non-empty.".to_string(),
            None,
        );
    }

    let mut payload_val = req.payload.clone().unwrap_or(serde_json::json!({}));
    if let Some(tid) = &req.orchestration_trace_id {
        if let serde_json::Value::Object(ref mut obj) = payload_val {
            let mut orch = obj
                .get("orchestration")
                .cloned()
                .unwrap_or(serde_json::json!({}));
            if let Some(om) = orch.as_object_mut() {
                om.insert("trace_id".to_string(), serde_json::json!(tid));
            }
            obj.insert("orchestration".to_string(), orch);
        } else {
            payload_val = serde_json::json!({ "orchestration": { "trace_id": tid } });
        }
    }
    let payload_ref = if payload_val == serde_json::json!({}) {
        None
    } else {
        Some(&payload_val)
    };

    match state
        .kernel
        .memory
        .task_post(
            &req.title,
            &req.description,
            req.assigned_to.as_deref(),
            Some("ui-user"),
            payload_ref,
            req.priority,
        )
        .await
    {
        Ok(task_id) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "ok": true,
                "task_id": task_id,
            })),
        ),
        Err(e) => api_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &rid,
            PATH,
            "Failed to post task",
            format!("{e}"),
            None,
        ),
    }
}

// ── Dashboard Authentication (username/password sessions) ──

/// POST /api/auth/login — Authenticate with username/password, returns session token.
pub async fn auth_login(
    State(state): State<Arc<AppState>>,
    ext: Option<Extension<RequestId>>,
    Json(req): Json<serde_json::Value>,
) -> axum::response::Response {
    use axum::body::Body;
    use axum::response::Response;

    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/auth/login";

    let auth_cfg = &state.kernel.config.auth;
    if !auth_cfg.enabled {
        let (st, Json(body)) = api_json_error(
            StatusCode::NOT_FOUND,
            &rid,
            PATH,
            "Auth not enabled",
            "Dashboard username/password auth is disabled in config.".to_string(),
            None,
        );
        return Response::builder()
            .status(st)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
    }

    let username = req.get("username").and_then(|v| v.as_str()).unwrap_or("");
    let password = req.get("password").and_then(|v| v.as_str()).unwrap_or("");

    // Constant-time username comparison to prevent timing attacks
    let username_ok = {
        use subtle::ConstantTimeEq;
        let stored = auth_cfg.username.as_bytes();
        let provided = username.as_bytes();
        if stored.len() != provided.len() {
            false
        } else {
            bool::from(stored.ct_eq(provided))
        }
    };

    if !username_ok || !crate::session_auth::verify_password(password, &auth_cfg.password_hash) {
        // Audit log the failed attempt
        state.kernel.audit_log.record(
            "system",
            openfang_runtime::audit::AuditAction::AuthAttempt,
            "dashboard login failed",
            format!("username: {username}"),
        );
        let (st, Json(body)) = api_json_error(
            StatusCode::UNAUTHORIZED,
            &rid,
            PATH,
            "Invalid credentials",
            "Username or password did not match.".to_string(),
            None,
        );
        return Response::builder()
            .status(st)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
    }

    // Derive the session secret the same way as server.rs
    let api_key = state.kernel.config.api_key.trim().to_string();
    let secret = if !api_key.is_empty() {
        api_key
    } else {
        auth_cfg.password_hash.clone()
    };

    let token =
        crate::session_auth::create_session_token(username, &secret, auth_cfg.session_ttl_hours);
    let ttl_secs = auth_cfg.session_ttl_hours * 3600;
    let cookie =
        format!("openfang_session={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={ttl_secs}");

    state.kernel.audit_log.record(
        "system",
        openfang_runtime::audit::AuditAction::AuthAttempt,
        "dashboard login success",
        format!("username: {username}"),
    );

    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/json")
        .header("set-cookie", &cookie)
        .body(Body::from(
            serde_json::json!({
                "status": "ok",
                "token": token,
                "username": username,
            })
            .to_string(),
        ))
        .unwrap()
}

/// POST /api/auth/logout — Clear the session cookie.
pub async fn auth_logout() -> impl IntoResponse {
    let cookie = "openfang_session=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0";
    (
        StatusCode::OK,
        [("content-type", "application/json"), ("set-cookie", cookie)],
        serde_json::json!({"status": "ok"}).to_string(),
    )
}

/// GET /api/auth/check — Check current authentication state.
pub async fn auth_check(
    State(state): State<Arc<AppState>>,
    request: axum::http::Request<axum::body::Body>,
) -> impl IntoResponse {
    let auth_cfg = &state.kernel.config.auth;
    if !auth_cfg.enabled {
        return Json(serde_json::json!({
            "authenticated": true,
            "mode": "none",
        }));
    }

    // Derive the session secret the same way as server.rs
    let api_key = state.kernel.config.api_key.trim().to_string();
    let secret = if !api_key.is_empty() {
        api_key
    } else {
        auth_cfg.password_hash.clone()
    };

    // Check session cookie
    let session_user = request
        .headers()
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(|cookies| {
            cookies.split(';').find_map(|c| {
                c.trim()
                    .strip_prefix("openfang_session=")
                    .map(|v| v.to_string())
            })
        })
        .and_then(|token| crate::session_auth::verify_session_token(&token, &secret));

    if let Some(username) = session_user {
        Json(serde_json::json!({
            "authenticated": true,
            "mode": "session",
            "username": username,
        }))
    } else {
        Json(serde_json::json!({
            "authenticated": false,
            "mode": "session",
        }))
    }
}

/// Remove a `[section]` and its contents from a TOML string.
#[allow(dead_code)]
fn backup_config(config_path: &std::path::Path) {
    let backup = config_path.with_extension("toml.bak");
    let _ = std::fs::copy(config_path, backup);
}

fn remove_toml_section(content: &str, section: &str) -> String {
    let header = format!("[{}]", section);
    let mut result = String::new();
    let mut skipping = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == header {
            skipping = true;
            continue;
        }
        if skipping && trimmed.starts_with('[') {
            skipping = false;
        }
        if !skipping {
            result.push_str(line);
            result.push('\n');
        }
    }
    result
}

// ---------------------------------------------------------------------------
// Orchestration traces & quota tree
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Deserialize)]
pub struct OrchestrationTracesQuery {
    #[serde(default)]
    pub limit: Option<u32>,
}

/// GET /api/orchestration/traces — Recent orchestration traces (summaries).
pub async fn list_orchestration_traces(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(q): axum::extract::Query<OrchestrationTracesQuery>,
) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(50).clamp(1, 200) as usize;
    let summaries = state.kernel.orchestration_traces.list_summaries(limit);
    Json(summaries)
}

/// GET /api/orchestration/traces/:trace_id — All events for one trace.
pub async fn get_orchestration_trace(
    State(state): State<Arc<AppState>>,
    Path(trace_id): Path<String>,
) -> impl IntoResponse {
    let events = state
        .kernel
        .orchestration_traces
        .events_for_trace(&trace_id);
    if events.is_empty() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "trace not found"})),
        )
            .into_response();
    }
    Json(events).into_response()
}

/// GET /api/orchestration/traces/:trace_id/live — Best-effort shared_vars + budget snapshot (in-process).
pub async fn get_orchestration_trace_live(
    State(state): State<Arc<AppState>>,
    Path(trace_id): Path<String>,
) -> impl IntoResponse {
    match state.kernel.orchestration_trace_live.get(&trace_id) {
        Some(v) => Json(v.value().clone()).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "no live snapshot for this trace"})),
        )
            .into_response(),
    }
}

/// GET /api/orchestration/traces/:trace_id/tree — Reconstructed delegation tree.
pub async fn get_orchestration_trace_tree(
    State(state): State<Arc<AppState>>,
    Path(trace_id): Path<String>,
) -> impl IntoResponse {
    match state.kernel.orchestration_traces.trace_tree(&trace_id) {
        Some(tree) => Json(tree).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "trace not found"})),
        )
            .into_response(),
    }
}

/// GET /api/orchestration/traces/:trace_id/cost — Token/cost rollup from completed steps.
pub async fn get_orchestration_trace_cost(
    State(state): State<Arc<AppState>>,
    Path(trace_id): Path<String>,
) -> impl IntoResponse {
    match state.kernel.orchestration_traces.trace_cost(&trace_id) {
        Some(cost) => Json(cost).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "trace not found"})),
        )
            .into_response(),
    }
}

/// GET /api/orchestration/quota-tree/:agent_id — Quota + usage for an agent and descendants.
pub async fn get_orchestration_quota_tree(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
) -> impl IntoResponse {
    let id: openfang_types::agent::AgentId = match agent_id.parse() {
        Ok(id) => id,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "invalid agent id"})),
            )
                .into_response();
        }
    };
    match state.kernel.orchestration_quota_tree(id) {
        Some(tree) => Json(tree).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "agent not found"})),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// AINL library (synced programs under ~/.armaraos/ainl-library)
// ---------------------------------------------------------------------------

const PYPI_AINATIVELANG_JSON: &str = "https://pypi.org/pypi/ainativelang/json";

/// GET /api/ainl/runtime-version — Host `ainl` CLI + `pip show ainativelang` and PyPI latest (best-effort).
///
/// Always returns HTTP 200 when the subprocess probe runs; PyPI failures populate `pypi_error`.
pub async fn get_ainl_runtime_version() -> impl IntoResponse {
    let probe = match tokio::task::spawn_blocking(|| {
        openfang_runtime::host_ainl_snapshot::probe_host_ainl_toolchain()
    })
    .await
    {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("join: {e}") })),
            )
                .into_response();
        }
    };

    let client = match reqwest::Client::builder()
        .user_agent(concat!(
            "ArmaraOS/",
            env!("CARGO_PKG_VERSION"),
            " (daemon; +https://github.com/sbhooley/armaraos)"
        ))
        .timeout(std::time::Duration::from_secs(20))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("HTTP client: {e}") })),
            )
                .into_response();
        }
    };

    let mut pypi_latest_version: Option<String> = None;
    let mut pypi_error: Option<String> = None;
    match client.get(PYPI_AINATIVELANG_JSON).send().await {
        Ok(resp) => {
            let status = resp.status();
            if !status.is_success() {
                pypi_error = Some(format!("PyPI HTTP {}", status.as_u16()));
            } else {
                match resp.json::<serde_json::Value>().await {
                    Ok(v) => {
                        pypi_latest_version = v["info"]["version"]
                            .as_str()
                            .map(std::string::ToString::to_string);
                        if pypi_latest_version.is_none() {
                            pypi_error = Some("PyPI response missing info.version".into());
                        }
                    }
                    Err(e) => pypi_error = Some(format!("PyPI JSON: {e}")),
                }
            }
        }
        Err(e) => pypi_error = Some(format!("PyPI request: {e}")),
    }

    Json(serde_json::json!({
        "ainl_cli_line": probe.ainl_cli_line,
        "pip_version": probe.pip_version,
        "pip_excerpt": probe.pip_excerpt,
        "pypi_latest_version": pypi_latest_version,
        "pypi_error": pypi_error,
    }))
    .into_response()
}

fn ainl_library_query_hints_enabled(params: &HashMap<String, String>) -> bool {
    params.get("hints").is_some_and(|s| {
        let t = s.trim();
        t == "1" || t.eq_ignore_ascii_case("true") || t.eq_ignore_ascii_case("yes")
    })
}

fn ainl_library_query_max_hints(params: &HashMap<String, String>) -> usize {
    params
        .get("max_hints")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(500)
        .clamp(1, 2000)
}

/// GET /api/ainl/library — List discovered `.ainl` / `.lang` files (grouped + flat).
///
/// Query: `hints=1` — include first `#` comment line per file (capped by `max_hints`, default 500).
pub async fn get_ainl_library(
    Query(params): Query<HashMap<String, String>>,
    State(state): State<Arc<AppState>>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/ainl/library";
    let home = &state.kernel.config.home_dir;
    let root = home.join("ainl-library");
    let root_display = root.display().to_string();
    let sync_meta = openfang_kernel::ainl_library::read_ainl_library_sync_metadata(home);
    let want_hints = ainl_library_query_hints_enabled(&params);
    let max_hints = ainl_library_query_max_hints(&params);
    match openfang_kernel::ainl_library::walk_ainl_files(&root) {
        Ok(files) => {
            let hints_truncated = want_hints && files.len() > max_hints;
            let mut by_cat: BTreeMap<String, Vec<serde_json::Value>> = BTreeMap::new();
            let programs: Vec<serde_json::Value> = files
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let absolute = p.display().to_string();
                    let rel = p.strip_prefix(&root).unwrap_or(p.as_path());
                    let rel_s = rel.to_string_lossy().replace('\\', "/");
                    let name = p
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or(&rel_s)
                        .to_string();
                    let cat_key = rel_s
                        .split('/')
                        .next()
                        .filter(|s| !s.is_empty())
                        .unwrap_or("other")
                        .to_string();
                    let hint = if want_hints && i < max_hints {
                        openfang_kernel::ainl_library::ainl_source_first_hint(p)
                    } else {
                        None
                    };
                    let entry = if let Some(h) = hint {
                        serde_json::json!({
                            "path": rel_s,
                            "absolute": absolute,
                            "name": name,
                            "category": cat_key,
                            "hint": h,
                        })
                    } else {
                        serde_json::json!({
                            "path": rel_s,
                            "absolute": absolute,
                            "name": name,
                            "category": cat_key,
                        })
                    };
                    by_cat
                        .entry(cat_key.clone())
                        .or_default()
                        .push(entry.clone());
                    entry
                })
                .collect();

            let priority = ["armaraos-programs", "demo", "examples", "intelligence"];
            let mut categories: Vec<serde_json::Value> = Vec::new();
            for key in priority {
                if let Some(items) = by_cat.remove(key) {
                    categories.push(serde_json::json!({
                        "id": key,
                        "label": match key {
                            "armaraos-programs" => "ArmaraOS programs",
                            "demo" => "Demo",
                            "examples" => "Examples",
                            "intelligence" => "Intelligence",
                            _ => key,
                        },
                        "programs": items,
                    }));
                }
            }
            for (id, items) in by_cat {
                categories.push(serde_json::json!({
                    "id": id,
                    "label": id,
                    "programs": items,
                }));
            }

            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "root": root_display,
                    "total": programs.len(),
                    "programs": programs,
                    "categories": categories,
                    "sync": sync_meta,
                    "library_present": root.is_dir(),
                    "hints_enabled": want_hints,
                    "hints_truncated": hints_truncated,
                    "max_hints_applied": want_hints.then_some(max_hints),
                })),
            )
        }
        Err(e) => api_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &rid,
            PATH,
            "Library scan failed",
            e,
            None,
        ),
    }
}

/// GET /api/ainl/library/curated — Static catalog used for optional cron registration.
pub async fn get_ainl_library_curated(ext: Option<Extension<RequestId>>) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/ainl/library/curated";
    match serde_json::from_str::<serde_json::Value>(
        openfang_kernel::ainl_library::CURATED_AINL_CRON_JSON,
    ) {
        Ok(v) => (StatusCode::OK, Json(v)),
        Err(e) => api_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &rid,
            PATH,
            "Embedded catalog invalid",
            format!("embedded catalog: {e}"),
            None,
        ),
    }
}

/// POST /api/ainl/library/register-curated — Re-run idempotent curated cron registration.
///
/// Per-IP sliding window (5 calls / 60s) in addition to GCRA rate limiting.
pub async fn post_ainl_register_curated(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<Arc<AppState>>,
    ext: Option<Extension<RequestId>>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/ainl/library/register-curated";
    const WINDOW_MS: u64 = 60_000;
    const MAX_PER_WINDOW: usize = 5;
    let ip = addr.ip();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    match state.ainl_register_hits.entry(ip) {
        Entry::Occupied(mut o) => {
            let v = o.get_mut();
            v.retain(|t| now.saturating_sub(*t) < WINDOW_MS);
            if v.len() >= MAX_PER_WINDOW {
                return api_json_error(
                    StatusCode::TOO_MANY_REQUESTS,
                    &rid,
                    PATH,
                    "Rate limited",
                    "Too many register-curated requests from this IP; try again in about a minute."
                        .to_string(),
                    None,
                );
            }
            v.push(now);
        }
        Entry::Vacant(v) => {
            v.insert(vec![now]);
        }
    }

    let embedded_written =
        match openfang_kernel::embedded_ainl_programs::materialize_embedded_programs(
            &state.kernel.config.home_dir,
        ) {
            Ok(n) => n,
            Err(e) => {
                tracing::warn!("AINL embedded programs materialize (register-curated): {e}");
                0
            }
        };
    if let Err(e) = openfang_kernel::embedded_ainl_programs::ensure_ainl_library_pointer_files(
        &state.kernel.config.home_dir,
    ) {
        tracing::warn!("AINL library pointer files (register-curated): {e}");
    }

    let (cron_reassigned, cron_deduped, cron_ainl_deduped) =
        state.kernel.reconcile_persisted_cron_jobs();

    match openfang_kernel::ainl_library::register_curated_ainl_cron_jobs(&state.kernel) {
        Ok(curation) => {
            let _ = state.kernel.cron_scheduler.persist();
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "registered": curation.added,
                    "updated": curation.updated,
                    "pruned": curation.pruned,
                    "cron_reassigned": cron_reassigned,
                    "cron_deduped": cron_deduped,
                    "cron_ainl_deduped": cron_ainl_deduped,
                    "embedded_programs_written": embedded_written,
                })),
            )
        }
        Err(e) => api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Registration failed",
            e,
            None,
        ),
    }
}

// ---------------------------------------------------------------------------
// Learning / skill drafts
// ---------------------------------------------------------------------------

/// POST /api/learning/skill-draft — Write `~/.armaraos/skills/staging/draft-<run_id>-<unix>.md` from [`LearningFrameV1`].
pub async fn post_learning_skill_draft(
    State(state): State<Arc<AppState>>,
    ext: Option<Extension<RequestId>>,
    Json(frame): Json<openfang_types::learning_frame::LearningFrameV1>,
) -> impl IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/learning/skill-draft";
    match openfang_kernel::skills_staging::write_skill_draft_markdown(
        &state.kernel.config.home_dir,
        &frame,
    ) {
        Ok(path) => {
            let display = path.display().to_string();
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "written",
                    "path": display,
                })),
            )
        }
        Err(e) => api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Draft write failed",
            e,
            None,
        ),
    }
}

#[cfg(test)]
mod channel_config_tests {
    use super::*;

    #[test]
    fn test_is_channel_configured_wecom_none() {
        let config = openfang_types::config::ChannelsConfig::default();
        assert!(!is_channel_configured(&config, "wecom"));
    }

    #[test]
    fn test_is_channel_configured_wecom_some() {
        let mut config = openfang_types::config::ChannelsConfig::default();
        config.wecom = Some(openfang_types::config::WeComConfig {
            corp_id: "test_corp".to_string(),
            agent_id: "test_agent".to_string(),
            secret_env: "WECOM_SECRET".to_string(),
            webhook_port: 8454,
            token: Some("token".to_string()),
            encoding_aes_key: Some("aes_key".to_string()),
            default_agent: Some("assistant".to_string()),
            overrides: openfang_types::config::ChannelOverrides::default(),
        });
        assert!(is_channel_configured(&config, "wecom"));
    }

    #[test]
    fn test_wecom_in_channel_registry() {
        let wecom_meta = CHANNEL_REGISTRY.iter().find(|c| c.name == "wecom");
        assert!(wecom_meta.is_some());
        let meta = wecom_meta.unwrap();
        assert_eq!(meta.display_name, "WeCom");
        assert_eq!(meta.category, "messaging");
        assert!(
            meta.fields
                .iter()
                .find(|f| f.key == "corp_id")
                .unwrap()
                .required
        );
        assert!(
            meta.fields
                .iter()
                .find(|f| f.key == "secret_env")
                .unwrap()
                .required
        );
    }
}
