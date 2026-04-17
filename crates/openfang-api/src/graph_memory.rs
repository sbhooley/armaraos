//! Graph-memory API for dashboard visualization and governance actions.
//!
//! Endpoints:
//! - `GET /api/graph-memory`
//! - `GET /api/graph-memory/snapshots`
//! - `GET /api/graph-memory/snapshot-graph`
//! - `GET /api/graph-memory/audit`
//! - `POST /api/graph-memory/snapshot`
//! - `POST /api/graph-memory/rollback`
//! - `POST /api/graph-memory/reset`
//! - `POST /api/graph-memory/delete-node`

use ainl_memory::{AinlMemoryNode, AinlNodeType, GraphMemory};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::Json;
use rusqlite::OptionalExtension;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::routes::AppState;

#[derive(serde::Deserialize)]
pub struct GraphMemoryQuery {
    pub agent_id: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default = "default_since_seconds")]
    pub since_seconds: i64,
}

#[derive(serde::Deserialize)]
pub struct GraphMemorySnapshotsQuery {
    pub agent_id: String,
}

#[derive(serde::Deserialize)]
pub struct GraphMemorySnapshotGraphQuery {
    pub agent_id: String,
    pub snapshot_id: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default = "default_since_seconds")]
    pub since_seconds: i64,
}

#[derive(serde::Deserialize)]
pub struct GraphMemoryAuditQuery {
    pub agent_id: String,
    #[serde(default = "default_audit_limit")]
    pub limit: usize,
}

#[derive(serde::Deserialize)]
pub struct GraphMemorySnapshotRequest {
    pub agent_id: String,
    pub label: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct GraphMemoryRollbackRequest {
    pub agent_id: String,
    pub snapshot_id: String,
    pub reason: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct GraphMemoryResetRequest {
    pub agent_id: String,
    pub reason: Option<String>,
    pub create_snapshot: Option<bool>,
}

#[derive(serde::Deserialize)]
pub struct GraphMemoryDeleteNodeRequest {
    pub agent_id: String,
    pub node_id: String,
    pub reason: Option<String>,
}

const fn default_limit() -> usize {
    200
}

const fn default_since_seconds() -> i64 {
    7_776_000 // 90 days
}

const fn default_audit_limit() -> usize {
    100
}

#[derive(Serialize)]
struct GraphMemoryNodeOut {
    id: String,
    kind: &'static str,
    label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    strength: Option<f32>,
    created_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    vitals_gate: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    vitals_phase: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    vitals_trust: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    meta: Option<Value>,
}

#[derive(Serialize)]
struct GraphMemoryEdgeOut {
    source: String,
    target: String,
    rel: String,
}

fn node_kind(row_type: &str) -> &'static str {
    match row_type {
        "episode" => "episode",
        "semantic" => "semantic",
        "procedural" => "procedural",
        "persona" => "persona",
        "runtime_state" => "runtime_state",
        _ => "semantic",
    }
}

fn id_prefix(id: impl std::fmt::Display) -> String {
    id.to_string()
        .chars()
        .filter(|c| *c != '-')
        .take(8)
        .collect()
}

fn now_unix_ms() -> u64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_millis() as u64,
        Err(_) => 0,
    }
}

fn sanitize_agent_id(raw: &str) -> Result<String, String> {
    let s = raw.trim();
    if s.is_empty() {
        return Err("agent_id is required".to_string());
    }
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        Ok(s.to_string())
    } else {
        Err("agent_id contains unsupported characters".to_string())
    }
}

fn graph_db_path(home_dir: &Path, agent_id: &str) -> PathBuf {
    home_dir
        .join("agents")
        .join(agent_id)
        .join("ainl_memory.db")
}

fn governance_dir(home_dir: &Path, agent_id: &str) -> PathBuf {
    home_dir.join("agents").join(agent_id).join(".graph-memory")
}

fn snapshots_dir(home_dir: &Path, agent_id: &str) -> PathBuf {
    governance_dir(home_dir, agent_id).join("snapshots")
}

