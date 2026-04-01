//! Sync `demo/`, `examples/`, and `intelligence/` from the public `ainativelang` GitHub repo
//! into app data and mirror to `~/.armaraos/ainl-library/` for CLI/editor access.
//!
//! Disable with `ARMARAOS_AINL_LIBRARY_SYNC=0`. Skips re-download when the `main` commit SHA matches
//! the last successful sync (see `upstream_manifest.json`).

use std::fs;
use std::io::Read;
use std::path::Path;
use std::time::Duration;

use flate2::read::GzDecoder;
use openfang_kernel::ainl_library::LIBRARY_SYNC_META_FILENAME;
use openfang_kernel::config::openfang_home;
use serde::{Deserialize, Serialize};
use tar::Archive;
use tauri::AppHandle;
use tracing::{info, warn};

use crate::ainl::app_ainl_root;
use crate::ainl::AinlStatus;

const UPSTREAM_MANIFEST: &str = "upstream_manifest.json";
const GITHUB_REPO: &str = "sbhooley/ainativelang";
const BRANCH: &str = "main";
const TARBALL_URL: &str =
    "https://codeload.github.com/sbhooley/ainativelang/tar.gz/refs/heads/main";
const COMMITS_API: &str = "https://api.github.com/repos/sbhooley/ainativelang/commits/main";

/// Directories to pull from the upstream repo (first-class AINL programs).
const UPSTREAM_DIRS: &[&str] = &["demo", "examples", "intelligence"];

#[derive(Debug, Serialize, Deserialize)]
struct UpstreamManifest {
    repo: String,
    branch: String,
    commit_sha: String,
    synced_at_unix: u64,
    app_data_root: String,
    mirror_path: String,
}

/// Enrich `AinlStatus` with `library_*` fields from `upstream_manifest.json` on disk.
pub fn enrich_status_from_manifest(app: &AppHandle, status: &mut AinlStatus) {
    let Ok(root) = app_ainl_root(app) else {
        return;
    };
    let manifest_path = root.join(UPSTREAM_MANIFEST);
    let Ok(bytes) = fs::read(&manifest_path) else {
        return;
    };
    let Ok(m) = serde_json::from_slice::<UpstreamManifest>(&bytes) else {
        return;
    };
    let short = m.commit_sha.chars().take(8).collect::<String>();
    status.library_sync_ok = Some(true);
    status.library_root = Some(m.app_data_root);
    status.library_mirror = Some(m.mirror_path);
    status.upstream_commit = Some(m.commit_sha.clone());
    status.library_sync_detail = Some(format!("Synced {}@{} (commit {})", m.repo, m.branch, short));
}

pub fn apply_library_sync(app: &AppHandle, status: &mut AinlStatus) {
    if !status.ok {
        return;
    }
    if std::env::var("ARMARAOS_AINL_LIBRARY_SYNC")
        .map(|v| v == "0" || v.eq_ignore_ascii_case("false"))
        .unwrap_or(false)
    {
        status.library_sync_ok = Some(false);
        status.library_sync_detail = Some("Skipped (ARMARAOS_AINL_LIBRARY_SYNC=0)".to_string());
        return;
    }

    match sync_upstream_library(app) {
        Ok(msg) => {
            status.library_sync_ok = Some(true);
            status.library_sync_detail = Some(msg);
            enrich_status_from_manifest(app, status);
        }
        Err(e) => {
            warn!("AINL upstream library sync failed: {e}");
            status.library_sync_ok = Some(false);
            status.library_sync_detail = Some(e);
        }
    }
}

fn github_user_agent() -> String {
    format!("ArmaraOS-Desktop/{}", env!("CARGO_PKG_VERSION"))
}

/// Public `main` commit SHA for `sbhooley/ainativelang` (e.g. version UI).
pub fn fetch_main_commit_sha() -> Result<String, String> {
    let resp = ureq::get(COMMITS_API)
        .set("User-Agent", &github_user_agent())
        .set("Accept", "application/vnd.github+json")
        .timeout(Duration::from_secs(45))
        .call()
        .map_err(|e| format!("GitHub commits API: {e}"))?;
    if resp.status() != 200 {
        return Err(format!(
            "GitHub commits API HTTP {} (rate limit or network)",
            resp.status()
        ));
    }
    let mut body = String::new();
    resp.into_reader()
        .read_to_string(&mut body)
        .map_err(|e| format!("read GitHub body: {e}"))?;
    let v: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("parse GitHub JSON: {e}"))?;
    let sha = v
        .get("sha")
        .and_then(|s| s.as_str())
        .ok_or_else(|| "no sha in GitHub response".to_string())?
        .to_string();
    Ok(sha)
}

fn read_manifest_commit(app: &AppHandle) -> Option<String> {
    let root = app_ainl_root(app).ok()?;
    let bytes = fs::read(root.join(UPSTREAM_MANIFEST)).ok()?;
    let m: UpstreamManifest = serde_json::from_slice(&bytes).ok()?;
    Some(m.commit_sha)
}

