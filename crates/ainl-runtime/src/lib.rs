//! AINL Runtime - graph-based agent programming runtime
//!
//! This crate provides the execution runtime for AINL (AI Native Language),
//! integrating with the ainl-memory graph substrate for persistent memory.

use ainl_memory::{GraphMemory, GraphStore};
use serde::{Deserialize, Serialize};

/// Configuration for the AINL runtime
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RuntimeConfig {
    /// Maximum depth for delegation chains
    pub max_delegation_depth: u32,

    /// Enable graph-based memory persistence
    pub enable_graph_memory: bool,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            max_delegation_depth: 10,
            enable_graph_memory: true,
        }
    }
}

/// AINL runtime context for execution
pub struct RuntimeContext {
    _config: RuntimeConfig,
    memory: Option<GraphMemory>,
}

impl RuntimeContext {
    /// Create a new runtime context with the given memory backend
    pub fn new(config: RuntimeConfig, memory: Option<GraphMemory>) -> Self {
        Self {
            _config: config,
            memory,
        }
    }

    /// Record an agent delegation as an episode node
    pub fn record_delegation(
        &self,
        delegated_to: String,
        trace_event: Option<serde_json::Value>,
    ) -> Result<uuid::Uuid, String> {
        if let Some(ref memory) = self.memory {
            memory.write_episode(
                vec!["agent_delegate".to_string()],
                Some(delegated_to),
                trace_event,
            )
        } else {
            Err("Memory not initialized".to_string())
        }
    }

    /// Record a tool execution as an episode node
    pub fn record_tool_execution(
        &self,
        tool_name: String,
        trace_event: Option<serde_json::Value>,
    ) -> Result<uuid::Uuid, String> {
        if let Some(ref memory) = self.memory {
            memory.write_episode(vec![tool_name], None, trace_event)
        } else {
            Err("Memory not initialized".to_string())
        }
    }

    /// Get direct access to the underlying store for advanced queries
    pub fn store(&self) -> Option<&dyn GraphStore> {
        self.memory.as_ref().map(|m| m.store())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_runtime_config_default() {
        let config = RuntimeConfig::default();
        assert_eq!(config.max_delegation_depth, 10);
        assert!(config.enable_graph_memory);
    }
}
