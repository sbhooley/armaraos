//! Metadata-driven `RawSignal` extraction from graph memory nodes.

use crate::axes::PersonaAxis;
use ainl_memory::{AinlMemoryNode, EpisodicNode, PersonaNode, ProceduralNode, SemanticNode};
use serde_json::Value;
use std::collections::HashSet;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryNodeType {
    Episodic,
    Semantic,
    Procedural,
    Persona,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RawSignal {
    pub axis: PersonaAxis,
    pub reward: f32,
    pub weight: f32,
    pub source_node_id: Uuid,
    pub source_node_type: MemoryNodeType,
}

fn trace_obj(ep: &EpisodicNode) -> Option<&serde_json::Map<String, Value>> {
    ep.trace_event.as_ref()?.as_object()
}

fn trace_outcome(ep: &EpisodicNode) -> String {
    trace_obj(ep)
        .and_then(|m| m.get("outcome"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn trace_duration_ms(ep: &EpisodicNode) -> u64 {
    trace_obj(ep)
        .and_then(|m| m.get("duration_ms"))
        .and_then(|v| v.as_u64().or_else(|| v.as_f64().map(|f| f as u64)))
        .unwrap_or(0)
}

fn trace_tool_name(ep: &EpisodicNode) -> Option<String> {
    let from_trace = trace_obj(ep)
        .and_then(|m| m.get("tool_name"))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    if from_trace.is_some() {
        return from_trace;
    }
    ep.effective_tools()
        .first()
        .map(std::string::ToString::to_string)
}

fn trace_byte_count(ep: &EpisodicNode) -> u64 {
    trace_obj(ep)
        .and_then(|m| m.get("byte_count"))
        .and_then(|v| v.as_u64().or_else(|| v.as_f64().map(|f| f as u64)))
        .unwrap_or(0)
}

fn is_instrumentality_tool(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.contains("shell")
        || n.contains("cli")
        || n.contains("mcp")
        || n.contains("compile")
        || n.contains("compiler")
        || n == "ainl"
        || n.contains("ainl_")
}

fn is_web_tool(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.contains("web_search") || n.contains("web_fetch") || n.contains("web.fetch")
}

pub fn episodic_should_process(ep: &EpisodicNode) -> bool {
    let outcome = trace_outcome(ep);
    if outcome == "success" {
        return true;
    }
    if !ep.persona_signals_emitted.is_empty() {
        return true;
    }
    false
}

/// Extract persona signals from `EpisodicNode::vitals_*` fields.
///
/// These complement the existing heuristic signals (tool names, outcomes, duration)
/// with signals derived from the LLM's own token-level confidence during generation.
///
/// Axis mappings:
/// - `Systematicity` ← high trust / reasoning/retrieval gate=pass (the agent generated
///   structured, confident output)
/// - `Curiosity` ← hallucination or creative phase (the agent was probing uncertain territory)
/// - `Persistence` ← warn/fail gate but non-zero trust (the agent continued under uncertainty)
/// - `Verbosity` ← creative phase (open-ended, expansive generation)
pub fn extract_vitals_signals(node_id: Uuid, ep: &EpisodicNode) -> Vec<RawSignal> {
    let gate = match ep.vitals_gate.as_deref() {
        Some(g) => g,
        None => return Vec::new(),
    };
    let phase = ep.vitals_phase.as_deref().unwrap_or("");
    let trust = ep.vitals_trust.unwrap_or(0.5).clamp(0.0, 1.0);

    let phase_kind = phase.split(':').next().unwrap_or("");

    let mut out = Vec::new();

    match gate {
        "pass" => {
            match phase_kind {
                "reasoning" | "retrieval" => {
                    // Confident structured output → Systematicity reward.
                    out.push(RawSignal {
                        axis: PersonaAxis::Systematicity,
                        reward: 0.7 + 0.3 * trust,
                        weight: 0.65,
                        source_node_id: node_id,
                        source_node_type: MemoryNodeType::Episodic,
                    });
                }
                "creative" => {
                    out.push(RawSignal {
                        axis: PersonaAxis::Curiosity,
                        reward: 0.65,
                        weight: 0.5,
                        source_node_id: node_id,
                        source_node_type: MemoryNodeType::Episodic,
                    });
                    out.push(RawSignal {
                        axis: PersonaAxis::Verbosity,
                        reward: 0.6,
                        weight: 0.45,
                        source_node_id: node_id,
                        source_node_type: MemoryNodeType::Episodic,
                    });
                }
                _ => {}
            }
        }
        "warn" => {
            match phase_kind {
                "hallucination" => {
                    // Agent ventured into uncertain territory — mild Curiosity nudge,
                    // negative Systematicity (reduces structured-output score slightly).
                    out.push(RawSignal {
                        axis: PersonaAxis::Curiosity,
                        reward: 0.55,
                        weight: 0.4,
                        source_node_id: node_id,
                        source_node_type: MemoryNodeType::Episodic,
                    });
                    out.push(RawSignal {
                        axis: PersonaAxis::Systematicity,
                        reward: 0.2,
                        weight: 0.4,
                        source_node_id: node_id,
                        source_node_type: MemoryNodeType::Episodic,
                    });
                }
                "refusal" => {
                    // Refusal with Warn → cautious agent; mild Systematicity signal.
                    out.push(RawSignal {
                        axis: PersonaAxis::Systematicity,
                        reward: 0.5,
                        weight: 0.3,
                        source_node_id: node_id,
                        source_node_type: MemoryNodeType::Episodic,
                    });
                }
                _ => {
                    // Generic Warn: Persistence signal if trust is non-trivial.
                    if trust > 0.3 {
                        out.push(RawSignal {
                            axis: PersonaAxis::Persistence,
                            reward: 0.55,
                            weight: 0.4,
                            source_node_id: node_id,
                            source_node_type: MemoryNodeType::Episodic,
                        });
                    }
                }
            }
        }
        "fail" => {
            // Adversarial or high-entropy hallucination: suppress Systematicity,
            // weak Persistence if the agent was still producing something.
            out.push(RawSignal {
                axis: PersonaAxis::Systematicity,
                reward: 0.1,
                weight: 0.5,
                source_node_id: node_id,
                source_node_type: MemoryNodeType::Episodic,
            });
            if trust > 0.2 {
                out.push(RawSignal {
                    axis: PersonaAxis::Persistence,
                    reward: 0.4,
                    weight: 0.3,
                    source_node_id: node_id,
                    source_node_type: MemoryNodeType::Episodic,
                });
            }
        }
        _ => {}
    }

    out
}

pub fn extract_episodic_signals(node_id: Uuid, ep: &EpisodicNode) -> Vec<RawSignal> {
    let mut out = Vec::new();

    // Vitals-derived signals (first, so they can be overridden/complemented by heuristics).
    out.extend(extract_vitals_signals(node_id, ep));

    for hint in &ep.persona_signals_emitted {
        if let Some(sig) = parse_axis_hint(node_id, hint) {
            out.push(sig);
        }
    }

    let mut tool_names: HashSet<String> = HashSet::new();
    if let Some(t) = trace_tool_name(ep) {
        tool_names.insert(t);
    }
    for t in ep.effective_tools() {
        tool_names.insert(t.clone());
    }
    for tool in tool_names {
        if is_instrumentality_tool(&tool) {
            out.push(RawSignal {
                axis: PersonaAxis::Instrumentality,
                reward: 0.8,
                weight: 0.6,
                source_node_id: node_id,
                source_node_type: MemoryNodeType::Episodic,
            });
        }
        if is_web_tool(&tool) {
            out.push(RawSignal {
                axis: PersonaAxis::Curiosity,
                reward: 0.7,
                weight: 0.5,
                source_node_id: node_id,
                source_node_type: MemoryNodeType::Episodic,
            });
        }
        if tool.to_ascii_lowercase().contains("file_write") && trace_byte_count(ep) >= 4096 {
            out.push(RawSignal {
                axis: PersonaAxis::Verbosity,
                reward: 0.6,
                weight: 0.4,
                source_node_id: node_id,
                source_node_type: MemoryNodeType::Episodic,
            });
        }
    }

    let outcome = trace_outcome(ep);
    match outcome.as_str() {
        "success" => {
            out.push(RawSignal {
                axis: PersonaAxis::Systematicity,
                reward: 0.6,
                weight: 0.5,
                source_node_id: node_id,
                source_node_type: MemoryNodeType::Episodic,
            });
        }
        "retry" | "error" => {
            out.push(RawSignal {
                axis: PersonaAxis::Curiosity,
                reward: 0.5,
                weight: 0.5,
                source_node_id: node_id,
                source_node_type: MemoryNodeType::Episodic,
            });
        }
        _ => {}
    }

    if trace_duration_ms(ep) > 5000 {
        out.push(RawSignal {
            axis: PersonaAxis::Persistence,
            reward: 0.6,
            weight: 0.4,
            source_node_id: node_id,
            source_node_type: MemoryNodeType::Episodic,
        });
    }

    out
}

fn parse_axis_hint(node_id: Uuid, hint: &str) -> Option<RawSignal> {
    let (axis_part, reward_part) = hint.split_once(':')?;
    let axis = PersonaAxis::parse(axis_part)?;
    let reward: f32 = reward_part.trim().parse().ok()?;
    Some(RawSignal {
        axis,
        reward: reward.clamp(0.0, 1.0),
        weight: 0.8,
        source_node_id: node_id,
        source_node_type: MemoryNodeType::Episodic,
    })
}

pub fn semantic_should_process(sem: &SemanticNode) -> bool {
    sem.recurrence_count >= 2
}

fn semantic_tag_strings(sem: &SemanticNode) -> Vec<String> {
    let mut tags: Vec<String> = sem.tags.iter().map(|t| t.to_ascii_lowercase()).collect();
    if let Some(tc) = &sem.topic_cluster {
        for p in tc.split([',', ';']) {
            let s = p.trim().to_ascii_lowercase();
            if !s.is_empty() {
                tags.push(s);
            }
        }
    }
    tags
}

pub fn extract_semantic_signals(node_id: Uuid, sem: &SemanticNode) -> Vec<RawSignal> {
    let mut out = Vec::new();
    let tags = semantic_tag_strings(sem);

    for t in &tags {
        if t.contains("research") || t.contains("reference") || t.contains("documentation") {
            out.push(RawSignal {
                axis: PersonaAxis::Curiosity,
                reward: 0.7,
                weight: 0.5,
                source_node_id: node_id,
                source_node_type: MemoryNodeType::Semantic,
            });
        }
        if t.contains("pattern") || t.contains("template") || t.contains("schema") {
            out.push(RawSignal {
                axis: PersonaAxis::Systematicity,
                reward: 0.7,
                weight: 0.5,
                source_node_id: node_id,
                source_node_type: MemoryNodeType::Semantic,
            });
        }
        if t.contains("summary") || t.contains("output") || t.contains("result") {
            out.push(RawSignal {
                axis: PersonaAxis::Verbosity,
                reward: 0.5,
                weight: 0.5,
                source_node_id: node_id,
                source_node_type: MemoryNodeType::Semantic,
            });
        }
    }

    if sem.recurrence_count >= 3 {
        out.push(RawSignal {
            axis: PersonaAxis::Persistence,
            reward: 0.7,
            weight: 0.6,
            source_node_id: node_id,
            source_node_type: MemoryNodeType::Semantic,
        });
    }

    out
}

pub fn procedural_should_process(proc: &ProceduralNode) -> bool {
    proc.patch_version >= 1
}

fn procedural_fitness(proc: &ProceduralNode) -> f32 {
    proc.fitness.unwrap_or(proc.success_rate).clamp(0.0, 1.0)
}

pub fn extract_procedural_signals(node_id: Uuid, proc: &ProceduralNode) -> Vec<RawSignal> {
    let mut out = Vec::new();

    if proc.patch_version >= 2 {
        out.push(RawSignal {
            axis: PersonaAxis::Systematicity,
            reward: 0.8,
            weight: 0.55,
            source_node_id: node_id,
            source_node_type: MemoryNodeType::Procedural,
        });
    }

    if procedural_fitness(proc) >= 0.7 {
        out.push(RawSignal {
            axis: PersonaAxis::Persistence,
            reward: 0.7,
            weight: 0.55,
            source_node_id: node_id,
            source_node_type: MemoryNodeType::Procedural,
        });
    }

    if !proc.declared_reads.is_empty() {
        out.push(RawSignal {
            axis: PersonaAxis::Instrumentality,
            reward: 0.6,
            weight: 0.5,
            source_node_id: node_id,
            source_node_type: MemoryNodeType::Procedural,
        });
    }

    out
}

pub fn extract_persona_priors(node_id: Uuid, persona: &PersonaNode) -> Vec<RawSignal> {
    let mut out = Vec::new();
    for (name, score) in &persona.axis_scores {
        if let Some(axis) = PersonaAxis::parse(name) {
            out.push(RawSignal {
                axis,
                reward: (*score).clamp(0.0, 1.0),
                weight: 0.3,
                source_node_id: node_id,
                source_node_type: MemoryNodeType::Persona,
            });
        }
    }
    out
}

pub fn extract_from_node(node: &AinlMemoryNode) -> Vec<RawSignal> {
    let id = node.id;
    match &node.node_type {
        ainl_memory::AinlNodeType::Episode { episodic } => {
            if episodic_should_process(episodic) {
                extract_episodic_signals(id, episodic)
            } else {
                Vec::new()
            }
        }
        ainl_memory::AinlNodeType::Semantic { semantic } => {
            if semantic_should_process(semantic) {
                extract_semantic_signals(id, semantic)
            } else {
                Vec::new()
            }
        }
        ainl_memory::AinlNodeType::Procedural { procedural } => {
            if procedural_should_process(procedural) {
                extract_procedural_signals(id, procedural)
            } else {
                Vec::new()
            }
        }
        ainl_memory::AinlNodeType::Persona { persona } => extract_persona_priors(id, persona),
        ainl_memory::AinlNodeType::RuntimeState { .. } => Vec::new(),
    }
}
