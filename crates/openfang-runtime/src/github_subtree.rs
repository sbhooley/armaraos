//! `github_subtree_download` — clone a single subdirectory of a public (or
//! token-authenticated) GitHub repo into the agent workspace, in one tool call.
//!
//! Why this exists: agents told to "download the apollo-x-bot/ directory from
//! github.com/sbhooley/ainativelang" otherwise loop on `web_fetch` one URL at
//! a time, get bored, and **claim success** despite only writing a partial set
//! of files. This tool eliminates that failure mode by doing the enumeration
//! server-side via the GitHub Trees API and returning a manifest of every
//! file it actually wrote (and every file it skipped, with a reason). The
//! agent cannot "miss" files silently — the manifest is ground truth.
//!
//! Pipeline:
//! 1. Parse `repo` → `(owner, name)` (accepts `owner/name`, `https://github.com/owner/name(.git)`,
//!    or those with a path suffix that the agent might paste).
//! 2. `GET {api_base}/repos/{owner}/{name}/git/trees/{branch}?recursive=1`
//!    (try `branch`, fall back to `main` then `master` on 404 if branch unset).
//! 3. Filter `tree[]` to `type=="blob"` entries whose `path` is inside the
//!    requested `path` (or the whole tree if `path` is empty).
//! 4. Apply `extensions` / `exclude` filters; enforce `max_files` and
//!    `max_total_bytes` caps.
//! 5. For each surviving entry: `GET {raw_base}/{owner}/{name}/{branch}/{path}`
//!    and write to the workspace under `{dest}/{relpath}` via the existing
//!    workspace sandbox (path traversal blocked, missing dirs auto-created).
//! 6. Return a JSON manifest. If any file failed mid-flight the tool returns
//!    `is_error: true` with the partial-write list, which the agent loop
//!    reports back to the model — there is no "claimed success on partial".
//!
//! Auth: pass `token` for private repos or to lift the unauthenticated rate
//! limit (~60 req/h vs 5000 req/h with a PAT). **Do not** pass an empty
//! string — that would send `Authorization: Bearer` with no value and GitHub
//! returns 401 "Bad credentials" even for public repos; omit the field or use
//! a real PAT. Headers when token is set:
//!   `Authorization: Bearer <token>`, `Accept: application/vnd.github+json`,
//!   `X-GitHub-Api-Version: 2022-11-28`.
//!
//! SSRF / safety: the tool is hardcoded to `api.github.com` and
//! `raw.githubusercontent.com` (overridable in tests via `api_base` /
//! `raw_base`), so it cannot be coerced into hitting an arbitrary host even
//! if the agent supplies a malicious repo string. Path traversal is blocked
//! by `workspace_sandbox::resolve_sandbox_path`. Per-file size cap and total
//! size cap prevent runaway downloads.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tracing::{debug, warn};

const DEFAULT_API_BASE: &str = "https://api.github.com";
const DEFAULT_RAW_BASE: &str = "https://raw.githubusercontent.com";

/// Hard ceiling on a single file (10 MiB). Larger blobs are skipped with a
/// reason rather than silently truncated.
const MAX_PER_FILE_BYTES: u64 = 10 * 1024 * 1024;

/// Default soft ceiling on total payload bytes (50 MiB) — overridable by the
/// `max_total_bytes` arg up to [`MAX_TOTAL_BYTES_HARD`].
const DEFAULT_MAX_TOTAL_BYTES: u64 = 50 * 1024 * 1024;
const MAX_TOTAL_BYTES_HARD: u64 = 200 * 1024 * 1024;

/// Default soft ceiling on file count (500) — overridable by `max_files` up to
/// [`MAX_FILES_HARD`]. Most legitimate "fetch a subdir" calls are well under
/// the default; the cap exists to stop an agent from accidentally cloning
/// `linux/` into a dashboard workspace.
const DEFAULT_MAX_FILES: usize = 500;
const MAX_FILES_HARD: usize = 5000;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

fn default_token_source_none() -> String {
    "none".to_string()
}

