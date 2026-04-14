//! Patch adapter registry and reference [`GraphPatchAdapter`] for GraphPatch-shaped procedural nodes.

mod graph_patch;

pub use graph_patch::{GraphPatchAdapter, GraphPatchHostDispatch};

use std::collections::HashMap;

use crate::engine::PatchDispatchContext;

/// Trait for patch/tool adapters. Implement to give dispatched patches a host execution target.
/// Register via [`crate::AinlRuntime::register_adapter`].
///
/// Label-keyed dispatch calls [`Self::execute_patch`] with a [`PatchDispatchContext`]. The default
/// implementation delegates to [`Self::execute`] with the patch label and frame (back-compat for
/// simple adapters). The reference [`GraphPatchAdapter`] overrides [`Self::execute_patch`] to emit a
/// structured GraphPatch envelope (and optional host hook).
pub trait PatchAdapter: Send + Sync {
    /// Canonical registry key (often the procedural IR `label`, e.g. `bash` or `L_my_patch`).
    fn name(&self) -> &str;

    /// Legacy entrypoint; used by the default [`Self::execute_patch`].
    fn execute(
        &self,
        label: &str,
        frame: &HashMap<String, serde_json::Value>,
    ) -> Result<serde_json::Value, String>;

    /// Rich dispatch context (node id, procedural payload, frame). Override for GraphPatch-style hosts.
    fn execute_patch(&self, ctx: &PatchDispatchContext<'_>) -> Result<serde_json::Value, String> {
        self.execute(ctx.patch_label, ctx.frame)
    }
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
        self.adapters
            .insert(adapter.name().to_string(), Box::new(adapter));
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
