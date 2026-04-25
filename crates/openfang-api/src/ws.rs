//! WebSocket handler for real-time agent chat.
//!
//! Provides a persistent bidirectional channel between the client
//! and an agent. Messages are exchanged as JSON:
//!
//! Client → Server: `{"type":"message","content":"..."}`
//! Server → Client: `{"type":"typing","state":"start|tool|stop"}`
//! Server → Client: `{"type":"text_delta","content":"..."}`
//! Server → Client: `{"type":"response","content":"...","input_tokens":N,"output_tokens":N,"iterations":N,"turn_wall_ms":N,"skill_draft_path":?}`
//! Server → Client: `{"type":"error","content":"..."}`
//! Server → Client: `{"type":"agents_updated","agents":[...]}`
//! Server → Client: `{"type":"silent_complete",...,"turn_outcome":"user_silent",...}` (intentional NO_REPLY / `[[silent]]`; not used for tool-only turns with no streamed text)
//! Server → Client: `{"type":"response",...,"turn_outcome":"completed"|...}` — same taxonomy as `POST /api/agents/:id/message`
//! Server → Client: `{"type":"canvas","canvas_id":"...","html":"...","title":"..."}`

use crate::middleware::RequestId;
use crate::routes::AppState;
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{ConnectInfo, Path, State, WebSocketUpgrade};
use axum::http::{HeaderName, HeaderValue};
use axum::response::IntoResponse;
use dashmap::DashMap;
use futures::stream::SplitSink;
use futures::{SinkExt, StreamExt};
use openfang_runtime::kernel_handle::KernelHandle;
use openfang_runtime::llm_driver::StreamEvent;
use openfang_runtime::llm_errors;
use openfang_types::agent::AgentId;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

/// When the user message has the `[learn]` prefix, write a skill draft (same as HTTP `POST .../message`).
fn skill_draft_path_for_learn_message(
    home: &std::path::Path,
    user_message: &str,
    assistant_response: &str,
    agent_id: &str,
    silent: bool,
) -> Option<String> {
    let intent = openfang_kernel::skills_staging::learn_prefixed_intent(user_message)?;
    let frame = openfang_kernel::skills_staging::frame_from_agent_learn_turn(
        intent,
        user_message,
        assistant_response,
        agent_id,
        silent,
    );
    match openfang_kernel::skills_staging::write_skill_draft_markdown(home, &frame) {
        Ok(p) => {
            info!(
                path = %p.display(),
                agent = %agent_id,
                "skill draft from [learn] message (ws)"
            );
            Some(p.display().to_string())
        }
        Err(e) => {
            warn!(
                error = %e,
                agent = %agent_id,
                "skill draft write failed (ws)"
            );
            None
        }
    }
}

/// Per-IP WebSocket connection tracker.
/// Max 5 concurrent WS connections per IP address.
const MAX_WS_PER_IP: usize = 5;

/// Idle timeout: close WS after 30 minutes of no client messages.
const WS_IDLE_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// Text delta debounce interval.
const DEBOUNCE_MS: u64 = 100;

/// Flush text buffer when it exceeds this many characters.
const DEBOUNCE_CHARS: usize = 200;

// ---------------------------------------------------------------------------
// Verbose Level
// ---------------------------------------------------------------------------

/// Per-connection tool detail verbosity.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(u8)]
enum VerboseLevel {
    /// Suppress tool details (only tool name + success/fail).
    Off = 0,
    /// Truncated tool details.
    On = 1,
    /// Full tool details (default).
    Full = 2,
}

impl VerboseLevel {
    fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Off,
            1 => Self::On,
            _ => Self::Full,
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Off => Self::On,
            Self::On => Self::Full,
            Self::Full => Self::Off,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::On => "on",
            Self::Full => "full",
        }
    }
}

// ---------------------------------------------------------------------------
// Connection Tracking
// ---------------------------------------------------------------------------

/// Global connection tracker (DashMap<IpAddr, AtomicUsize>).
fn ws_tracker() -> &'static DashMap<IpAddr, AtomicUsize> {
    static TRACKER: std::sync::OnceLock<DashMap<IpAddr, AtomicUsize>> = std::sync::OnceLock::new();
    TRACKER.get_or_init(DashMap::new)
}

/// RAII guard that decrements the connection count on drop.
struct WsConnectionGuard {
    ip: IpAddr,
}

impl Drop for WsConnectionGuard {
    fn drop(&mut self) {
        if let Some(entry) = ws_tracker().get(&self.ip) {
            let prev = entry.value().fetch_sub(1, Ordering::Relaxed);
            if prev <= 1 {
                drop(entry);
                ws_tracker().remove(&self.ip);
            }
        }
    }
}

/// Try to acquire a WS connection slot for the given IP.
/// Returns None if the IP has reached MAX_WS_PER_IP.
fn try_acquire_ws_slot(ip: IpAddr) -> Option<WsConnectionGuard> {
    let entry = ws_tracker()
        .entry(ip)
        .or_insert_with(|| AtomicUsize::new(0));
    let current = entry.value().fetch_add(1, Ordering::Relaxed);
    if current >= MAX_WS_PER_IP {
        entry.value().fetch_sub(1, Ordering::Relaxed);
        return None;
    }
    Some(WsConnectionGuard { ip })
}

/// Returns `true` when the WebSocket upgrade should proceed from an API-key perspective.
///
/// When `api_key_trimmed` is empty (after trim), all clients are allowed (local dev mode).
/// Otherwise the client must present `Authorization: Bearer <key>` or `?token=<key>`.
pub(crate) fn ws_upgrade_api_key_allowed(
    api_key_trimmed: &str,
    headers: &axum::http::HeaderMap,
    uri: &axum::http::Uri,
) -> bool {
    if api_key_trimmed.is_empty() {
        return true;
    }
    let ct_eq = |token: &str, key: &str| -> bool {
        use subtle::ConstantTimeEq;
        if token.len() != key.len() {
            return false;
        }
        token.as_bytes().ct_eq(key.as_bytes()).into()
    };

    let header_auth = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|token| ct_eq(token, api_key_trimmed))
        .unwrap_or(false);

    let query_auth = uri
        .query()
        .and_then(|q| q.split('&').find_map(|pair| pair.strip_prefix("token=")))
        .map(|token| ct_eq(token, api_key_trimmed))
        .unwrap_or(false);

    header_auth || query_auth
}

// ---------------------------------------------------------------------------
// WS Upgrade Handler
// ---------------------------------------------------------------------------

