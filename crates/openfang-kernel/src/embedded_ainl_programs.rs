//! Ship the repo `programs/` tree into `~/.armaraos/ainl-library/armaraos-programs/` at kernel boot.
//!
//! Upstream desktop sync mirrors `demo/`, `examples/`, `intelligence/` at the `ainl-library` root.
//! Embedded ArmaraOS graphs live under `armaraos-programs/` to avoid filename clashes.
//!
//! File bytes come from `build.rs` → `$OUT_DIR/embedded_programs.rs` (`include_bytes!` per file).

use std::fs;
use std::path::Path;

include!(concat!(env!("OUT_DIR"), "/embedded_programs.rs"));

/// Marker file with the build revision; bumped when embedded content changes meaningfully.
pub const EMBEDDED_PROGRAMS_REVISION: &str = "5";

/// `~/.armaraos/ainl-library/armaraos-programs/`
pub fn armaraos_programs_subdir(home_dir: &Path) -> std::path::PathBuf {
    home_dir.join("ainl-library").join("armaraos-programs")
}

/// Skip materialization (tests, air-gapped debugging) when set to `1` or `true`.
fn skip_embedded() -> bool {
    std::env::var("ARMARAOS_SKIP_EMBEDDED_AINL_PROGRAMS")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Write embedded `programs/` files under `armaraos-programs/`. Idempotent: overwrites when bytes differ.
/// Returns the number of files written or updated.
pub fn materialize_embedded_programs(home_dir: &Path) -> Result<usize, String> {
    if skip_embedded() {
        tracing::debug!("Skipping embedded AINL programs (ARMARAOS_SKIP_EMBEDDED_AINL_PROGRAMS)");
        return Ok(0);
    }

    let dest_base = armaraos_programs_subdir(home_dir);
    fs::create_dir_all(&dest_base).map_err(|e| format!("create {}: {e}", dest_base.display()))?;

    let mut n = 0usize;
    for (rel, bytes) in EMBEDDED_AINL_FILES {
        let out_path = dest_base.join(rel);
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
        }
        let write = match fs::read(&out_path) {
            Ok(existing) => existing != *bytes,
            Err(_) => true,
        };
        if write {
            fs::write(&out_path, bytes)
                .map_err(|e| format!("write {}: {e}", out_path.display()))?;
            n += 1;
        }
    }

    let manifest_path = dest_base.join(".embedded-revision.txt");
    let manifest_body = format!(
        "{}\n{}\n",
        EMBEDDED_PROGRAMS_REVISION,
        "Embedded ArmaraOS AINL programs (see crates/openfang-kernel/build.rs)."
    );
    let _ = fs::write(&manifest_path, &manifest_body);

    if n > 0 {
        tracing::info!(
            count = n,
            dir = %dest_base.display(),
            "Materialized embedded AINL programs"
        );
    }
    Ok(n)
}

/// Create small pointer files at `~/ainl-library/` so `file_read` matches common agent expectations.
/// (Upstream overview is `README_ARMARAOS.md`; many models guess `README.md`. Embedded revision
/// lives under `armaraos-programs/` — we mirror it at the library root for discovery.)
pub fn ensure_ainl_library_pointer_files(home_dir: &Path) -> Result<(), String> {
    let lib_root = home_dir.join("ainl-library");
    fs::create_dir_all(&lib_root).map_err(|e| format!("create {}: {e}", lib_root.display()))?;

    let readme = lib_root.join("README.md");
    if !readme.exists() {
        let body = "# AINL library (ArmaraOS)\n\n\
            **Full overview:** `README_ARMARAOS.md` (same directory).\n\n\
            - `demo/`, `examples/`, `intelligence/` — upstream examples (after desktop sync)\n\
            - `armaraos-programs/` — embedded ArmaraOS bundles (canonical `.embedded-revision.txt` for that tree)\n\n\
            **Tools:** `file_list` a directory before `file_read`; `file_read` only works on files, not folders.\n";
        fs::write(&readme, body).map_err(|e| format!("write {}: {e}", readme.display()))?;
    }

    let programs_revision = lib_root
        .join("armaraos-programs")
        .join(".embedded-revision.txt");
    let root_revision = lib_root.join(".embedded-revision.txt");
    let body = if programs_revision.exists() {
        fs::read_to_string(&programs_revision)
            .map_err(|e| format!("read {}: {e}", programs_revision.display()))?
    } else {
        format!(
            "{}\n{}\n",
            EMBEDDED_PROGRAMS_REVISION,
            "Embedded ArmaraOS AINL programs (see armaraos-programs/ after kernel materializes them)."
        )
    };
    let write = match fs::read_to_string(&root_revision) {
        Ok(existing) => existing != body,
        Err(_) => true,
    };
    if write {
        fs::write(&root_revision, body)
            .map_err(|e| format!("write {}: {e}", root_revision.display()))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn embedded_includes_subdirectories() {
        let paths: Vec<&str> = super::EMBEDDED_AINL_FILES.iter().map(|(p, _)| *p).collect();
        // Core smoke programs
        assert!(
            paths.iter().any(|p| p.contains("armaraos_health_ping")),
            "expected armaraos_health_ping in embed, got: {paths:?}"
        );
        // New operational programs (revision 4+)
        assert!(
            paths.iter().any(|p| p.contains("agent_health_monitor")),
            "expected agent_health_monitor in embed, got: {paths:?}"
        );
        assert!(
            paths.iter().any(|p| p.contains("daily_budget_digest")),
            "expected daily_budget_digest in embed, got: {paths:?}"
        );
        assert!(
            paths.iter().any(|p| p.contains("budget_threshold_alert")),
            "expected budget_threshold_alert in embed, got: {paths:?}"
        );
        assert!(
            paths.iter().any(|p| p.contains("new_version_checker")),
            "expected new_version_checker in embed, got: {paths:?}"
        );
    }
}
