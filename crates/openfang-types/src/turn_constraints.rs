//! Optional per-turn limits passed from API / bridges into the kernel.

use serde::{Deserialize, Serialize};

/// Constraints applied to a **single** agent turn (not persisted on the agent).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TurnConstraints {
    /// When true, the kernel drops a small set of tools for this turn only: `web_search`,
    /// `shell_exec`, outbound channels, AINL MCP, and Google Workspace MCP — used for
    /// dashboard **voice STT** turns where models otherwise misfire on transcribed proper nouns.
    #[serde(default)]
    pub voice_stt_tool_clamp: bool,
}
