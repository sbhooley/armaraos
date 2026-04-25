//! Session cache + graph-memory snapshots for AINL MCP (`mcp_ainl_*`) prompt hints.
//!
//! - **In-process cache** (per chat [`SessionId`]): fast path; entries update only when content
//!   hashes change to avoid steady token burn.
//! - **SQLite graph** (`semantic` nodes, tags `mcp:ainl:capabilities` / `mcp:ainl:recommended_next`):
//!   survives daemon restarts; [`resolve_ainl_mcp_prompt_extras`] prefers DB over cache.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use ainl_memory::GraphMemory;
use hex::encode as hex_encode;
use openfang_types::agent::SessionId;
use serde_json::Value as JsonValue;
use sha2::{Digest, Sha256};

struct CacheEntry {
    caps_hash: Option<u64>,
    rec_hash: Option<u64>,
    caps_digest: Option<String>,
    rec_next: Option<String>,
    updated: Instant,
}

const STALE: Duration = Duration::from_secs(86_400);

const TAG_MCP_AINL_CAPABILITIES: &str = "mcp:ainl:capabilities";
const TAG_MCP_AINL_RECOMMENDED: &str = "mcp:ainl:recommended_next";

/// Flags set when in-memory cache **changed** (hash moved), so graph-memory can persist a new row.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct McpAinlApplyResult {
    pub new_capabilities_for_graph: bool,
    pub new_recommended_next_for_graph: bool,
}

static CACHE: OnceLock<Mutex<HashMap<uuid::Uuid, CacheEntry>>> = OnceLock::new();

fn map() -> &'static Mutex<HashMap<uuid::Uuid, CacheEntry>> {
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn prune_stale(locked: &mut HashMap<uuid::Uuid, CacheEntry>, now: Instant) {
    locked.retain(|_, v| now.duration_since(v.updated) < STALE);
}

