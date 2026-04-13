//! Patch adapter registry (stub). v0.3+ will add real HTTP/shell/MCP implementations.

use std::collections::HashMap;

/// Trait for patch/tool adapters. Implement to give dispatched patches
/// real execution targets. Register via [`crate::AinlRuntime::register_adapter`].
///
/// v0.3.0+ will ship HTTP, shell, and MCP adapter implementations.
/// When no adapter is registered for a label, dispatch is metadata-only
/// (existing v0.2 behavior preserved).
pub trait PatchAdapter: Send + Sync {
    /// Canonical slug matching ainl-semantic-tagger tool names
    /// e.g. "bash", "search_web", "mcp", "python_repl"
    fn name(&self) -> &str;

    /// Execute the patch. Called when declared_reads are satisfied.
    fn execute(
        &self,
        label: &str,
        frame: &HashMap<String, serde_json::Value>,
    ) -> Result<serde_json::Value, String>;
}

#[derive(Default)]
pub struct AdapterRegistry {
    adapters: HashMap<String, Box<dyn PatchAdapter>>,
}

impl AdapterRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, adapter: impl PatchAdapter + 'static) {
        self.adapters.insert(adapter.name().to_string(), Box::new(adapter));
    }

    pub fn get(&self, name: &str) -> Option<&dyn PatchAdapter> {
        self.adapters.get(name).map(|boxed| boxed.as_ref())
    }

    pub fn registered_names(&self) -> Vec<&str> {
        self.adapters.keys().map(|s| s.as_str()).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.adapters.is_empty()
    }
}
