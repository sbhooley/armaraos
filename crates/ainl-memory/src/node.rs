//! AINL graph node types - the vocabulary of agent memory.
//!
//! Four core memory types: Episode (episodic), Semantic, Procedural, Persona.
//! Designed to be standalone (zero ArmaraOS deps) yet compatible with
//! OrchestrationTraceEvent serialization.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Coarse node kind for store queries (matches `node_type` column values).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AinlNodeKind {
    Episode,
    Semantic,
    Procedural,
    Persona,
}

impl AinlNodeKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Episode => "episode",
            Self::Semantic => "semantic",
            Self::Procedural => "procedural",
            Self::Persona => "persona",
        }
    }
}

/// Memory category aligned with the four memory families (episodic ↔ `Episode` nodes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryCategory {
    Persona,
    Semantic,
    Episodic,
    Procedural,
    /// Agent-scoped runtime session counters / cache hints (persisted by `ainl-runtime`).
    RuntimeState,
}

impl MemoryCategory {
    pub fn from_node_type(node_type: &AinlNodeType) -> Self {
        match node_type {
            AinlNodeType::Episode { .. } => MemoryCategory::Episodic,
            AinlNodeType::Semantic { .. } => MemoryCategory::Semantic,
            AinlNodeType::Procedural { .. } => MemoryCategory::Procedural,
            AinlNodeType::Persona { .. } => MemoryCategory::Persona,
            AinlNodeType::RuntimeState { .. } => MemoryCategory::RuntimeState,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersonaLayer {
    #[default]
    Base,
    Delta,
    Injection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersonaSource {
    SystemDefault,
    #[default]
    UserConfigured,
    Evolved,
    Feedback,
    Injection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sentiment {
    Positive,
    Neutral,
    Negative,
    Mixed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcedureType {
    #[default]
    ToolSequence,
    ResponsePattern,
    WorkflowStep,
    BehavioralRule,
}

/// One strength adjustment on a persona trait (evolution / provenance).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct StrengthEvent {
    pub delta: f32,
    pub reason: String,
    pub episode_id: String,
    pub timestamp: u64,
}

fn default_importance_score() -> f32 {
    0.5
}

fn default_semantic_confidence() -> f32 {
    0.7
}

fn default_decay_eligible() -> bool {
    true
}

fn default_success_rate() -> f32 {
    0.5
}

fn default_strength_floor() -> f32 {
    0.0
}

/// Canonical persona payload (flattened under `AinlNodeType::Persona` in JSON).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct PersonaNode {
    pub trait_name: String,
    pub strength: f32,
    #[serde(default)]
    pub learned_from: Vec<Uuid>,
    #[serde(default)]
    pub layer: PersonaLayer,
    #[serde(default)]
    pub source: PersonaSource,
    #[serde(default = "default_strength_floor")]
    pub strength_floor: f32,
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub relevance_score: f32,
    #[serde(default)]
    pub provenance_episode_ids: Vec<String>,
    #[serde(default)]
    pub evolution_log: Vec<StrengthEvent>,
    /// Optional axis-evolution bundle (`ainl-persona`); omitted in JSON → empty map.
    #[serde(default)]
    pub axis_scores: HashMap<String, f32>,
    #[serde(default)]
    pub evolution_cycle: u32,
    /// ISO-8601 timestamp of last persona evolution pass.
    #[serde(default)]
    pub last_evolved: String,
    /// Redundant copy of owning agent id (mirrors `AinlMemoryNode.agent_id` for payload consumers).
    #[serde(default)]
    pub agent_id: String,
    /// Soft labels: axes above the high-spectrum threshold, not discrete classes.
    #[serde(default)]
    pub dominant_axes: Vec<String>,
}

/// Semantic / factual memory payload.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct SemanticNode {
    pub fact: String,
    #[serde(default = "default_semantic_confidence")]
    pub confidence: f32,
    pub source_turn_id: Uuid,
    #[serde(default)]
    pub topic_cluster: Option<String>,
    #[serde(default)]
    pub source_episode_id: String,
    #[serde(default)]
    pub contradiction_ids: Vec<String>,
    #[serde(default)]
    pub last_referenced_at: u64,
    /// How many times this node has been retrieved from the store.
    /// Managed by the recall path only — never written by extractors.
    #[serde(default)]
    pub reference_count: u32,
    #[serde(default = "default_decay_eligible")]
    pub decay_eligible: bool,
    /// Optional tag hints for analytics / persona (`ainl-persona`); omitted → empty.
    #[serde(default)]
    pub tags: Vec<String>,
    /// How many times this exact fact has recurred across separate extraction events.
    /// Written by `graph_extractor` when the same fact is observed again.
    ///
    /// Do **not** use `reference_count` as a substitute: that field tracks retrieval frequency,
    /// not extraction recurrence. They measure different things. `graph_extractor` (Prompt 2)
    /// must write `recurrence_count` directly; persona / domain extractors gate on this field only.
    #[serde(default)]
    pub recurrence_count: u32,
    /// `reference_count` snapshot from the last graph-extractor pass (JSON key `_last_ref_snapshot`).
    #[serde(rename = "_last_ref_snapshot", default)]
    pub last_ref_snapshot: u32,
}

/// Episodic memory payload (one turn / moment).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct EpisodicNode {
    pub turn_id: Uuid,
    pub timestamp: i64,
    #[serde(default)]
    pub tool_calls: Vec<String>,
    #[serde(default)]
    pub delegation_to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_event: Option<serde_json::Value>,
    #[serde(default)]
    pub turn_index: u32,
    #[serde(default)]
    pub user_message_tokens: u32,
    #[serde(default)]
    pub assistant_response_tokens: u32,
    /// Preferred list of tools for analytics; mirrors `tool_calls` when not set explicitly.
    #[serde(default)]
    pub tools_invoked: Vec<String>,
    /// Persona signal names emitted this turn (`Vec`, never `Option`). Omitted JSON → `[]`.
    /// Serialized even when empty (no `skip_serializing_if`). Backfill: `read_node` → patch → `write_node`.
    #[serde(default)]
    pub persona_signals_emitted: Vec<String>,
    #[serde(default)]
    pub sentiment: Option<Sentiment>,
    #[serde(default)]
    pub flagged: bool,
    #[serde(default)]
    pub conversation_id: String,
    #[serde(default)]
    pub follows_episode_id: Option<String>,
    /// Optional raw user message for offline extractors (`ainl-graph-extractor`); omitted unless set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_message: Option<String>,
    /// Optional assistant reply text for offline extractors; omitted unless set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_response: Option<String>,
}

impl EpisodicNode {
    /// Effective tool list: `tools_invoked` if non-empty, else `tool_calls`.
    pub fn effective_tools(&self) -> &[String] {
        if !self.tools_invoked.is_empty() {
            &self.tools_invoked
        } else {
            &self.tool_calls
        }
    }
}

/// Procedural memory payload.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ProceduralNode {
    pub pattern_name: String,
    #[serde(default)]
    pub compiled_graph: Vec<u8>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_sequence: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
    #[serde(default)]
    pub procedure_type: ProcedureType,
    #[serde(default)]
    pub trigger_conditions: Vec<String>,
    #[serde(default)]
    pub success_count: u32,
    #[serde(default)]
    pub failure_count: u32,
    #[serde(default = "default_success_rate")]
    pub success_rate: f32,
    #[serde(default)]
    pub last_invoked_at: u64,
    #[serde(default)]
    pub reinforcement_episode_ids: Vec<String>,
    #[serde(default)]
    pub suppression_episode_ids: Vec<String>,
    /// Graph-patch / refinement generation (`ainl-persona`); omitted JSON → 0 (skip persona extract until bumped).
    #[serde(default)]
    pub patch_version: u32,
    /// Optional fitness score in \[0,1\]; when absent, consumers may fall back to `success_rate`.
    #[serde(default)]
    pub fitness: Option<f32>,
    /// Declared read dependencies for the procedure (metadata-only hints).
    #[serde(default)]
    pub declared_reads: Vec<String>,
    /// When true, excluded from [`crate::GraphQuery::active_patches`] and skipped by patch dispatch.
    #[serde(default)]
    pub retired: bool,
    /// IR label for graph-patch identity (empty → runtimes may fall back to [`Self::pattern_name`]).
    #[serde(default)]
    pub label: String,
}

impl ProceduralNode {
    pub fn recompute_success_rate(&mut self) {
        let total = self.success_count.saturating_add(self.failure_count);
        self.success_rate = if total == 0 {
            0.5
        } else {
            self.success_count as f32 / total as f32
        };
    }
}

/// Persisted session counters and persona prompt cache for one agent (`ainl-runtime` ↔ SQLite).
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct RuntimeStateNode {
    pub agent_id: String,
    pub turn_count: u32,
    pub last_extraction_turn: u32,
    pub last_persona_prompt: Option<String>,
    pub updated_at: String,
}

/// Core AINL node types - the vocabulary of agent memory.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AinlNodeType {
    /// Episodic memory: what happened during an agent turn
    Episode {
        #[serde(flatten)]
        episodic: EpisodicNode,
    },

