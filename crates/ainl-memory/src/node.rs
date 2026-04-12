//! AINL graph node types - the vocabulary of agent memory.
//!
//! Four core memory types: Episode, Semantic, Procedural, Persona.
//! Designed to be standalone (zero ArmaraOS deps) yet compatible with
//! OrchestrationTraceEvent serialization.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Core AINL node types - the vocabulary of agent memory.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AinlNodeType {
    /// Episodic memory: what happened during an agent turn
    Episode {
        /// Unique turn identifier
        turn_id: Uuid,

        /// When this episode occurred (Unix timestamp)
        timestamp: i64,

        /// Tool calls executed during this turn
        tool_calls: Vec<String>,

        /// Agent this turn delegated to (if any)
        delegation_to: Option<String>,

        /// Orchestration trace event (serialized, compatible with OrchestrationTraceEvent)
        /// This allows ArmaraOS to embed full trace context without creating a dependency
        #[serde(skip_serializing_if = "Option::is_none")]
        trace_event: Option<serde_json::Value>,
    },

    /// Semantic memory: facts learned, with confidence
    Semantic {
        /// The fact itself (natural language)
        fact: String,

        /// Confidence score (0.0-1.0)
        confidence: f32,

        /// Which turn generated this fact
        source_turn_id: Uuid,
    },

    /// Procedural memory: reusable compiled workflow patterns
    Procedural {
        /// Name/identifier for this pattern
        pattern_name: String,

        /// Compiled graph representation (binary format)
        compiled_graph: Vec<u8>,
    },

    /// Persona memory: traits learned over time
    Persona {
        /// Name of the trait (e.g., "prefers_concise_responses")
        trait_name: String,

        /// Strength of this trait (0.0-1.0)
        strength: f32,

        /// Turn IDs where this trait was observed
        learned_from: Vec<Uuid>,
    },
}

/// A node in the AINL memory graph
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AinlMemoryNode {
    /// Unique node identifier
    pub id: Uuid,

    /// The node's type and payload
    pub node_type: AinlNodeType,

    /// Edges to other nodes
    pub edges: Vec<AinlEdge>,
}

/// Typed edge connecting memory nodes
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AinlEdge {
    /// Target node ID
    pub target_id: Uuid,

    /// Edge label (e.g., "delegated_to", "learned_from", "caused_by")
    pub label: String,
}

impl AinlMemoryNode {
    /// Create a new episode node
    pub fn new_episode(
        turn_id: Uuid,
        timestamp: i64,
        tool_calls: Vec<String>,
        delegation_to: Option<String>,
        trace_event: Option<serde_json::Value>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            node_type: AinlNodeType::Episode {
                turn_id,
                timestamp,
                tool_calls,
                delegation_to,
                trace_event,
            },
            edges: Vec::new(),
        }
    }

    /// Create a new semantic fact node
    pub fn new_fact(fact: String, confidence: f32, source_turn_id: Uuid) -> Self {
        Self {
            id: Uuid::new_v4(),
            node_type: AinlNodeType::Semantic {
                fact,
                confidence,
                source_turn_id,
            },
            edges: Vec::new(),
        }
    }

    /// Create a new procedural pattern node
    pub fn new_pattern(pattern_name: String, compiled_graph: Vec<u8>) -> Self {
        Self {
            id: Uuid::new_v4(),
            node_type: AinlNodeType::Procedural {
                pattern_name,
                compiled_graph,
            },
            edges: Vec::new(),
        }
    }

    /// Create a new persona trait node
    pub fn new_persona(trait_name: String, strength: f32, learned_from: Vec<Uuid>) -> Self {
        Self {
            id: Uuid::new_v4(),
            node_type: AinlNodeType::Persona {
                trait_name,
                strength,
                learned_from,
            },
            edges: Vec::new(),
        }
    }

    /// Add an edge to another node
    pub fn add_edge(&mut self, target_id: Uuid, label: impl Into<String>) {
        self.edges.push(AinlEdge {
            target_id,
            label: label.into(),
        });
    }
}
