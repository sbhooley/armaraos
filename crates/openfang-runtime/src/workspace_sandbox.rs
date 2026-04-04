//! Workspace filesystem sandboxing.
//!
//! Confines agent file operations to their workspace directory.
//! Prevents path traversal, symlink escapes, and access outside the sandbox.
//!
//! Read-only tools may also resolve paths under the host **AINL library** tree
//! (e.g. `~/.armaraos/ainl-library`) via the virtual prefix `ainl-library/...` or an
//! absolute path inside that directory — see [`resolve_sandbox_path_read`].

use std::path::{Path, PathBuf};

/// Canonicalize `candidate` relative to `root` and ensure the result stays under `root`
/// (after resolving symlinks). Used for workspace and AINL library roots.
fn finalize_within_root(candidate: PathBuf, root: &Path) -> Result<PathBuf, String> {
    let canon_root = root
        .canonicalize()
        .map_err(|e| format!("Failed to resolve sandbox root: {e}"))?;

    let canon_candidate = if candidate.exists() {
        candidate
            .canonicalize()
            .map_err(|e| format!("Failed to resolve path: {e}"))?
    } else {
        let parent = candidate
            .parent()
            .ok_or_else(|| "Invalid path: no parent directory".to_string())?;
        let filename = candidate
            .file_name()
            .ok_or_else(|| "Invalid path: no filename".to_string())?;
        let canon_parent = parent
            .canonicalize()
            .map_err(|e| format!("Failed to resolve parent directory: {e}"))?;
        canon_parent.join(filename)
    };

    if !canon_candidate.starts_with(&canon_root) {
        return Err(format!(
            "Access denied: path '{}' escapes allowed root '{}'",
            candidate.display(),
            root.display()
        ));
    }

    Ok(canon_candidate)
}

/// Resolve a user-supplied path within a workspace sandbox.
///
/// - Rejects `..` components outright.
/// - Relative paths are joined with `workspace_root`.
/// - Absolute paths are checked against the workspace root after canonicalization.
/// - For new files: canonicalizes the parent directory and appends the filename.
/// - The final canonical path must start with the canonical workspace root.
pub fn resolve_sandbox_path(user_path: &str, workspace_root: &Path) -> Result<PathBuf, String> {
    let path = Path::new(user_path);

    // Reject any `..` components
    for component in path.components() {
        if matches!(component, std::path::Component::ParentDir) {
            return Err("Path traversal denied: '..' components are forbidden".to_string());
        }
    }

    // Build the candidate path
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace_root.join(path)
    };

    finalize_within_root(candidate, workspace_root).map_err(|e| {
        if e.contains("escapes allowed root") {
            format!(
                "Access denied: path '{}' resolves outside workspace. \
                 To read synced AINL programs, use paths starting with `ainl-library/` \
                 (e.g. `ainl-library/examples/...`) or an absolute path under that directory.",
                user_path
            )
        } else {
            e
        }
    })
}

/// Resolve a path for **read** operations: workspace sandbox **or** the host AINL library tree.
///
/// - **`ainl-library/` prefix:** virtual path into `ainl_library_root` (e.g. `~/.armaraos/ainl-library`).
///   Example: `ainl-library/examples/wishlist/01_cache_and_memory.ainl`
/// - **Absolute path:** if it resolves under `ainl_library_root`, allowed for reads.
/// - Otherwise same as [`resolve_sandbox_path`] on `workspace_root`.
pub fn resolve_sandbox_path_read(
    user_path: &str,
    workspace_root: &Path,
    ainl_library_root: Option<&Path>,
) -> Result<PathBuf, String> {
    let trimmed = user_path.trim();

    if let Some(lib_root) = ainl_library_root {
        const PREFIX: &str = "ainl-library/";
        if trimmed == "ainl-library" || trimmed.starts_with(PREFIX) {
            let rest = trimmed
                .strip_prefix("ainl-library")
                .unwrap_or("")
                .trim_start_matches('/');
            let rest_path = Path::new(rest);
            for component in rest_path.components() {
                if matches!(component, std::path::Component::ParentDir) {
                    return Err(
                        "Path traversal denied: '..' not allowed in ainl-library paths".to_string(),
                    );
                }
            }
            let candidate = lib_root.join(rest_path);
            return finalize_within_root(candidate, lib_root).map_err(|e| {
                if e.contains("Failed to resolve sandbox root") {
                    format!(
                        "AINL library directory not available: {}. \
                         Sync or bootstrap the library (see ArmaraOS AINL docs).",
                        lib_root.display()
                    )
                } else {
                    e
                }
            });
        }

        // Absolute path: allow if under the AINL library root
        let p = Path::new(trimmed);
        if p.is_absolute() && lib_root.exists() {
            if let Ok(res) = finalize_within_root(p.to_path_buf(), lib_root) {
                return Ok(res);
            }
        }
    }

    resolve_sandbox_path(trimmed, workspace_root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_relative_path_inside_workspace() {
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        std::fs::write(data_dir.join("test.txt"), "hello").unwrap();

        let result = resolve_sandbox_path("data/test.txt", dir.path());
        assert!(result.is_ok());
        let resolved = result.unwrap();
        assert!(resolved.starts_with(dir.path().canonicalize().unwrap()));
    }

    #[test]
    fn test_absolute_path_inside_workspace() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("file.txt"), "ok").unwrap();
        let abs_path = dir.path().join("file.txt");

        let result = resolve_sandbox_path(abs_path.to_str().unwrap(), dir.path());
        assert!(result.is_ok());
    }

    #[test]
    fn test_absolute_path_outside_workspace_blocked() {
        let dir = TempDir::new().unwrap();
        let outside = std::env::temp_dir().join("outside_test.txt");
        std::fs::write(&outside, "nope").unwrap();

        let result = resolve_sandbox_path(outside.to_str().unwrap(), dir.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Access denied"));

        let _ = std::fs::remove_file(&outside);
    }

    #[test]
    fn test_dotdot_component_blocked() {
        let dir = TempDir::new().unwrap();
        let result = resolve_sandbox_path("../../../etc/passwd", dir.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Path traversal denied"));
    }

    #[test]
    fn test_nonexistent_file_with_valid_parent() {
        let dir = TempDir::new().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();

        let result = resolve_sandbox_path("data/new_file.txt", dir.path());
        assert!(result.is_ok());
        let resolved = result.unwrap();
        assert!(resolved.starts_with(dir.path().canonicalize().unwrap()));
        assert!(resolved.ends_with("new_file.txt"));
    }

    #[cfg(unix)]
    #[test]
    fn test_symlink_escape_blocked() {
        let dir = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        std::fs::write(outside.path().join("secret.txt"), "secret").unwrap();

        // Create a symlink inside the workspace pointing outside
        let link_path = dir.path().join("escape");
        std::os::unix::fs::symlink(outside.path(), &link_path).unwrap();

        let result = resolve_sandbox_path("escape/secret.txt", dir.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Access denied"));
    }
}