    /// Semantic memory: facts learned, with confidence
    Semantic {
        #[serde(flatten)]
        semantic: SemanticNode,
    },

    /// Procedural memory: reusable compiled workflow patterns
    Procedural {
        #[serde(flatten)]
        procedural: ProceduralNode,
    },

    /// Persona memory: traits learned over time
    Persona {
        #[serde(flatten)]
        persona: PersonaNode,
    },

    /// Runtime session state (turn counters, extraction cadence, persona cache snapshot).
    RuntimeState {
        runtime_state: RuntimeStateNode,
    },
}

/// A node in the AINL memory graph
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct AinlMemoryNode {
    pub id: Uuid,
    pub memory_category: MemoryCategory,
    pub importance_score: f32,
    pub agent_id: String,
    pub node_type: AinlNodeType,
    pub edges: Vec<AinlEdge>,
}

#[derive(Deserialize)]
struct AinlMemoryNodeWire {
    id: Uuid,
    #[serde(default)]
    memory_category: Option<MemoryCategory>,
    #[serde(default)]
    importance_score: Option<f32>,
    #[serde(default)]
    agent_id: Option<String>,
    node_type: AinlNodeType,
    #[serde(default)]
    edges: Vec<AinlEdge>,
}

impl From<AinlMemoryNodeWire> for AinlMemoryNode {
    fn from(w: AinlMemoryNodeWire) -> Self {
        let memory_category = w
            .memory_category
            .unwrap_or_else(|| MemoryCategory::from_node_type(&w.node_type));
        let importance_score = w.importance_score.unwrap_or_else(default_importance_score);
        Self {
            id: w.id,
            memory_category,
            importance_score,
            agent_id: w.agent_id.unwrap_or_default(),
            node_type: w.node_type,
            edges: w.edges,
        }
    }
}

