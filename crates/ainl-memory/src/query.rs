//! Graph traversal and querying utilities.
//!
//! Two layers:
//!
//! 1. **Free functions** (e.g. [`recall_recent`], [`walk_from`], [`find_patterns`]) — take [`GraphStore`]
//!    plus parameters such as `agent_id` where filtering applies.
//! 2. **[`GraphQuery`]** — returned by [`SqliteGraphStore::query`]; holds `agent_id` and offers SQL-backed
//!    filters (`episodes`, `lineage`, `by_tag`, `subgraph_edges`, [`GraphQuery::read_runtime_state`], …).
//!
//! Serialized node JSON uses top-level `agent_id` and nested `node_type` fields (e.g. `$.node_type.outcome`);
//! edge rows use SQLite columns `from_id`, `to_id`, `label` (see [`crate::snapshot::SnapshotEdge`] for the
//! export/import naming `source_id` / `target_id` / `edge_type`).

use crate::node::{
    AinlMemoryNode, AinlNodeType, EpisodicNode, PersonaLayer, PersonaNode, ProceduralNode,
    ProcedureType, RuntimeStateNode, SemanticNode, StrengthEvent,
};
use crate::snapshot::SnapshotEdge;
use crate::store::{GraphStore, SqliteGraphStore};
use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension};
use std::collections::{HashMap, HashSet, VecDeque};
use uuid::Uuid;

fn node_matches_agent(node: &AinlMemoryNode, agent_id: &str) -> bool {
    node.agent_id.is_empty() || node.agent_id == agent_id
}

/// Walk the graph from a starting node, following edges with a specific label
pub fn walk_from(
    store: &dyn GraphStore,
    start_id: Uuid,
    edge_label: &str,
    max_depth: usize,
) -> Result<Vec<AinlMemoryNode>, String> {
    let mut visited = std::collections::HashSet::new();
    let mut result = Vec::new();
    let mut current_level = vec![start_id];

    for _ in 0..max_depth {
        if current_level.is_empty() {
            break;
        }

        let mut next_level = Vec::new();

        for node_id in current_level {
            if visited.contains(&node_id) {
                continue;
            }
            visited.insert(node_id);

            if let Some(node) = store.read_node(node_id)? {
                result.push(node.clone());

                for next_node in store.walk_edges(node_id, edge_label)? {
                    if !visited.contains(&next_node.id) {
                        next_level.push(next_node.id);
                    }
                }
            }
        }

        current_level = next_level;
    }

    Ok(result)
}

/// Recall recent episodes, optionally filtered by tool usage
pub fn recall_recent(
    store: &dyn GraphStore,
    since_timestamp: i64,
    limit: usize,
    tool_filter: Option<&str>,
) -> Result<Vec<AinlMemoryNode>, String> {
    let episodes = store.query_episodes_since(since_timestamp, limit)?;

    if let Some(tool_name) = tool_filter {
        Ok(episodes
            .into_iter()
            .filter(|node| match &node.node_type {
                AinlNodeType::Episode { episodic } => {
                    episodic.effective_tools().contains(&tool_name.to_string())
                }
                _ => false,
            })
            .collect())
    } else {
        Ok(episodes)
    }
}

/// Find procedural patterns by name prefix
pub fn find_patterns(
    store: &dyn GraphStore,
    name_prefix: &str,
) -> Result<Vec<AinlMemoryNode>, String> {
    let all_procedural = store.find_by_type("procedural")?;

    Ok(all_procedural
        .into_iter()
        .filter(|node| match &node.node_type {
            AinlNodeType::Procedural { procedural } => {
                procedural.pattern_name.starts_with(name_prefix)
            }
            _ => false,
        })
        .collect())
}

/// Find semantic facts with confidence above a threshold
pub fn find_high_confidence_facts(
    store: &dyn GraphStore,
    min_confidence: f32,
) -> Result<Vec<AinlMemoryNode>, String> {
    let all_semantic = store.find_by_type("semantic")?;

    Ok(all_semantic
        .into_iter()
        .filter(|node| match &node.node_type {
            AinlNodeType::Semantic { semantic } => semantic.confidence >= min_confidence,
            _ => false,
        })
        .collect())
}