fn download_tarball() -> Result<Vec<u8>, String> {
    let resp = ureq::get(TARBALL_URL)
        .set("User-Agent", &github_user_agent())
        .timeout(Duration::from_secs(180))
        .call()
        .map_err(|e| format!("download tarball: {e}"))?;
    if !(200..300).contains(&resp.status()) {
        return Err(format!("tarball HTTP {}", resp.status()));
    }
    let mut body = Vec::new();
    resp.into_reader()
        .read_to_end(&mut body)
        .map_err(|e| format!("read tarball: {e}"))?;
    if body.len() < 1000 {
        return Err("download too small or empty".to_string());
    }
    Ok(body)
}

fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for ent in fs::read_dir(src)? {
        let ent = ent?;
        let ty = ent.file_type()?;
        let s = ent.path();
        let d = dst.join(ent.file_name());
        if ty.is_dir() {
            copy_dir_all(&s, &d)?;
        } else {
            fs::copy(&s, &d)?;
        }
    }
    Ok(())
}

fn sync_upstream_library(app: &AppHandle) -> Result<String, String> {
    let want_sha = fetch_main_commit_sha()?;
    if let Some(have) = read_manifest_commit(app) {
        if have == want_sha {
            return Ok(format!(
                "Upstream library already up to date ({})",
                &want_sha[..8.min(want_sha.len())]
            ));
        }
    }

    let bytes = download_tarball()?;

    let tmp = std::env::temp_dir().join(format!(
        "armaraos-ainl-src-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    if tmp.exists() {
        let _ = fs::remove_dir_all(&tmp);
    }
    fs::create_dir_all(&tmp).map_err(|e| format!("temp dir: {e}"))?;

    let decoder = GzDecoder::new(&bytes[..]);
    let mut archive = Archive::new(decoder);
    archive.unpack(&tmp).map_err(|e| format!("untar: {e}"))?;

    let mut root = None;
    for ent in fs::read_dir(&tmp).map_err(|e| format!("read temp: {e}"))? {
        let ent = ent.map_err(|e| format!("dir entry: {e}"))?;
        let p = ent.path();
        if p.is_dir() {
            root = Some(p);
            break;
        }
    }
    let repo_root = root.ok_or_else(|| "no root folder in tarball".to_string())?;

    let ainl = app_ainl_root(app)?;
    fs::create_dir_all(&ainl).map_err(|e| format!("ainl root: {e}"))?;
    let extract = ainl.join("upstream").join("ainativelang");
    if extract.exists() {
        fs::remove_dir_all(&extract).map_err(|e| format!("remove old extract: {e}"))?;
    }
    fs::create_dir_all(&extract).map_err(|e| format!("create extract: {e}"))?;

    for dir in UPSTREAM_DIRS {
        let src = repo_root.join(dir);
        if !src.is_dir() {
            warn!("Upstream repo missing {dir}/ — skipping");
            continue;
        }
        let dst = extract.join(dir);
        copy_dir_all(&src, &dst).map_err(|e| format!("copy {dir}: {e}"))?;
    }

    // README for humans / agents
    let readme = format!(
        r#"# AINL upstream library (ArmaraOS)

Synced subset from [sbhooley/ainativelang](https://github.com/{repo}) branch `{branch}`.

- **commit:** `{commit}`
- **folders:** `demo/`, `examples/`, `intelligence/` (not the full repo)

Use the internal venv `ainl` CLI, or run from this tree with `ainl validate` / `ainl run`.
Some graphs assume a workspace layout (e.g. `memory/`); adapt paths or run from a suitable home directory.

Kernel budgeting and scheduling are independent — these are **reference and optional** automation graphs.
"#,
        repo = GITHUB_REPO,
        branch = BRANCH,
        commit = want_sha
    );
    fs::write(extract.join("README_ARMARAOS.md"), readme)
        .map_err(|e| format!("write README: {e}"))?;

    // Mirror to ~/.armaraos/ainl-library for Finder/terminal
    let home_root = openfang_home();
    fs::create_dir_all(&home_root).map_err(|e| format!("home dir: {e}"))?;
    let mirror = home_root.join("ainl-library");
    if mirror.exists() {
        fs::remove_dir_all(&mirror).map_err(|e| format!("remove old mirror: {e}"))?;
    }
    copy_dir_all(&extract, &mirror).map_err(|e| format!("mirror to home: {e}"))?;

    let synced_at_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let meta = serde_json::json!({
        "commit_sha": want_sha,
        "branch": BRANCH,
        "repo": GITHUB_REPO,
        "synced_at_unix": synced_at_unix,
    });
    fs::write(
        mirror.join(LIBRARY_SYNC_META_FILENAME),
        serde_json::to_string_pretty(&meta).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("write library meta: {e}"))?;

    let manifest = UpstreamManifest {
        repo: GITHUB_REPO.to_string(),
        branch: BRANCH.to_string(),
        commit_sha: want_sha.clone(),
        synced_at_unix: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        app_data_root: extract.display().to_string(),
        mirror_path: mirror.display().to_string(),
    };
    let json = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
    fs::write(ainl.join(UPSTREAM_MANIFEST), json).map_err(|e| e.to_string())?;

    let _ = fs::remove_dir_all(&tmp);

    info!(
        "Synced AINL upstream library to {} and mirror {}",
        extract.display(),
        mirror.display()
    );
    Ok(format!(
        "Pulled demo/, examples/, intelligence/ @ {} (mirrored to ~/.armaraos/ainl-library/)",
        &want_sha[..8.min(want_sha.len())]
    ))
}
