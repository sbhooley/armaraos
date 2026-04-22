//! Cognitive vitals — re-export from [`ainl_contracts::vitals`] (canonical for cross-runtime use).
//!
//! Types live in `ainl-contracts` so non-OpenFang hosts (`ainl-inference-server`, MCP tooling) can share
//! the same schema without depending on `openfang-types`.

pub use ainl_contracts::vitals::{CognitivePhase, CognitiveVitals, VitalsGate};