/// User-facing manifest returned to the agent.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DownloadManifest {
    pub ok: bool,
    /// How auth was provided: `none`, `tool_parameter`, `env:GITHUB_TOKEN`, or
    /// `env:GH_TOKEN`. Does not leak the secret; helps agents debug “why 401/404?”.
    #[serde(default = "default_token_source_none")]
    pub token_source: String,
    pub repo: String,
    pub branch: String,
    pub path: String,
    pub dest: String,
    pub files_written: Vec<WrittenFile>,
    pub total_bytes: u64,
    pub skipped: Vec<SkippedFile>,
    pub errors: Vec<FileError>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WrittenFile {
    pub repo_path: String,
    pub local_path: String,
    pub bytes: u64,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkippedFile {
    pub repo_path: String,
    pub reason: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileError {
    pub repo_path: String,
    pub error: String,
}

/// Repo identity parsed from the agent-supplied `repo` field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoSlug {
    pub owner: String,
    pub name: String,
}

/// Accept any of:
///   - `owner/name`
///   - `https://github.com/owner/name`
///   - `https://github.com/owner/name.git`
///   - `https://github.com/owner/name/tree/branch/path/...` (we drop the suffix)
///   - `git@github.com:owner/name.git`
pub fn parse_repo_slug(input: &str) -> Result<RepoSlug, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("`repo` must be `owner/name` or a github.com URL".to_string());
    }

    // SSH form: git@github.com:owner/name(.git)?
    if let Some(rest) = trimmed.strip_prefix("git@github.com:") {
        return parse_repo_slug(rest);
    }

    // Strip protocol + GitHub host. Non-github hosts must be rejected — the
    // tool is hardcoded to api.github.com / raw.githubusercontent.com, so a
    // URL like `https://example.com/owner/name` would fetch from GitHub
    // anyway and silently produce wrong results. Better to fail fast.
    let mut path_part = trimmed;
    let mut matched_github_prefix = false;
    for prefix in [
        "https://github.com/",
        "http://github.com/",
        "https://www.github.com/",
        "github.com/",
    ] {
        if let Some(rest) = path_part.strip_prefix(prefix) {
            path_part = rest;
            matched_github_prefix = true;
            break;
        }
    }
    if !matched_github_prefix
        && (trimmed.starts_with("http://") || trimmed.starts_with("https://"))
    {
        return Err(format!(
            "`{input}` is not a github.com URL — this tool only mirrors GitHub repos"
        ));
    }

    let segments: Vec<&str> = path_part.split('/').filter(|s| !s.is_empty()).collect();
    if segments.len() < 2 {
        return Err(format!(
            "Could not parse `{input}` as a repo — expected `owner/name` or a github.com URL"
        ));
    }
    let owner = segments[0].to_string();
    let name = segments[1].trim_end_matches(".git").to_string();
    if owner.is_empty() || name.is_empty() {
        return Err(format!("Empty owner or name parsed from `{input}`"));
    }
    Ok(RepoSlug { owner, name })
}

/// A single entry from the GitHub Trees API response (`tree[*]`).
#[derive(Debug, Deserialize, Clone)]
pub struct TreeEntry {
    pub path: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub size: Option<u64>,
}

/// Filter the recursive tree down to the blobs the user asked for, applying
/// the `path`, `extensions`, and `exclude` filters. Returns the surviving
/// entries plus a list of `(path, reason)` for each entry skipped by a filter.
pub fn filter_tree_entries(
    entries: &[TreeEntry],
    subpath: &str,
    extensions: &[String],
    exclude: &[String],
) -> (Vec<TreeEntry>, Vec<SkippedFile>) {
    let normalized_sub = subpath.trim_matches('/');
    let prefix = if normalized_sub.is_empty() {
        String::new()
    } else {
        format!("{normalized_sub}/")
    };

    let mut kept: Vec<TreeEntry> = Vec::new();
    let mut skipped: Vec<SkippedFile> = Vec::new();

    for entry in entries {
        if entry.kind != "blob" {
            continue;
        }
        // Subpath gate — exact match counts when caller passed a single file.
        if !prefix.is_empty()
            && !entry.path.starts_with(&prefix)
            && entry.path != normalized_sub
        {
            continue;
        }
        // Exclude-substring filter (cheap; not a full glob to keep the surface
        // small — agents have repeatedly invented their own glob shapes).
        if let Some(needle) = exclude.iter().find(|n| !n.is_empty() && entry.path.contains(n.as_str()))
        {
            skipped.push(SkippedFile {
                repo_path: entry.path.clone(),
                reason: format!("matched `exclude` token `{needle}`"),
            });
            continue;
        }
        // Extension filter — skip when the caller specified a list and this
        // entry doesn't match any of them.
        if !extensions.is_empty() {
            let lower = entry.path.to_lowercase();
            let ok = extensions.iter().any(|e| {
                let e = e.trim_start_matches('.').to_lowercase();
                lower.ends_with(&format!(".{e}"))
            });
            if !ok {
                skipped.push(SkippedFile {
                    repo_path: entry.path.clone(),
                    reason: "did not match `extensions` filter".to_string(),
                });
                continue;
            }
        }
        // Per-file size guard (best-effort: GitHub returns size for blobs).
        if let Some(sz) = entry.size {
            if sz > MAX_PER_FILE_BYTES {
                skipped.push(SkippedFile {
                    repo_path: entry.path.clone(),
                    reason: format!(
                        "blob is {} bytes (per-file cap {} bytes)",
                        sz, MAX_PER_FILE_BYTES
                    ),
                });
                continue;
            }
        }
        kept.push(entry.clone());
    }

    (kept, skipped)
}

