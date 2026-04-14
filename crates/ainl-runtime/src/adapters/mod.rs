//! Patch adapter registry and reference [`GraphPatchAdapter`] for procedural patch dispatch.

mod graph_patch;

pub use graph_patch::{GraphPatchAdapter, GraphPatchHostDispatch};

use std::collections::HashMap;

use crate::engine::PatchDispatchContext;

/// Label-keyed procedural patch executor. Register via [`crate::AinlRuntime::register_adapter`].
///
/// Dispatch: [`crate::AinlRuntime`] resolves an adapter by procedural `label` first, then falls
/// back to [`GraphPatchAdapter::NAME`] when registered via [`crate::AinlRuntime::register_default_patch_adapters`].
pub trait PatchAdapter: Send + Sync {
    /// Label this adapter handles (matched against the procedural patch `label`).
    fn name(&self) -> &str;

    /// Execute the patch. Returns a JSON value the host can inspect.
    ///
    /// Non-fatal at the runtime layer: on [`Err`], the runtime logs and continues as a metadata
    /// dispatch (fitness update still proceeds when applicable).
    fn execute_patch(&self, ctx: &PatchDispatchContext<'_>) -> Result<serde_json::Value, String>;
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

    pub fn get(&self, label: &str) -> Option<&dyn PatchAdapter> {
        self.adapters.get(label).map(|boxed| boxed.as_ref())
    }

    pub fn registered_names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.adapters.keys().map(|s| s.as_str()).collect();
        names.sort_unstable();
        names
    }

    pub fn is_empty(&self) -> bool {
        self.adapters.is_empty()
    }
}
