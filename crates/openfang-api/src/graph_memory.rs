//! GET /api/graph-memory — read AINL graph nodes/edges from an agent's `ainl_memory.db`.

use ainl_memory::{AinlMemoryNode, AinlNodeType};
use axum::extract::Query;
use axum::Json;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashSet;

#[derive(serde::Deserialize)]
pub struct GraphMemoryQuery {
    pub agent_id: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default = "default_since_seconds")]
    pub since_seconds: i64,
}

const fn default_limit() -> usize {
    200
}

const fn default_since_seconds() -> i64 {
    7_776_000 // 90 days
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

/// Returns `(label, strength, vitals_gate, vitals_phase, vitals_trust)`.
fn label_for_node(node: &AinlMemoryNode) -> (String, Option<f32>, Option<String>, Option<String>, Option<f32>) {
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
            )
        }
        AinlNodeType::Semantic { semantic } => {
            let label = if semantic.fact.is_empty() {
                format!("Fact {id_short}")
            } else {
                semantic.fact.clone()
            };
            (label, None, None, None, None)
        }
        AinlNodeType::Procedural { procedural } => {
            let label = if procedural.pattern_name.is_empty() {
                format!("Pattern {id_short}")
            } else {
                procedural.pattern_name.clone()
            };
            (label, None, None, None, None)
        }
        AinlNodeType::Persona { persona } => (
            format!("{} ({:.2})", persona.trait_name, persona.strength),
            Some(persona.strength),
            None,
            None,
            None,
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

/// GET /api/graph-memory?agent_id=…&limit=…&since_seconds=…
pub async fn get_graph_memory(Query(q): Query<GraphMemoryQuery>) -> Json<Value> {
    let limit = q.limit.clamp(1, 2000);
    // Same layout as `GraphMemoryWriter::db_path`: `openfang_home_dir()/agents/<id>/ainl_memory.db`.
    let path = openfang_types::config::openfang_home_dir()
        .join("agents")
        .join(q.agent_id.trim())
        .join("ainl_memory.db");
    if !path.exists() {
        return Json(json!({ "nodes": [], "edges": [] }));
    }
    let Ok(conn) = rusqlite::Connection::open(&path) else {
        return Json(json!({ "nodes": [], "edges": [] }));
    };
    let since_ts = chrono::Utc::now().timestamp() - q.since_seconds;

    let Ok(mut stmt) = conn.prepare(
        "SELECT id, node_type, payload, timestamp FROM ainl_graph_nodes
         WHERE timestamp >= ?1
         ORDER BY timestamp DESC
         LIMIT ?2",
    ) else {
        return Json(json!({ "nodes": [], "edges": [] }));
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
        Err(_) => return Json(json!({ "nodes": [], "edges": [] })),
    };

    let mut nodes_out: Vec<GraphMemoryNodeOut> = Vec::new();
    let mut id_set: HashSet<String> = HashSet::new();

    for row in rows.flatten() {
        let (id, node_type, payload, ts) = row;
        let Ok(node) = serde_json::from_str::<AinlMemoryNode>(&payload) else {
            continue;
        };
        let (label, strength, vitals_gate, vitals_phase, vitals_trust) = label_for_node(&node);
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

    Json(json!({
        "nodes": nodes_out,
        "edges": edges_out,
    }))
}