fn short_hash64(s: &str) -> u64 {
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// First 8 bytes of SHA-256 as 16 hex chars (stable `v:` tag for semantic rows).
#[must_use]
pub fn content_sha16(s: &str) -> String {
    let d = Sha256::digest(s.as_bytes());
    hex_encode(&d[..8])
}

/// (capabilities digest, `recommended_next_tools` echo) for this session (cache only).
#[must_use]
pub fn session_prompt_extras(sid: SessionId) -> (Option<String>, Option<String>) {
    let now = Instant::now();
    let Ok(mut m) = map().lock() else {
        return (None, None);
    };
    prune_stale(&mut m, now);
    m.get(&sid.0)
        .map(|e| (e.caps_digest.clone(), e.rec_next.clone()))
        .unwrap_or((None, None))
}

/// Merge order: **graph memory** (persistent) first, then in-process **session cache** for fields still empty.
#[must_use]
pub fn resolve_ainl_mcp_prompt_extras(
    session_id: SessionId,
    agent_id: Option<&str>,
) -> (Option<String>, Option<String>) {
    let (mut cap, mut rec) = (None, None);
    if let Some(aid) = agent_id {
        if let Some(c) = read_capabilities_digest_from_graph(aid) {
            cap = Some(c);
        }
        if let Some(r) = read_recommended_next_from_graph(aid) {
            rec = Some(r);
        }
    }
    let (s_cap, s_rec) = session_prompt_extras(session_id);
    (cap.or(s_cap), rec.or(s_rec))
}

/// Read the latest `mcp:ainl:capabilities` semantic fact for `agent_id`, if the DB exists.
#[must_use]
pub fn read_capabilities_digest_from_graph(agent_id: &str) -> Option<String> {
    let path =
        crate::graph_memory_writer::GraphMemoryWriter::sqlite_database_path_for_agent(agent_id)
            .ok()?;
    if !path.is_file() {
        return None;
    }
    let mem = GraphMemory::new(&path).ok()?;
    let n = mem
        .sqlite_store()
        .query(agent_id)
        .latest_semantic_with_tag(TAG_MCP_AINL_CAPABILITIES)
        .ok()??;
    n.semantic().map(|s| s.fact.clone())
}

/// Read the latest `mcp:ainl:recommended_next` semantic fact.
#[must_use]
pub fn read_recommended_next_from_graph(agent_id: &str) -> Option<String> {
    let path =
        crate::graph_memory_writer::GraphMemoryWriter::sqlite_database_path_for_agent(agent_id)
            .ok()?;
    if !path.is_file() {
        return None;
    }
    let mem = GraphMemory::new(&path).ok()?;
    let n = mem
        .sqlite_store()
        .query(agent_id)
        .latest_semantic_with_tag(TAG_MCP_AINL_RECOMMENDED)
        .ok()??;
    n.semantic().map(|s| s.fact.clone())
}

fn upsert(sid: SessionId, f: impl FnOnce(&mut CacheEntry)) {
    let now = Instant::now();
    let Ok(mut m) = map().lock() else {
        return;
    };
    prune_stale(&mut m, now);
    let e = m.entry(sid.0).or_insert_with(|| CacheEntry {
        caps_hash: None,
        rec_hash: None,
        caps_digest: None,
        rec_next: None,
        updated: now,
    });
    f(e);
    e.updated = now;
}

/// Format a small digest from the JSON returned by the `ainl_capabilities` MCP tool.
#[must_use]
pub fn format_capabilities_digest(v: &JsonValue) -> Option<String> {
    let adapters = v.get("adapters")?.as_object()?;
    let mut names: Vec<&str> = adapters.keys().map(|s| s.as_str()).collect();
    names.sort_unstable();
    let mcp_res = v
        .get("mcp_resources")
        .and_then(|x| x.as_array())
        .map(|a| a.as_slice())
        .unwrap_or(&[]);
    const MAX: usize = 1_800;
    let mut out = String::from(
        "**AINL capabilities snapshot** (from last `mcp_ainl_ainl_capabilities` in this session)\n",
    );
    out.push_str("Adapters (strict / `R` lines): ");
    for (i, n) in names.iter().take(32).enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        if out.len() > MAX {
            out.push('…');
            return Some(out);
        }
        out.push_str(n);
    }
    if names.len() > 32 {
        out.push_str(&format!(" … (+{} more)", names.len().saturating_sub(32)));
    }
    out.push('\n');

    for aname in names.iter().take(16) {
        let Some(obj) = adapters.get(*aname).and_then(|x| x.as_object()) else {
            continue;
        };
        let verbs = obj
            .get("verbs")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        let first_verbs: String = obj
            .get("verbs")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str())
                    .take(8)
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        let line = if first_verbs.is_empty() {
            format!("- `{aname}`: {verbs} verbs\n")
        } else {
            format!(
                "- `{aname}`: {first_verbs}{}\n",
                if verbs > 8 { " …" } else { "" }
            )
        };
        if out.len() + line.len() > MAX {
            out.push_str("…\n");
            return Some(out);
        }
        out.push_str(&line);
    }
    if !mcp_res.is_empty() {
        out.push_str("MCP resource URIs (read via `mcp_resource_read`):\n");
        for e in mcp_res.iter().take(12) {
            let uri = e.get("uri").and_then(|u| u.as_str()).unwrap_or("?");
            let title = e.get("title").and_then(|t| t.as_str()).unwrap_or("");
            let line = format!("- `{uri}` — {title}\n");
            if out.len() + line.len() > MAX {
                out.push('…');
                return Some(out);
            }
            out.push_str(&line);
        }
    }
    if out.len() < 80 {
        return None;
    }
    Some(out)
}

/// Format `recommended_next_tools` for a short system-prompt echo (bounded size).
#[must_use]
pub fn format_recommended_next_tools_echo(v: &JsonValue) -> Option<String> {
    let t = v.get("recommended_next_tools")?.clone();
    let s = serde_json::to_string_pretty(&t).ok()?;
    const MAX: usize = 900;
    if s.len() <= MAX {
        return Some(format!(
            "**Last AINL MCP `recommended_next_tools` (this session)**\n```json\n{s}\n```"
        ));
    }
    let cut: String = s.chars().take(MAX).collect();
    Some(format!(
        "**Last AINL MCP `recommended_next_tools` (this session, truncated)**\n```json\n{cut}…\n```"
    ))
}