fn audit_log_path(home_dir: &Path, agent_id: &str) -> PathBuf {
    governance_dir(home_dir, agent_id).join("audit.jsonl")
}

fn sanitize_label(raw: &str) -> String {
    let mut out = String::new();
    for c in raw.chars() {
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            out.push(c);
        }
    }
    if out.is_empty() {
        "snapshot".to_string()
    } else {
        out
    }
}

fn sanitize_snapshot_id(raw: &str) -> Result<String, String> {
    let s = raw.trim();
    if s.is_empty() {
        return Err("snapshot_id is required".to_string());
    }
    if s.contains('/') || s.contains('\\') || s.contains("..") {
        return Err("snapshot_id is invalid".to_string());
    }
    Ok(s.to_string())
}

fn append_audit(
    home_dir: &Path,
    agent_id: &str,
    action: &str,
    detail: Value,
) -> Result<(), String> {
    let dir = governance_dir(home_dir, agent_id);
    std::fs::create_dir_all(&dir).map_err(|e| format!("create governance dir: {e}"))?;
    let path = audit_log_path(home_dir, agent_id);
    let mut line = json!({
        "ts_ms": now_unix_ms(),
        "agent_id": agent_id,
        "action": action,
        "detail": detail,
    })
    .to_string();
    line.push('\n');
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("open audit log: {e}"))?;
    f.write_all(line.as_bytes())
        .map_err(|e| format!("write audit log: {e}"))
}

static GRAPH_MEMORY_AGENT_LOCKS: OnceLock<
    std::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
> = OnceLock::new();

fn lock_map() -> &'static std::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>> {
    GRAPH_MEMORY_AGENT_LOCKS.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

async fn acquire_agent_write_lock(agent_id: &str) -> tokio::sync::OwnedMutexGuard<()> {
    let agent_key = agent_id.to_string();
    let m = {
        let mut map = lock_map().lock().expect("graph memory lock map poisoned");
        map.entry(agent_key)
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    };
    m.lock_owned().await
}

fn open_conn_with_fk(path: &Path) -> Result<rusqlite::Connection, String> {
    let conn = rusqlite::Connection::open(path).map_err(|e| e.to_string())?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .map_err(|e| e.to_string())?;
    Ok(conn)
}

fn copy_db_with_sidecars(src_db: &Path, dst_db: &Path) -> Result<(), String> {
    if let Some(parent) = dst_db.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create snapshot dir: {e}"))?;
    }
    if dst_db.exists() {
        std::fs::remove_file(dst_db).map_err(|e| format!("remove existing snapshot db: {e}"))?;
    }
    std::fs::copy(src_db, dst_db).map_err(|e| format!("copy snapshot db: {e}"))?;
    for suffix in ["-wal", "-shm"] {
        let src_side = PathBuf::from(format!("{}{}", src_db.display(), suffix));
        let dst_side = PathBuf::from(format!("{}{}", dst_db.display(), suffix));
        if src_side.exists() {
            let _ = std::fs::remove_file(&dst_side);
            std::fs::copy(&src_side, &dst_side)
                .map_err(|e| format!("copy snapshot sidecar {suffix}: {e}"))?;
        } else if dst_side.exists() {
            let _ = std::fs::remove_file(&dst_side);
        }
    }
    Ok(())
}

fn count_nodes_by_kind(conn: &rusqlite::Connection) -> Value {
    let mut out = json!({
        "episode": 0_u64,
        "semantic": 0_u64,
        "procedural": 0_u64,
        "persona": 0_u64,
        "runtime_state": 0_u64,
        "total": 0_u64
    });
    if let Ok(mut stmt) =
        conn.prepare("SELECT node_type, COUNT(*) FROM ainl_graph_nodes GROUP BY node_type")
    {
        if let Ok(rows) = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        }) {
            let mut total = 0_u64;
            for row in rows.flatten() {
                let (k, c) = row;
                let c_u = c.max(0) as u64;
                out[k.as_str()] = json!(c_u);
                total += c_u;
            }
            out["total"] = json!(total);
        }
    }
    out
}