/// Compute the workspace-relative output path for a tree entry, given the
/// requested `subpath` (which is stripped) and `dest` directory.
pub fn local_path_for(entry_path: &str, subpath: &str, dest: &str) -> PathBuf {
    let normalized_sub = subpath.trim_matches('/');
    let rel = if normalized_sub.is_empty() {
        entry_path.to_string()
    } else if entry_path == normalized_sub {
        // Single-file mode: place under dest with original filename.
        Path::new(entry_path)
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_else(|| entry_path.to_string())
    } else {
        entry_path
            .strip_prefix(&format!("{normalized_sub}/"))
            .unwrap_or(entry_path)
            .to_string()
    };
    PathBuf::from(dest.trim_matches('/')).join(rel)
}

/// Build a reqwest client for hitting the GitHub API / raw-content host with
/// the user-supplied PAT (if any).
fn build_client(token: Option<&str>) -> Result<reqwest::Client, String> {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::USER_AGENT,
        reqwest::header::HeaderValue::from_static("openfang-github-subtree/1.0"),
    );
    headers.insert(
        reqwest::header::ACCEPT,
        reqwest::header::HeaderValue::from_static("application/vnd.github+json"),
    );
    headers.insert(
        reqwest::header::HeaderName::from_static("x-github-api-version"),
        reqwest::header::HeaderValue::from_static("2022-11-28"),
    );
    // `Authorization: Bearer ` with an empty value makes GitHub return 401
    // "Bad credentials" even for public repos — so only send the header when
    // the token is non-empty after trim.
    if let Some(t) = token.map(str::trim).filter(|s| !s.is_empty()) {
        let val = format!("Bearer {}", t);
        let hv = reqwest::header::HeaderValue::from_str(&val)
            .map_err(|e| format!("Invalid `token`: {e}"))?;
        headers.insert(reqwest::header::AUTHORIZATION, hv);
    }
    reqwest::Client::builder()
        .default_headers(headers)
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))
}

/// Resolve a GitHub PAT: explicit `token` in the tool call wins, then
/// `GITHUB_TOKEN`, then `GH_TOKEN` (same as `gh` CLI / common CI). This does
/// **not** read agent “memory” or project `.env` files — only the process
/// environment the daemon was started with (e.g. `~/.armaraos/.env` when the
/// host loads it). Trims; empty/whitespace-only values are ignored.
fn resolve_github_token(explicit: Option<String>) -> (Option<String>, &'static str) {
    if let Some(ref t) = explicit {
        let t = t.trim();
        if !t.is_empty() {
            return (Some(t.to_string()), "tool_parameter");
        }
    }
    if let Ok(t) = std::env::var("GITHUB_TOKEN") {
        let t = t.trim();
        if !t.is_empty() {
            return (Some(t.to_string()), "env:GITHUB_TOKEN");
        }
    }
    if let Ok(t) = std::env::var("GH_TOKEN") {
        let t = t.trim();
        if !t.is_empty() {
            return (Some(t.to_string()), "env:GH_TOKEN");
        }
    }
    (None, "none")
}

fn enrich_tree_fetch_error(err: String, had_auth: bool) -> String {
    if had_auth {
        return err;
    }
    if err.contains(" 404 ") {
        return format!(
            "{err}\n\n\
             Hint: GitHub often returns **404** for **private** repos when no valid token is sent, \
             and sometimes for a wrong `branch` / typo in `owner/name`. For public repos, verify the \
             slug and branch. To authenticate, pass `token` in this call or set **`GITHUB_TOKEN`** \
             (or `GH_TOKEN`) in the process environment (e.g. daemon `~/.armaraos/.env`) — the tool does \
             not read `.env` from the workspace or `memory` automatically."
        );
    }
    if err.contains(" 401 ") || err.contains("Bad credentials") {
        return format!(
            "{err}\n\n\
             Hint: **`token` in the tool call must be a real GitHub PAT** or be omitted. Empty strings \
             are invalid. The chat LLM provider key is **not** a GitHub token. Check `GITHUB_TOKEN` / `GH_TOKEN` \
             in the daemon environment if you expect automatic auth."
        );
    }
    if err.contains(" 403 ") || err.contains("rate limit") {
        return format!(
            "{err}\n\n\
             Hint: Unauthenticated GitHub API quota is low (~60 req/h). Set **`GITHUB_TOKEN`** or pass \
             `token` to raise limits."
        );
    }
    err
}

#[derive(Debug, Deserialize)]
struct TreeResponse {
    tree: Vec<TreeEntry>,
    #[serde(default)]
    truncated: bool,
}

