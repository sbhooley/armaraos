//! AINLBundle env + best-effort export for scheduled `ainl run` (kernel cron).
//!
//! Wired from **`openfang-kernel`** (`Kernel::cron_run_job` → `CronAction::AinlRun`):
//! - **`apply_ainl_bundle_env`** sets `AINL_BUNDLE_PATH` + `AINL_AGENT_ID` on the `ainl` child when
//!   `~/.armaraos/agents/<agent_id>/bundle.ainlbundle` exists.
//! - After a successful `ainl` exit, the kernel runs **`export_ainl_bundle_after_ainl_run_best_effort`**
//!   on the blocking pool (`tokio::task::spawn_blocking`) so Python can merge the live
//!   **`ainl_graph_memory`** bridge back into the bundle.
//!
//! See ArmaraOS **`docs/scheduled-ainl.md`** (section *AINL bundle + graph memory*).

use std::path::PathBuf;
use tokio::process::Command;
use tracing::{debug, warn};

fn agent_bundle_path(agent_id: &str) -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".armaraos")
        .join("agents")
        .join(agent_id)
        .join("bundle.ainlbundle")
}

/// If `~/.armaraos/agents/<agent_id>/bundle.ainlbundle` exists, pass its path to the AINL subprocess.
pub fn apply_ainl_bundle_env(cmd: &mut Command, agent_id: &str) {
    let bundle_path = agent_bundle_path(agent_id);
    if bundle_path.exists() {
        cmd.env("AINL_BUNDLE_PATH", bundle_path.as_os_str());
        cmd.env("AINL_AGENT_ID", agent_id);
        debug!(
            agent_id = %agent_id,
            path = %bundle_path.display(),
            "Pre-seeding AINL graph store from bundle"
        );
    }
}

const EXPORT_SCRIPT: &str = r#"
import os, pathlib, sys
agent_id = os.environ.get("AINL_EXPORT_AGENT_ID") or ""
if not agent_id:
    raise SystemExit(0)
lib = pathlib.Path(os.path.expanduser("~/.armaraos/ainl-library"))
if lib.is_dir():
    sys.path.insert(0, str(lib))
from armaraos.bridge.ainl_graph_memory import AINLGraphMemoryBridge
from runtime.ainl_bundle import AINLBundleBuilder

b = AINLGraphMemoryBridge()
b.boot(agent_id=agent_id)
bundle_path = pathlib.Path.home() / ".armaraos" / "agents" / agent_id / "bundle.ainlbundle"
bundle_path.parent.mkdir(parents=True, exist_ok=True)
if bundle_path.exists():
    src = bundle_path.read_text(encoding="utf-8")
else:
    src = "S app core noop\n\nL1:\n J 0\n"
bundle = AINLBundleBuilder(agent_id=agent_id).build(src, b)
bundle.save(str(bundle_path))
print("bundle saved:", str(bundle_path))
"#;

/// Export updated graph store back to the agent bundle (non-fatal).
pub fn export_ainl_bundle_after_ainl_run_best_effort(agent_id: &str) {
    let out = std::process::Command::new("python3")
        .env("AINL_EXPORT_AGENT_ID", agent_id)
        .arg("-c")
        .arg(EXPORT_SCRIPT)
        .output();
    if let Err(e) = out {
        warn!("AINL bundle export spawn failed: {e}");
    }
}