type NodeLabelTuple = (
    String,
    Option<f32>,
    Option<String>,
    Option<String>,
    Option<f32>,
    Option<Value>,
);

/// Returns `(label, strength, vitals_gate, vitals_phase, vitals_trust, meta)`.
fn label_for_node(node: &AinlMemoryNode) -> NodeLabelTuple {
    let id_short = id_prefix(node.id);
    match &node.node_type {
        AinlNodeType::Episode { episodic } => {
            let mut s = if let Some(to) = &episodic.delegation_to {
                format!("Delegated to {to} · ")
            } else {
                String::new()
            };
            if episodic.tool_calls.is_empty() {
                s.push_str(&format!("Episode {id_short}"));
            } else {
                s.push_str(&episodic.tool_calls.join(", "));
            }
            let s = if s.chars().count() > 60 {
                format!("{}…", s.chars().take(60).collect::<String>())
            } else {
                s
            };
            (
                s,
                None,
                episodic.vitals_gate.clone(),
                episodic.vitals_phase.clone(),
                episodic.vitals_trust,
                Some(json!({
                    "turn_id": episodic.turn_id.to_string(),
                    "tool_calls": episodic.tool_calls,
                    "delegation_to": episodic.delegation_to,
                    "persona_signals_emitted": episodic.persona_signals_emitted,
                    "tags": episodic.tags,
                    "conversation_id": episodic.conversation_id,
                    "follows_episode_id": episodic.follows_episode_id,
                })),
            )
        }
        AinlNodeType::Semantic { semantic } => {
            let label = if semantic.fact.is_empty() {
                format!("Fact {id_short}")
            } else {
                semantic.fact.clone()
            };
            (
                label,
                None,
                None,
                None,
                None,
                Some(json!({
                    "fact": semantic.fact,
                    "confidence": semantic.confidence,
                    "topic_cluster": semantic.topic_cluster,
                    "source_episode_id": semantic.source_episode_id,
                    "recurrence_count": semantic.recurrence_count,
                })),
            )
        }
        AinlNodeType::Procedural { procedural } => {
            let label = if procedural.pattern_name.is_empty() {
                format!("Pattern {id_short}")
            } else {
                procedural.pattern_name.clone()
            };
            (
                label,
                None,
                None,
                None,
                None,
                Some(json!({
                    "pattern_name": procedural.pattern_name,
                    "tool_sequence": procedural.tool_sequence,
                    "confidence": procedural.confidence,
                    "success_rate": procedural.success_rate,
                    "procedure_type": procedural.procedure_type,
                    "trace_id": procedural.trace_id,
                })),
            )
        }
        AinlNodeType::Persona { persona } => (
            format!("{} ({:.2})", persona.trait_name, persona.strength),
            Some(persona.strength),
            None,
            None,
            None,
            Some(json!({
                "trait_name": persona.trait_name,
                "strength": persona.strength,
                "source": persona.source,
                "dominant_axes": persona.dominant_axes,
                "layer": persona.layer,
            })),
        ),
        AinlNodeType::RuntimeState { runtime_state } => (
            format!(
                "Runtime state · turns {} · last extract {}",
                runtime_state.turn_count, runtime_state.last_extraction_at_turn
            ),
            None,
            None,
            None,
            None,
            Some(json!({
                "turn_count": runtime_state.turn_count,
                "last_extraction_at_turn": runtime_state.last_extraction_at_turn,
            })),
        ),
    }
}

fn normalize_rel(label: &str) -> String {
    match label {
        "learned_from" => "learned_from".to_string(),
        "delegated_to" => "delegated_to".to_string(),
        "follows" => "follows".to_string(),
        "caused" | "caused_by" => "caused".to_string(),
        _ => "related".to_string(),
    }
}