/// Fetch the recursive tree for `(owner, name, branch)`. Returns the parsed
/// tree plus whether GitHub reported truncation (so we can warn the agent).
async fn fetch_tree(
    client: &reqwest::Client,
    api_base: &str,
    owner: &str,
    name: &str,
    branch: &str,
) -> Result<TreeResponse, String> {
    let url = format!(
        "{api_base}/repos/{owner}/{name}/git/trees/{branch}?recursive=1"
    );
    debug!(url, "github_subtree: fetching tree");
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("GitHub API request failed for `{url}`: {e}"))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .unwrap_or_else(|_| "<no body>".to_string());
    if !status.is_success() {
        return Err(format!(
            "GitHub API returned {status} for {url}: {}",
            crate::str_utils::safe_truncate_str(&text, 400)
        ));
    }
    serde_json::from_str::<TreeResponse>(&text)
        .map_err(|e| format!("Failed to parse GitHub tree response: {e}"))
}

/// Fetch a single blob from raw.githubusercontent.com.
async fn fetch_blob(
    client: &reqwest::Client,
    raw_base: &str,
    owner: &str,
    name: &str,
    branch: &str,
    path: &str,
) -> Result<Vec<u8>, String> {
    let url = format!("{raw_base}/{owner}/{name}/{branch}/{path}");
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Raw GET failed for `{url}`: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("Raw GET {status} for {url}"));
    }
    resp.bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| format!("Failed to read body for {url}: {e}"))
}

/// Inputs accepted by the `github_subtree_download` tool, parsed once so the
/// async path is straight-line.
#[derive(Debug)]
pub struct DownloadParams {
    pub repo: RepoSlug,
    pub branch: String,
    pub branch_explicit: bool,
    pub path: String,
    pub dest: String,
    pub token: Option<String>,
    pub extensions: Vec<String>,
    pub exclude: Vec<String>,
    pub max_files: usize,
    pub max_total_bytes: u64,
    pub api_base: String,
    pub raw_base: String,
}

