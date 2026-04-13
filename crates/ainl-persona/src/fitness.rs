//! Persona snapshot + fitness helpers (EMA targets live on `AxisState`).

use crate::axes::{AxisState, PersonaAxis};
use chrono::{DateTime, Utc};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct PersonaSnapshot {
    pub agent_id: String,
    pub axes: HashMap<PersonaAxis, AxisState>,
    pub captured_at: DateTime<Utc>,
}

impl PersonaSnapshot {
    pub fn score(&self, axis: PersonaAxis) -> f32 {
        self.axes.get(&axis).map(|s| s.score).unwrap_or(0.5)
    }
}