fn load_graph_from_db(path: &Path, limit: usize, since_seconds: i64) -> Value {
    let limit = limit.clamp(1, 2000);
    if !path.exists() {
        return json!({ "nodes": [], "edges": [] });
    }
    let Ok(conn) = rusqlite::Connection::open(path) else {
        return json!({ "nodes": [], "edges": [] });
    };
    let since_ts = chrono::Utc::now().timestamp() - since_seconds;

    let Ok(mut stmt) = conn.prepare(
        "SELECT id, node_type, payload, timestamp FROM ainl_graph_nodes
         WHERE timestamp >= ?1
         ORDER BY timestamp DESC
         LIMIT ?2",
    ) else {
        return json!({ "nodes": [], "edges": [] });
    };

    let rows = match stmt.query_map(rusqlite::params![since_ts, limit as i64], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?,
        ))
    }) {
        Ok(r) => r,
        Err(_) => return json!({ "nodes": [], "edges": [] }),
    };

    let mut nodes_out: Vec<GraphMemoryNodeOut> = Vec::new();
    let mut id_set: HashSet<String> = HashSet::new();

    for row in rows.flatten() {
        let (id, node_type, payload, ts) = row;
        let Ok(node) = serde_json::from_str::<AinlMemoryNode>(&payload) else {
            continue;
        };
        let (label, strength, vitals_gate, vitals_phase, vitals_trust, meta) =
            label_for_node(&node);
        id_set.insert(id.clone());
        nodes_out.push(GraphMemoryNodeOut {
            id,
            kind: node_kind(node_type.as_str()),
            label,
            strength,
            created_at: ts,
            vitals_gate,
            vitals_phase,
            vitals_trust,
            meta,
        });
    }

    let mut edges_out: Vec<GraphMemoryEdgeOut> = Vec::new();
    if !id_set.is_empty() {
        if let Ok(mut estmt) = conn.prepare("SELECT from_id, to_id, label FROM ainl_graph_edges") {
            if let Ok(erows) = estmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            }) {
                for er in erows.flatten() {
                    let (from_id, to_id, label) = er;
                    if id_set.contains(&from_id) && id_set.contains(&to_id) {
                        edges_out.push(GraphMemoryEdgeOut {
                            source: from_id,
                            target: to_id,
                            rel: normalize_rel(&label),
                        });
                    }
                }
            }
        }
    }

    json!({
        "nodes": nodes_out,
        "edges": edges_out,
    })
}

/// GET /api/graph-memory?agent_id=…&limit=…&since_seconds=…
pub async fn get_graph_memory(
    State(state): State<Arc<AppState>>,
    Query(q): Query<GraphMemoryQuery>,
) -> Json<Value> {
    let Ok(agent_id) = sanitize_agent_id(&q.agent_id) else {
        return Json(json!({ "nodes": [], "edges": [] }));
    };
    let path = graph_db_path(&state.kernel.config.home_dir, &agent_id);
    Json(load_graph_from_db(&path, q.limit, q.since_seconds))
}

/// GET /api/graph-memory/snapshot-graph?agent_id=…&snapshot_id=…
pub async fn get_graph_memory_snapshot_graph(
    State(state): State<Arc<AppState>>,
    Query(q): Query<GraphMemorySnapshotGraphQuery>,
) -> Json<Value> {
    let Ok(agent_id) = sanitize_agent_id(&q.agent_id) else {
        return Json(json!({ "nodes": [], "edges": [] }));
    };
    let Ok(snapshot_id) = sanitize_snapshot_id(&q.snapshot_id) else {
        return Json(json!({ "nodes": [], "edges": [] }));
    };
    let path = snapshots_dir(&state.kernel.config.home_dir, &agent_id).join(snapshot_id);
    Json(load_graph_from_db(&path, q.limit, q.since_seconds))
}

