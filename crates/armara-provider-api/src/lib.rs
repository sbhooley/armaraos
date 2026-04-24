//! Typed request/response for `/armara/v1/*`.

use ainl_agent_snapshot::{AgentSnapshot, RepairContext};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Optional planner telemetry aligned with `tooling/ainl_policy_contract.json` / ArmaraOS `ainl-contracts`.
/// Lives on [`InferRequest`] so published `ainl-agent-snapshot` stays stable for crates.io consumers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct PlannerContextHints {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_intelligence_mcp_ready: Option<bool>,
    /// `"fresh"` | `"stale"` | `"unknown"`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_freshness: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freshness_known_at_plan_time: Option<bool>,
    /// JSON key avoids the substring `exec` (WASM `policy_checker` scans raw request JSON).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "impact_considered_prior_to_run",
        alias = "impact_considered_before_execute"
    )]
    pub impact_considered_before_execute: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferRequest {
    #[serde(default = "uuid_new")]
    pub request_id: Uuid,
    #[serde(default)]
    pub tenant_id: Option<String>,
    #[serde(default)]
    pub session: Option<SessionRef>,
    #[serde(default)]
    pub model: ModelHint,
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub graph_context: Option<GraphContext>,
    #[serde(default)]
    pub constraints: Constraints,
    #[serde(default)]
    pub policy: Policy,
    #[serde(default)]
    pub backend_preference: Vec<BackendKind>,
    /// Bounded agent graph + caps for planner mode (optional).
    #[serde(default)]
    pub agent_snapshot: Option<AgentSnapshot>,
    /// Single-step repair context for `LocalPatch` escalation (optional).
    #[serde(default)]
    pub repair_context: Option<RepairContext>,
    /// Telemetry-only hints for deterministic planner / observability (optional).
    #[serde(default)]
    pub planner_context_hints: Option<PlannerContextHints>,
}

