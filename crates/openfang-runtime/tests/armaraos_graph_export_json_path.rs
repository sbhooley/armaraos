//! Per-agent paths for `AINL_GRAPH_MEMORY_ARMARAOS_EXPORT` JSON snapshots.

use openfang_runtime::graph_memory_writer::armaraos_graph_memory_export_json_path;
use std::sync::Mutex;
use tempfile::tempdir;

static GRAPH_EXPORT_PATH_TEST_LOCK: Mutex<()> = Mutex::new(());

struct EnvGuard {
    key: &'static str,
    previous: Option<String>,
}

impl EnvGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let previous = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, previous }
    }

    fn unset(key: &'static str) -> Self {
        let previous = std::env::var(key).ok();
        std::env::remove_var(key);
        Self { key, previous }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(v) => std::env::set_var(self.key, v),
            None => std::env::remove_var(self.key),
        }
    }
}

#[test]
fn export_dir_env_yields_distinct_per_agent_files() {
    let _lock = GRAPH_EXPORT_PATH_TEST_LOCK.lock().unwrap();
    let dir = tempdir().unwrap();
    let _exp = EnvGuard::set(
        "AINL_GRAPH_MEMORY_ARMARAOS_EXPORT",
        dir.path().to_str().unwrap(),
    );

    let p_a = armaraos_graph_memory_export_json_path("agent-aaa");
    let p_b = armaraos_graph_memory_export_json_path("agent-bbb");
    assert_ne!(p_a, p_b);
    assert_eq!(p_a.file_name().unwrap(), "agent-aaa_graph_export.json");
    assert_eq!(p_b.file_name().unwrap(), "agent-bbb_graph_export.json");
    assert_eq!(p_a.parent().unwrap(), dir.path());
    assert_eq!(p_b.parent().unwrap(), dir.path());
}

#[test]
fn export_without_env_uses_openfang_home_agents_layout() {
    let _lock = GRAPH_EXPORT_PATH_TEST_LOCK.lock().unwrap();
    let home = tempdir().unwrap();
    let _h = EnvGuard::set("ARMARAOS_HOME", home.path().to_str().unwrap());
    let _exp = EnvGuard::unset("AINL_GRAPH_MEMORY_ARMARAOS_EXPORT");
    let _of = EnvGuard::unset("OPENFANG_HOME");

    let aid = "cafebabe-cafe-4afe-8afe-cafebabecafe";
    let p = armaraos_graph_memory_export_json_path(aid);
    let expected = home
        .path()
        .join("agents")
        .join(aid)
        .join("ainl_graph_memory_export.json");
    assert_eq!(p, expected);
}