/// GET /api/graph-memory/snapshots?agent_id=…
pub async fn get_graph_memory_snapshots(
    State(state): State<Arc<AppState>>,
    Query(q): Query<GraphMemorySnapshotsQuery>,
) -> Json<Value> {
    let Ok(agent_id) = sanitize_agent_id(&q.agent_id) else {
        return Json(json!({ "snapshots": [] }));
    };
    let dir = snapshots_dir(&state.kernel.config.home_dir, &agent_id);
    let mut snapshots = Vec::<Value>::new();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for ent in rd.flatten() {
            let p = ent.path();
            if p.extension().and_then(|e| e.to_str()) != Some("db") {
                continue;
            }
            let Ok(md) = ent.metadata() else { continue };
            let id = match p.file_name().and_then(|s| s.to_str()) {
                Some(v) => v.to_string(),
                None => continue,
            };
            let created_ms = md
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            snapshots.push(json!({
                "id": id,
                "size_bytes": md.len(),
                "created_ms": created_ms,
            }));
        }
    }
    snapshots.sort_by(|a, b| {
        let am = a.get("created_ms").and_then(|v| v.as_u64()).unwrap_or(0);
        let bm = b.get("created_ms").and_then(|v| v.as_u64()).unwrap_or(0);
        bm.cmp(&am)
    });
    Json(json!({ "snapshots": snapshots }))
}

/// GET /api/graph-memory/audit?agent_id=…&limit=…
pub async fn get_graph_memory_audit(
    State(state): State<Arc<AppState>>,
    Query(q): Query<GraphMemoryAuditQuery>,
) -> Json<Value> {
    let Ok(agent_id) = sanitize_agent_id(&q.agent_id) else {
        return Json(json!({ "entries": [] }));
    };
    let limit = q.limit.clamp(1, 500);
    let path = audit_log_path(&state.kernel.config.home_dir, &agent_id);
    let Ok(txt) = std::fs::read_to_string(path) else {
        return Json(json!({ "entries": [] }));
    };
    let mut entries = Vec::<Value>::new();
    for line in txt.lines().rev() {
        if entries.len() >= limit {
            break;
        }
        if let Ok(v) = serde_json::from_str::<Value>(line) {
            entries.push(v);
        }
    }
    Json(json!({ "entries": entries }))
}

/// POST /api/graph-memory/snapshot
pub async fn post_graph_memory_snapshot(
    State(state): State<Arc<AppState>>,
    Json(req): Json<GraphMemorySnapshotRequest>,
) -> (StatusCode, Json<Value>) {
    let agent_id = match sanitize_agent_id(&req.agent_id) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "ok": false, "error": e })),
            )
        }
    };
    let home = state.kernel.config.home_dir.clone();
    let src = graph_db_path(&home, &agent_id);
    if !src.exists() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "ok": false, "error": "graph memory db not found" })),
        );
    }
    let _guard = acquire_agent_write_lock(&agent_id).await;
    let dir = snapshots_dir(&home, &agent_id);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "ok": false, "error": format!("create snapshots dir: {e}") })),
        );
    }
    let label = sanitize_label(req.label.as_deref().unwrap_or("manual"));
    let id = format!("{}__{}.db", now_unix_ms(), label);
    let dst = dir.join(&id);
    let conn = match open_conn_with_fk(&src) {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "ok": false, "error": format!("open graph db: {e}") })),
            )
        }
    };
    if let Err(e) = conn.execute_batch("BEGIN IMMEDIATE;") {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "ok": false, "error": format!("acquire write lock: {e}") })),
        );
    }
    let copy_res = copy_db_with_sidecars(&src, &dst);
    if copy_res.is_ok() {
        let _ = conn.execute_batch("COMMIT;");
    } else {
        let _ = conn.execute_batch("ROLLBACK;");
    }
    if let Err(e) = copy_res {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "ok": false, "error": e })),
        );
    }
    let _ = append_audit(
        &home,
        &agent_id,
        "snapshot_created",
        json!({ "snapshot_id": id, "label": label }),
    );
    (
        StatusCode::OK,
        Json(json!({ "ok": true, "snapshot_id": id })),
    )
}