/// GET /api/agents/:id/ws — Upgrade to WebSocket for real-time chat.
///
/// SECURITY: Authenticates via Bearer token in Authorization header
/// or `?token=` query parameter (for browser WebSocket clients that
/// cannot set custom headers).
pub async fn agent_ws(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
    uri: axum::http::Uri,
) -> impl IntoResponse {
    // SECURITY: Authenticate WebSocket upgrades (bypasses middleware).
    // Trim whitespace so empty/whitespace-only api_key disables auth.
    let api_key_raw = &state.kernel.config.api_key;
    let api_key = api_key_raw.trim();
    if !ws_upgrade_api_key_allowed(api_key, &headers, &uri) {
        warn!("WebSocket upgrade rejected: invalid auth");
        return axum::http::StatusCode::UNAUTHORIZED.into_response();
    }

    // Wallet Premium: same SPL minimum as HTTP when a premium session token is present (header or `?premium_ainl=`).
    let mut premium_headers = headers.clone();
    if let Some(tok) =
        crate::premium_ainl::premium_token_from_headers_or_query(&headers, Some(&uri))
    {
        if let Ok(val) = HeaderValue::from_str(&tok) {
            premium_headers.insert(HeaderName::from_static("x-armaraos-premium-ainl"), val);
        }
    }
    let rid = RequestId("ws".to_string());
    if let Err(resp) = crate::premium_ainl::require_premium_wallet_holdings_when_wallet_session(
        &state,
        &premium_headers,
        &rid,
        "/api/agents/:id/ws",
    )
    .await
    {
        return resp.into_response();
    }

    // SECURITY: Enforce per-IP WebSocket connection limit
    let ip = addr.ip();

    let guard = match try_acquire_ws_slot(ip) {
        Some(g) => g,
        None => {
            warn!(ip = %ip, "WebSocket rejected: too many connections from IP (max {MAX_WS_PER_IP})");
            return axum::http::StatusCode::TOO_MANY_REQUESTS.into_response();
        }
    };

    let agent_id: AgentId = match id.parse() {
        Ok(id) => id,
        Err(_) => {
            return axum::http::StatusCode::BAD_REQUEST.into_response();
        }
    };

    // Verify agent exists
    if state.kernel.registry.get(agent_id).is_none() {
        return axum::http::StatusCode::NOT_FOUND.into_response();
    }

    let id_str = id.clone();
    ws.on_upgrade(move |socket| handle_agent_ws(socket, state, agent_id, id_str, guard))
        .into_response()
}

// ---------------------------------------------------------------------------
// WS Connection Handler
// ---------------------------------------------------------------------------

/// Handle a WebSocket connection to an agent.
///
/// The `_guard` is an RAII handle that decrements the per-IP connection
/// counter when this function returns (connection closes).
async fn handle_agent_ws(
    socket: WebSocket,
    state: Arc<AppState>,
    agent_id: AgentId,
    id_str: String,
    _guard: WsConnectionGuard,
) {
    info!(agent_id = %id_str, "WebSocket connected");

    let (sender, mut receiver) = socket.split();
    let sender = Arc::new(Mutex::new(sender));

    // Per-connection verbose level (default: Full)
    let verbose = Arc::new(AtomicU8::new(VerboseLevel::Full as u8));

    // Send initial connection confirmation
    let _ = send_json(
        &sender,
        &serde_json::json!({
            "type": "connected",
            "agent_id": id_str,
        }),
    )
    .await;

    // Spawn background task: periodic agent list updates with change detection
    let sender_clone = Arc::clone(&sender);
    let state_clone = Arc::clone(&state);
    let update_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        let mut last_hash: u64 = 0;
        loop {
            interval.tick().await;
            let agents: Vec<serde_json::Value> = state_clone
                .kernel
                .registry
                .list()
                .into_iter()
                .map(|e| {
                    serde_json::json!({
                        "id": e.id.to_string(),
                        "name": e.name,
                        "state": format!("{:?}", e.state),
                        "model_provider": e.manifest.model.provider,
                        "model_name": e.manifest.model.model,
                    })
                })
                .collect();

            // Change detection: hash the agent list and only send on change
            let mut hasher = DefaultHasher::new();
            for a in &agents {
                serde_json::to_string(a)
                    .unwrap_or_default()
                    .hash(&mut hasher);
            }
            let new_hash = hasher.finish();
            if new_hash == last_hash {
                continue; // No change — skip broadcast
            }
            last_hash = new_hash;

            if send_json(
                &sender_clone,
                &serde_json::json!({
                    "type": "agents_updated",
                    "agents": agents,
                }),
            )
            .await
            .is_err()
            {
                break; // Client disconnected
            }
        }
    });

    // Per-connection rate limiting: max 10 messages per 60 seconds
    let mut msg_times: Vec<std::time::Instant> = Vec::new();
    const MAX_PER_MIN: usize = 10;
    const WINDOW: Duration = Duration::from_secs(60);

    // Track last activity for idle timeout
    let mut last_activity = std::time::Instant::now();

    // Main message loop with idle timeout
    loop {
        let msg = tokio::select! {
            msg = receiver.next() => {
                match msg {
                    Some(m) => m,
                    None => break, // Stream ended
                }
            }
            _ = tokio::time::sleep(WS_IDLE_TIMEOUT.saturating_sub(last_activity.elapsed())) => {
                info!(agent_id = %id_str, "WebSocket idle timeout (30 min)");
                let _ = send_json(
                    &sender,
                    &serde_json::json!({
                        "type": "error",
                        "content": "Connection closed due to inactivity (30 min timeout)",
                    }),
                ).await;
                break;
            }
        };

        let msg = match msg {
            Ok(m) => m,
            Err(e) => {
                debug!(error = %e, "WebSocket receive error");
                break;
            }
        };

        match msg {
            Message::Text(text) => {
                last_activity = std::time::Instant::now();

                // SECURITY: Reject oversized WebSocket messages (64KB max)
                const MAX_WS_MSG_SIZE: usize = 64 * 1024;
                if text.len() > MAX_WS_MSG_SIZE {
                    let _ = send_json(
                        &sender,
                        &serde_json::json!({
                            "type": "error",
                            "content": "Message too large (max 64KB)",
                        }),
                    )
                    .await;
                    continue;
                }

                // SECURITY: Per-connection rate limiting
                let now = std::time::Instant::now();
                msg_times.retain(|t| now.duration_since(*t) < WINDOW);
                if msg_times.len() >= MAX_PER_MIN {
                    let _ = send_json(
                        &sender,
                        &serde_json::json!({
                            "type": "error",
                            "content": "Rate limit exceeded. Max 10 messages per minute.",
                        }),
                    )
                    .await;
                    continue;
                }
                msg_times.push(now);

                handle_text_message(&sender, &state, agent_id, &text, &verbose).await;
            }
            Message::Close(_) => {
                info!(agent_id = %id_str, "WebSocket closed by client");
                break;
            }
            Message::Ping(data) => {
                last_activity = std::time::Instant::now();
                let mut s = sender.lock().await;
                let _ = s.send(Message::Pong(data)).await;
            }
            _ => {} // Ignore binary and pong
        }
    }

    // Cleanup
    update_handle.abort();
    info!(agent_id = %id_str, "WebSocket disconnected");
}

// ---------------------------------------------------------------------------
// Message Handler
// ---------------------------------------------------------------------------

