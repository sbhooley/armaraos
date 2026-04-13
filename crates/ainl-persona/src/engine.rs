//! `EvolutionEngine` — orchestrates extract → ingest → persist persona snapshot.

use crate::axes::{default_axis_map, AxisState, PersonaAxis};
use crate::extractor::GraphExtractor;
use crate::fitness::PersonaSnapshot;
use crate::persona_node;
use crate::signals::RawSignal;
use ainl_memory::SqliteGraphStore;
use chrono::Utc;
use std::collections::HashMap;

pub struct EvolutionEngine {
    pub agent_id: String,
    pub axes: HashMap<PersonaAxis, AxisState>,
}

impl EvolutionEngine {
    pub fn new(agent_id: impl Into<String>) -> Self {
        let agent_id = agent_id.into();
        Self {
            axes: default_axis_map(0.5),
            agent_id,
        }
    }

    pub fn ingest_signals(&mut self, signals: Vec<RawSignal>) {
        for sig in signals {
            if let Some(state) = self.axes.get_mut(&sig.axis) {
                state.update_weighted(sig.reward, sig.weight);
            }
        }
    }

    pub fn evolve(&mut self, store: &SqliteGraphStore) -> Result<PersonaSnapshot, String> {
        let signals = GraphExtractor::extract(store, &self.agent_id)?;
        self.ingest_signals(signals);
        persona_node::write_evolved_persona_snapshot(store, &self.agent_id, &self.axes)?;
        Ok(PersonaSnapshot {
            agent_id: self.agent_id.clone(),
            axes: self.axes.clone(),
            captured_at: Utc::now(),
        })
    }

    pub fn correction_tick(&mut self, axis: PersonaAxis, correction: f32) {
        if let Some(state) = self.axes.get_mut(&axis) {
            state.update_weighted(correction.clamp(0.0, 1.0), 1.0);
        }
    }
}
