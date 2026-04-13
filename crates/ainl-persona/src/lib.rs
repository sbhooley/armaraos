//! Persona evolution over AINL graph memory — soft axes, metadata-only signals.
//!
//! See [`EvolutionEngine`] for the main entry point.

/// Canonical [`ainl_memory::PersonaNode::trait_name`] for axis-evolution bundles.
/// `graph_extractor` (Prompt 2) should import this from the crate root when selecting
/// persona rows for domain / formality signals — do not duplicate the string.
pub const EVOLUTION_TRAIT_NAME: &str = "axis_evolution_snapshot";
/// Stable alias for the same trait string (documentation / import ergonomics).
pub const AXIS_EVOLUTION_SNAPSHOT: &str = EVOLUTION_TRAIT_NAME;

pub mod axes;
pub mod engine;
pub mod extractor;
pub mod fitness;
pub mod persona_node;
pub mod signals;

pub use axes::{AxisState, PersonaAxis};
pub use engine::{EvolutionEngine, INGEST_SCORE_EPSILON};
pub use extractor::GraphExtractor;
pub use fitness::PersonaSnapshot;
pub use signals::{MemoryNodeType, RawSignal};
