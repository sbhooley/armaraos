//! Named persona axes — soft spectra, not discrete classes.

use chrono::{DateTime, Utc};
use std::collections::HashMap;

/// α for exponential moving average updates.
pub const EMA_ALPHA: f32 = 0.2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PersonaAxis {
    Instrumentality,
    Verbosity,
    Persistence,
    Systematicity,
    Curiosity,
}

impl PersonaAxis {
    pub const ALL: [PersonaAxis; 5] = [
        PersonaAxis::Instrumentality,
        PersonaAxis::Verbosity,
        PersonaAxis::Persistence,
        PersonaAxis::Systematicity,
        PersonaAxis::Curiosity,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            PersonaAxis::Instrumentality => "Instrumentality",
            PersonaAxis::Verbosity => "Verbosity",
            PersonaAxis::Persistence => "Persistence",
            PersonaAxis::Systematicity => "Systematicity",
            PersonaAxis::Curiosity => "Curiosity",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "Instrumentality" | "instrumentality" => Some(Self::Instrumentality),
            "Verbosity" | "verbosity" => Some(Self::Verbosity),
            "Persistence" | "persistence" => Some(Self::Persistence),
            "Systematicity" | "systematicity" => Some(Self::Systematicity),
            "Curiosity" | "curiosity" => Some(Self::Curiosity),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AxisState {
    pub axis: PersonaAxis,
    /// EMA score in \[0, 1\].
    pub score: f32,
    pub sample_count: u32,
    pub last_updated: DateTime<Utc>,
}

impl AxisState {
    pub fn new(axis: PersonaAxis, initial_score: f32) -> Self {
        Self {
            axis,
            score: initial_score.clamp(0.0, 1.0),
            sample_count: 0,
            last_updated: Utc::now(),
        }
    }

    /// Plain EMA toward `reward` (no per-signal weighting).
    pub fn update_score(&mut self, reward: f32) {
        let r = reward.clamp(0.0, 1.0);
        self.score = (EMA_ALPHA * r + (1.0 - EMA_ALPHA) * self.score).clamp(0.0, 1.0);
        self.sample_count = self.sample_count.saturating_add(1);
        self.last_updated = Utc::now();
    }

    /// Weighted EMA: effective target is `reward * weight` (clamped to \[0,1\]).
    pub fn update_weighted(&mut self, reward: f32, weight: f32) {
        let w = weight.clamp(0.0, 1.0);
        let r = reward.clamp(0.0, 1.0);
        let target = (r * w).clamp(0.0, 1.0);
        self.score = (EMA_ALPHA * target + (1.0 - EMA_ALPHA) * self.score).clamp(0.0, 1.0);
        self.sample_count = self.sample_count.saturating_add(1);
        self.last_updated = Utc::now();
    }
}

pub fn default_axis_map(initial: f32) -> HashMap<PersonaAxis, AxisState> {
    PersonaAxis::ALL
        .iter()
        .copied()
        .map(|a| (a, AxisState::new(a, initial)))
        .collect()
}
