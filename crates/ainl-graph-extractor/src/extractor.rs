//! One-shot extraction pass wired to [`ainl_persona::EvolutionEngine`].
//!
//! Instrumentality from episode tools is emitted by [`ainl_persona::GraphExtractor`] when the
//! episode is processable; `extract_pass` skips its redundant `tool_affinity` leg in that case
//! (see [`crate::persona_signals::extract_pass`]).

use crate::persona_signals::{extract_pass, PersonaSignalExtractorState};
use crate::recurrence::update_semantic_recurrence;
use ainl_memory::{GraphStore, SqliteGraphStore};
use ainl_persona::{EvolutionEngine, PersonaSnapshot, RawSignal};
use chrono::{DateTime, Utc};

fn store_has_any_persona_for_agent(store: &SqliteGraphStore, agent_id: &str) -> Result<bool, String> {
    for n in store.find_by_type("persona")? {
        if n.agent_id == agent_id {
            return Ok(true);
        }
    }
    Ok(false)
}

pub struct GraphExtractorTask {
    pub agent_id: String,
    pub evolution_engine: EvolutionEngine,
    pub signal_state: PersonaSignalExtractorState,
}

#[derive(Debug, Clone)]
pub struct ExtractionReport {
    pub agent_id: String,
    pub semantic_nodes_updated: usize,
    /// Raw signals returned from the store this pass (diagnostics / "what the agent saw").
    pub signals_extracted: usize,
    /// Signals that moved an axis EMA by more than the persona ingest epsilon (sparkline input).
    pub signals_applied: usize,
    /// Merged graph + pattern signals ingested this pass (diagnostics, tests).
    pub merged_signals: Vec<RawSignal>,
    pub persona_snapshot: PersonaSnapshot,
    pub timestamp: DateTime<Utc>,
}

impl GraphExtractorTask {
    pub fn new(agent_id: &str) -> Self {
        Self {
            agent_id: agent_id.to_string(),
            evolution_engine: EvolutionEngine::new(agent_id),
            signal_state: PersonaSignalExtractorState::new(),
        }
    }

    /// Semantic recurrence updates, then extract → ingest → snapshot → explicit persona write.
    pub fn run_pass(&mut self, store: &SqliteGraphStore) -> Result<ExtractionReport, String> {
        let semantic_nodes_updated = update_semantic_recurrence(store, &self.agent_id)?;
        let mut signals = self.evolution_engine.extract_signals(store)?;
        signals.extend(extract_pass(store, &self.agent_id, &mut self.signal_state)?);
        let signals_extracted = signals.len();
        let merged_signals = signals.clone();
        let signals_applied = self.evolution_engine.ingest_signals(signals);
        let persona_snapshot = self.evolution_engine.snapshot();
        // Avoid persisting a default 0.5-axis evolution node on a totally cold graph (no signals,
        // no recurrence work, no prior persona). Still persist when this pass touched semantics or
        // the agent already has any persona row so follow-up passes remain idempotent.
        let should_persist_persona = signals_extracted > 0
            || semantic_nodes_updated > 0
            || store_has_any_persona_for_agent(store, &self.agent_id)?;
        if should_persist_persona {
            self.evolution_engine
                .write_persona_node(store, &persona_snapshot)?;
        }
        Ok(ExtractionReport {
            agent_id: self.agent_id.clone(),
            semantic_nodes_updated,
            signals_extracted,
            signals_applied,
            merged_signals,
            persona_snapshot,
            timestamp: Utc::now(),
        })
    }
}
