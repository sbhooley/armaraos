//! Drain Python `AinlMemorySyncWriter` inbox JSON into the per-agent `ainl_memory.db` graph.
//!
//! File: `{openfang_home}/agents/<agent_id>/ainl_graph_memory_inbox.json` (same path as **ainativelang**
//! `armaraos/bridge/ainl_memory_sync.py`). After a successful import the inbox is reset to an empty
//! graph envelope so Python can append again.

use std::borrow::Cow;
use std::path::Path;

use ainl_memory::{
    AgentGraphSnapshot, AinlMemoryNode, AinlNodeType, PersonaSource, SnapshotEdge,
    SNAPSHOT_SCHEMA_VERSION,
};
use chrono::Utc;
use serde_json::{Map, Value};
use tracing::{debug, warn};
use uuid::Uuid;

use crate::graph_memory_writer::GraphMemoryWriter;

const INBOX_FILENAME: &str = "ainl_graph_memory_inbox.json";
const REQUIRES_AINL_TAGGER: &str = "requires_ainl_tagger";

fn inbox_path(agent_id: &str) -> std::path::PathBuf {
    openfang_types::config::openfang_home_dir()
        .join("agents")
        .join(agent_id)
        .join(INBOX_FILENAME)
}

fn stable_uuid_from_inbox_id(s: &str) -> Uuid {
    Uuid::new_v5(&Uuid::NAMESPACE_URL, s.as_bytes())
}