fn uuid_new() -> Uuid {
    Uuid::new_v4()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionRef {
    pub agent_id: Option<String>,
    pub turn_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelHint {
    pub policy: Option<String>,
    pub hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GraphContext {
    #[serde(default)]
    pub episodic: Vec<String>,
    #[serde(default)]
    pub semantic: Vec<String>,
    #[serde(default)]
    pub procedural: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Constraints {
    #[serde(default)]
    pub json_schema: Option<serde_json::Value>,
    #[serde(default)]
    pub grammar: Option<String>,
    #[serde(default)]
    pub tool_contracts: Vec<ToolContract>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolContract {
    pub name: String,
    #[serde(default)]
    pub input_schema: Option<serde_json::Value>,
}

/// Map OpenAI Chat Completions `tools` array into internal [`ToolContract`] entries for the infer pipeline.
pub fn tool_contracts_from_openai_tools(tools: &[serde_json::Value]) -> Vec<ToolContract> {
    let mut out = Vec::new();
    for t in tools {
        let Some(obj) = t.as_object() else {
            continue;
        };
        if obj.get("type").and_then(|x| x.as_str()) != Some("function") {
            continue;
        }
        let Some(func) = obj.get("function").and_then(|x| x.as_object()) else {
            continue;
        };
        let Some(name) = func.get("name").and_then(|x| x.as_str()) else {
            continue;
        };
        let params = func
            .get("parameters")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({"type": "object"}));
        out.push(ToolContract {
            name: name.to_string(),
            input_schema: Some(params),
        });
    }
    out
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Policy {
    #[serde(default)]
    pub allow_tools: Vec<String>,
    #[serde(default)]
    pub deny_tools: Vec<String>,
    #[serde(default)]
    pub max_repair_attempts: Option<u32>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum BackendKind {
    LlamaCpp,
    Vllm,
    OpenRouter,
    Anthropic,
    OpenAi,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferResponse {
    pub request_id: Uuid,
    pub provider_trace_id: Uuid,
    pub backend: BackendInfo,
    pub output: InferOutput,
    pub validation: ValidationResult,
    #[serde(default)]
    pub usage: TokenUsage,
    #[serde(default)]
    pub decision_log: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendInfo {
    pub kind: BackendKind,
    pub model: String,
    pub latency_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InferOutput {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default)]
    pub structured: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub name: String,
    pub args: serde_json::Value,
}

/// Structured validation issue (machine-readable `code` + human `message`).
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ViolationDetail {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ValidationResult {
    pub schema_ok: bool,
    pub tool_calls_ok: bool,
    pub repair_attempts: u32,
    /// Human-readable messages (mirrors [`Self::violation_details`] for backward compatibility).
    #[serde(default)]
    pub violations: Vec<String>,
    #[serde(default)]
    pub violation_details: Vec<ViolationDetail>,
}

/// Stable decision-log entries (serialized to the same string tokens as before).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecisionLogCode {
    BackendSelected,
    ValidationRun,
    RepairAttempt(u32),
    RepairSucceeded,
    RepairExhausted,
}

impl DecisionLogCode {
    pub fn as_api_str(&self) -> String {
        match self {
            DecisionLogCode::BackendSelected => "backend_selected".into(),
            DecisionLogCode::ValidationRun => "validation_run".into(),
            DecisionLogCode::RepairAttempt(n) => format!("repair_attempt_{n}"),
            DecisionLogCode::RepairSucceeded => "repair_succeeded".into(),
            DecisionLogCode::RepairExhausted => "repair_exhausted".into(),
        }
    }

    pub fn to_strings(entries: &[DecisionLogCode]) -> Vec<String> {
        entries.iter().map(|e| e.as_api_str()).collect()
    }
}

/// Request body for `POST /armara/v1/tools/validate`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolsValidateRequest {
    #[serde(default)]
    pub constraints: Constraints,
    #[serde(default)]
    pub policy: Policy,
    /// Raw model output (JSON with `tool_calls` or plain text).
    pub output_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolsValidateResponse {
    pub validation: ValidationResult,
    #[serde(default)]
    pub decision_log: Vec<String>,
}

/// Request for `POST /armara/v1/ainl/compile-constraints` (delegates to `ainl serve`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompileConstraintsRequest {
    pub source: String,
    #[serde(default)]
    pub strict: bool,
    /// `validate` → `POST …/validate`; `compile` → `POST …/compile`.
    #[serde(default = "default_compile_mode")]
    pub mode: String,
}

fn default_compile_mode() -> String {
    "validate".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompileConstraintsResponse {
    pub ok: bool,
    #[serde(default)]
    pub result: serde_json::Value,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// Token estimate using the same ~4 characters per token heuristic as ArmaraOS prompt tooling
/// (conservative lower bound for display/budget; not a tokenizer).
#[inline]
pub fn estimate_tokens_from_char_count(chars: usize) -> u64 {
    (chars / 4).saturating_add(1) as u64
}

/// Rough prompt size from chat messages (role + content + small overhead per message).
pub fn estimate_prompt_tokens_from_messages(messages: &[ChatMessage]) -> u64 {
    let chars: usize = messages
        .iter()
        .map(|m| {
            m.role
                .len()
                .saturating_add(m.content.len())
                .saturating_add(8)
        })
        .sum();
    estimate_tokens_from_char_count(chars)
}

/// OpenAI-compatible chat completion (subset).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiChatRequest {
    pub model: String,
    pub messages: Vec<OpenAiMessage>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub temperature: Option<f32>,
    /// When true, response must be `text/event-stream` (OpenAI SSE). ArmaraOS chat uses this.
    #[serde(default)]
    pub stream: bool,
    /// OpenAI `tools` array — forwarded into [`Constraints::tool_contracts`] for the infer pipeline (llama.cpp / vLLM).
    #[serde(default)]
    pub tools: Option<Vec<serde_json::Value>>,
    /// Optional client `tool_choice`; the control plane may still apply `tool_choice: required` for AINL runs.
    #[serde(default)]
    pub tool_choice: Option<serde_json::Value>,
}

/// One tool definition from an OpenAI `tools` entry (`type` + `function`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiChatToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: OpenAiChatToolFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiChatToolFunction {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiMessage {
    pub role: String,
    #[serde(default)]
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<OpenAiChatToolCall>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

/// `usage` field on [`OpenAiChatResponse`] — matches OpenAI field names (`prompt_tokens`, etc.).
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct OpenAiUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiChatResponse {
    pub id: String,
    pub model: String,
    pub choices: Vec<OpenAiChoice>,
    /// Set by `ainl-inference-server` (non-zero when upstream omits counts, via heuristic estimates).
    #[serde(default)]
    pub usage: OpenAiUsage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiChoice {
    pub message: OpenAiMessage,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

/// Result of scanning assistant text for embedded `{"tool_calls":[...]}` JSON.
///
/// Smaller models (e.g. Gemma 4B Q4) frequently emit OpenAI-style tool calls
/// **inside the assistant message body** instead of in the structured
/// `tool_calls` field. The text variants seen in production include:
///
/// 1. JSON-only body: `{"tool_calls":[{"name":"web_search","args":{...}}]}`
/// 2. Multiple concatenated objects:
///    `{"tool_calls":[...]}{"tool_calls":[...]}Now I'll explain...`
/// 3. JSON followed by free prose (most common):
///    `{"tool_calls":[...]}\nBased on the search results, here is...`
/// 4. Markdown-fenced: ```json\n{"tool_calls":[...]}\n```
///
/// `extract_embedded_tool_calls` walks the input with brace-balance scanning
/// (so nested `{}` inside `args` don't trip it up), pulls every well-formed
/// `{"tool_calls": [...]}` object, lifts each `{name, args}` entry into
/// `ToolCall`, and returns the cleaned text with those JSON blobs removed.
#[derive(Debug, Default, Clone)]
pub struct EmbeddedToolCallsExtraction {
    pub tool_calls: Vec<ToolCall>,
    pub cleaned_text: String,
}

/// Find every `{"tool_calls":[...]}` JSON object inside `input` and lift its
/// entries into [`ToolCall`]s. Returns the input with those blobs stripped.
///
/// Designed to be conservative: only triggers on objects that genuinely have
/// a top-level `"tool_calls"` array — random `{ ... }` shapes (e.g. AINL
/// graph blocks, code samples) are left in the cleaned text.
pub fn extract_embedded_tool_calls(input: &str) -> EmbeddedToolCallsExtraction {
    let bytes = input.as_bytes();
    let mut tool_calls: Vec<ToolCall> = Vec::new();
    // Pairs of (start, end_exclusive) byte offsets to remove from the text.
    let mut strip_ranges: Vec<(usize, usize)> = Vec::new();

    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'{' {
            i += 1;
            continue;
        }

        // Brace-balance scan starting at `i` to find the matching close.
        // Tracks whether we're inside a JSON string literal so braces inside
        // strings don't confuse the depth counter.
        let mut depth: i32 = 0;
        let mut in_string = false;
        let mut escape = false;
        let mut end = None;
        for (j, &b) in bytes.iter().enumerate().skip(i) {
            if in_string {
                if escape {
                    escape = false;
                } else if b == b'\\' {
                    escape = true;
                } else if b == b'"' {
                    in_string = false;
                }
                continue;
            }
            match b {
                b'"' => in_string = true,
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(j + 1);
                        break;
                    }
                }
                _ => {}
            }
        }

        let Some(end) = end else {
            // Unbalanced — nothing more to do.
            break;
        };

        let candidate = &input[i..end];

        // Try to parse and confirm it carries a `tool_calls` array.
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(candidate) {
            if let Some(arr) = v.get("tool_calls").and_then(|x| x.as_array()) {
                let mut added = 0;
                for item in arr {
                    let Some(name) = item.get("name").and_then(|x| x.as_str()) else {
                        continue;
                    };
                    if name.is_empty() {
                        continue;
                    }
                    let args = item
                        .get("args")
                        .or_else(|| item.get("arguments"))
                        .cloned()
                        .unwrap_or_else(|| serde_json::json!({}));
                    // Some models emit `arguments` as a stringified JSON.
                    let args = if let serde_json::Value::String(s) = &args {
                        serde_json::from_str(s).unwrap_or_else(|_| serde_json::json!({}))
                    } else {
                        args
                    };
                    tool_calls.push(ToolCall {
                        name: name.to_string(),
                        args,
                    });
                    added += 1;
                }
                if added > 0 {
                    strip_ranges.push((i, end));
                    // Skip past the consumed object before scanning again.
                    i = end;
                    continue;
                }
            }
        }

        // Not a tool_calls object — advance by one byte (we may find one later).
        i += 1;
    }

    if tool_calls.is_empty() {
        return EmbeddedToolCallsExtraction {
            tool_calls,
            cleaned_text: input.to_string(),
        };
    }

    // Build the cleaned text by skipping the strip ranges. Also peel off
    // trailing markdown fences / whitespace adjacent to a stripped span so
    // ```json\n{"tool_calls":[...]}\n``` collapses cleanly.
    let mut cleaned = String::with_capacity(input.len());
    let mut cursor = 0;
    for (start, end) in &strip_ranges {
        // Walk back from `start` over whitespace + an optional ```json fence.
        let mut s = *start;
        while s > cursor {
            let c = bytes[s - 1];
            if c == b' ' || c == b'\t' || c == b'\n' || c == b'\r' {
                s -= 1;
            } else {
                break;
            }
        }
        // Optional ```json or ``` opener.
        for marker in ["```json", "```"] {
            if s >= cursor + marker.len() && &input[s - marker.len()..s] == marker {
                s -= marker.len();
                while s > cursor && matches!(bytes[s - 1], b' ' | b'\t' | b'\n' | b'\r') {
                    s -= 1;
                }
                break;
            }
        }
        // Walk forward from `end` over whitespace + an optional closing ```.
        let mut e = *end;
        while e < bytes.len() && matches!(bytes[e], b' ' | b'\t' | b'\n' | b'\r') {
            e += 1;
        }
        if e + 3 <= bytes.len() && &input[e..e + 3] == "```" {
            e += 3;
            while e < bytes.len() && matches!(bytes[e], b' ' | b'\t' | b'\n' | b'\r') {
                e += 1;
            }
        }
        cleaned.push_str(&input[cursor..s]);
        cursor = e;
    }
    cleaned.push_str(&input[cursor..]);

    EmbeddedToolCallsExtraction {
        tool_calls,
        cleaned_text: cleaned.trim().to_string(),
    }
}

/// Build an OpenAI-style assistant `message` from infer output (native `tool_calls` or legacy JSON in `text`).
pub fn openai_assistant_message_from_infer_output(output: &InferOutput) -> OpenAiMessage {
    let from_list = |calls: &[ToolCall]| -> Option<OpenAiMessage> {
        if calls.is_empty() {
            return None;
        }
        let mut oai_calls = Vec::new();
        for (i, c) in calls.iter().enumerate() {
            let args_str = serde_json::to_string(&c.args).unwrap_or_else(|_| "{}".to_string());
            oai_calls.push(OpenAiChatToolCall {
                id: format!("call_{i}"),
                call_type: "function".into(),
                function: OpenAiChatToolFunction {
                    name: c.name.clone(),
                    arguments: args_str,
                },
            });
        }
        Some(OpenAiMessage {
            role: "assistant".into(),
            content: String::new(),
            tool_calls: Some(oai_calls),
            tool_call_id: None,
        })
    };

    if let Some(m) = from_list(&output.tool_calls) {
        return m;
    }

    // Robust extraction: scan for any embedded `{"tool_calls":[...]}` JSON
    // (handles prefix/suffix prose, multiple concatenated objects, and
    // markdown fences). Replaces the previous strict full-string parse,
    // which only matched JSON-only bodies and silently fell through whenever
    // a small model added even a trailing newline of prose.
    let extraction = extract_embedded_tool_calls(&output.text);
    if !extraction.tool_calls.is_empty() {
        if let Some(mut m) = from_list(&extraction.tool_calls) {
            m.content = extraction.cleaned_text;
            return m;
        }
    }

    OpenAiMessage {
        role: "assistant".into(),
        content: output.text.clone(),
        tool_calls: None,
        tool_call_id: None,
    }
}

#[cfg(test)]
mod embedded_tool_call_tests {
    use super::*;

    #[test]
    fn extract_handles_json_followed_by_prose() {
        let input = "{\"tool_calls\":[{\"name\":\"web_search\",\"args\":{\"query\":\"apple stock price\",\"max_results\":3}}]}\nBased on the search results, here is the latest price.";
        let out = extract_embedded_tool_calls(input);
        assert_eq!(out.tool_calls.len(), 1);
        assert_eq!(out.tool_calls[0].name, "web_search");
        assert_eq!(out.tool_calls[0].args["query"], "apple stock price");
        assert!(
            out.cleaned_text.starts_with("Based on the search results"),
            "cleaned_text should drop the JSON prefix; got: {:?}",
            out.cleaned_text
        );
    }

    #[test]
    fn extract_handles_multiple_concatenated_blobs() {
        let input = "{\"tool_calls\":[{\"name\":\"file_write\",\"args\":{\"path\":\"a.ainl\",\"content\":\"x\"}}]}{\"tool_calls\":[{\"name\":\"file_write\",\"args\":{\"path\":\"b.ainl\",\"content\":\"y\"}}]}Trailing prose.";
        let out = extract_embedded_tool_calls(input);
        assert_eq!(out.tool_calls.len(), 2);
        assert_eq!(out.tool_calls[0].args["path"], "a.ainl");
        assert_eq!(out.tool_calls[1].args["path"], "b.ainl");
        assert_eq!(out.cleaned_text, "Trailing prose.");
    }

    #[test]
    fn extract_handles_markdown_fenced_json() {
        let input = "Here is the call:\n```json\n{\"tool_calls\":[{\"name\":\"mcp_ainl_ainl_validate\",\"args\":{\"path\":\"hello.ainl\"}}]}\n```\nThen we run it.";
        let out = extract_embedded_tool_calls(input);
        assert_eq!(out.tool_calls.len(), 1);
        assert_eq!(out.tool_calls[0].name, "mcp_ainl_ainl_validate");
        assert!(
            !out.cleaned_text.contains("```json"),
            "cleaned_text should drop the fence; got: {:?}",
            out.cleaned_text
        );
        assert!(out.cleaned_text.contains("Here is the call:"));
        assert!(out.cleaned_text.contains("Then we run it."));
    }

    #[test]
    fn extract_ignores_non_tool_call_braces() {
        let input = "Reply with {\"foo\": 1, \"bar\": [1,2,3]}. Also { not even json } here.";
        let out = extract_embedded_tool_calls(input);
        assert!(out.tool_calls.is_empty());
        assert_eq!(out.cleaned_text, input);
    }

    #[test]
    fn extract_handles_arguments_alias_and_stringified_args() {
        let input = "{\"tool_calls\":[{\"name\":\"file_read\",\"arguments\":\"{\\\"path\\\":\\\"x.txt\\\"}\"}]}";
        let out = extract_embedded_tool_calls(input);
        assert_eq!(out.tool_calls.len(), 1);
        assert_eq!(out.tool_calls[0].name, "file_read");
        assert_eq!(out.tool_calls[0].args["path"], "x.txt");
    }

    #[test]
    fn extract_handles_braces_inside_string_args() {
        let input = "{\"tool_calls\":[{\"name\":\"file_write\",\"args\":{\"path\":\"hello.ainl\",\"content\":\"hello:\\n  out \\\"{}\\\"\\n\"}}]}";
        let out = extract_embedded_tool_calls(input);
        assert_eq!(out.tool_calls.len(), 1);
        assert_eq!(out.tool_calls[0].args["path"], "hello.ainl");
    }

    #[test]
    fn openai_assistant_message_lifts_text_embedded_tool_calls() {
        let output = InferOutput {
            text: "{\"tool_calls\":[{\"name\":\"web_search\",\"args\":{\"query\":\"x\"}}]}\nDone."
                .into(),
            tool_calls: vec![],
            structured: None,
        };
        let msg = openai_assistant_message_from_infer_output(&output);
        let calls = msg.tool_calls.expect("expected tool_calls to be lifted");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "web_search");
        assert_eq!(msg.content, "Done.");
    }
}

#[cfg(test)]
mod decision_log_tests {
    use super::*;

    #[test]
    fn decision_log_codes_match_stable_api_strings() {
        assert_eq!(
            DecisionLogCode::BackendSelected.as_api_str(),
            "backend_selected"
        );
        assert_eq!(
            DecisionLogCode::ValidationRun.as_api_str(),
            "validation_run"
        );
        assert_eq!(
            DecisionLogCode::RepairAttempt(2).as_api_str(),
            "repair_attempt_2"
        );
        assert_eq!(
            DecisionLogCode::RepairSucceeded.as_api_str(),
            "repair_succeeded"
        );
        let v = DecisionLogCode::to_strings(&[
            DecisionLogCode::BackendSelected,
            DecisionLogCode::RepairAttempt(1),
        ]);
        assert_eq!(v, vec!["backend_selected", "repair_attempt_1"]);
    }
}

#[cfg(test)]
mod token_estimate_tests {
    use super::*;

    #[test]
    fn estimate_short_text() {
        assert_eq!(estimate_tokens_from_char_count(0), 1);
        assert_eq!(estimate_tokens_from_char_count(3), 1);
        assert_eq!(estimate_tokens_from_char_count(4), 2);
    }

    #[test]
    fn estimate_messages_non_empty() {
        let m = vec![ChatMessage {
            role: "user".into(),
            content: "hello".into(),
        }];
        assert!(estimate_prompt_tokens_from_messages(&m) >= 1);
    }
}