/// Parse and validate the agent-supplied JSON input.
pub fn parse_params(input: &serde_json::Value) -> Result<DownloadParams, String> {
    let repo_str = input["repo"]
        .as_str()
        .ok_or("Missing `repo` (e.g. `owner/name` or `https://github.com/owner/name`)")?;
    let repo = parse_repo_slug(repo_str)?;

    let path = input["path"]
        .as_str()
        .map(|s| s.trim_matches('/').to_string())
        .unwrap_or_default();

    let dest = input["dest"]
        .as_str()
        .map(|s| s.trim_matches('/').to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            if path.is_empty() {
                repo.name.clone()
            } else {
                Path::new(&path)
                    .file_name()
                    .map(|f| f.to_string_lossy().into_owned())
                    .unwrap_or_else(|| repo.name.clone())
            }
        });

    let branch_explicit = input.get("branch").is_some();
    let branch = input["branch"]
        .as_str()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "main".to_string());

    // Agents often pass `"token": ""` when they mean "no token". Treat empty /
    // whitespace the same as omitted — do not send `Authorization: Bearer `.
    let token = input
        .get("token")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let extensions: Vec<String> = input["extensions"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let exclude: Vec<String> = input["exclude"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let max_files = input["max_files"]
        .as_u64()
        .map(|n| (n as usize).min(MAX_FILES_HARD))
        .unwrap_or(DEFAULT_MAX_FILES);
    let max_total_bytes = input["max_total_bytes"]
        .as_u64()
        .map(|n| n.min(MAX_TOTAL_BYTES_HARD))
        .unwrap_or(DEFAULT_MAX_TOTAL_BYTES);

    let api_base = input["api_base"]
        .as_str()
        .map(|s| s.trim_end_matches('/').to_string())
        .unwrap_or_else(|| DEFAULT_API_BASE.to_string());
    let raw_base = input["raw_base"]
        .as_str()
        .map(|s| s.trim_end_matches('/').to_string())
        .unwrap_or_else(|| DEFAULT_RAW_BASE.to_string());

    Ok(DownloadParams {
        repo,
        branch,
        branch_explicit,
        path,
        dest,
        token,
        extensions,
        exclude,
        max_files,
        max_total_bytes,
        api_base,
        raw_base,
    })
}

/// Top-level entry point invoked from `tool_runner`. Returns a JSON-encoded
/// [`DownloadManifest`] on success and `Err(json)` on whole-call failure (no
/// tree fetched / no params parsed). Per-file failures show up in
/// `manifest.errors` with `manifest.ok = false`.
pub async fn run(
    input: &serde_json::Value,
    workspace_root: Option<&Path>,
) -> Result<String, String> {
    let params = parse_params(input)?;

    let (effective_token, token_source_label) = resolve_github_token(params.token.clone());
    let had_auth = effective_token.is_some();
    let client = build_client(effective_token.as_deref())?;

    // Fetch the tree, falling back from `main` → `master` only when the agent
    // did not pin a branch explicitly. This avoids the most common "wrong
    // default branch" failure without surprising callers who specified one.
    let tree_resp = match fetch_tree(
        &client,
        &params.api_base,
        &params.repo.owner,
        &params.repo.name,
        &params.branch,
    )
    .await
    {
        Ok(t) => t,
        Err(e) if !params.branch_explicit && e.contains(" 404 ") => {
            warn!(branch = %params.branch, "github_subtree: trying `master` after `main` 404");
            match fetch_tree(
                &client,
                &params.api_base,
                &params.repo.owner,
                &params.repo.name,
                "master",
            )
            .await
            {
                Ok(t) => t,
                Err(e2) => return Err(enrich_tree_fetch_error(e2, had_auth)),
            }
        }
        Err(e) => return Err(enrich_tree_fetch_error(e, had_auth)),
    };
    let effective_branch = if tree_resp.tree.is_empty() && !params.branch_explicit {
        "master".to_string()
    } else {
        params.branch.clone()
    };

    let (mut kept, mut skipped) = filter_tree_entries(
        &tree_resp.tree,
        &params.path,
        &params.extensions,
        &params.exclude,
    );

    // Enforce file count cap with explicit skip reasons (so the manifest
    // tells the agent why some entries are missing rather than leaving them
    // out silently).
    if kept.len() > params.max_files {
        let overflow = kept.split_off(params.max_files);
        for entry in overflow {
            skipped.push(SkippedFile {
                repo_path: entry.path,
                reason: format!("max_files cap ({}) reached", params.max_files),
            });
        }
    }

    let mut written: Vec<WrittenFile> = Vec::new();
    let mut errors: Vec<FileError> = Vec::new();
    let mut total_bytes: u64 = 0;

    for entry in &kept {
        if total_bytes >= params.max_total_bytes {
            skipped.push(SkippedFile {
                repo_path: entry.path.clone(),
                reason: format!(
                    "max_total_bytes cap ({} bytes) reached before this file",
                    params.max_total_bytes
                ),
            });
            continue;
        }

        let local_rel = local_path_for(&entry.path, &params.path, &params.dest);
        let local_rel_str = local_rel.to_string_lossy().to_string();
        let resolved = match crate::tool_runner::resolve_file_path(&local_rel_str, workspace_root) {
            Ok(p) => p,
            Err(e) => {
                errors.push(FileError {
                    repo_path: entry.path.clone(),
                    error: format!("path rejected by sandbox: {e}"),
                });
                continue;
            }
        };

        let bytes = match fetch_blob(
            &client,
            &params.raw_base,
            &params.repo.owner,
            &params.repo.name,
            &effective_branch,
            &entry.path,
        )
        .await
        {
            Ok(b) => b,
            Err(e) => {
                errors.push(FileError {
                    repo_path: entry.path.clone(),
                    error: e,
                });
                continue;
            }
        };

        let bytes_len = bytes.len() as u64;
        if bytes_len > MAX_PER_FILE_BYTES {
            skipped.push(SkippedFile {
                repo_path: entry.path.clone(),
                reason: format!(
                    "blob body is {} bytes (per-file cap {} bytes)",
                    bytes_len, MAX_PER_FILE_BYTES
                ),
            });
            continue;
        }
        if total_bytes + bytes_len > params.max_total_bytes {
            skipped.push(SkippedFile {
                repo_path: entry.path.clone(),
                reason: format!(
                    "would exceed max_total_bytes cap ({} bytes)",
                    params.max_total_bytes
                ),
            });
            continue;
        }

        if let Some(parent) = resolved.parent() {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                errors.push(FileError {
                    repo_path: entry.path.clone(),
                    error: format!("create_dir_all failed: {e}"),
                });
                continue;
            }
        }
        if let Err(e) = tokio::fs::write(&resolved, &bytes).await {
            errors.push(FileError {
                repo_path: entry.path.clone(),
                error: format!("write failed: {e}"),
            });
            continue;
        }
        total_bytes += bytes_len;
        written.push(WrittenFile {
            repo_path: entry.path.clone(),
            local_path: resolved.to_string_lossy().to_string(),
            bytes: bytes_len,
        });
    }

    if tree_resp.truncated {
        skipped.push(SkippedFile {
            repo_path: "<github-tree-truncated>".to_string(),
            reason: "GitHub reported the recursive tree was truncated; some files in deep paths \
                     may be missing from this manifest. Re-run with a narrower `path` to ensure \
                     full coverage."
                .to_string(),
        });
    }

    let ok = errors.is_empty();
    let manifest = DownloadManifest {
        ok,
        token_source: token_source_label.to_string(),
        repo: format!("{}/{}", params.repo.owner, params.repo.name),
        branch: effective_branch,
        path: params.path,
        dest: params.dest,
        files_written: written,
        total_bytes,
        skipped,
        errors,
    };
    let json = serde_json::to_string_pretty(&manifest)
        .map_err(|e| format!("Failed to serialize manifest: {e}"))?;
    if ok {
        Ok(json)
    } else {
        // Returning Err here makes the agent loop record this as a failed
        // tool call, which prevents the model from claiming success. The
        // manifest body is still in the error message so the model sees
        // exactly which files landed and which didn't.
        Err(json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Serializes env var tests — `GITHUB_TOKEN` / `GH_TOKEN` are process-global.
    static GITHUB_ENV_TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn resolve_github_token_prefers_tool_parameter() {
        let _g = GITHUB_ENV_TEST_LOCK.lock().unwrap();
        std::env::remove_var("GH_TOKEN");
        std::env::set_var("GITHUB_TOKEN", "from_env");
        let (t, src) = resolve_github_token(Some("from_param".to_string()));
        assert_eq!(t.as_deref(), Some("from_param"));
        assert_eq!(src, "tool_parameter");
        std::env::remove_var("GITHUB_TOKEN");
    }

    #[test]
    fn resolve_github_token_uses_github_token_env() {
        let _g = GITHUB_ENV_TEST_LOCK.lock().unwrap();
        std::env::remove_var("GH_TOKEN");
        std::env::set_var("GITHUB_TOKEN", "ghp_from_env");
        let (t, src) = resolve_github_token(None);
        assert_eq!(t.as_deref(), Some("ghp_from_env"));
        assert_eq!(src, "env:GITHUB_TOKEN");
        std::env::remove_var("GITHUB_TOKEN");
    }

    #[test]
    fn resolve_github_token_falls_back_to_gh_token() {
        let _g = GITHUB_ENV_TEST_LOCK.lock().unwrap();
        std::env::remove_var("GITHUB_TOKEN");
        std::env::set_var("GH_TOKEN", "gh_from_gh_token");
        let (t, src) = resolve_github_token(None);
        assert_eq!(t.as_deref(), Some("gh_from_gh_token"));
        assert_eq!(src, "env:GH_TOKEN");
        std::env::remove_var("GH_TOKEN");
    }

    #[test]
    fn parses_owner_name_shortform() {
        let s = parse_repo_slug("sbhooley/ainativelang").unwrap();
        assert_eq!(s.owner, "sbhooley");
        assert_eq!(s.name, "ainativelang");
    }

    #[test]
    fn parses_https_url_with_git_suffix() {
        let s = parse_repo_slug("https://github.com/sbhooley/ainativelang.git").unwrap();
        assert_eq!(s.owner, "sbhooley");
        assert_eq!(s.name, "ainativelang");
    }

    #[test]
    fn parses_url_with_tree_suffix() {
        let s = parse_repo_slug(
            "https://github.com/sbhooley/ainativelang/tree/main/apollo-x-bot",
        )
        .unwrap();
        assert_eq!(s.owner, "sbhooley");
        assert_eq!(s.name, "ainativelang");
    }

    #[test]
    fn parses_ssh_form() {
        let s = parse_repo_slug("git@github.com:sbhooley/ainativelang.git").unwrap();
        assert_eq!(s.owner, "sbhooley");
        assert_eq!(s.name, "ainativelang");
    }

    #[test]
    fn rejects_garbage_repo() {
        assert!(parse_repo_slug("").is_err());
        assert!(parse_repo_slug("not-a-repo").is_err());
        assert!(parse_repo_slug("https://example.com/owner/name").is_err());
    }

    fn entry(path: &str, kind: &str, size: Option<u64>) -> TreeEntry {
        TreeEntry {
            path: path.to_string(),
            kind: kind.to_string(),
            size,
        }
    }

    #[test]
    fn filter_keeps_only_blobs_under_subpath() {
        let entries = vec![
            entry("apollo-x-bot/README.md", "blob", Some(100)),
            entry("apollo-x-bot/modules/apollo/follow_manager.ainl", "blob", Some(200)),
            entry("apollo-x-bot", "tree", None),
            entry("apollo-x-bot/modules", "tree", None),
            entry("other/file.txt", "blob", Some(50)),
        ];
        let (kept, skipped) =
            filter_tree_entries(&entries, "apollo-x-bot", &[], &[]);
        let kept_paths: Vec<_> = kept.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(
            kept_paths,
            vec![
                "apollo-x-bot/README.md",
                "apollo-x-bot/modules/apollo/follow_manager.ainl",
            ]
        );
        assert!(skipped.is_empty());
    }

    #[test]
    fn filter_applies_extension_filter() {
        let entries = vec![
            entry("apollo-x-bot/README.md", "blob", Some(100)),
            entry("apollo-x-bot/follow.ainl", "blob", Some(50)),
            entry("apollo-x-bot/script.sh", "blob", Some(30)),
        ];
        let (kept, skipped) = filter_tree_entries(
            &entries,
            "apollo-x-bot",
            &["ainl".into(), ".md".into()],
            &[],
        );
        let kept_paths: Vec<_> = kept.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(
            kept_paths,
            vec!["apollo-x-bot/README.md", "apollo-x-bot/follow.ainl"]
        );
        assert_eq!(skipped.len(), 1);
        assert_eq!(skipped[0].repo_path, "apollo-x-bot/script.sh");
        assert!(skipped[0].reason.contains("extensions"));
    }

    #[test]
    fn filter_applies_exclude_substrings() {
        let entries = vec![
            entry("apollo-x-bot/secrets.env", "blob", Some(10)),
            entry("apollo-x-bot/keep.ainl", "blob", Some(10)),
        ];
        let (kept, skipped) = filter_tree_entries(
            &entries,
            "apollo-x-bot",
            &[],
            &["secrets".into()],
        );
        let kept_paths: Vec<_> = kept.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(kept_paths, vec!["apollo-x-bot/keep.ainl"]);
        assert_eq!(skipped[0].repo_path, "apollo-x-bot/secrets.env");
        assert!(skipped[0].reason.contains("exclude"));
    }

    #[test]
    fn filter_skips_oversized_blob() {
        let entries = vec![entry(
            "huge/binary.bin",
            "blob",
            Some(MAX_PER_FILE_BYTES + 1),
        )];
        let (kept, skipped) = filter_tree_entries(&entries, "huge", &[], &[]);
        assert!(kept.is_empty());
        assert_eq!(skipped.len(), 1);
        assert!(skipped[0].reason.contains("per-file cap"));
    }

    #[test]
    fn filter_with_empty_subpath_takes_all_blobs() {
        let entries = vec![
            entry("a/b.txt", "blob", Some(1)),
            entry("c.txt", "blob", Some(1)),
            entry("d", "tree", None),
        ];
        let (kept, _) = filter_tree_entries(&entries, "", &[], &[]);
        let paths: Vec<_> = kept.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, vec!["a/b.txt", "c.txt"]);
    }

    #[test]
    fn local_path_strips_subpath_prefix() {
        let p = local_path_for(
            "apollo-x-bot/modules/apollo/follow.ainl",
            "apollo-x-bot",
            "apollo-x-bot",
        );
        assert_eq!(
            p,
            PathBuf::from("apollo-x-bot/modules/apollo/follow.ainl")
        );
    }

    #[test]
    fn local_path_handles_custom_dest() {
        let p = local_path_for(
            "apollo-x-bot/modules/apollo/follow.ainl",
            "apollo-x-bot",
            "downloaded/bot",
        );
        assert_eq!(
            p,
            PathBuf::from("downloaded/bot/modules/apollo/follow.ainl")
        );
    }

    #[test]
    fn local_path_single_file_mode() {
        let p = local_path_for("apollo-x-bot/README.md", "apollo-x-bot/README.md", "out");
        assert_eq!(p, PathBuf::from("out/README.md"));
    }

    #[test]
    fn parse_params_defaults_dest_to_subpath_basename() {
        let input = serde_json::json!({
            "repo": "sbhooley/ainativelang",
            "path": "apollo-x-bot/modules"
        });
        let p = parse_params(&input).unwrap();
        assert_eq!(p.dest, "modules");
    }

    #[test]
    fn parse_params_defaults_dest_to_repo_name_when_path_empty() {
        let input = serde_json::json!({ "repo": "sbhooley/ainativelang" });
        let p = parse_params(&input).unwrap();
        assert_eq!(p.dest, "ainativelang");
        assert_eq!(p.path, "");
        assert_eq!(p.branch, "main");
        assert!(!p.branch_explicit);
        assert_eq!(p.max_files, DEFAULT_MAX_FILES);
        assert_eq!(p.max_total_bytes, DEFAULT_MAX_TOTAL_BYTES);
    }

    #[test]
    fn parse_params_clamps_caps_to_hard_max() {
        let input = serde_json::json!({
            "repo": "o/n",
            "max_files": 999_999,
            "max_total_bytes": 999_999_999_999u64
        });
        let p = parse_params(&input).unwrap();
        assert_eq!(p.max_files, MAX_FILES_HARD);
        assert_eq!(p.max_total_bytes, MAX_TOTAL_BYTES_HARD);
    }

    #[test]
    fn parse_params_empty_or_whitespace_token_is_no_auth() {
        for token in ["", "   ", "\n"] {
            let input = serde_json::json!({
                "repo": "sbhooley/ainativelang",
                "token": token
            });
            let p = parse_params(&input).unwrap();
            assert_eq!(p.token, None, "token={token:?}");
        }
    }

    #[tokio::test]
    async fn end_to_end_download_with_mock_github() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let api = MockServer::start().await;
        let raw = MockServer::start().await;

        // Trees API: one subdir "apollo-x-bot" with 2 blobs + 1 file outside.
        let tree_body = serde_json::json!({
            "sha": "deadbeef",
            "tree": [
                { "path": "apollo-x-bot", "type": "tree" },
                { "path": "apollo-x-bot/README.md", "type": "blob", "size": 11 },
                { "path": "apollo-x-bot/modules/follow.ainl", "type": "blob", "size": 8 },
                { "path": "other/file.txt", "type": "blob", "size": 4 }
            ],
            "truncated": false
        });
        Mock::given(method("GET"))
            .and(path("/repos/sbhooley/ainativelang/git/trees/main"))
            .respond_with(ResponseTemplate::new(200).set_body_json(tree_body))
            .mount(&api)
            .await;
        Mock::given(method("GET"))
            .and(path("/sbhooley/ainativelang/main/apollo-x-bot/README.md"))
            .respond_with(ResponseTemplate::new(200).set_body_string("hello world"))
            .mount(&raw)
            .await;
        Mock::given(method("GET"))
            .and(path(
                "/sbhooley/ainativelang/main/apollo-x-bot/modules/follow.ainl",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_string("graph {}"))
            .mount(&raw)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path();

        let input = serde_json::json!({
            "repo": "sbhooley/ainativelang",
            "path": "apollo-x-bot",
            "dest": "apollo-x-bot",
            "api_base": api.uri(),
            "raw_base": raw.uri(),
        });
        let out = run(&input, Some(workspace)).await.expect("download ok");
        let m: DownloadManifest = serde_json::from_str(&out).unwrap();
        assert!(m.ok);
        assert_eq!(m.token_source, "none");
        assert_eq!(m.files_written.len(), 2);
        assert_eq!(m.total_bytes, 11 + 8);
        assert!(m.errors.is_empty());

        // Verify both files actually landed on disk under the workspace.
        let readme = workspace.join("apollo-x-bot/README.md");
        let follow = workspace.join("apollo-x-bot/modules/follow.ainl");
        assert_eq!(tokio::fs::read_to_string(&readme).await.unwrap(), "hello world");
        assert_eq!(tokio::fs::read_to_string(&follow).await.unwrap(), "graph {}");

        // The file outside the requested subpath must not have been written.
        assert!(!workspace.join("other/file.txt").exists());
    }

    #[tokio::test]
    async fn partial_failure_returns_error_with_manifest() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let api = MockServer::start().await;
        let raw = MockServer::start().await;

        let tree_body = serde_json::json!({
            "sha": "x",
            "tree": [
                { "path": "sub/keep.txt", "type": "blob", "size": 4 },
                { "path": "sub/missing.txt", "type": "blob", "size": 4 }
            ],
            "truncated": false
        });
        Mock::given(method("GET"))
            .and(path("/repos/o/n/git/trees/main"))
            .respond_with(ResponseTemplate::new(200).set_body_json(tree_body))
            .mount(&api)
            .await;
        Mock::given(method("GET"))
            .and(path("/o/n/main/sub/keep.txt"))
            .respond_with(ResponseTemplate::new(200).set_body_string("OKAY"))
            .mount(&raw)
            .await;
        Mock::given(method("GET"))
            .and(path("/o/n/main/sub/missing.txt"))
            .respond_with(ResponseTemplate::new(404).set_body_string("not found"))
            .mount(&raw)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        let input = serde_json::json!({
            "repo": "o/n",
            "path": "sub",
            "dest": "sub",
            "api_base": api.uri(),
            "raw_base": raw.uri(),
        });
        // Per-file failure → tool returns Err so the agent loop marks the call
        // as failed (preventing "claimed success on partial download").
        let err = run(&input, Some(tmp.path())).await.expect_err("should err");
        let m: DownloadManifest = serde_json::from_str(&err).unwrap();
        assert!(!m.ok);
        assert_eq!(m.files_written.len(), 1);
        assert_eq!(m.errors.len(), 1);
        assert!(m.errors[0].error.contains("404"));
        assert!(tmp.path().join("sub/keep.txt").exists());
        assert!(!tmp.path().join("sub/missing.txt").exists());
    }
}
