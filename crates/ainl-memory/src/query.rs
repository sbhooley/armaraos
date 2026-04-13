//! Graph traversal and querying utilities.
//!
//! Higher-level query functions built on top of GraphStore.

use crate::node::{
    AinlMemoryNode, AinlNodeType, EpisodicNode, PersonaLayer, PersonaNode, ProcedureType,
    ProceduralNode, SemanticNode, StrengthEvent,
};
use crate::store::GraphStore;
use std::collections::HashMap;
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

pub fn recall_contradictions(store: &dyn GraphStore, node_id: Uuid) -> Result<Vec<SemanticNode>, String> {
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
            let total = procedural.success_count.saturating_add(procedural.failure_count);
            if total > 0 && procedural.success_rate < threshold {
                out.push(procedural.clone());
            }
        }
    }
    Ok(out)
}

// --- Persona helpers ---

pub fn recall_strength_history(store: &dyn GraphStore, node_id: Uuid) -> Result<Vec<StrengthEvent>, String> {
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

        let delegations = recall_recent(&store, now - 100, 10, Some("agent_delegate"))
            .expect("Query failed");

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
}