/// POST /api/graph-memory/rollback
pub async fn post_graph_memory_rollback(
    State(state): State<Arc<AppState>>,
    Json(req): Json<GraphMemoryRollbackRequest>,
) -> (StatusCode, Json<Value>) {
    let agent_id = match sanitize_agent_id(&req.agent_id) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "ok": false, "error": e })),
            )
        }
    };
    let snapshot_id = match sanitize_snapshot_id(&req.snapshot_id) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "ok": false, "error": e })),
            )
        }
    };
    let home = state.kernel.config.home_dir.clone();
    let live_db = graph_db_path(&home, &agent_id);
    let src = snapshots_dir(&home, &agent_id).join(&snapshot_id);
    if !src.exists() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "ok": false, "error": "snapshot not found" })),
        );
    }
    let _guard = acquire_agent_write_lock(&agent_id).await;
    if let Some(parent) = live_db.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if !live_db.exists() {
        let _ = GraphMemory::new(&live_db);
    }
    if live_db.exists() {
        let auto_id = format!("{}__auto_pre_rollback.db", now_unix_ms());
        let auto_path = snapshots_dir(&home, &agent_id).join(auto_id);
        let _ = std::fs::create_dir_all(snapshots_dir(&home, &agent_id));
        let _ = copy_db_with_sidecars(&live_db, &auto_path);
    }
    let conn = match open_conn_with_fk(&live_db) {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "ok": false, "error": format!("open live db failed: {e}") })),
            )
        }
    };
    let tx_res: Result<(), String> = (|| {
        conn.execute_batch("BEGIN IMMEDIATE;")
            .map_err(|e| e.to_string())?;
        conn.execute(
            "ATTACH DATABASE ?1 AS snap",
            rusqlite::params![src.to_string_lossy().to_string()],
        )
        .map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM ainl_graph_edges", [])
            .map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM ainl_graph_nodes", [])
            .map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO ainl_graph_nodes (id, node_type, payload, timestamp)
             SELECT id, node_type, payload, timestamp FROM snap.ainl_graph_nodes",
            [],
        )
        .map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO ainl_graph_edges (from_id, to_id, label, weight, metadata)
             SELECT from_id, to_id, label, weight, metadata FROM snap.ainl_graph_edges",
            [],
        )
        .map_err(|e| e.to_string())?;
        conn.execute_batch("COMMIT;").map_err(|e| e.to_string())?;
        Ok(())
    })();
    if let Err(e) = tx_res {
        let _ = conn.execute_batch("ROLLBACK;");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "ok": false, "error": format!("rollback restore failed: {e}") })),
        );
    }
    let _ = append_audit(
        &home,
        &agent_id,
        "snapshot_rollback",
        json!({
            "snapshot_id": snapshot_id,
            "reason": req.reason.unwrap_or_default()
        }),
    );
    let counts = open_conn_with_fk(&live_db)
        .ok()
        .map(|c| count_nodes_by_kind(&c))
        .unwrap_or_else(|| json!({}));
    (
        StatusCode::OK,
        Json(json!({ "ok": true, "counts": counts })),
    )
}

