//! One-shot extraction pass wired to [`ainl_persona::EvolutionEngine`].

use crate::recurrence::update_semantic_recurrence;
use ainl_memory::SqliteGraphStore;
use ainl_persona::{EvolutionEngine, PersonaSnapshot};
use chrono::{DateTime, Utc};

pub struct GraphExtractorTask {
    pub agent_id: String,
    pub evolution_engine: EvolutionEngine,
}

#[derive(Debug, Clone)]
pub struct ExtractionReport {
    pub agent_id: String,
    pub semantic_nodes_updated: usize,
    pub signals_ingested: usize,
    pub persona_snapshot: PersonaSnapshot,
    pub timestamp: DateTime<Utc>,
}

impl GraphExtractorTask {
    pub fn new(agent_id: &str) -> Self {
        Self {
            agent_id: agent_id.to_string(),
            evolution_engine: EvolutionEngine::new(agent_id),
        }
    }

    /// Semantic recurrence updates, then extract → ingest → snapshot → explicit persona write.
    pub fn run_pass(&mut self, store: &SqliteGraphStore) -> Result<ExtractionReport, String> {
        let semantic_nodes_updated = update_semantic_recurrence(store, &self.agent_id)?;
        let signals = self.evolution_engine.extract_signals(store)?;
        let signals_ingested = signals.len();
        self.evolution_engine.ingest_signals(signals);
        let persona_snapshot = self.evolution_engine.snapshot();
        self.evolution_engine
            .write_persona_node(store, &persona_snapshot)?;
        Ok(ExtractionReport {
            agent_id: self.agent_id.clone(),
            semantic_nodes_updated,
            signals_ingested,
            persona_snapshot,
            timestamp: Utc::now(),
        })
    }
}
