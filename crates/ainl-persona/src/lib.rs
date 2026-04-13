//! Persona evolution over AINL graph memory — soft axes, metadata-only signals.
//!
//! See [`EvolutionEngine`] for the main entry point.

pub mod axes;
pub mod engine;
pub mod extractor;
pub mod fitness;
pub mod persona_node;
pub mod signals;

pub use axes::{AxisState, PersonaAxis};
pub use engine::EvolutionEngine;
pub use extractor::GraphExtractor;
pub use fitness::PersonaSnapshot;
pub use signals::{MemoryNodeType, RawSignal};
