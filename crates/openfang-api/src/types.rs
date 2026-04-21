//! Request/response types for the OpenFang API.

use openfang_types::message::ToolTurnRecord;
use serde::{Deserialize, Serialize};

#[allow(clippy::trivially_copy_pass_by_ref)]
pub(crate) fn is_zero_u8(v: &u8) -> bool {
    *v == 0
}

/// Request to spawn an agent from a TOML manifest string or a template name.
#[derive(Debug, Deserialize)]
pub struct SpawnRequest {
    /// Agent manifest as TOML string (optional if `template` is provided).
    #[serde(default)]
    pub manifest_toml: String,
    /// Template name from `~/.openfang/agents/{template}/agent.toml`.
    /// When provided and `manifest_toml` is empty, the template is loaded automatically.
    #[serde(default)]
    pub template: Option<String>,
    /// Optional Ed25519 signed manifest envelope (JSON).
    /// When present, the signature is verified before spawning.
    #[serde(default)]
    pub signed_manifest: Option<String>,
}

/// Response after spawning an agent.
#[derive(Debug, Serialize)]
pub struct SpawnResponse {
    pub agent_id: String,
    pub name: String,
}

/// A file attachment reference (from a prior upload).
#[derive(Debug, Clone, Deserialize)]
pub struct AttachmentRef {
    pub file_id: String,
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub content_type: String,
}

/// Request to send a message to an agent.
#[derive(Debug, Deserialize)]
pub struct MessageRequest {
    pub message: String,
    /// Optional file attachments (uploaded via /upload endpoint).
    #[serde(default)]
    pub attachments: Vec<AttachmentRef>,
    /// When true and `[local_voice]` Piper is configured, synthesize the assistant reply to speech and return a playable URL.
    #[serde(default)]
    pub voice_reply: bool,
    /// Sender identity (e.g. WhatsApp phone number, Telegram user ID).
    #[serde(default)]
    pub sender_id: Option<String>,
    /// Sender display name.
    #[serde(default)]
    pub sender_name: Option<String>,
}

/// Response from sending a message.
#[derive(Debug, Serialize)]
pub struct MessageResponse {
    pub response: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub iterations: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    /// Wall time for the full agent loop (LLM + tools), when measured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    /// When a fallback model or OpenRouter free-tier path was used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub llm_fallback_note: Option<String>,
    /// Path when a skill draft was written (e.g. `[learn]` prefix on `POST .../message`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skill_draft_path: Option<String>,
    /// Percentage of input tokens saved by the prompt compressor (0 = off / no compression).
    #[serde(skip_serializing_if = "crate::types::is_zero_u8")]
    pub compression_savings_pct: u8,
    /// The compressed version of the user message (only present when savings_pct > 0; powers diff UI).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compressed_input: Option<String>,
    /// Optional semantic preservation score for compressed input (0.0..1.0).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compression_semantic_score: Option<f32>,
    /// Optional adaptive eco policy confidence (0.0–1.0) when adaptive metadata is present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adaptive_confidence: Option<f32>,
    /// Optional counterfactual compression receipt (applied vs recommendation / baselines).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eco_counterfactual: Option<openfang_types::adaptive_eco::EcoCounterfactualReceipt>,
    /// Effective eco mode after kernel adaptive policy (when present).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adaptive_eco_effective_mode: Option<String>,
    /// Resolver recommendation (may differ in shadow mode).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adaptive_eco_recommended_mode: Option<String>,
    /// Machine-readable adaptive policy reason codes for this turn.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adaptive_eco_reason_codes: Option<Vec<String>>,
    /// Tool calls from this turn (HTTP clients without WebSocket tool events).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolTurnRecord>,
    /// Structured telemetry for ainl-runtime-engine turns.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ainl_runtime_telemetry: Option<serde_json::Value>,
    /// Playable URL (e.g. `/api/uploads/…`) for a local Piper TTS rendering of `response` when `voice_reply` was requested.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voice_reply_audio_url: Option<String>,
}

/// Request to install a skill from the marketplace.
#[derive(Debug, Deserialize)]
pub struct SkillInstallRequest {
    pub name: String,
}

/// Request to uninstall a skill.
#[derive(Debug, Deserialize)]
pub struct SkillUninstallRequest {
    pub name: String,
}

/// Request to update an agent's manifest.
#[derive(Debug, Deserialize)]
pub struct AgentUpdateRequest {
    pub manifest_toml: String,
}

/// Request to change an agent's operational mode.
#[derive(Debug, Deserialize)]
pub struct SetModeRequest {
    pub mode: openfang_types::agent::AgentMode,
}

/// Request to run a migration.
#[derive(Debug, Deserialize)]
pub struct MigrateRequest {
    pub source: String,
    pub source_dir: String,
    pub target_dir: String,
    #[serde(default)]
    pub dry_run: bool,
}

/// Request to scan a directory for migration.
#[derive(Debug, Deserialize)]
pub struct MigrateScanRequest {
    pub path: String,
}

/// Request to install a skill from ClawHub.
#[derive(Debug, Deserialize)]
pub struct ClawHubInstallRequest {
    /// ClawHub skill slug (e.g., "github-helper").
    pub slug: String,
}

#[cfg(test)]
mod message_response_contract_tests {
    use super::MessageResponse;
    use openfang_types::message::ToolTurnRecord;

    #[test]
    fn message_response_serializes_adaptive_eco_explainability_fields() {
        let msg = MessageResponse {
            response: "ok".to_string(),
            input_tokens: 1,
            output_tokens: 2,
            iterations: 1,
            cost_usd: None,
            latency_ms: Some(10),
            llm_fallback_note: None,
            skill_draft_path: None,
            compression_savings_pct: 0,
            compressed_input: None,
            compression_semantic_score: None,
            adaptive_confidence: Some(0.88),
            eco_counterfactual: None,
            adaptive_eco_effective_mode: Some("balanced".to_string()),
            adaptive_eco_recommended_mode: Some("aggressive".to_string()),
            adaptive_eco_reason_codes: Some(vec![
                "adaptive_eco:v1".to_string(),
                "policy:post_circuit_cooldown".to_string(),
            ]),
            tools: Vec::<ToolTurnRecord>::new(),
            ainl_runtime_telemetry: None,
            voice_reply_audio_url: None,
        };

        let v = serde_json::to_value(&msg).expect("serialize MessageResponse");
        assert_eq!(v["adaptive_eco_effective_mode"], "balanced");
        assert_eq!(v["adaptive_eco_recommended_mode"], "aggressive");
        let codes = v["adaptive_eco_reason_codes"]
            .as_array()
            .expect("codes array");
        assert!(codes
            .iter()
            .any(|x| x.as_str() == Some("policy:post_circuit_cooldown")));
        let conf = v["adaptive_confidence"]
            .as_f64()
            .expect("adaptive_confidence number");
        assert!((conf - 0.88_f64).abs() < 1e-4);
    }
}