fn parse_source_features(root: &Value) -> Vec<String> {
    root.get("source_features")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn parse_uuid_or_stable(raw: Option<&Value>, fallback_key: &str) -> Uuid {
    if let Some(v) = raw {
        if let Some(s) = v.as_str() {
            if let Ok(u) = Uuid::parse_str(s) {
                return u;
            }
            return stable_uuid_from_inbox_id(s);
        }
    }
    stable_uuid_from_inbox_id(fallback_key)
}

/// When `requires_ainl_tagger` is present in inbox metadata but this binary was built without
/// `ainl-tagger`, skip semantic nodes that carry non-empty tags (likely tagger-enriched).
fn skip_semantic_for_tagger_policy(
    source_features: &[String],
    tags: &[String],
    cfg_tagger: bool,
) -> bool {
    if cfg_tagger {
        return false;
    }
    source_features
        .iter()
        .any(|s| s == REQUIRES_AINL_TAGGER)
        && !tags.is_empty()
}

fn value_as_str(v: Option<&Value>) -> Option<&str> {
    v.and_then(|x| x.as_str())
}

fn value_as_f32(v: Option<&Value>, default: f32) -> f32 {
    v.and_then(|x| x.as_f64())
        .map(|f| f as f32)
        .unwrap_or(default)
}

fn value_as_i64(v: Option<&Value>) -> Option<i64> {
    match v? {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

fn value_string_vec(v: Option<&Value>) -> Vec<String> {
    v.and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Try full Rust snapshot node JSON (`AinlMemoryNode`), then `payload.rust_snapshot`, then the
/// Python `MemoryNode` envelope from `ainl_graph_memory.py`.
fn convert_one_inbox_node(
    raw: &Value,
    writer_agent_id: &str,
    source_features: &[String],
) -> Option<AinlMemoryNode> {
    if let Ok(node) = serde_json::from_value::<AinlMemoryNode>(raw.clone()) {
        return Some(node);
    }

    if let Some(rs) = raw
        .get("payload")
        .and_then(|p| p.get("rust_snapshot"))
        .cloned()
    {
        if let Ok(node) = serde_json::from_value::<AinlMemoryNode>(rs) {
            return Some(node);
        }
    }

    let obj = raw.as_object()?;
    let id_str = value_as_str(obj.get("id")).unwrap_or("unknown");
    let node_type = value_as_str(obj.get("node_type"))
        .unwrap_or("")
        .to_ascii_lowercase();
    let _declared_agent_id = value_as_str(obj.get("agent_id"))
        .unwrap_or(writer_agent_id)
        .to_string();
    let label = value_as_str(obj.get("label")).unwrap_or("").to_string();
    let payload = obj.get("payload").cloned().unwrap_or(Value::Object(Map::new()));
    let tags = value_string_vec(obj.get("tags"));
    let created_at = value_as_f32(obj.get("created_at"), 0.0);

    let map = match payload {
        Value::Object(m) => m,
        _ => Map::new(),
    };

    let mut node = match node_type.as_str() {
        "semantic" => {
            if skip_semantic_for_tagger_policy(source_features, &tags, cfg!(feature = "ainl-tagger"))
            {
                debug!(
                    agent_id = %writer_agent_id,
                    inbox_id = %id_str,
                    "inbox: skipped semantic node (requires_ainl_tagger + non-empty tags; binary has ainl-tagger off)"
                );
                return None;
            }
            let fact = value_as_str(map.get("fact"))
                .map(str::to_string)
                .filter(|s| !s.is_empty())
                .or_else(|| {
                    if label.is_empty() {
                        None
                    } else {
                        Some(label.clone())
                    }
                })
                .unwrap_or_else(|| format!("(inbox semantic {})", id_str));
            let confidence = value_as_f32(map.get("confidence"), 0.7);
            let source_turn_id = parse_uuid_or_stable(map.get("source_turn_id"), id_str);
            let mut n = AinlMemoryNode::new_fact(fact, confidence, source_turn_id);
            if let AinlNodeType::Semantic { ref mut semantic } = n.node_type {
                semantic.tags = tags.clone();
                semantic.source_episode_id = value_as_str(map.get("source_episode_id"))
                    .unwrap_or("")
                    .to_string();
            }
            n
        }
        "episodic" | "episode" => {
            let turn_id = parse_uuid_or_stable(map.get("turn_id"), id_str);
            let timestamp = value_as_i64(map.get("timestamp"))
                .unwrap_or(created_at as i64);
            let tool_calls = value_string_vec(map.get("tool_calls").or_else(|| map.get("tools")));
            let delegation_to = map
                .get("delegation_to")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let trace_event = map.get("trace_event").cloned();
            let mut n = AinlMemoryNode::new_episode(
                turn_id,
                timestamp,
                tool_calls.clone(),
                delegation_to,
                trace_event,
            );
            n.id = parse_uuid_or_stable(Some(&Value::String(id_str.to_string())), id_str);
            if let AinlNodeType::Episode { ref mut episodic } = n.node_type {
                episodic.tags.clone_from(&tags);
                episodic.turn_index = map
                    .get("turn_index")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32;
                episodic.user_message_tokens = map
                    .get("user_message_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32;
                episodic.assistant_response_tokens = map
                    .get("assistant_response_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32;
                episodic.persona_signals_emitted =
                    value_string_vec(map.get("persona_signals_emitted"));
                episodic.flagged = map.get("flagged").and_then(|v| v.as_bool()).unwrap_or(false);
                episodic.conversation_id = map
                    .get("conversation_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                episodic.follows_episode_id = map
                    .get("follows_episode_id")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                episodic.user_message = map
                    .get("user_message")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                episodic.assistant_response = map
                    .get("assistant_response")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                let invoked = value_string_vec(map.get("tools_invoked"));
                if !invoked.is_empty() {
                    episodic.tools_invoked = invoked;
                }
                // Cognitive vitals (Gap K) — optional; absent on non-OpenAI and pre-Styxx episodes.
                episodic.vitals_gate = map
                    .get("vitals_gate")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                episodic.vitals_phase = map
                    .get("vitals_phase")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                episodic.vitals_trust = map
                    .get("vitals_trust")
                    .and_then(|v| v.as_f64())
                    .map(|f| f as f32);
            }
            n
        }
        "procedural" => {
            let pattern_name = value_as_str(map.get("pattern_name"))
                .unwrap_or(label.as_str())
                .to_string();
            let tool_sequence = value_string_vec(map.get("tool_sequence"));
            let confidence = value_as_f32(map.get("confidence"), 0.75);
            let mut n = AinlMemoryNode::new_procedural_tools(pattern_name, tool_sequence, confidence);
            if let AinlNodeType::Procedural { ref mut procedural } = n.node_type {
                procedural.compiled_graph = map
                    .get("compiled_graph")
                    .and_then(|v| {
                        v.as_array().map(|arr| {
                            arr.iter()
                                .filter_map(|b| b.as_u64().and_then(|u| u8::try_from(u).ok()))
                                .collect::<Vec<u8>>()
                        })
                    })
                    .unwrap_or_default();
                procedural.procedure_type = map
                    .get("procedure_type")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default();
                procedural.label = map
                    .get("label")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                procedural.trace_id = map
                    .get("trace_id")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                procedural.patch_version = map
                    .get("patch_version")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(1) as u32;
            }
            n
        }
        "persona" => {
            let trait_name = value_as_str(map.get("trait_name"))
                .unwrap_or_else(|| {
                    label
                        .strip_prefix("persona:")
                        .unwrap_or(label.as_str())
                })
                .to_string();
            let strength = value_as_f32(map.get("strength"), 0.5);
            let learned_from: Vec<Uuid> = map
                .get("learned_from")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().and_then(|s| Uuid::parse_str(s).ok()))
                        .collect()
                })
                .unwrap_or_default();
            let mut n = AinlMemoryNode::new_persona(trait_name, strength, learned_from);
            if let AinlNodeType::Persona { ref mut persona } = n.node_type {
                persona.agent_id = _declared_agent_id.clone();
                persona.source = map
                    .get("source")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or(PersonaSource::Evolved);
            }
            n
        }
        "patch" => {
            // Python PatchRecord → ProceduralNode with patch_version ≥ 1 and label set.
            // Python fields: label_name, pattern_name, source_episode_ids, patch_version,
            //                patched_at, parent_patch_id, retired_at.
            let pattern_name = value_as_str(map.get("pattern_name"))
                .filter(|s| !s.is_empty())
                .or_else(|| value_as_str(map.get("label_name")).filter(|s| !s.is_empty()))
                .unwrap_or(label.as_str())
                .to_string();
            let label_name = value_as_str(map.get("label_name"))
                .unwrap_or("")
                .to_string();
            let patch_version = map
                .get("patch_version")
                .and_then(|v| v.as_u64())
                .unwrap_or(1)
                .max(1) as u32;
            let retired = map
                .get("retired_at")
                .map(|v| !v.is_null())
                .unwrap_or(false);
            let source_episode_ids = value_string_vec(map.get("source_episode_ids"));
            let mut n = AinlMemoryNode::new_procedural_tools(
                pattern_name,
                source_episode_ids,
                0.75,
            );
            if let AinlNodeType::Procedural { ref mut procedural } = n.node_type {
                procedural.patch_version = patch_version;
                procedural.label = label_name;
                procedural.retired = retired;
                // Store parent patch linkage as trace_id if present.
                procedural.trace_id = map
                    .get("parent_patch_id")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(str::to_string);
            }
            debug!(
                agent_id = %writer_agent_id,
                inbox_id = %id_str,
                patch_version = patch_version,
                "inbox: importing patch node as procedural"
            );
            n
        }
        other => {
            warn!(
                agent_id = %writer_agent_id,
                inbox_id = %id_str,
                node_type = %other,
                "inbox: unknown node_type — skipped"
            );
            return None;
        }
    };

    node.id = parse_uuid_or_stable(Some(&Value::String(id_str.to_string())), id_str);
    node.agent_id = writer_agent_id.to_string();
    Some(node)
}

fn convert_inbox_edge(raw: &Value) -> Option<SnapshotEdge> {
    let obj = raw.as_object()?;
    let src = value_as_str(obj.get("src"))
        .or_else(|| obj.get("source_id").and_then(|v| v.as_str()))?;
    let dst = value_as_str(obj.get("dst"))
        .or_else(|| obj.get("target_id").and_then(|v| v.as_str()))?;
    let edge_type = value_as_str(obj.get("edge_type"))
        .or_else(|| obj.get("label").and_then(|v| v.as_str()))
        .unwrap_or("references");
    let weight = value_as_f32(obj.get("confidence").or_else(|| obj.get("weight")), 1.0);
    let source_id = Uuid::parse_str(src).unwrap_or_else(|_| stable_uuid_from_inbox_id(src));
    let target_id = Uuid::parse_str(dst).unwrap_or_else(|_| stable_uuid_from_inbox_id(dst));
    Some(SnapshotEdge {
        source_id,
        target_id,
        edge_type: edge_type.to_string(),
        weight,
        metadata: obj.get("meta").cloned(),
    })
}

async fn atomic_write_inbox(path: &Path, value: &Value) -> Result<(), String> {
    let parent = path.parent().ok_or_else(|| "inbox path has no parent".to_string())?;
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|e| format!("create_dir_all inbox parent: {e}"))?;
    let body = serde_json::to_vec_pretty(value).map_err(|e| format!("serialize inbox: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    tokio::fs::write(&tmp, &body)
        .await
        .map_err(|e| format!("write inbox tmp: {e}"))?;
    tokio::fs::rename(&tmp, path)
        .await
        .map_err(|e| format!("rename inbox tmp: {e}"))?;
    Ok(())
}

/// Import inbox nodes/edges into `writer`'s SQLite graph, then reset the inbox file.
pub async fn drain_inbox(writer: &GraphMemoryWriter) -> Result<(), String> {
    let path = inbox_path(writer.agent_id());
    let bytes = match tokio::fs::read(&path).await {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("read inbox: {e}")),
    };
    if bytes.is_empty() || bytes.iter().all(|u| u.is_ascii_whitespace()) {
        return Ok(());
    }
    let root: Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("inbox json: {e}"))?;
    let source_features = parse_source_features(&root);
    let nodes_raw = root
        .get("nodes")
        .and_then(|n| n.as_array())
        .cloned()
        .unwrap_or_default();
    let edges_raw = root
        .get("edges")
        .and_then(|n| n.as_array())
        .cloned()
        .unwrap_or_default();

    if nodes_raw.is_empty() && edges_raw.is_empty() {
        return Ok(());
    }

    let mut nodes: Vec<AinlMemoryNode> = Vec::new();
    for n in &nodes_raw {
        if let Some(conv) = convert_one_inbox_node(n, writer.agent_id(), &source_features) {
            nodes.push(conv);
        }
    }

    let mut edges: Vec<SnapshotEdge> = Vec::new();
    for e in &edges_raw {
        if let Some(edge) = convert_inbox_edge(e) {
            edges.push(edge);
        }
    }

    if nodes.is_empty() && edges.is_empty() {
        return Ok(());
    }

    let node_count = nodes.len();
    let edge_count = edges.len();
    let snapshot = AgentGraphSnapshot {
        agent_id: writer.agent_id().to_string(),
        exported_at: Utc::now(),
        schema_version: Cow::Borrowed(SNAPSHOT_SCHEMA_VERSION),
        nodes,
        edges,
    };

    {
        let mut gm = writer.inner.lock().await;
        gm.import_graph(&snapshot, true)
            .map_err(|e| format!("import_graph from inbox: {e}"))?;
    }

    let empty = serde_json::json!({
        "nodes": [],
        "edges": [],
        "source_features": [],
        "schema_version": "1",
    });
    atomic_write_inbox(&path, &empty).await?;
    debug!(
        agent_id = %writer.agent_id(),
        path = %path.display(),
        imported_nodes = node_count,
        imported_edges = edge_count,
        "AINL graph memory inbox drained"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ainl_memory::GraphMemory;
    use serde_json::json;

    const AGENT: &str = "inbox-vitals-test";

    fn open_writer_in_mem() -> (crate::graph_memory_writer::GraphMemoryWriter, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = dir.path().join("ainl_memory.db");
        let memory = GraphMemory::new(&db).expect("GraphMemory");
        let writer =
            crate::graph_memory_writer::GraphMemoryWriter::from_memory_for_tests(memory, AGENT, None);
        (writer, dir)
    }

    fn inbox_episode_node(
        vitals_gate: Option<&str>,
        vitals_phase: Option<&str>,
        vitals_trust: Option<f64>,
    ) -> Value {
        let now = chrono::Utc::now().timestamp();
        let mut payload = serde_json::Map::new();
        payload.insert("tool_calls".into(), json!(["shell_exec"]));
        payload.insert("turn_id".into(), json!("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"));
        payload.insert("timestamp".into(), json!(now));
        if let Some(g) = vitals_gate {
            payload.insert("vitals_gate".into(), json!(g));
        }
        if let Some(p) = vitals_phase {
            payload.insert("vitals_phase".into(), json!(p));
        }
        if let Some(t) = vitals_trust {
            payload.insert("vitals_trust".into(), json!(t));
        }
        json!({
            "id": "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb",
            "node_type": "episodic",
            "agent_id": AGENT,
            "label": "test_episode",
            "payload": Value::Object(payload),
            "tags": [],
            "created_at": now as f64,
        })
    }

    async fn drain_raw_into_writer(
        writer: &crate::graph_memory_writer::GraphMemoryWriter,
        raw_inbox: &Value,
    ) {
        let tmpdir = tempfile::tempdir().expect("tempdir");
        let inbox_dir = tmpdir
            .path()
            .join("agents")
            .join(writer.agent_id());
        tokio::fs::create_dir_all(&inbox_dir).await.unwrap();
        let path = inbox_dir.join(INBOX_FILENAME);
        let body = serde_json::to_vec_pretty(raw_inbox).unwrap();
        tokio::fs::write(&path, &body).await.unwrap();

        let source_features = parse_source_features(raw_inbox);
        let nodes_raw = raw_inbox
            .get("nodes")
            .and_then(|n| n.as_array())
            .cloned()
            .unwrap_or_default();

        let mut nodes: Vec<AinlMemoryNode> = Vec::new();
        for n in &nodes_raw {
            if let Some(conv) = convert_one_inbox_node(n, writer.agent_id(), &source_features) {
                nodes.push(conv);
            }
        }

        if nodes.is_empty() {
            return;
        }

        let snapshot = AgentGraphSnapshot {
            agent_id: writer.agent_id().to_string(),
            exported_at: chrono::Utc::now(),
            schema_version: Cow::Borrowed(SNAPSHOT_SCHEMA_VERSION),
            nodes,
            edges: vec![],
        };
        let mut gm = writer.inner.lock().await;
        gm.import_graph(&snapshot, true).expect("import_graph");
    }

    /// Gap K — episode with vitals in inbox JSON maps to episodic node vitals fields.
    #[tokio::test]
    async fn test_inbox_drain_maps_vitals_on_episodic_node() {
        let (writer, _dir) = open_writer_in_mem();
        let inbox = json!({
            "schema_version": "1",
            "nodes": [inbox_episode_node(Some("pass"), Some("reasoning:0.69"), Some(0.69))],
            "edges": [],
            "source_features": [],
        });
        drain_raw_into_writer(&writer, &inbox).await;

        let nodes = writer.recall_recent(60 * 60 * 24 * 365).await;
        let ep = nodes.iter().find(|n| {
            matches!(&n.node_type, AinlNodeType::Episode { .. })
        });
        assert!(ep.is_some(), "no episodic node after drain");
        if let AinlNodeType::Episode { episodic } = &ep.unwrap().node_type {
            assert_eq!(episodic.vitals_gate.as_deref(), Some("pass"));
            assert_eq!(episodic.vitals_phase.as_deref(), Some("reasoning:0.69"));
            let trust = episodic.vitals_trust.expect("vitals_trust must be Some");
            assert!((trust - 0.69_f32).abs() < 1e-4, "trust={trust}");
        }
    }

    /// Gap N — patch node from Python inbox is imported as ProceduralNode.
    #[tokio::test]
    async fn test_inbox_drain_imports_patch_as_procedural() {
        let (writer, _dir) = open_writer_in_mem();
        let inbox = json!({
            "schema_version": "1",
            "nodes": [{
                "id": "cccccccc-cccc-4ccc-8ccc-cccccccccccc",
                "node_type": "patch",
                "agent_id": AGENT,
                "label": "my_patch",
                "payload": {
                    "label_name": "L_my_patch",
                    "pattern_name": "my_pattern",
                    "source_episode_ids": ["ep1", "ep2"],
                    "patch_version": 2,
                    "patched_at": 1700000000,
                    "parent_patch_id": "parent-patch-xyz",
                    "retired_at": null
                },
                "tags": [],
                "created_at": 0.0
            }],
            "edges": [],
            "source_features": [],
        });
        drain_raw_into_writer(&writer, &inbox).await;

        let gm = writer.inner.lock().await;
        let snapshot = gm.export_graph(AGENT).expect("export_graph");
        drop(gm);

        let procedural = snapshot.nodes.iter().find(|n| {
            matches!(&n.node_type, AinlNodeType::Procedural { procedural } if procedural.patch_version >= 1)
        });
        assert!(procedural.is_some(), "no procedural node found after patch import");
        if let AinlNodeType::Procedural { procedural } = &procedural.unwrap().node_type {
            assert_eq!(procedural.patch_version, 2);
            assert_eq!(procedural.label.as_str(), "L_my_patch");
            assert!(!procedural.retired, "should not be retired (retired_at was null)");
            assert_eq!(
                procedural.trace_id.as_deref(),
                Some("parent-patch-xyz"),
                "parent_patch_id should be stored as trace_id"
            );
            assert!(procedural.pattern_name == "my_pattern", "pattern_name mismatch");
        }
    }

    /// Gap N — retired patch node sets retired = true.
    #[tokio::test]
    async fn test_inbox_drain_retired_patch_sets_retired_flag() {
        let (writer, _dir) = open_writer_in_mem();
        let inbox = json!({
            "schema_version": "1",
            "nodes": [{
                "id": "dddddddd-dddd-4ddd-8ddd-dddddddddddd",
                "node_type": "patch",
                "agent_id": AGENT,
                "label": "old_patch",
                "payload": {
                    "label_name": "L_old_patch",
                    "pattern_name": "old_pattern",
                    "source_episode_ids": [],
                    "patch_version": 1,
                    "patched_at": 1700000000,
                    "retired_at": 1710000000
                },
                "tags": [],
                "created_at": 0.0
            }],
            "edges": [],
            "source_features": [],
        });
        drain_raw_into_writer(&writer, &inbox).await;

        let gm = writer.inner.lock().await;
        let snapshot = gm.export_graph(AGENT).expect("export_graph");
        drop(gm);

        let procedural = snapshot.nodes.iter().find(|n| {
            matches!(&n.node_type, AinlNodeType::Procedural { procedural } if procedural.patch_version >= 1)
        });
        assert!(procedural.is_some(), "no procedural node after retired patch import");
        if let AinlNodeType::Procedural { procedural } = &procedural.unwrap().node_type {
            assert!(procedural.retired, "retired_at present should set retired=true");
        }
    }

    /// Gap K regression — episode without vitals drains without panic; fields remain None.
    #[tokio::test]
    async fn test_inbox_drain_episode_without_vitals_still_imports() {
        let (writer, _dir) = open_writer_in_mem();
        let inbox = json!({
            "schema_version": "1",
            "nodes": [inbox_episode_node(None, None, None)],
            "edges": [],
            "source_features": [],
        });
        drain_raw_into_writer(&writer, &inbox).await;

        let nodes = writer.recall_recent(60 * 60 * 24 * 365).await;
        let ep = nodes.iter().find(|n| {
            matches!(&n.node_type, AinlNodeType::Episode { .. })
        });
        assert!(ep.is_some(), "episode without vitals failed to drain");
        if let AinlNodeType::Episode { episodic } = &ep.unwrap().node_type {
            assert!(episodic.vitals_gate.is_none());
            assert!(episodic.vitals_phase.is_none());
            assert!(episodic.vitals_trust.is_none());
        }
    }
}