/// Handle a text message from the WebSocket client.
async fn handle_text_message(
    sender: &Arc<Mutex<SplitSink<WebSocket, Message>>>,
    state: &Arc<AppState>,
    agent_id: AgentId,
    text: &str,
    verbose: &Arc<AtomicU8>,
) {
    // Parse the message
    let parsed: serde_json::Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => {
            // Treat plain text as a message
            serde_json::json!({"type": "message", "content": text})
        }
    };

    let msg_type = parsed["type"].as_str().unwrap_or("message");

    match msg_type {
        "message" => {
            let raw_content = match parsed["content"].as_str() {
                Some(c) if !c.trim().is_empty() => c.to_string(),
                _ => {
                    let _ = send_json(
                        sender,
                        &serde_json::json!({
                            "type": "error",
                            "content": "Missing or empty 'content' field",
                        }),
                    )
                    .await;
                    return;
                }
            };

            // Attachment refs (used for vision blocks + workspace file materialization)
            let attachment_refs: Vec<crate::types::AttachmentRef> = parsed["attachments"]
                .as_array()
                .map(|attachments| {
                    attachments
                        .iter()
                        .filter_map(|a| serde_json::from_value(a.clone()).ok())
                        .collect()
                })
                .unwrap_or_default();

            let voice_reply = parsed["voice_reply"].as_bool().unwrap_or(false);

            // Sanitize inbound user input, then prepend voice `file_id` / MIME so the model sees
            // them at the start of the user turn (same as non-WS `routes` message path).
            let user_sanitized = sanitize_user_input(&raw_content);
            if user_sanitized.is_empty() {
                let _ = send_json(
                    sender,
                    &serde_json::json!({
                        "type": "error",
                        "content": "Message content is empty after sanitization",
                    }),
                )
                .await;
                return;
            }
            let base_user = crate::routes::prepare_user_message_for_audio_turn(
                state.kernel.as_ref(),
                &user_sanitized,
                &attachment_refs,
            )
            .await;
            let lead = crate::routes::audio_upload_ingress_lead(&attachment_refs);
            let mut content = if lead.is_empty() {
                base_user
            } else {
                format!("{lead}\n\n{base_user}")
            };

            if !attachment_refs.is_empty() {
                if let Some(entry) = state.kernel.registry.get(agent_id) {
                    if let Some(ref ws) = entry.manifest.workspace {
                        let hint =
                            crate::routes::workspace_upload_hints(ws.as_path(), &attachment_refs);
                        if !hint.is_empty() {
                            content.push_str(&hint);
                        }
                    }
                }
                let audio_hint = crate::routes::audio_upload_tool_hints(&attachment_refs);
                if !audio_hint.is_empty() {
                    content.push_str(&audio_hint);
                }
                if let Some(policy) =
                    crate::routes::voice_transcript_policy_suffix(&attachment_refs)
                {
                    content.push_str(policy);
                }
            }

            // Resolve file attachments into image content blocks
            let mut has_images = false;
            let mut ws_content_blocks: Option<Vec<openfang_types::message::ContentBlock>> = None;
            if !attachment_refs.is_empty() {
                let image_blocks = crate::routes::resolve_attachments(&attachment_refs);
                if !image_blocks.is_empty() {
                    has_images = true;
                    ws_content_blocks = Some(image_blocks);
                }
            }

            // Warn if the model doesn't support vision but images were attached
            if has_images {
                let model_name = state
                    .kernel
                    .registry
                    .get(agent_id)
                    .map(|e| e.manifest.model.model.clone())
                    .unwrap_or_default();
                let supports_vision = state
                    .kernel
                    .model_catalog
                    .read()
                    .ok()
                    .and_then(|cat| cat.find_model(&model_name).map(|m| m.supports_vision))
                    .unwrap_or(false);
                if !supports_vision {
                    let _ = send_json(
                        sender,
                        &serde_json::json!({
                            "type": "command_result",
                            "message": format!(
                                "**Vision not supported** — the current model `{}` cannot analyze images. \
                                 Switch to a vision-capable model (e.g. `gemini-2.5-flash`, `claude-sonnet-4-20250514`, `gpt-4o`) \
                                 with `/model <name>` for image analysis.",
                                model_name
                            ),
                        }),
                    )
                    .await;
                }
            }

            // Send typing lifecycle: start
            let _ = send_json(
                sender,
                &serde_json::json!({
                    "type": "typing",
                    "state": "start",
                }),
            )
            .await;

            // Wall time for the full streamed turn (LLM + tools until stream closes) — shown in chat telemetry.
            let turn_wall_t0 = std::time::Instant::now();

            // Send message to agent with streaming
            let kernel_handle: Arc<dyn KernelHandle> =
                state.kernel.clone() as Arc<dyn KernelHandle>;
            let turn_constraints = crate::routes::voice_stt_turn_tool_constraints(&attachment_refs);
            match state.kernel.send_message_streaming(
                agent_id,
                &content,
                Some(kernel_handle),
                None,
                None,
                ws_content_blocks,
                None,
                turn_constraints,
            ) {
                Ok((mut rx, handle)) => {
                    // Forward stream events to WebSocket with debouncing.
                    //
                    // The stream_task also accumulates the full response text and
                    // captures ContentComplete usage data. This lets us send the
                    // `response` event immediately when the stream channel closes
                    // (after `drop(phase_cb)` in the kernel), WITHOUT waiting for
                    // post-processing (canonical session writes, JSONL, compaction)
                    // that happens in the kernel task after the loop.
                    let sender_stream = Arc::clone(sender);
                    let verbose_clone = Arc::clone(verbose);
                    let stream_task = tokio::spawn(async move {
                        let mut text_buffer = String::new();
                        let mut accumulated_text = String::new();
                        let mut stream_usage_total: openfang_types::message::TokenUsage =
                            openfang_types::message::TokenUsage::default();
                        let mut compression_savings_pct: u8 = 0;
                        let mut compressed_input: Option<String> = None;
                        let mut compression_semantic_score: Option<f32> = None;
                        let mut adaptive_confidence_ws: Option<f32> = None;
                        let mut eco_counterfactual_ws: Option<
                            openfang_types::adaptive_eco::EcoCounterfactualReceipt,
                        > = None;
                        let mut adaptive_eco_effective_mode_ws: Option<String> = None;
                        let mut adaptive_eco_recommended_mode_ws: Option<String> = None;
                        let mut adaptive_eco_reason_codes_ws: Option<Vec<String>> = None;
                        let far_future = tokio::time::Instant::now() + Duration::from_secs(86400);
                        let mut flush_deadline = far_future;

                        loop {
                            let sleep = tokio::time::sleep_until(flush_deadline);
                            tokio::pin!(sleep);

                            tokio::select! {
                                event = rx.recv() => {
                                    let vlevel = VerboseLevel::from_u8(
                                        verbose_clone.load(Ordering::Relaxed),
                                    );
                                    match event {
                                        None => {
                                            // Stream ended — flush remaining text
                                            let _ = flush_text_buffer(
                                                &sender_stream,
                                                &mut text_buffer,
                                            )
                                            .await;
                                            break;
                                        }
                                        Some(ev) => {
                                            // Capture ContentComplete for immediate response
                                            if let StreamEvent::ContentComplete { usage, .. } = &ev {
                                                stream_usage_total.input_tokens += usage.input_tokens;
                                                stream_usage_total.output_tokens +=
                                                    usage.output_tokens;
                                                stream_usage_total.cache_creation_input_tokens +=
                                                    usage.cache_creation_input_tokens;
                                                stream_usage_total.cache_read_input_tokens +=
                                                    usage.cache_read_input_tokens;
                                                // Don't forward — handled below
                                                continue;
                                            }
                                            // Capture compression stats (emitted once before any LLM call)
                                            if let StreamEvent::CompressionStats {
                                                savings_pct,
                                                compressed_text,
                                                semantic_score,
                                                adaptive_confidence,
                                                counterfactual,
                                                adaptive_eco_effective_mode,
                                                adaptive_eco_recommended_mode,
                                                adaptive_eco_reason_codes,
                                            } = &ev
                                            {
                                                compression_savings_pct = *savings_pct;
                                                if !compressed_text.is_empty() {
                                                    compressed_input = Some(compressed_text.clone());
                                                }
                                                compression_semantic_score = *semantic_score;
                                                adaptive_confidence_ws = *adaptive_confidence;
                                                eco_counterfactual_ws = counterfactual.clone();
                                                adaptive_eco_effective_mode_ws =
                                                    adaptive_eco_effective_mode.clone();
                                                adaptive_eco_recommended_mode_ws =
                                                    adaptive_eco_recommended_mode.clone();
                                                adaptive_eco_reason_codes_ws =
                                                    adaptive_eco_reason_codes.clone();
                                                continue;
                                            }

                                            if let StreamEvent::TextDelta { ref text } = ev {
                                                accumulated_text.push_str(text);
                                                text_buffer.push_str(text);
                                                if text_buffer.len() >= DEBOUNCE_CHARS {
                                                    let _ = flush_text_buffer(
                                                        &sender_stream,
                                                        &mut text_buffer,
                                                    )
                                                    .await;
                                                    flush_deadline = far_future;
                                                } else if flush_deadline >= far_future {
                                                    flush_deadline =
                                                        tokio::time::Instant::now()
                                                            + Duration::from_millis(DEBOUNCE_MS);
                                                }
                                            } else {
                                                // Flush pending text before non-text events
                                                let _ = flush_text_buffer(
                                                    &sender_stream,
                                                    &mut text_buffer,
                                                )
                                                .await;
                                                flush_deadline = far_future;

                                                // Send typing indicator for tool events
                                                if let StreamEvent::ToolUseStart {
                                                    ref name, ..
                                                } = ev
                                                {
                                                    let _ = send_json(
                                                        &sender_stream,
                                                        &serde_json::json!({
                                                            "type": "typing",
                                                            "state": "tool",
                                                            "tool": name,
                                                        }),
                                                    )
                                                    .await;
                                                }

                                                // Map event to JSON with verbose filtering
                                                if let Some(json) =
                                                    map_stream_event(&ev, vlevel)
                                                {
                                                    if send_json(&sender_stream, &json)
                                                        .await
                                                        .is_err()
                                                    {
                                                        break;
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                _ = &mut sleep => {
                                    // Timer fired — flush text buffer
                                    let _ = flush_text_buffer(
                                        &sender_stream,
                                        &mut text_buffer,
                                    )
                                    .await;
                                    flush_deadline = far_future;
                                }
                            }
                        }

                        (
                            accumulated_text,
                            stream_usage_total,
                            compression_savings_pct,
                            compressed_input,
                            compression_semantic_score,
                            adaptive_confidence_ws,
                            eco_counterfactual_ws,
                            adaptive_eco_effective_mode_ws,
                            adaptive_eco_recommended_mode_ws,
                            adaptive_eco_reason_codes_ws,
                        )
                    });

                    // Wait for the stream to finish (fast — closes as soon as
                    // drop(phase_cb) runs after the agent loop). This does NOT
                    // wait for post-processing.
                    let stream_result = stream_task.await;

                    // Spawn the kernel task in the background for cleanup
                    // (canonical session writes, JSONL mirror, compaction).
                    // We don't need its result for the response event.
                    let sender_bg = Arc::clone(sender);
                    tokio::spawn(async move {
                        match handle.await {
                            Ok(Err(e)) => {
                                warn!("Agent post-processing failed: {e}");
                                let user_msg = classify_streaming_error(&e);
                                let _ = send_json(
                                    &sender_bg,
                                    &serde_json::json!({
                                        "type": "error",
                                        "content": user_msg,
                                    }),
                                )
                                .await;
                            }
                            Err(e) => {
                                warn!("Agent task panicked: {e}");
                                let _ = send_json(
                                    &sender_bg,
                                    &serde_json::json!({
                                        "type": "error",
                                        "content": "Internal error occurred",
                                    }),
                                )
                                .await;
                            }
                            Ok(Ok(_)) => {
                                // Post-processing completed successfully — nothing to send
                            }
                        }
                    });

                    // Send the response immediately from stream data
                    match stream_result {
                        Ok((
                            accumulated_text,
                            stream_usage,
                            compression_savings_pct,
                            compressed_input,
                            compression_semantic_score,
                            adaptive_confidence_ws,
                            eco_counterfactual_ws,
                            adaptive_eco_effective_mode_ws,
                            adaptive_eco_recommended_mode_ws,
                            adaptive_eco_reason_codes_ws,
                        )) => {
                            let user_message = content.clone();
                            let agent_id_str = agent_id.to_string();
                            let home = state.kernel.config.home_dir.as_path();

                            // Send typing lifecycle: stop
                            let _ = send_json(
                                sender,
                                &serde_json::json!({
                                    "type": "typing",
                                    "state": "stop",
                                }),
                            )
                            .await;

                            let usage = stream_usage;
                            let turn_wall_ms = turn_wall_t0.elapsed().as_millis() as u64;

                            let cleaned = strip_think_tags(&accumulated_text);
                            let (directive_visible, directive_set) =
                                openfang_runtime::reply_directives::parse_directives(&cleaned);
                            let is_silent =
                                openfang_runtime::reply_directives::assistant_intended_silent(
                                    &directive_visible,
                                    &directive_set,
                                );

                            if is_silent {
                                let skill_path = skill_draft_path_for_learn_message(
                                    home,
                                    &user_message,
                                    "",
                                    &agent_id_str,
                                    true,
                                );
                                let mut silent_payload = serde_json::json!({
                                    "type": "silent_complete",
                                    "turn_outcome": openfang_types::agent::TurnOutcome::UserSilent,
                                    "input_tokens": usage.input_tokens,
                                    "output_tokens": usage.output_tokens,
                                    "turn_wall_ms": turn_wall_ms,
                                });
                                if let Some(p) = skill_path {
                                    silent_payload["skill_draft_path"] = serde_json::json!(p);
                                }
                                if compression_savings_pct > 0 {
                                    silent_payload["compression_savings_pct"] =
                                        serde_json::json!(compression_savings_pct);
                                }
                                if let Some(ref ci) = compressed_input {
                                    silent_payload["compressed_input"] = serde_json::json!(ci);
                                }
                                if let Some(score) = compression_semantic_score {
                                    silent_payload["compression_semantic_score"] =
                                        serde_json::json!(score);
                                }
                                if let Some(ac) = adaptive_confidence_ws {
                                    silent_payload["adaptive_confidence"] = serde_json::json!(ac);
                                }
                                if let Some(ref cf) = eco_counterfactual_ws {
                                    silent_payload["eco_counterfactual"] = serde_json::json!(cf);
                                }
                                if let Some(ref m) = adaptive_eco_effective_mode_ws {
                                    silent_payload["adaptive_eco_effective_mode"] =
                                        serde_json::json!(m);
                                }
                                if let Some(ref m) = adaptive_eco_recommended_mode_ws {
                                    silent_payload["adaptive_eco_recommended_mode"] =
                                        serde_json::json!(m);
                                }
                                if let Some(ref c) = adaptive_eco_reason_codes_ws {
                                    silent_payload["adaptive_eco_reason_codes"] =
                                        serde_json::json!(c);
                                }
                                let _ = send_json(sender, &silent_payload).await;
                                return;
                            }

                            let response_text = if directive_visible.trim().is_empty() {
                                if usage.input_tokens == 0 && usage.output_tokens == 0 {
                                    "[No assistant text (0 tokens). The model call did not complete — usually a missing or invalid provider API key. For OpenRouter: Settings → Providers, or set OPENROUTER_API_KEY for the daemon. If an error line appears below, that is the real cause.]".to_string()
                                } else {
                                    format!(
                                        "[The agent completed processing but returned no text response. ({} in / {} out)]",
                                        usage.input_tokens, usage.output_tokens,
                                    )
                                }
                            } else {
                                directive_visible
                            };

                            let turn_outcome =
                                openfang_types::agent::TurnOutcome::classify(&response_text, false);

                            let skill_path = skill_draft_path_for_learn_message(
                                home,
                                &user_message,
                                &response_text,
                                &agent_id_str,
                                false,
                            );

                            // Estimate context pressure
                            let ctx_pct =
                                (usage.input_tokens as f64 / 200_000.0 * 100.0).min(100.0);
                            let pressure = if ctx_pct > 85.0 {
                                "critical"
                            } else if ctx_pct > 70.0 {
                                "high"
                            } else if ctx_pct > 50.0 {
                                "medium"
                            } else {
                                "low"
                            };

                            let mut response_payload = serde_json::json!({
                                "type": "response",
                                "content": response_text,
                                "turn_outcome": turn_outcome,
                                "input_tokens": usage.input_tokens,
                                "output_tokens": usage.output_tokens,
                                "iterations": 0, // Not available from stream; handle updates later if needed
                                "cost_usd": null,
                                "context_pressure": pressure,
                                "turn_wall_ms": turn_wall_ms,
                            });
                            if let Some(p) = skill_path {
                                response_payload["skill_draft_path"] = serde_json::json!(p);
                            }
                            if compression_savings_pct > 0 {
                                response_payload["compression_savings_pct"] =
                                    serde_json::json!(compression_savings_pct);
                            }
                            if let Some(ref ci) = compressed_input {
                                response_payload["compressed_input"] = serde_json::json!(ci);
                            }
                            if let Some(score) = compression_semantic_score {
                                response_payload["compression_semantic_score"] =
                                    serde_json::json!(score);
                            }
                            if let Some(ac) = adaptive_confidence_ws {
                                response_payload["adaptive_confidence"] = serde_json::json!(ac);
                            }
                            if let Some(ref cf) = eco_counterfactual_ws {
                                response_payload["eco_counterfactual"] = serde_json::json!(cf);
                            }
                            if let Some(ref m) = adaptive_eco_effective_mode_ws {
                                response_payload["adaptive_eco_effective_mode"] =
                                    serde_json::json!(m);
                            }
                            if let Some(ref m) = adaptive_eco_recommended_mode_ws {
                                response_payload["adaptive_eco_recommended_mode"] =
                                    serde_json::json!(m);
                            }
                            if let Some(ref c) = adaptive_eco_reason_codes_ws {
                                response_payload["adaptive_eco_reason_codes"] =
                                    serde_json::json!(c);
                            }
                            if voice_reply && !response_text.trim().is_empty() {
                                let local_voice = state.kernel.local_voice_effective();
                                if local_voice.local_tts_ready() {
                                    match openfang_runtime::tts::synthesize_local_tts(
                                        &response_text,
                                        &local_voice,
                                    )
                                    .await
                                    {
                                        Ok(tts) => {
                                            match crate::routes::register_generated_upload(
                                                "audio/wav",
                                                "voice_reply.wav",
                                                tts.audio_data,
                                            ) {
                                                Ok(url) => {
                                                    response_payload["voice_reply_audio_url"] =
                                                        serde_json::json!(url);
                                                    response_payload["voice_reply_provider"] =
                                                        serde_json::json!(tts.provider);
                                                }
                                                Err(e) => {
                                                    tracing::warn!(
                                                        error = %e,
                                                        "WS: voice reply upload failed"
                                                    );
                                                    response_payload["voice_reply_error"] = serde_json::json!(
                                                        format!("voice upload failed: {e}")
                                                    );
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            tracing::warn!(
                                                error = %e,
                                                "WS: local TTS voice reply failed"
                                            );
                                            response_payload["voice_reply_error"] =
                                                serde_json::json!(e);
                                        }
                                    }
                                } else {
                                    response_payload["voice_reply_error"] = serde_json::json!(
                                        "local TTS not ready: no Piper bundle and no \
                                         macOS `say` available — see /api/system/local-voice"
                                    );
                                }
                            }
                            let _ = send_json(sender, &response_payload).await;
                        }
                        Err(e) => {
                            warn!("Stream task panicked: {e}");
                            let _ = send_json(
                                sender,
                                &serde_json::json!({
                                    "type": "typing", "state": "stop",
                                }),
                            )
                            .await;
                            let _ = send_json(
                                sender,
                                &serde_json::json!({
                                    "type": "error",
                                    "content": "Internal error occurred",
                                }),
                            )
                            .await;
                        }
                    }
                }
                Err(e) => {
                    warn!("Streaming setup failed: {e}");
                    let _ = send_json(
                        sender,
                        &serde_json::json!({
                            "type": "typing", "state": "stop",
                        }),
                    )
                    .await;
                    let user_msg = classify_streaming_error(&e);
                    let _ = send_json(
                        sender,
                        &serde_json::json!({
                            "type": "error",
                            "content": user_msg,
                        }),
                    )
                    .await;
                }
            }
        }
        "command" => {
            let cmd = parsed["command"].as_str().unwrap_or("");
            let args = parsed["args"].as_str().unwrap_or("");
            let response = handle_command(sender, state, agent_id, cmd, args, verbose).await;
            let _ = send_json(sender, &response).await;
        }
        "ping" => {
            let _ = send_json(sender, &serde_json::json!({"type": "pong"})).await;
        }
        other => {
            warn!(msg_type = other, "Unknown WebSocket message type");
            let _ = send_json(
                sender,
                &serde_json::json!({
                    "type": "error",
                    "content": format!("Unknown message type: {other}"),
                }),
            )
            .await;
        }
    }
}

// ---------------------------------------------------------------------------
// Command Handler
// ---------------------------------------------------------------------------

/// Handle a WS command and return the response JSON.
async fn handle_command(
    _sender: &Arc<Mutex<SplitSink<WebSocket, Message>>>,
    state: &Arc<AppState>,
    agent_id: AgentId,
    cmd: &str,
    args: &str,
    verbose: &Arc<AtomicU8>,
) -> serde_json::Value {
    match cmd {
        "new" | "reset" => match state.kernel.reset_session(agent_id) {
            Ok(()) => {
                serde_json::json!({"type": "command_result", "command": cmd, "message": "Session reset. Chat history cleared."})
            }
            Err(e) => serde_json::json!({"type": "error", "content": format!("Reset failed: {e}")}),
        },
        "compact" => match state.kernel.compact_agent_session(agent_id).await {
            Ok(msg) => {
                serde_json::json!({"type": "command_result", "command": cmd, "message": msg})
            }
            Err(e) => {
                serde_json::json!({"type": "error", "content": format!("Compaction failed: {e}")})
            }
        },
        "stop" => match state.kernel.stop_agent_run(agent_id) {
            Ok(true) => {
                serde_json::json!({"type": "command_result", "command": cmd, "message": "Run cancelled."})
            }
            Ok(false) => {
                serde_json::json!({"type": "command_result", "command": cmd, "message": "No active run to cancel."})
            }
            Err(e) => serde_json::json!({"type": "error", "content": format!("Stop failed: {e}")}),
        },
        "model" => {
            if args.is_empty() {
                if let Some(entry) = state.kernel.registry.get(agent_id) {
                    serde_json::json!({"type": "command_result", "command": cmd, "message": format!("Current model: {} (provider: {})", entry.manifest.model.model, entry.manifest.model.provider)})
                } else {
                    serde_json::json!({"type": "error", "content": "Agent not found"})
                }
            } else {
                match state.kernel.set_agent_model(agent_id, args, None) {
                    Ok(()) => {
                        if let Some(entry) = state.kernel.registry.get(agent_id) {
                            let model = &entry.manifest.model.model;
                            let provider = &entry.manifest.model.provider;
                            serde_json::json!({
                                "type": "command_result",
                                "command": cmd,
                                "message": format!("Model switched to: {model} (provider: {provider})"),
                                "model": model,
                                "provider": provider
                            })
                        } else {
                            serde_json::json!({"type": "command_result", "command": cmd, "message": format!("Model switched to: {args}")})
                        }
                    }
                    Err(e) => {
                        serde_json::json!({"type": "error", "content": format!("Model switch failed: {e}")})
                    }
                }
            }
        }
        "usage" => match state.kernel.session_usage_cost(agent_id) {
            Ok((input, output, cost)) => {
                let mut msg = format!(
                    "Session usage: ~{input} in / ~{output} out (~{} total)",
                    input + output
                );
                if cost > 0.0 {
                    msg.push_str(&format!(" | ${cost:.4}"));
                }
                serde_json::json!({"type": "command_result", "command": cmd, "message": msg})
            }
            Err(e) => {
                serde_json::json!({"type": "error", "content": format!("Usage query failed: {e}")})
            }
        },
        "context" => match state.kernel.context_report(agent_id) {
            Ok(report) => {
                let formatted = openfang_runtime::compactor::format_context_report(&report);
                serde_json::json!({
                    "type": "command_result",
                    "command": cmd,
                    "message": formatted,
                    "context_pressure": format!("{:?}", report.pressure).to_lowercase(),
                })
            }
            Err(e) => {
                serde_json::json!({"type": "error", "content": format!("Context report failed: {e}")})
            }
        },
        "verbose" => {
            let new_level = match args.to_lowercase().as_str() {
                "off" => VerboseLevel::Off,
                "on" => VerboseLevel::On,
                "full" => VerboseLevel::Full,
                _ => {
                    // Cycle to next level
                    let current = VerboseLevel::from_u8(verbose.load(Ordering::Relaxed));
                    current.next()
                }
            };
            verbose.store(new_level as u8, Ordering::Relaxed);
            serde_json::json!({
                "type": "command_result",
                "command": cmd,
                "message": format!("Verbose level: **{}**", new_level.label()),
            })
        }
        "queue" => {
            let is_running = state.kernel.running_tasks.contains_key(&agent_id);
            let msg = if is_running {
                "Agent is processing a request..."
            } else {
                "Agent is idle."
            };
            serde_json::json!({"type": "command_result", "command": cmd, "message": msg})
        }
        "budget" => {
            let budget = &state.kernel.config.budget;
            let status = state.kernel.metering.budget_status(budget);
            let fmt = |v: f64| -> String {
                if v > 0.0 {
                    format!("${v:.2}")
                } else {
                    "unlimited".to_string()
                }
            };
            let msg = format!(
                "Hourly: ${:.4} / {}  |  Daily: ${:.4} / {}  |  Monthly: ${:.4} / {}",
                status.hourly_spend,
                fmt(status.hourly_limit),
                status.daily_spend,
                fmt(status.daily_limit),
                status.monthly_spend,
                fmt(status.monthly_limit),
            );
            serde_json::json!({"type": "command_result", "command": cmd, "message": msg})
        }
        "peers" => {
            let msg = if !state.kernel.config.network_enabled {
                "OFP network disabled.".to_string()
            } else {
                match state.kernel.peer_registry.get() {
                    Some(registry) => {
                        let peers = registry.all_peers();
                        if peers.is_empty() {
                            "No peers connected.".to_string()
                        } else {
                            peers
                                .iter()
                                .map(|p| format!("{} — {} ({:?})", p.node_id, p.address, p.state))
                                .collect::<Vec<_>>()
                                .join("\n")
                        }
                    }
                    None => "OFP peer node not started.".to_string(),
                }
            };
            serde_json::json!({"type": "command_result", "command": cmd, "message": msg})
        }
        "a2a" => {
            let agents = state
                .kernel
                .a2a_external_agents
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let msg = if agents.is_empty() {
                "No external A2A agents discovered.".to_string()
            } else {
                agents
                    .iter()
                    .map(|(url, card)| format!("{} — {}", card.name, url))
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            serde_json::json!({"type": "command_result", "command": cmd, "message": msg})
        }
        _ => serde_json::json!({"type": "error", "content": format!("Unknown command: {cmd}")}),
    }
}

// ---------------------------------------------------------------------------
// Stream Event Mapping (verbose-aware)
// ---------------------------------------------------------------------------

/// Map a stream event to a JSON value, applying verbose filtering.
fn map_stream_event(event: &StreamEvent, verbose: VerboseLevel) -> Option<serde_json::Value> {
    match event {
        StreamEvent::TextDelta { .. } => None, // Handled by debounce buffer
        StreamEvent::ToolUseStart { id, name, .. } => Some(serde_json::json!({
            "type": "tool_start",
            "id": id,
            "tool": name,
        })),
        StreamEvent::ToolUseEnd {
            id, name, input, ..
        } if name == "canvas_present" => {
            let html = input.get("html").and_then(|v| v.as_str()).unwrap_or("");
            let title = input
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("Canvas");
            Some(serde_json::json!({
                "type": "canvas",
                "id": id,
                "canvas_id": uuid::Uuid::new_v4().to_string(),
                "html": html,
                "title": title,
            }))
        }
        StreamEvent::ToolUseEnd {
            id, name, input, ..
        } => match verbose {
            VerboseLevel::Off => None,
            VerboseLevel::On => {
                let input_preview: String = serde_json::to_string(input)
                    .unwrap_or_default()
                    .chars()
                    .take(100)
                    .collect();
                Some(serde_json::json!({
                    "type": "tool_end",
                    "id": id,
                    "tool": name,
                    "input": input_preview,
                }))
            }
            VerboseLevel::Full => {
                let input_preview: String = serde_json::to_string(input)
                    .unwrap_or_default()
                    .chars()
                    .take(500)
                    .collect();
                Some(serde_json::json!({
                    "type": "tool_end",
                    "id": id,
                    "tool": name,
                    "input": input_preview,
                }))
            }
        },
        StreamEvent::ToolExecutionResult {
            id,
            name,
            result_preview,
            is_error,
        } => match verbose {
            VerboseLevel::Off => Some(serde_json::json!({
                "type": "tool_result",
                "id": id,
                "tool": name,
                "is_error": is_error,
            })),
            VerboseLevel::On => {
                let truncated: String = result_preview.chars().take(200).collect();
                Some(serde_json::json!({
                    "type": "tool_result",
                    "id": id,
                    "tool": name,
                    "result": truncated,
                    "is_error": is_error,
                }))
            }
            VerboseLevel::Full => Some(serde_json::json!({
                "type": "tool_result",
                "id": id,
                "tool": name,
                "result": result_preview,
                "is_error": is_error,
            })),
        },
        StreamEvent::PhaseChange { phase, detail } => Some(serde_json::json!({
            "type": "phase",
            "phase": phase,
            "detail": detail,
        })),
        StreamEvent::AinlRuntimeTelemetry { payload } => Some(serde_json::json!({
            "type": "ainl_runtime_telemetry",
            "telemetry": payload,
        })),
        _ => None, // Skip ToolInputDelta, ContentComplete
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Flush accumulated text buffer as a single text_delta event.
async fn flush_text_buffer(
    sender: &Arc<Mutex<SplitSink<WebSocket, Message>>>,
    buffer: &mut String,
) -> Result<(), axum::Error> {
    if buffer.is_empty() {
        return Ok(());
    }
    let result = send_json(
        sender,
        &serde_json::json!({
            "type": "text_delta",
            "content": buffer.as_str(),
        }),
    )
    .await;
    buffer.clear();
    result
}

/// Helper to send a JSON value over WebSocket.
async fn send_json(
    sender: &Arc<Mutex<SplitSink<WebSocket, Message>>>,
    value: &serde_json::Value,
) -> Result<(), axum::Error> {
    let text = serde_json::to_string(value).unwrap_or_default();
    let mut s = sender.lock().await;
    s.send(Message::Text(text.into()))
        .await
        .map_err(axum::Error::new)
}

/// Sanitize inbound user input.
///
/// - If content looks like a JSON envelope, extract the `content` field.
/// - Strip control characters (except \n, \t).
/// - Trim excessive whitespace.
fn sanitize_user_input(content: &str) -> String {
    // If content looks like a JSON envelope, try to extract the content field
    if content.starts_with('{') {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(content) {
            if let Some(inner) = val.get("content").and_then(|v| v.as_str()) {
                return sanitize_text(inner);
            }
        }
    }
    sanitize_text(content)
}

/// Strip control characters and normalize whitespace.
fn sanitize_text(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect::<String>()
        .trim()
        .to_string()
}

/// Classify a streaming/setup error into a user-friendly message.
///
/// Uses the proper LLM error classifier from `openfang_runtime::llm_errors`
/// for comprehensive 20-provider coverage with actionable advice.
fn classify_streaming_error(err: &openfang_kernel::error::KernelError) -> String {
    let inner = format!("{err}");

    // Check for agent-specific errors first (not LLM errors)
    if inner.contains("Agent not found") {
        return "Agent not found. It may have been stopped or deleted.".to_string();
    }
    if inner.contains("quota") || inner.contains("Quota") {
        return "Token quota exceeded. Try /compact or /new to free up space.".to_string();
    }

    // Use the LLM error classifier for everything else
    let status = extract_status_code(&inner);
    let classified = llm_errors::classify_error(&inner, status);

    // Build a user-facing message. The classified.sanitized_message now
    // includes a redacted excerpt of the raw error (issue #493 fix), so we
    // use it as the base and only override for cases that need extra context.
    match classified.category {
        llm_errors::LlmErrorCategory::ContextOverflow => {
            "Context is full. Try /compact or /new.".to_string()
        }
        llm_errors::LlmErrorCategory::RateLimit => {
            if let Some(delay_ms) = classified.suggested_delay_ms {
                let secs = (delay_ms / 1000).max(1);
                format!("Rate limited. Wait ~{secs}s and try again.")
            } else {
                "Rate limited. Wait a moment and try again.".to_string()
            }
        }
        llm_errors::LlmErrorCategory::Billing => {
            format!("Billing issue. {}", classified.sanitized_message)
        }
        llm_errors::LlmErrorCategory::Auth => {
            // Show the actual error detail so users can diagnose (issue #493).
            // The sanitized_message already redacts secrets.
            classified.sanitized_message.clone()
        }
        llm_errors::LlmErrorCategory::ModelNotFound => {
            if inner.contains("localhost:11434") || inner.contains("ollama") {
                "Model not found on Ollama. Run `ollama pull <model>` first. Use /model to see options.".to_string()
            } else {
                format!(
                    "{}. Use /model to see options.",
                    classified.sanitized_message
                )
            }
        }
        llm_errors::LlmErrorCategory::Format => {
            // Claude Code CLI errors have actionable messages — pass them through
            if inner.contains("Claude Code CLI") || inner.contains("claude auth") {
                classified.raw_message.clone()
            } else {
                classified.sanitized_message.clone()
            }
        }
        _ => classified.sanitized_message,
    }
}

/// Try to extract an HTTP status code from an error string.
fn extract_status_code(s: &str) -> Option<u16> {
    // "API error (NNN):" — the format produced by LlmError::Api Display impl
    if let Some(idx) = s.find("API error (") {
        let after = &s[idx + 11..];
        let num: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(code) = num.parse::<u16>() {
            return Some(code);
        }
    }
    // "status: NNN"
    if let Some(idx) = s.find("status: ") {
        let after = &s[idx + 8..];
        let num: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(code) = num.parse() {
            return Some(code);
        }
    }
    // "HTTP NNN"
    if let Some(idx) = s.find("HTTP ") {
        let after = &s[idx + 5..];
        let num: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(code) = num.parse() {
            return Some(code);
        }
    }
    // "StatusCode(NNN)"
    if let Some(idx) = s.find("StatusCode(") {
        let after = &s[idx + 11..];
        let num: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(code) = num.parse() {
            return Some(code);
        }
    }
    None
}

/// Strip `<think>...</think>` blocks from model output.
///
/// Some models (MiniMax, DeepSeek, etc.) wrap their reasoning in `<think>` tags.
/// These are internal chain-of-thought and shouldn't be shown to the user.
pub fn strip_think_tags(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut remaining = text;
    while let Some(start) = remaining.find("<think>") {
        result.push_str(&remaining[..start]);
        if let Some(end) = remaining[start..].find("</think>") {
            remaining = &remaining[(start + end + 8)..]; // 8 = "</think>".len()
        } else {
            // Unclosed <think> tag — strip to end
            remaining = "";
            break;
        }
    }
    result.push_str(remaining);
    result
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ws_module_loads() {
        // Verify module compiles and loads correctly
        let _ = VerboseLevel::Off;
    }

    #[test]
    fn test_verbose_level_cycle() {
        assert_eq!(VerboseLevel::Off.next(), VerboseLevel::On);
        assert_eq!(VerboseLevel::On.next(), VerboseLevel::Full);
        assert_eq!(VerboseLevel::Full.next(), VerboseLevel::Off);
    }

    #[test]
    fn test_verbose_level_roundtrip() {
        for v in [VerboseLevel::Off, VerboseLevel::On, VerboseLevel::Full] {
            assert_eq!(VerboseLevel::from_u8(v as u8), v);
        }
    }

    #[test]
    fn test_verbose_level_labels() {
        assert_eq!(VerboseLevel::Off.label(), "off");
        assert_eq!(VerboseLevel::On.label(), "on");
        assert_eq!(VerboseLevel::Full.label(), "full");
    }

    #[test]
    fn test_sanitize_user_input_plain_text() {
        assert_eq!(sanitize_user_input("hello world"), "hello world");
    }

    #[test]
    fn test_sanitize_user_input_strips_control_chars() {
        assert_eq!(sanitize_user_input("hello\x00world"), "helloworld");
        // Newlines and tabs are preserved
        assert_eq!(sanitize_user_input("hello\nworld"), "hello\nworld");
        assert_eq!(sanitize_user_input("hello\tworld"), "hello\tworld");
    }

    #[test]
    fn test_sanitize_user_input_extracts_json_content() {
        let envelope = r#"{"type":"message","content":"actual message"}"#;
        assert_eq!(sanitize_user_input(envelope), "actual message");
    }

    #[test]
    fn test_sanitize_user_input_leaves_non_envelope_json() {
        // JSON that doesn't have a content field is left as-is (after control-char stripping)
        let json = r#"{"key":"value"}"#;
        assert_eq!(sanitize_user_input(json), r#"{"key":"value"}"#);
    }

    #[test]
    fn test_extract_status_code() {
        assert_eq!(extract_status_code("status: 429, body: ..."), Some(429));
        assert_eq!(
            extract_status_code("HTTP 503 Service Unavailable"),
            Some(503)
        );
        assert_eq!(extract_status_code("StatusCode(401)"), Some(401));
        assert_eq!(extract_status_code("some random error"), None);
        // LlmError::Api Display format (issue #493 fix)
        assert_eq!(
            extract_status_code("LLM driver error: API error (403): quota exceeded"),
            Some(403)
        );
        assert_eq!(
            extract_status_code("API error (401): invalid api key"),
            Some(401)
        );
    }

    #[test]
    fn test_sanitize_trims_whitespace() {
        assert_eq!(sanitize_user_input("  hello  "), "hello");
    }

    #[test]
    fn test_strip_think_tags() {
        assert_eq!(
            strip_think_tags("<think>reasoning here</think>The answer is 42."),
            "The answer is 42."
        );
        assert_eq!(
            strip_think_tags("Hello <think>\nsome thinking\n</think> world"),
            "Hello  world"
        );
        assert_eq!(strip_think_tags("No thinking here"), "No thinking here");
        assert_eq!(strip_think_tags("<think>all thinking</think>"), "");
    }

    #[test]
    fn ws_upgrade_api_key_allows_when_key_empty_even_with_bogus_bearer() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderValue::from_static("Bearer nope"),
        );
        let uri: axum::http::Uri = "http://x/ws".parse().unwrap();
        assert!(ws_upgrade_api_key_allowed("", &headers, &uri));
    }

    #[test]
    fn ws_upgrade_api_key_accepts_bearer_or_query_token() {
        let key = "super-secret-key-for-ws-tests";
        let uri_with_token: axum::http::Uri = format!("http://x/ws?token={key}").parse().unwrap();
        let headers_empty = axum::http::HeaderMap::new();
        assert!(ws_upgrade_api_key_allowed(
            key,
            &headers_empty,
            &uri_with_token
        ));

        let mut headers_bearer = axum::http::HeaderMap::new();
        headers_bearer.insert(
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderValue::from_str(&format!("Bearer {key}")).unwrap(),
        );
        let uri_no_query: axum::http::Uri = "http://x/ws".parse().unwrap();
        assert!(ws_upgrade_api_key_allowed(
            key,
            &headers_bearer,
            &uri_no_query
        ));

        let mut headers_wrong = axum::http::HeaderMap::new();
        headers_wrong.insert(
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderValue::from_static("Bearer wrong"),
        );
        assert!(!ws_upgrade_api_key_allowed(
            key,
            &headers_wrong,
            &uri_no_query
        ));
    }

    #[test]
    fn ws_per_ip_slot_exhaustion_then_release() {
        use std::net::{IpAddr, Ipv4Addr};
        let ip = IpAddr::V4(Ipv4Addr::new(198, 51, 100, 44));
        let mut guards = Vec::new();
        for _ in 0..MAX_WS_PER_IP {
            guards.push(try_acquire_ws_slot(ip).expect("expected slot"));
        }
        assert!(try_acquire_ws_slot(ip).is_none(), "should hit per-IP cap");
        drop(guards);
        assert!(try_acquire_ws_slot(ip).is_some());
    }
}
