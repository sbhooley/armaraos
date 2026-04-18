//! HTTP client for `ainl-inference-server` native `POST /armara/v1/infer`.
//!
//! Also implements [`crate::llm_driver::LlmDriver`] by mapping [`CompletionRequest`] to a minimal
//! [`InferRequest`] (no `agent_snapshot`) for the standard chat path.

use armara_provider_api::{
    ChatMessage as InfChatMessage, InferRequest, InferResponse, ModelHint, SessionRef,
};
use async_trait::async_trait;
use uuid::Uuid;

use crate::llm_driver::{CompletionRequest, CompletionResponse, LlmDriver, LlmError};
use crate::planner_mode::normalize_infer_base_url;
use openfang_types::message::{ContentBlock, Message, Role, StopReason, TokenUsage};

/// Client for `POST {base}/armara/v1/infer`.
pub struct NativeInferDriver {
    pub base_url: String,
    pub api_key: Option<String>,
    client: reqwest::Client,
}

impl NativeInferDriver {
    pub fn new(base_url: String, api_key: Option<String>, client: reqwest::Client) -> Self {
        Self {
            base_url: normalize_infer_base_url(&base_url),
            api_key,
            client,
        }
    }

    fn infer_url(&self) -> String {
        format!("{}/armara/v1/infer", self.base_url.trim_end_matches('/'))
    }

    /// Native infer with full request (planner: include `agent_snapshot` / `repair_context`).
    pub async fn infer(&self, req: InferRequest) -> Result<InferResponse, LlmError> {
        let mut r = self.client.post(self.infer_url()).json(&req);
        if let Some(ref k) = self.api_key {
            if !k.is_empty() {
                r = r.bearer_auth(k);
            }
        }
        let resp = r.send().await.map_err(|e| LlmError::Http(e.to_string()))?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let t = resp.text().await.unwrap_or_default();
            return Err(LlmError::Api {
                status,
                message: t,
            });
        }
        resp.json::<InferResponse>()
            .await
            .map_err(|e| LlmError::Parse(e.to_string()))
    }
}

fn openfang_message_to_infer(m: &Message) -> Option<InfChatMessage> {
    let role = match m.role {
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::System => "system",
    };
    let content = message_body_for_infer(m);
    if content.is_empty() {
        return None;
    }
    Some(InfChatMessage {
        role: role.to_string(),
        content,
    })
}

fn message_body_for_infer(m: &Message) -> String {
    use openfang_types::message::{ContentBlock, MessageContent};
    match &m.content {
        MessageContent::Text(s) => s.clone(),
        MessageContent::Blocks(blocks) => {
            let mut parts = Vec::new();
            for b in blocks {
                match b {
                    ContentBlock::Text { text, .. } => parts.push(text.clone()),
                    ContentBlock::ToolUse { name, input, .. } => {
                        parts.push(format!("[tool_use {name}] {}", input));
                    }
                    ContentBlock::ToolResult { content, .. } => parts.push(content.clone()),
                    ContentBlock::Thinking { thinking } => parts.push(thinking.clone()),
                    _ => {}
                }
            }
            parts.join("\n")
        }
    }
}

#[async_trait]
impl LlmDriver for NativeInferDriver {
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let mut messages: Vec<InfChatMessage> = Vec::new();
        if let Some(ref sys) = req.system {
            if !sys.is_empty() {
                messages.push(InfChatMessage {
                    role: "system".into(),
                    content: sys.clone(),
                });
            }
        }
        for m in &req.messages {
            if let Some(cm) = openfang_message_to_infer(m) {
                messages.push(cm);
            }
        }
        let infer = InferRequest {
            request_id: Uuid::new_v4(),
            tenant_id: None,
            session: Some(SessionRef {
                agent_id: None,
                turn_id: None,
            }),
            model: ModelHint {
                policy: None,
                hint: Some(req.model.clone()),
            },
            messages,
            graph_context: None,
            constraints: Default::default(),
            policy: Default::default(),
            backend_preference: vec![],
            agent_snapshot: None,
            repair_context: None,
        };
        let out = self.infer(infer).await?;
        let text = out.output.text.clone();
        let usage = TokenUsage {
            input_tokens: out.usage.input_tokens,
            output_tokens: out.usage.output_tokens,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
        };
        Ok(CompletionResponse {
            content: vec![ContentBlock::Text {
                text,
                provider_metadata: None,
            }],
            stop_reason: StopReason::EndTurn,
            tool_calls: vec![],
            usage,
            vitals: None,
        })
    }

}