/// POST /api/graph-memory/reset
pub async fn post_graph_memory_reset(
    State(state): State<Arc<AppState>>,
    Json(req): Json<GraphMemoryResetRequest>,
) -> (StatusCode, Json<Value>) {
    let agent_id = match sanitize_agent_id(&req.agent_id) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "ok": false, "error": e })),
            )
        }
    };
    let home = state.kernel.config.home_dir.clone();
    let db = graph_db_path(&home, &agent_id);
    let _guard = acquire_agent_write_lock(&agent_id).await;
    if let Some(parent) = db.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if !db.exists() {
        let _ = GraphMemory::new(&db);
    }
    if req.create_snapshot.unwrap_or(true) && db.exists() {
        let dir = snapshots_dir(&home, &agent_id);
        let _ = std::fs::create_dir_all(&dir);
        let auto = format!("{}__pre_reset.db", now_unix_ms());
        let _ = copy_db_with_sidecars(&db, &dir.join(auto));
    }
    let conn = match open_conn_with_fk(&db) {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "ok": false, "error": format!("open db failed: {e}") })),
            )
        }
    };
    let tx_res: Result<(), String> = (|| {
        conn.execute_batch("BEGIN IMMEDIATE;")
            .map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM ainl_graph_edges", [])
            .map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM ainl_graph_nodes", [])
            .map_err(|e| e.to_string())?;
        conn.execute_batch("COMMIT;").map_err(|e| e.to_string())?;
        Ok(())
    })();
    if let Err(e) = tx_res {
        let _ = conn.execute_batch("ROLLBACK;");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "ok": false, "error": format!("reset failed: {e}") })),
        );
    }
    let _ = append_audit(
        &home,
        &agent_id,
        "graph_reset",
        json!({ "reason": req.reason.unwrap_or_default() }),
    );
    (StatusCode::OK, Json(json!({ "ok": true })))
}

/// POST /api/graph-memory/delete-node
pub async fn post_graph_memory_delete_node(
    State(state): State<Arc<AppState>>,
    Json(req): Json<GraphMemoryDeleteNodeRequest>,
) -> (StatusCode, Json<Value>) {
    let agent_id = match sanitize_agent_id(&req.agent_id) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "ok": false, "error": e })),
            )
        }
    };
    let node_id = req.node_id.trim().to_string();
    if node_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "error": "node_id is required" })),
        );
    }
    let home = state.kernel.config.home_dir.clone();
    let db = graph_db_path(&home, &agent_id);
    let _guard = acquire_agent_write_lock(&agent_id).await;
    let Ok(conn) = open_conn_with_fk(&db) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "ok": false, "error": "graph memory db not found" })),
        );
    };
    let before_payload: Option<String> = conn
        .query_row(
            "SELECT payload FROM ainl_graph_nodes WHERE id = ?1",
            rusqlite::params![node_id],
            |r| r.get(0),
        )
        .optional()
        .unwrap_or(None);
    if conn.execute_batch("BEGIN IMMEDIATE;").is_err() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "ok": false, "error": "could not acquire delete lock" })),
        );
    }
    let deleted = conn
        .execute(
            "DELETE FROM ainl_graph_nodes WHERE id = ?1",
            rusqlite::params![req.node_id.trim()],
        )
        .unwrap_or(0);
    if deleted == 0 {
        let _ = conn.execute_batch("ROLLBACK;");
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "ok": false, "error": "node not found" })),
        );
    }
    if conn.execute_batch("COMMIT;").is_err() {
        let _ = conn.execute_batch("ROLLBACK;");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "ok": false, "error": "delete commit failed" })),
        );
    }
    let mut node_kind = String::new();
    let mut node_label = String::new();
    if let Some(payload) = before_payload {
        if let Ok(node) = serde_json::from_str::<AinlMemoryNode>(&payload) {
            node_kind = match &node.node_type {
                AinlNodeType::Episode { .. } => "episode".to_string(),
                AinlNodeType::Semantic { .. } => "semantic".to_string(),
                AinlNodeType::Procedural { .. } => "procedural".to_string(),
                AinlNodeType::Persona { .. } => "persona".to_string(),
                AinlNodeType::RuntimeState { .. } => "runtime_state".to_string(),
            };
            node_label = label_for_node(&node).0;
        }
    }
    let _ = append_audit(
        &home,
        &agent_id,
        "node_deleted",
        json!({
            "node_id": req.node_id.trim(),
            "node_kind": node_kind,
            "node_label": node_label,
            "reason": req.reason.unwrap_or_default()
        }),
    );
    let counts = count_nodes_by_kind(&conn);
    (
        StatusCode::OK,
        Json(json!({ "ok": true, "deleted": 1, "counts": counts })),
    )
}