/// Find persona traits sorted by strength
pub fn find_strong_traits(store: &dyn GraphStore) -> Result<Vec<AinlMemoryNode>, String> {
    let mut all_persona = store.find_by_type("persona")?;

    all_persona.sort_by(|a, b| {
        let strength_a = match &a.node_type {
            AinlNodeType::Persona { persona } => persona.strength,
            _ => 0.0,
        };
        let strength_b = match &b.node_type {
            AinlNodeType::Persona { persona } => persona.strength,
            _ => 0.0,
        };
        strength_b
            .partial_cmp(&strength_a)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    Ok(all_persona)
}

// --- Semantic helpers ---

pub fn recall_by_topic_cluster(
    store: &dyn GraphStore,
    agent_id: &str,
    cluster: &str,
) -> Result<Vec<SemanticNode>, String> {
    let mut out = Vec::new();
    for node in store.find_by_type("semantic")? {
        if !node_matches_agent(&node, agent_id) {
            continue;
        }
        if let AinlNodeType::Semantic { semantic } = &node.node_type {
            if semantic.topic_cluster.as_deref() == Some(cluster) {
                out.push(semantic.clone());
            }
        }
    }
    Ok(out)
}

pub fn recall_contradictions(
    store: &dyn GraphStore,
    node_id: Uuid,
) -> Result<Vec<SemanticNode>, String> {
    let Some(node) = store.read_node(node_id)? else {
        return Ok(Vec::new());
    };
    let contradiction_ids: Vec<String> = match &node.node_type {
        AinlNodeType::Semantic { semantic } => semantic.contradiction_ids.clone(),
        _ => return Ok(Vec::new()),
    };
    let mut out = Vec::new();
    for cid in contradiction_ids {
        if let Ok(uuid) = Uuid::parse_str(&cid) {
            if let Some(n) = store.read_node(uuid)? {
                if let AinlNodeType::Semantic { semantic } = &n.node_type {
                    out.push(semantic.clone());
                }
            }
        }
    }
    Ok(out)
}

pub fn count_by_topic_cluster(
    store: &dyn GraphStore,
    agent_id: &str,
) -> Result<HashMap<String, usize>, String> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for node in store.find_by_type("semantic")? {
        if !node_matches_agent(&node, agent_id) {
            continue;
        }
        if let AinlNodeType::Semantic { semantic } = &node.node_type {
            if let Some(cluster) = semantic.topic_cluster.as_deref() {
                if cluster.is_empty() {
                    continue;
                }
                *counts.entry(cluster.to_string()).or_insert(0) += 1;
            }
        }
    }
    Ok(counts)
}

// --- Episodic helpers ---

pub fn recall_flagged_episodes(
    store: &dyn GraphStore,
    agent_id: &str,
    limit: usize,
) -> Result<Vec<EpisodicNode>, String> {
    let mut out: Vec<(i64, EpisodicNode)> = Vec::new();
    for node in store.find_by_type("episode")? {
        if !node_matches_agent(&node, agent_id) {
            continue;
        }
        if let AinlNodeType::Episode { episodic } = &node.node_type {
            if episodic.flagged {
                out.push((episodic.timestamp, episodic.clone()));
            }
        }
    }
    out.sort_by(|a, b| b.0.cmp(&a.0));
    out.truncate(limit);
    Ok(out.into_iter().map(|(_, e)| e).collect())
}

pub fn recall_episodes_by_conversation(
    store: &dyn GraphStore,
    conversation_id: &str,
) -> Result<Vec<EpisodicNode>, String> {
    let mut out: Vec<(u32, EpisodicNode)> = Vec::new();
    for node in store.find_by_type("episode")? {
        if let AinlNodeType::Episode { episodic } = &node.node_type {
            if episodic.conversation_id == conversation_id {
                out.push((episodic.turn_index, episodic.clone()));
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out.into_iter().map(|(_, e)| e).collect())
}

pub fn recall_episodes_with_signal(
    store: &dyn GraphStore,
    agent_id: &str,
    signal_type: &str,
) -> Result<Vec<EpisodicNode>, String> {
    let mut out = Vec::new();
    for node in store.find_by_type("episode")? {
        if !node_matches_agent(&node, agent_id) {
            continue;
        }
        if let AinlNodeType::Episode { episodic } = &node.node_type {
            if episodic
                .persona_signals_emitted
                .iter()
                .any(|s| s == signal_type)
            {
                out.push(episodic.clone());
            }
        }
    }
    Ok(out)
}

// --- Procedural helpers ---

pub fn recall_by_procedure_type(
    store: &dyn GraphStore,
    agent_id: &str,
    procedure_type: ProcedureType,
) -> Result<Vec<ProceduralNode>, String> {
    let mut out = Vec::new();
    for node in store.find_by_type("procedural")? {
        if !node_matches_agent(&node, agent_id) {
            continue;
        }
        if let AinlNodeType::Procedural { procedural } = &node.node_type {
            if procedural.procedure_type == procedure_type {
                out.push(procedural.clone());
            }
        }
    }
    Ok(out)
}

pub fn recall_low_success_procedures(
    store: &dyn GraphStore,
    agent_id: &str,
    threshold: f32,
) -> Result<Vec<ProceduralNode>, String> {
    let mut out = Vec::new();
    for node in store.find_by_type("procedural")? {
        if !node_matches_agent(&node, agent_id) {
            continue;
        }
        if let AinlNodeType::Procedural { procedural } = &node.node_type {
            let total = procedural
                .success_count
                .saturating_add(procedural.failure_count);
            if total > 0 && procedural.success_rate < threshold {
                out.push(procedural.clone());
            }
        }
    }
    Ok(out)
}

// --- Persona helpers ---

pub fn recall_strength_history(
    store: &dyn GraphStore,
    node_id: Uuid,
) -> Result<Vec<StrengthEvent>, String> {
    let Some(node) = store.read_node(node_id)? else {
        return Ok(Vec::new());
    };
    let mut events = match &node.node_type {
        AinlNodeType::Persona { persona } => persona.evolution_log.clone(),
        _ => return Ok(Vec::new()),
    };
    events.sort_by_key(|e| e.timestamp);
    Ok(events)
}

pub fn recall_delta_by_relevance(
    store: &dyn GraphStore,
    agent_id: &str,
    min_relevance: f32,
) -> Result<Vec<PersonaNode>, String> {
    let mut out = Vec::new();
    for node in store.find_by_type("persona")? {
        if !node_matches_agent(&node, agent_id) {
            continue;
        }
        if let AinlNodeType::Persona { persona } = &node.node_type {
            if persona.layer == PersonaLayer::Delta && persona.relevance_score >= min_relevance {
                out.push(persona.clone());
            }
        }
    }
    Ok(out)
}

// --- GraphQuery (builder over SqliteGraphStore, v0.1.4+) ---

/// Builder-style queries scoped to one `agent_id` (matches `json_extract(payload, '$.agent_id')`).
pub struct GraphQuery<'a> {
    store: &'a SqliteGraphStore,
    agent_id: String,
}

impl SqliteGraphStore {
    pub fn query<'a>(&'a self, agent_id: &str) -> GraphQuery<'a> {
        GraphQuery {
            store: self,
            agent_id: agent_id.to_string(),
        }
    }
}