impl<'de> Deserialize<'de> for AinlMemoryNode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let w = AinlMemoryNodeWire::deserialize(deserializer)?;
        Ok(Self::from(w))
    }
}

/// Typed edge connecting memory nodes
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct AinlEdge {
    /// Target node ID
    pub target_id: Uuid,

    /// Edge label (e.g., "delegated_to", "learned_from", "caused_by")
    pub label: String,
}

impl AinlMemoryNode {
    fn base(
        memory_category: MemoryCategory,
        importance_score: f32,
        agent_id: String,
        node_type: AinlNodeType,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            memory_category,
            importance_score,
            agent_id,
            node_type,
            edges: Vec::new(),
        }
    }

    /// Create a new episode node
    pub fn new_episode(
        turn_id: Uuid,
        timestamp: i64,
        tool_calls: Vec<String>,
        delegation_to: Option<String>,
        trace_event: Option<serde_json::Value>,
    ) -> Self {
        let tools_invoked = tool_calls.clone();
        let episodic = EpisodicNode {
            turn_id,
            timestamp,
            tool_calls,
            delegation_to,
            trace_event,
            turn_index: 0,
            user_message_tokens: 0,
            assistant_response_tokens: 0,
            tools_invoked,
            persona_signals_emitted: Vec::new(),
            sentiment: None,
            flagged: false,
            conversation_id: String::new(),
            follows_episode_id: None,
            user_message: None,
            assistant_response: None,
        };
        Self::base(
            MemoryCategory::Episodic,
            default_importance_score(),
            String::new(),
            AinlNodeType::Episode { episodic },
        )
    }

    /// Create a new semantic fact node
    pub fn new_fact(fact: String, confidence: f32, source_turn_id: Uuid) -> Self {
        let semantic = SemanticNode {
            fact,
            confidence,
            source_turn_id,
            topic_cluster: None,
            source_episode_id: String::new(),
            contradiction_ids: Vec::new(),
            last_referenced_at: 0,
            reference_count: 0,
            decay_eligible: true,
            tags: Vec::new(),
            recurrence_count: 0,
            last_ref_snapshot: 0,
        };
        Self::base(
            MemoryCategory::Semantic,
            default_importance_score(),
            String::new(),
            AinlNodeType::Semantic { semantic },
        )
    }

    /// Create a new procedural pattern node
    pub fn new_pattern(pattern_name: String, compiled_graph: Vec<u8>) -> Self {
        let mut procedural = ProceduralNode {
            pattern_name,
            compiled_graph,
            tool_sequence: Vec::new(),
            confidence: None,
            procedure_type: ProcedureType::default(),
            trigger_conditions: Vec::new(),
            success_count: 0,
            failure_count: 0,
            success_rate: default_success_rate(),
            last_invoked_at: 0,
            reinforcement_episode_ids: Vec::new(),
            suppression_episode_ids: Vec::new(),
            patch_version: 1,
            fitness: None,
            declared_reads: Vec::new(),
            retired: false,
            label: String::new(),
        };
        procedural.recompute_success_rate();
        Self::base(
            MemoryCategory::Procedural,
            default_importance_score(),
            String::new(),
            AinlNodeType::Procedural { procedural },
        )
    }

    /// Procedural node from a detected tool workflow (no compiled IR).
    pub fn new_procedural_tools(
        pattern_name: String,
        tool_sequence: Vec<String>,
        confidence: f32,
    ) -> Self {
        let mut procedural = ProceduralNode {
            pattern_name,
            compiled_graph: Vec::new(),
            tool_sequence,
            confidence: Some(confidence),
            procedure_type: ProcedureType::ToolSequence,
            trigger_conditions: Vec::new(),
            success_count: 0,
            failure_count: 0,
            success_rate: default_success_rate(),
            last_invoked_at: 0,
            reinforcement_episode_ids: Vec::new(),
            suppression_episode_ids: Vec::new(),
            patch_version: 1,
            fitness: None,
            declared_reads: Vec::new(),
            retired: false,
            label: String::new(),
        };
        procedural.recompute_success_rate();
        Self::base(
            MemoryCategory::Procedural,
            default_importance_score(),
            String::new(),
            AinlNodeType::Procedural { procedural },
        )
    }

    /// Create a new persona trait node
    pub fn new_persona(trait_name: String, strength: f32, learned_from: Vec<Uuid>) -> Self {
        let persona = PersonaNode {
            trait_name,
            strength,
            learned_from,
            layer: PersonaLayer::default(),
            source: PersonaSource::default(),
            strength_floor: default_strength_floor(),
            locked: false,
            relevance_score: 0.0,
            provenance_episode_ids: Vec::new(),
            evolution_log: Vec::new(),
            axis_scores: HashMap::new(),
            evolution_cycle: 0,
            last_evolved: String::new(),
            agent_id: String::new(),
            dominant_axes: Vec::new(),
        };
        Self::base(
            MemoryCategory::Persona,
            default_importance_score(),
            String::new(),
            AinlNodeType::Persona { persona },
        )
    }

    pub fn episodic(&self) -> Option<&EpisodicNode> {
        match &self.node_type {
            AinlNodeType::Episode { episodic } => Some(episodic),
            _ => None,
        }
    }

    pub fn semantic(&self) -> Option<&SemanticNode> {
        match &self.node_type {
            AinlNodeType::Semantic { semantic } => Some(semantic),
            _ => None,
        }
    }

    pub fn procedural(&self) -> Option<&ProceduralNode> {
        match &self.node_type {
            AinlNodeType::Procedural { procedural } => Some(procedural),
            _ => None,
        }
    }

    pub fn persona(&self) -> Option<&PersonaNode> {
        match &self.node_type {
            AinlNodeType::Persona { persona } => Some(persona),
            _ => None,
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
