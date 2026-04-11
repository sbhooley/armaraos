//! SQLite connection pool (`r2d2`) for concurrent memory access.
//!
//! Schema migrations run **once** on a bootstrap connection before the pool exists.
//! Each pooled connection only applies PRAGMAs (`with_init`), avoiding migration races.

use crate::migration::run_migrations;
use openfang_types::config::MemoryConfig;
use openfang_types::error::{OpenFangError, OpenFangResult};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::Connection;
use std::path::Path;
use std::time::Duration;
use uuid::Uuid;

/// Shared SQLite pool type used across memory stores, metering, and audit persistence.
pub type MemorySqlitePool = Pool<SqliteConnectionManager>;

fn pragma_init_sql(busy_timeout_ms: u64) -> String {
    format!(
        "PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA busy_timeout={};",
        busy_timeout_ms
    )
}

/// Open a file-backed pool (`r2d2`).
pub fn open_file_pool(db_path: &Path, config: &MemoryConfig) -> OpenFangResult<MemorySqlitePool> {
    let busy = config.busy_timeout_ms.max(1);
    {
        let conn = Connection::open(db_path).map_err(|e| OpenFangError::Memory(e.to_string()))?;
        conn.execute_batch(&format!(
            "PRAGMA journal_mode=WAL; PRAGMA busy_timeout={};",
            busy
        ))
        .map_err(|e| OpenFangError::Memory(e.to_string()))?;
        run_migrations(&conn).map_err(|e| OpenFangError::Memory(e.to_string()))?;
    }

    let path_str = db_path
        .to_str()
        .ok_or_else(|| OpenFangError::Memory("database path is not valid UTF-8".to_string()))?;
    let manager = SqliteConnectionManager::file(path_str);
    let init = pragma_init_sql(busy);
    let manager = manager.with_init(move |c| c.execute_batch(&init));

    let max_size = config.max_connections.clamp(1, 512) as u32;
    let acquire = Duration::from_millis(config.acquire_timeout_ms.max(1));
    Pool::builder()
        .max_size(max_size)
        .connection_timeout(acquire)
        .build(manager)
        .map_err(|e| OpenFangError::Memory(e.to_string()))
}

/// In-memory shared-cache SQLite pool (tests and `MemorySubstrate::open_in_memory`).
///
/// A temporary in-memory DB is deleted when the last connection closes; we therefore
/// build the pool first, then run migrations on a pooled connection so the schema stays
/// in the shared cache for subsequent checkouts.
pub fn open_in_memory_pool(config: &MemoryConfig) -> OpenFangResult<MemorySqlitePool> {
    let uri = format!(
        "file:armaraos_mem_{}?mode=memory&cache=shared",
        Uuid::new_v4()
    );
    let busy = config.busy_timeout_ms.max(1);

    let manager = SqliteConnectionManager::file(&uri);
    let init = pragma_init_sql(busy);
    let manager = manager.with_init(move |c| c.execute_batch(&init));

    let max_size = config.max_connections.clamp(1, 512) as u32;
    let pool = Pool::builder()
        .max_size(max_size)
        .connection_timeout(Duration::from_millis(config.acquire_timeout_ms.max(1)))
        .build(manager)
        .map_err(|e| OpenFangError::Memory(e.to_string()))?;

    {
        let conn = pool
            .get()
            .map_err(|e| OpenFangError::Memory(e.to_string()))?;
        run_migrations(&conn).map_err(|e| OpenFangError::Memory(e.to_string()))?;
    }

    Ok(pool)
}
