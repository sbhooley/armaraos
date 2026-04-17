//! Graph memory holder: direct [`GraphMemory`] by default, or `Arc<std::sync::Mutex<GraphMemory>>`
//! when the `async` feature is enabled (shared with `tokio::task::spawn_blocking` in `run_turn_async`).
//!
//! We use [`std::sync::Mutex`] (not `tokio::sync::Mutex`) so [`crate::AinlRuntime::new`] and
//! [`crate::AinlRuntime::sqlite_store`] can take short locks on **any** thread—including a Tokio
//! worker running `#[tokio::test]`—without the failure mode of locking a `tokio::sync::Mutex` from
//! a context where Tokio treats it as blocking the executor. Heavy graph / SQLite work in
//! `AinlRuntime::run_turn_async` still runs on the blocking pool (`tokio::task::spawn_blocking`).

use std::ops::Deref;
#[cfg(feature = "async")]
use std::sync::{Arc, Mutex, MutexGuard};

use ainl_memory::{GraphMemory, RuntimeStateNode, SqliteGraphStore};

/// Borrowed view of the backing SQLite store (see [`crate::AinlRuntime::sqlite_store`]).
pub struct SqliteStoreRef<'a> {
    #[cfg(not(feature = "async"))]
    inner: &'a SqliteGraphStore,
    #[cfg(feature = "async")]
    _guard: MutexGuard<'a, GraphMemory>,
}

impl<'a> SqliteStoreRef<'a> {
    #[cfg(not(feature = "async"))]
    pub(crate) fn borrowed(store: &'a SqliteGraphStore) -> Self {
        Self { inner: store }
    }

    #[cfg(feature = "async")]
    pub(crate) fn from_guard(guard: MutexGuard<'a, GraphMemory>) -> Self {
        Self { _guard: guard }
    }
}

impl Deref for SqliteStoreRef<'_> {
    type Target = SqliteGraphStore;

    fn deref(&self) -> &Self::Target {
        #[cfg(not(feature = "async"))]
        {
            self.inner
        }
        #[cfg(feature = "async")]
        {
            self._guard.sqlite_store()
        }
    }
}

pub(crate) struct GraphCell {
    #[cfg(not(feature = "async"))]
    inner: GraphMemory,
    #[cfg(feature = "async")]
    inner: Arc<Mutex<GraphMemory>>,
}

impl GraphCell {
    pub(crate) fn new(store: SqliteGraphStore) -> Self {
        let memory = GraphMemory::from_sqlite_store(store);
        #[cfg(not(feature = "async"))]
        {
            Self { inner: memory }
        }
        #[cfg(feature = "async")]
        {
            Self {
                inner: Arc::new(Mutex::new(memory)),
            }
        }
    }

    pub(crate) fn read_runtime_state(
        &self,
        agent_id: &str,
    ) -> Result<Option<RuntimeStateNode>, String> {
        self.with(|m| m.read_runtime_state(agent_id))
    }

    #[cfg(not(feature = "async"))]
    pub(crate) fn with<R, F: FnOnce(&GraphMemory) -> R>(&self, f: F) -> R {
        f(&self.inner)
    }

    #[cfg(feature = "async")]
    pub(crate) fn with<R, F: FnOnce(&GraphMemory) -> R>(&self, f: F) -> R {
        let g = self.inner.lock().expect("graph mutex poisoned");
        f(&g)
    }

    pub(crate) fn sqlite_ref(&self) -> SqliteStoreRef<'_> {
        #[cfg(not(feature = "async"))]
        {
            SqliteStoreRef::borrowed(self.inner.sqlite_store())
        }
        #[cfg(feature = "async")]
        {
            SqliteStoreRef::from_guard(self.inner.lock().expect("graph mutex"))
        }
    }

    #[cfg(feature = "async")]
    pub(crate) fn shared_arc(&self) -> Arc<Mutex<GraphMemory>> {
        Arc::clone(&self.inner)
    }
}