/// Detect a **soft failure** from an `mcp_ainl_*` tool: the MCP wire call succeeded (HTTP 200,
/// well-formed JSON) but the JSON body itself reports `ok: false`. Examples include
/// `ainl_validate` returning `{"ok": false, "errors": [...]}` for invalid AINL syntax,
/// `ainl_compile` returning compile errors, or `ainl_run` rejecting the program before execution.
///
/// We treat these as **tool errors** at the runtime layer so:
///
/// 1. The LLM sees an explicit failure (no risk of confabulating a successful run after a failed
///    `ainl_validate`).
/// 2. [`crate::loop_guard`] / [`crate::graph_memory_learning::LearningRecorder`] capture the
///    failure into the persistent failure store (FTS-recallable for future turns).
/// 3. The tool snapshot path ([`on_mcp_ainl_tool_result`]) does *not* poison the capabilities /
///    `recommended_next_tools` cache with a failure body.
///
/// Returns `Some(model_readable_message)` when soft failure is detected, otherwise `None`.
/// The message is bounded in size and ends with an explicit instruction to fix and re-validate.
///
/// Scoped to `mcp_ainl_*` tools because the `{ok, errors}` envelope is the documented AINL MCP
/// wire shape; other MCP servers do not follow this convention reliably.
#[must_use]
pub fn ainl_mcp_soft_failure_message(tool_name: &str, content: &str) -> Option<String> {
    if !tool_name.starts_with("mcp_ainl_") {
        return None;
    }

    const MAX_ERRORS: usize = 20;
    const MAX_REPAIR_STEPS: usize = 10;
    const MAX_PRIMARY_CHARS: usize = 600;
    const MAX_TOTAL_CHARS: usize = 4_000;

    let v: JsonValue = match serde_json::from_str(content) {
        Ok(v) => v,
        Err(_) => {
            if tool_name == "mcp_ainl_ainl_capabilities" {
                return None;
            }
            let preview: String = content.chars().take(800).collect();
            return Some(format!(
                "AINL MCP tool `{tool_name}` returned a non-JSON body, so the runtime cannot \
                 prove the operation succeeded. Treat this as a tool failure; do not claim success.\
                 \nbody_preview: {preview}\n\nNext step: retry the AINL MCP call or fix the source/tool arguments, then continue only after the tool returns a valid `ok: true` JSON envelope."
            ));
        }
    };
    match v.get("ok").and_then(|ok| ok.as_bool()) {
        Some(true) => return None,
        Some(false) => {}
        None => {
            if tool_name == "mcp_ainl_ainl_capabilities" {
                return None;
            }
            let preview = serde_json::to_string(&v).unwrap_or_else(|_| content.to_string());
            let preview: String = preview.chars().take(800).collect();
            return Some(format!(
                "AINL MCP tool `{tool_name}` returned JSON without a boolean `ok` field, so the \
                 runtime cannot prove the operation succeeded. Treat this as a tool failure; do \
                 not claim success.\nbody_preview: {preview}\n\nNext step: retry the AINL MCP call or fix the source/tool arguments, then continue only after the tool returns `ok: true`."
            ));
        }
    }

    let header = match tool_name {
        "mcp_ainl_ainl_validate" => "ainl_validate reports the AINL source is INVALID",
        "mcp_ainl_ainl_compile" => "ainl_compile failed to compile the AINL source",
        "mcp_ainl_ainl_run" => "ainl_run failed before execution (compile/policy/runtime error)",
        "mcp_ainl_ainl_security_report" => "ainl_security_report failed (invalid AINL source)",
        "mcp_ainl_ainl_ir_diff" => "ainl_ir_diff failed (one or both files have errors)",
        "mcp_ainl_ainl_ptc_signature_check" => "ainl_ptc_signature_check failed (invalid source)",
        _ => "AINL MCP tool reported ok: false",
    };

    let mut msg = String::with_capacity(512);
    msg.push_str(header);
    msg.push_str(
        ". You MUST fix the AINL source and call ainl_validate again until ok: true \
         before claiming the workflow ran or any other follow-up. Do NOT report success \
         on top of these errors.\n",
    );

    if let Some(s) = v.get("error").and_then(|e| e.as_str()) {
        msg.push_str("error: ");
        msg.push_str(s);
        msg.push('\n');
    }
    if let Some(s) = v.get("details").and_then(|e| e.as_str()) {
        msg.push_str("details: ");
        msg.push_str(s);
        msg.push('\n');
    }
    push_string_array(&mut msg, "errors", v.get("errors"), MAX_ERRORS);
    push_string_array(
        &mut msg,
        "policy_errors",
        v.get("policy_errors"),
        MAX_ERRORS,
    );
    push_string_array(&mut msg, "file1_errors", v.get("file1_errors"), MAX_ERRORS);
    push_string_array(&mut msg, "file2_errors", v.get("file2_errors"), MAX_ERRORS);

    if let Some(primary) = v.get("primary_diagnostic") {
        if let Ok(s) = serde_json::to_string(primary) {
            if s.len() > MAX_PRIMARY_CHARS {
                let cut: String = s.chars().take(MAX_PRIMARY_CHARS).collect();
                msg.push_str(&format!("primary_diagnostic: {cut}…\n"));
            } else {
                msg.push_str(&format!("primary_diagnostic: {s}\n"));
            }
        }
    }
    push_string_array(
        &mut msg,
        "agent_repair_steps",
        v.get("agent_repair_steps"),
        MAX_REPAIR_STEPS,
    );

    msg.push_str(
        "\nNext step: edit the AINL source to address the errors above, then re-run \
         ainl_validate. Only after ainl_validate returns ok: true may you proceed to \
         ainl_compile / ainl_run.",
    );

    if msg.len() > MAX_TOTAL_CHARS {
        msg = msg.chars().take(MAX_TOTAL_CHARS).collect();
        msg.push('…');
    }

    Some(msg)
}

