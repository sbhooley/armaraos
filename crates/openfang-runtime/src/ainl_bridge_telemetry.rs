//! Host-facing **ainl-runtime-engine** telemetry types.
//!
//! Defined outside [`crate::ainl_runtime_bridge`] so [`crate::agent_loop::AgentLoopResult`] can
//! name [`AinlBridgeTelemetry`] even when the `ainl-runtime-engine` Cargo feature is off: the bridge
//! module is not compiled in that configuration, but the per-turn shape is stable and
//! `ainl_runtime_telemetry` is still `Option<…>` (always [`None`](std::option::Option::None) when
//! the engine is unavailable).
//!
//! `AinlBridgeTurnStatus` mirrors `ainl_runtime::TurnStatus` so this module does not depend on
//! the `ainl_runtime` crate (the bridge maps between the two when the engine feature is on).

/// Soft outcome for step caps / disabled graph (mirrors `ainl_runtime::TurnStatus`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AinlBridgeTurnStatus {
    Ok,
    StepLimitExceeded { steps_executed: u32 },
    GraphMemoryDisabled,
}

/// OpenFang-facing summary of an **ainl-runtime** turn (mirrors fields surfaced on an EndTurn-style event).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AinlBridgeTelemetry {
    pub turn_status: AinlBridgeTurnStatus,
    pub partial_success: bool,
    pub warning_count: usize,
    pub has_extraction_report: bool,
    pub memory_context_recent_episodes: usize,
    pub memory_context_relevant_semantic: usize,
    pub memory_context_active_patches: usize,
    pub memory_context_has_persona_snapshot: bool,
    pub patch_dispatch_count: usize,
    pub patch_dispatch_adapter_output_count: usize,
    pub steps_executed: u64,
}
