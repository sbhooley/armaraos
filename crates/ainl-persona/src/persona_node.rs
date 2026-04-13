//! Persist evolved axis bundle as a dedicated `PersonaNode` row (read → patch → write).

use crate::axes::{AxisState, PersonaAxis};
use ainl_memory::{AinlMemoryNode, AinlNodeType, GraphStore, PersonaSource, SqliteGraphStore};
use chrono::Utc;
use std::collections::HashMap;

fn find_evolution_node(
    store: &SqliteGraphStore,
    agent_id: &str,
) -> Result<Option<AinlMemoryNode>, String> {
    let mut best: Option<AinlMemoryNode> = None;
    for n in store.find_by_type("persona")? {
        if n.agent_id != agent_id {
            continue;
        }
        if let AinlNodeType::Persona { persona } = &n.node_type {
            if persona.trait_name != crate::EVOLUTION_TRAIT_NAME {
                continue;
            }
            best = match best {
                None => Some(n),
                Some(prev) => {
                    let prev_cycle = match &prev.node_type {
                        AinlNodeType::Persona { persona: p } => p.evolution_cycle,
                        _ => 0,
                    };
                    let new_cycle = persona.evolution_cycle;
                    if new_cycle >= prev_cycle {
                        Some(n)
                    } else {
                        Some(prev)
                    }
                }
            };
        }
    }
    Ok(best)
}

pub fn write_evolved_persona_snapshot(
    store: &SqliteGraphStore,
    agent_id: &str,
    axes: &HashMap<PersonaAxis, AxisState>,
) -> Result<(), String> {
    let mut node = match find_evolution_node(store, agent_id)? {
        Some(n) => n,
        None => {
            let mut n =
                AinlMemoryNode::new_persona(crate::EVOLUTION_TRAIT_NAME.to_string(), 0.5, vec![]);
            n.agent_id = agent_id.to_string();
            n
        }
    };

    let persona = match &mut node.node_type {
        AinlNodeType::Persona { persona } => persona,
        _ => return Err("evolution node has unexpected type".to_string()),
    };

    let mut axis_scores: HashMap<String, f32> = HashMap::new();
    for ax in PersonaAxis::ALL {
        if let Some(st) = axes.get(&ax) {
            axis_scores.insert(ax.name().to_string(), st.score);
        }
    }

    let mut ranked: Vec<(PersonaAxis, f32)> = PersonaAxis::ALL
        .iter()
        .filter_map(|ax| axes.get(ax).map(|s| (*ax, s.score)))
        .filter(|(_, sc)| *sc > 0.65)
        .collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let dominant_axes: Vec<String> = ranked
        .into_iter()
        .map(|(a, _)| a.name().to_string())
        .collect();

    persona.axis_scores = axis_scores;
    persona.evolution_cycle = persona.evolution_cycle.saturating_add(1);
    persona.last_evolved = Utc::now().to_rfc3339();
    persona.agent_id = agent_id.to_string();
    persona.dominant_axes = dominant_axes;
    persona.source = PersonaSource::Evolved;

    store.write_node(&node)
}