fn push_string_array(out: &mut String, label: &str, v: Option<&JsonValue>, cap: usize) {
    let Some(arr) = v.and_then(|x| x.as_array()) else {
        return;
    };
    if arr.is_empty() {
        return;
    }
    out.push_str(label);
    out.push_str(":\n");
    for (i, e) in arr.iter().take(cap).enumerate() {
        if let Some(s) = e.as_str() {
            out.push_str(&format!("  {}. {s}\n", i + 1));
        } else if let Ok(s) = serde_json::to_string(e) {
            out.push_str(&format!("  {}. {s}\n", i + 1));
        }
    }
    if arr.len() > cap {
        out.push_str(&format!("  … (+{} more)\n", arr.len() - cap));
    }
}

/// Called from the agent loop after a successful `mcp_ainl_*` tool (non-error result body).
/// Updates the session cache only when **content hashes** change.
#[must_use]
pub fn on_mcp_ainl_tool_result(
    session_id: SessionId,
    tool_name: &str,
    content: &str,
) -> McpAinlApplyResult {
    let mut out = McpAinlApplyResult::default();
    if !tool_name.starts_with("mcp_ainl_") {
        return out;
    }
    let Ok(v) = serde_json::from_str::<JsonValue>(content) else {
        return out;
    };

    if tool_name == "mcp_ainl_ainl_capabilities" {
        if let Some(d) = format_capabilities_digest(&v) {
            let h = short_hash64(content);
            let mut changed = false;
            upsert(session_id, |e| {
                if e.caps_hash != Some(h) {
                    e.caps_hash = Some(h);
                    e.caps_digest = Some(d);
                    changed = true;
                }
            });
            if changed {
                out.new_capabilities_for_graph = true;
            }
        }
    }

    if v.get("recommended_next_tools").is_some() {
        if let Some(echo) = format_recommended_next_tools_echo(&v) {
            let h = short_hash64(&echo);
            let mut changed = false;
            upsert(session_id, |e| {
                if e.rec_hash != Some(h) {
                    e.rec_hash = Some(h);
                    e.rec_next = Some(echo);
                    changed = true;
                }
            });
            if changed {
                out.new_recommended_next_for_graph = true;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn digest_from_capabilities_shape() {
        let v = serde_json::json!({
            "adapters": {
                "http": { "verbs": ["GET", "POST"] },
                "core": { "verbs": ["ADD"] }
            },
            "mcp_resources": [{"uri": "ainl://x", "title": "Test"}]
        });
        let d = format_capabilities_digest(&v).expect("digest");
        assert!(d.contains("http"));
        assert!(d.contains("ainl://x"));
    }

    #[test]
    fn on_tool_skips_graph_flag_when_idempotent() {
        let sid = SessionId(Uuid::new_v4());
        let body = r#"{"adapters":{"http":{"verbs":["GET","POST"]},"core":{"verbs":["ADD"]}},"mcp_resources":[{"uri":"ainl://x","title":"T"}]}"#;
        let r1 = on_mcp_ainl_tool_result(sid, "mcp_ainl_ainl_capabilities", body);
        assert!(r1.new_capabilities_for_graph);
        let r2 = on_mcp_ainl_tool_result(sid, "mcp_ainl_ainl_capabilities", body);
        assert!(!r2.new_capabilities_for_graph);
    }

    #[test]
    fn soft_failure_validate_ok_false_returns_message() {
        let body = r#"{
            "ok": false,
            "errors": [
                "Line 10: label-only op 'Loop' used at top-level",
                "Line 13: Label '_analyze': node 'n1' uses unknown adapter.verb 'llm.CHAT'"
            ],
            "warnings": [],
            "primary_diagnostic": {"line": 10, "kind": "structural"}
        }"#;
        let m = ainl_mcp_soft_failure_message("mcp_ainl_ainl_validate", body)
            .expect("must detect ok:false");
        assert!(m.contains("INVALID"), "header should call out invalid AINL");
        assert!(m.contains("Line 10"), "errors must be present");
        assert!(m.contains("llm.CHAT"), "all errors must be included");
        assert!(m.contains("primary_diagnostic"), "primary diag must echo");
        assert!(
            m.contains("ainl_validate again"),
            "must instruct retry-until-ok"
        );
    }

    #[test]
    fn soft_failure_returns_none_when_ok_true() {
        let body = r#"{"ok": true, "errors": [], "warnings": []}"#;
        assert!(ainl_mcp_soft_failure_message("mcp_ainl_ainl_validate", body).is_none());
    }

    #[test]
    fn soft_failure_returns_none_for_non_ainl_tools() {
        // Non-AINL MCP tools may have unrelated `ok` semantics; do not transform.
        let body = r#"{"ok": false, "errors": ["x"]}"#;
        assert!(ainl_mcp_soft_failure_message("mcp_other_thing", body).is_none());
        assert!(ainl_mcp_soft_failure_message("file_read", body).is_none());
    }

    #[test]
    fn soft_failure_treats_non_json_ainl_body_as_error() {
        let m = ainl_mcp_soft_failure_message("mcp_ainl_ainl_validate", "not json")
            .expect("non-json AINL MCP body is unsafe");
        assert!(m.contains("non-JSON"));
        assert!(m.contains("do not claim success"));
    }

    #[test]
    fn soft_failure_returns_none_when_capabilities_ok_field_missing() {
        // `ainl_capabilities` body has no `ok` field; must not be flagged.
        let body = r#"{"adapters": {}, "mcp_resources": []}"#;
        assert!(ainl_mcp_soft_failure_message("mcp_ainl_ainl_capabilities", body).is_none());
    }

    #[test]
    fn soft_failure_treats_missing_ok_on_authoring_tool_as_error() {
        let body = r#"{"errors":[],"warnings":[]}"#;
        let m = ainl_mcp_soft_failure_message("mcp_ainl_ainl_validate", body)
            .expect("missing ok is unsafe for authoring tools");
        assert!(m.contains("without a boolean `ok` field"));
        assert!(m.contains("do not claim success"));
    }

    #[test]
    fn soft_failure_run_uses_singular_error_field() {
        let body = r#"{
            "ok": false,
            "trace_id": "abc",
            "error": "policy_violation",
            "policy_errors": ["adapter http not granted"]
        }"#;
        let m = ainl_mcp_soft_failure_message("mcp_ainl_ainl_run", body).expect("ok:false");
        assert!(m.contains("policy_violation"));
        assert!(m.contains("policy_errors"));
        assert!(m.contains("adapter http not granted"));
    }

    #[test]
    fn soft_failure_truncates_excessively_large_error_lists() {
        let mut errors: Vec<serde_json::Value> = (0..50)
            .map(|i| serde_json::Value::String(format!("err {i}")))
            .collect();
        errors.push(serde_json::json!("tail"));
        let v = serde_json::json!({"ok": false, "errors": errors});
        let body = serde_json::to_string(&v).unwrap();
        let m = ainl_mcp_soft_failure_message("mcp_ainl_ainl_validate", &body).expect("ok:false");
        assert!(m.contains("more)"), "must report truncated tail count");
        assert!(m.len() < 5_000, "must respect MAX_TOTAL_CHARS bound");
    }
}