fn load_nodes_from_payload_rows(
    rows: impl Iterator<Item = Result<String, rusqlite::Error>>,
) -> Result<Vec<AinlMemoryNode>, String> {
    let mut out = Vec::new();
    for row in rows {
        let payload = row.map_err(|e| e.to_string())?;
        let node: AinlMemoryNode = serde_json::from_str(&payload).map_err(|e| e.to_string())?;
        out.push(node);
    }
    Ok(out)
}

impl<'a> GraphQuery<'a> {
    fn conn(&self) -> &rusqlite::Connection {
        self.store.conn()
    }

    pub fn episodes(&self) -> Result<Vec<AinlMemoryNode>, String> {
        let mut stmt = self
            .conn()
            .prepare(
                "SELECT payload FROM ainl_graph_nodes
                 WHERE node_type = 'episode'
                   AND COALESCE(json_extract(payload, '$.agent_id'), '') = ?1",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![&self.agent_id], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        load_nodes_from_payload_rows(rows)
    }

    pub fn semantic_nodes(&self) -> Result<Vec<AinlMemoryNode>, String> {
        let mut stmt = self
            .conn()
            .prepare(
                "SELECT payload FROM ainl_graph_nodes
                 WHERE node_type = 'semantic'
                   AND COALESCE(json_extract(payload, '$.agent_id'), '') = ?1",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![&self.agent_id], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        load_nodes_from_payload_rows(rows)
    }

    pub fn procedural_nodes(&self) -> Result<Vec<AinlMemoryNode>, String> {
        let mut stmt = self
            .conn()
            .prepare(
                "SELECT payload FROM ainl_graph_nodes
                 WHERE node_type = 'procedural'
                   AND COALESCE(json_extract(payload, '$.agent_id'), '') = ?1",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![&self.agent_id], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        load_nodes_from_payload_rows(rows)
    }

    pub fn persona_nodes(&self) -> Result<Vec<AinlMemoryNode>, String> {
        let mut stmt = self
            .conn()
            .prepare(
                "SELECT payload FROM ainl_graph_nodes
                 WHERE node_type = 'persona'
                   AND COALESCE(json_extract(payload, '$.agent_id'), '') = ?1",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![&self.agent_id], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        load_nodes_from_payload_rows(rows)
    }

    pub fn recent_episodes(&self, limit: usize) -> Result<Vec<AinlMemoryNode>, String> {
        let mut stmt = self
            .conn()
            .prepare(
                "SELECT payload FROM ainl_graph_nodes
                 WHERE node_type = 'episode'
                   AND COALESCE(json_extract(payload, '$.agent_id'), '') = ?1
                 ORDER BY timestamp DESC
                 LIMIT ?2",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![&self.agent_id, limit as i64], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|e| e.to_string())?;
        load_nodes_from_payload_rows(rows)
    }

    pub fn since(&self, ts: DateTime<Utc>, node_type: &str) -> Result<Vec<AinlMemoryNode>, String> {
        let col = node_type.to_ascii_lowercase();
        let since_ts = ts.timestamp();
        let mut stmt = self
            .conn()
            .prepare(
                "SELECT payload FROM ainl_graph_nodes
                 WHERE node_type = ?1
                   AND timestamp >= ?2
                   AND COALESCE(json_extract(payload, '$.agent_id'), '') = ?3
                 ORDER BY timestamp ASC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![&col, since_ts, &self.agent_id], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|e| e.to_string())?;
        load_nodes_from_payload_rows(rows)
    }

    /// All directed edges whose **both** endpoints are nodes for this `agent_id` (same rule as [`SqliteGraphStore::export_graph`]).
    pub fn subgraph_edges(&self) -> Result<Vec<SnapshotEdge>, String> {
        self.store.agent_subgraph_edges(&self.agent_id)
    }

    pub fn neighbors(&self, node_id: Uuid, edge_type: &str) -> Result<Vec<AinlMemoryNode>, String> {
        let mut stmt = self
            .conn()
            .prepare(
                "SELECT to_id FROM ainl_graph_edges
                 WHERE from_id = ?1 AND label = ?2",
            )
            .map_err(|e| e.to_string())?;
        let ids: Vec<String> = stmt
            .query_map(params![node_id.to_string(), edge_type], |row| row.get(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for sid in ids {
            let id = Uuid::parse_str(&sid).map_err(|e| e.to_string())?;
            if let Some(n) = self.store.read_node(id)? {
                out.push(n);
            }
        }
        Ok(out)
    }

    pub fn lineage(&self, node_id: Uuid) -> Result<Vec<AinlMemoryNode>, String> {
        let mut visited: HashSet<Uuid> = HashSet::new();
        let mut out = Vec::new();
        let mut queue: VecDeque<(Uuid, u32)> = VecDeque::new();
        visited.insert(node_id);
        queue.push_back((node_id, 0));

        while let Some((nid, depth)) = queue.pop_front() {
            if depth >= 20 {
                continue;
            }
            let mut stmt = self
                .conn()
                .prepare(
                    "SELECT to_id FROM ainl_graph_edges
                     WHERE from_id = ?1 AND label IN ('DERIVED_FROM', 'CAUSED_PATCH')",
                )
                .map_err(|e| e.to_string())?;
            let targets: Vec<String> = stmt
                .query_map(params![nid.to_string()], |row| row.get(0))
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            for sid in targets {
                let tid = Uuid::parse_str(&sid).map_err(|e| e.to_string())?;
                if visited.insert(tid) {
                    if let Some(n) = self.store.read_node(tid)? {
                        out.push(n.clone());
                        queue.push_back((tid, depth + 1));
                    }
                }
            }
        }

        Ok(out)
    }

    pub fn by_tag(&self, tag: &str) -> Result<Vec<AinlMemoryNode>, String> {
        let mut stmt = self
            .conn()
            .prepare(
                "SELECT DISTINCT n.payload FROM ainl_graph_nodes n
                 WHERE COALESCE(json_extract(n.payload, '$.agent_id'), '') = ?1
                   AND (
                     EXISTS (
                       SELECT 1 FROM json_each(n.payload, '$.node_type.persona_signals_emitted') j
                       WHERE j.value = ?2
                     )
                     OR EXISTS (
                       SELECT 1 FROM json_each(n.payload, '$.node_type.tags') j
                       WHERE j.value = ?2
                     )
                   )",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![&self.agent_id, tag], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        load_nodes_from_payload_rows(rows)
    }

    pub fn by_topic_cluster(&self, cluster: &str) -> Result<Vec<AinlMemoryNode>, String> {
        let like = format!("%{cluster}%");
        let mut stmt = self
            .conn()
            .prepare(
                "SELECT payload FROM ainl_graph_nodes
                 WHERE node_type = 'semantic'
                   AND COALESCE(json_extract(payload, '$.agent_id'), '') = ?1
                   AND json_extract(payload, '$.node_type.topic_cluster') LIKE ?2 ESCAPE '\\'",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![&self.agent_id, like], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        load_nodes_from_payload_rows(rows)
    }

    pub fn pattern_by_name(&self, name: &str) -> Result<Option<AinlMemoryNode>, String> {
        let mut stmt = self
            .conn()
            .prepare(
                "SELECT payload FROM ainl_graph_nodes
                 WHERE node_type = 'procedural'
                   AND COALESCE(json_extract(payload, '$.agent_id'), '') = ?1
                   AND (
                     json_extract(payload, '$.node_type.pattern_name') = ?2
                     OR json_extract(payload, '$.node_type.label') = ?2
                   )
                 ORDER BY timestamp DESC
                 LIMIT 1",
            )
            .map_err(|e| e.to_string())?;
        let row = stmt
            .query_row(params![&self.agent_id, name], |row| row.get::<_, String>(0))
            .optional()
            .map_err(|e| e.to_string())?;
        match row {
            Some(payload) => {
                let node: AinlMemoryNode =
                    serde_json::from_str(&payload).map_err(|e| e.to_string())?;
                Ok(Some(node))
            }
            None => Ok(None),
        }
    }

    pub fn active_patches(&self) -> Result<Vec<AinlMemoryNode>, String> {
        let mut stmt = self
            .conn()
            .prepare(
                "SELECT payload FROM ainl_graph_nodes
                 WHERE node_type = 'procedural'
                   AND COALESCE(json_extract(payload, '$.agent_id'), '') = ?1
                   AND (
                     json_extract(payload, '$.node_type.retired') IS NULL
                     OR json_extract(payload, '$.node_type.retired') = 0
                     OR CAST(json_extract(payload, '$.node_type.retired') AS TEXT) = 'false'
                   )",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![&self.agent_id], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        load_nodes_from_payload_rows(rows)
    }

    pub fn successful_episodes(&self, limit: usize) -> Result<Vec<AinlMemoryNode>, String> {
        let mut stmt = self
            .conn()
            .prepare(
                "SELECT payload FROM ainl_graph_nodes
                 WHERE node_type = 'episode'
                   AND COALESCE(json_extract(payload, '$.agent_id'), '') = ?1
                   AND json_extract(payload, '$.node_type.outcome') = 'success'
                 ORDER BY timestamp DESC
                 LIMIT ?2",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![&self.agent_id, limit as i64], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|e| e.to_string())?;
        load_nodes_from_payload_rows(rows)
    }

    pub fn episodes_with_tool(
        &self,
        tool_name: &str,
        limit: usize,
    ) -> Result<Vec<AinlMemoryNode>, String> {
        let mut stmt = self
            .conn()
            .prepare(
                "SELECT payload FROM ainl_graph_nodes
                 WHERE node_type = 'episode'
                   AND COALESCE(json_extract(payload, '$.agent_id'), '') = ?1
                   AND (
                     EXISTS (
                       SELECT 1 FROM json_each(json_extract(payload, '$.node_type.tools_invoked')) e
                       WHERE e.value = ?2
                     )
                     OR EXISTS (
                       SELECT 1 FROM json_each(json_extract(payload, '$.node_type.tool_calls')) e
                       WHERE e.value = ?2
                     )
                   )
                 ORDER BY timestamp DESC
                 LIMIT ?3",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![&self.agent_id, tool_name, limit as i64], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|e| e.to_string())?;
        load_nodes_from_payload_rows(rows)
    }

    pub fn evolved_persona(&self) -> Result<Option<AinlMemoryNode>, String> {
        let mut stmt = self
            .conn()
            .prepare(
                "SELECT payload FROM ainl_graph_nodes
                 WHERE node_type = 'persona'
                   AND COALESCE(json_extract(payload, '$.agent_id'), '') = ?1
                   AND json_extract(payload, '$.node_type.trait_name') = 'axis_evolution_snapshot'
                 ORDER BY timestamp DESC
                 LIMIT 1",
            )
            .map_err(|e| e.to_string())?;
        let row = stmt
            .query_row(params![&self.agent_id], |row| row.get::<_, String>(0))
            .optional()
            .map_err(|e| e.to_string())?;
        match row {
            Some(payload) => {
                let node: AinlMemoryNode =
                    serde_json::from_str(&payload).map_err(|e| e.to_string())?;
                Ok(Some(node))
            }
            None => Ok(None),
        }
    }

    /// Latest persisted [`RuntimeStateNode`] for this query's `agent_id`.
    pub fn read_runtime_state(&self) -> Result<Option<RuntimeStateNode>, String> {
        self.store.read_runtime_state(&self.agent_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::AinlMemoryNode;
    use crate::store::SqliteGraphStore;

    #[test]
    fn test_recall_recent_with_tool_filter() {
        let temp_dir = std::env::temp_dir();
        let db_path = temp_dir.join("ainl_query_test_recall.db");
        let _ = std::fs::remove_file(&db_path);

        let store = SqliteGraphStore::open(&db_path).expect("Failed to open store");

        let now = chrono::Utc::now().timestamp();

        let node1 = AinlMemoryNode::new_episode(
            uuid::Uuid::new_v4(),
            now,
            vec!["file_read".to_string()],
            None,
            None,
        );

        let node2 = AinlMemoryNode::new_episode(
            uuid::Uuid::new_v4(),
            now + 1,
            vec!["agent_delegate".to_string()],
            Some("agent-B".to_string()),
            None,
        );

        store.write_node(&node1).expect("Write failed");
        store.write_node(&node2).expect("Write failed");

        let delegations =
            recall_recent(&store, now - 100, 10, Some("agent_delegate")).expect("Query failed");

        assert_eq!(delegations.len(), 1);
    }

    #[test]
    fn test_find_high_confidence_facts() {
        let temp_dir = std::env::temp_dir();
        let db_path = temp_dir.join("ainl_query_test_facts.db");
        let _ = std::fs::remove_file(&db_path);

        let store = SqliteGraphStore::open(&db_path).expect("Failed to open store");

        let turn_id = uuid::Uuid::new_v4();

        let fact1 = AinlMemoryNode::new_fact("User prefers Rust".to_string(), 0.95, turn_id);
        let fact2 = AinlMemoryNode::new_fact("User dislikes Python".to_string(), 0.45, turn_id);

        store.write_node(&fact1).expect("Write failed");
        store.write_node(&fact2).expect("Write failed");

        let high_conf = find_high_confidence_facts(&store, 0.7).expect("Query failed");

        assert_eq!(high_conf.len(), 1);
    }

    #[test]
    fn test_query_active_patches() {
        let path = std::env::temp_dir().join(format!(
            "ainl_query_active_patch_{}.db",
            uuid::Uuid::new_v4()
        ));
        let _ = std::fs::remove_file(&path);
        let store = SqliteGraphStore::open(&path).expect("open");
        let ag = "agent-active-patch";
        let mut p1 = AinlMemoryNode::new_pattern("pat_one".into(), vec![1, 2]);
        p1.agent_id = ag.into();
        let mut p2 = AinlMemoryNode::new_pattern("pat_two".into(), vec![3, 4]);
        p2.agent_id = ag.into();
        store.write_node(&p1).expect("w1");
        store.write_node(&p2).expect("w2");

        let conn = store.conn();
        let payload2: String = conn
            .query_row(
                "SELECT payload FROM ainl_graph_nodes WHERE id = ?1",
                [p2.id.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        let mut v: serde_json::Value = serde_json::from_str(&payload2).unwrap();
        v["node_type"]["retired"] = serde_json::json!(true);
        conn.execute(
            "UPDATE ainl_graph_nodes SET payload = ?1 WHERE id = ?2",
            rusqlite::params![v.to_string(), p2.id.to_string()],
        )
        .unwrap();

        let active = store.query(ag).active_patches().expect("q");
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, p1.id);
    }
}
