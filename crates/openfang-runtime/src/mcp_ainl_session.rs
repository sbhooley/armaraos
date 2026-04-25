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
    let path = crate::graph_memory_writer::GraphMemoryWriter::sqlite_database_path_for_agent(
        agent_id,
    )
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
    let path = crate::graph_memory_writer::GraphMemoryWriter::sqlite_database_path_for_agent(
        agent_id,
    )
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
    let mut out = String::from("**AINL capabilities snapshot** (from last `mcp_ainl_ainl_capabilities` in this session)\n");
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
            format!("- `{aname}`: {first_verbs}{}\n", if verbs > 8 { " …" } else { "" })
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
}
