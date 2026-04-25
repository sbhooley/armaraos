//! Settings → **Vault** — unified secrets & credentials center (Phases A–C).
//!
//! - Phase A: catalog + set/delete using existing vault + `secrets.env` + process env.
//! - Phase B: dependency map + key-specific connectivity tests.
//! - Phase C: telemetry (last set / last test) + “stale” hints for long-lived tokens.

use axum::extract::{Extension, Path as PathParam, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::middleware::RequestId;
use crate::routes::{
    api_json_error, audit_credential_mutation, remove_secret_env, resolve_request_id, write_secret_env,
    AppState,
};

const TELEMETRY_FILE: &str = "secret_center_telemetry.json";
const STALE_SECS: u64 = 90 * 24 * 60 * 60;

#[derive(Debug, Clone, Serialize)]
struct StaticSecretDef {
    id: &'static str,
    key: &'static str,
    title: &'static str,
    description: &'static str,
    category: &'static str,
    optional: bool,
    /// What this unlocks (short labels for the inventory table).
    used_by: &'static [&'static str],
}

const STATIC_SECRETS: &[StaticSecretDef] = &[
    StaticSecretDef {
        id: "github-token",
        key: "GITHUB_TOKEN",
        title: "GitHub personal access token",
        description: "Use for `github_subtree_download`, `git` HTTPS, and to lift the GitHub REST rate limit. Not the same as the chat LLM provider key.",
        category: "github",
        optional: true,
        used_by: &["github_subtree_download", "git HTTPS", "GitHub API"],
    },
    StaticSecretDef {
        id: "gh-token-alias",
        key: "GH_TOKEN",
        title: "GitHub token (gh CLI alias)",
        description: "Same role as GITHUB_TOKEN — the `gh` CLI and many scripts read `GH_TOKEN` first.",
        category: "github",
        optional: true,
        used_by: &["gh CLI", "GitHub API"],
    },
    StaticSecretDef {
        id: "google-oauth-id",
        key: "GOOGLE_OAUTH_CLIENT_ID",
        title: "Google OAuth client ID",
        description: "For Google Workspace MCP and related flows configured from Settings → Tools.",
        category: "integration",
        optional: true,
        used_by: &["google-workspace-mcp"],
    },
    StaticSecretDef {
        id: "google-oauth-secret",
        key: "GOOGLE_OAUTH_CLIENT_SECRET",
        title: "Google OAuth client secret",
        description: "Paired with the Google OAuth client ID for Workspace MCP when required.",
        category: "integration",
        optional: true,
        used_by: &["google-workspace-mcp"],
    },
];

#[derive(Debug, Deserialize, Serialize, Default, Clone)]
struct TelemetryFile {
    v: u32,
    #[serde(default)]
    keys: HashMap<String, KeyTelemetry>,
}

#[derive(Debug, Deserialize, Serialize, Default, Clone)]
struct KeyTelemetry {
    last_set_at: Option<u64>,
    last_test: Option<TestRecord>,
    /// Heuristic: true when `last_set_at` is older than ~90d (PAT rotation hint).
    #[serde(default)]
    rotation_suggested: bool,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
struct TestRecord {
    ok: bool,
    at: u64,
    ms: Option<u64>,
    detail: Option<String>,
}

fn telemetry_path(home: &Path) -> PathBuf {
    home.join(TELEMETRY_FILE)
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn load_telemetry(home: &Path) -> TelemetryFile {
    let p = telemetry_path(home);
    if !p.exists() {
        return TelemetryFile { v: 1, keys: HashMap::new() };
    }
    std::fs::read_to_string(&p)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| TelemetryFile { v: 1, keys: HashMap::new() })
}

fn save_telemetry(home: &Path, mut t: TelemetryFile) -> Result<(), String> {
    t.v = 1;
    let p = telemetry_path(home);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(&t).map_err(|e| e.to_string())?;
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &p).map_err(|e| e.to_string())?;
    Ok(())
}

/// Every env key the user may set through this API (static + all provider `api_key_env` values).
fn allowed_key_set(state: &AppState) -> HashSet<String> {
    let mut s: HashSet<String> = HashSet::new();
    for d in STATIC_SECRETS {
        s.insert(d.key.to_string());
    }
    if let Ok(cat) = state.kernel.model_catalog.read() {
        for p in cat.list_providers() {
            if !p.api_key_env.is_empty() {
                s.insert(p.api_key_env.clone());
            }
        }
    }
    s
}

fn is_allowed_key(state: &AppState, key: &str) -> bool {
    allowed_key_set(state).contains(key)
}

// ── Phase B: reverse dependency index (for agents / help text) ────────────

#[derive(Serialize)]
struct DependencyItem {
    feature: &'static str,
    keys: Vec<&'static str>,
    optional: bool,
    notes: &'static str,
}

/// GET /api/secrets/dependencies
pub async fn get_secrets_dependencies(
    State(_state): State<Arc<AppState>>,
    ext: Option<Extension<RequestId>>,
) -> impl axum::response::IntoResponse {
    let rid = resolve_request_id(ext);
    let items = vec![
        DependencyItem {
            feature: "github_subtree_download",
            keys: vec!["GITHUB_TOKEN", "GH_TOKEN"],
            optional: true,
            notes: "Public repos work with no token; private repos and higher API quotas need a PAT stored in Vault or `token` in the tool call.",
        },
        DependencyItem {
            feature: "llm chat",
            keys: vec![],
            optional: false,
            notes: "Provider API keys are listed per provider; see Settings → Vault entries matching each provider’s env var.",
        },
    ];
    (StatusCode::OK, Json(serde_json::json!({ "request_id": rid.0, "items": items })))
}

#[derive(Deserialize, Default)]
pub struct CatalogQuery {
    /// Optional filter: only entries relevant to a tool name (e.g. `github_subtree_download`).
    pub for_tool: Option<String>,
    /// Optional: narrow to secrets that may matter for a given agent (best-effort).
    pub for_agent: Option<String>,
}

/// GET /api/secrets/catalog
pub async fn get_secrets_catalog(
    State(state): State<Arc<AppState>>,
    Query(q): Query<CatalogQuery>,
    ext: Option<Extension<RequestId>>,
) -> impl axum::response::IntoResponse {
    let rid = resolve_request_id(ext);
    let home = state.kernel.config.home_dir.clone();
    let mut telem = load_telemetry(&home);
    let now = now_unix();

    let mut rows: Vec<serde_json::Value> = Vec::new();

    // Static + curated
    for d in STATIC_SECRETS {
        if !tool_filter_passes(&q, d) {
            continue;
        }
        if !agent_filter_passes(&state, &q, d.key) {
            continue;
        }
        let layer = state.kernel.credential_first_layer(d.key);
        let present = layer.is_some();
        let t = telem.keys.entry(d.key.to_string()).or_default();
        t.rotation_suggested = t
            .last_set_at
            .map(|t0| now.saturating_sub(t0) > STALE_SECS)
            .unwrap_or(false);
        let stale = t.rotation_suggested;
        rows.push(serde_json::json!({
            "id": d.id,
            "key": d.key,
            "title": d.title,
            "description": d.description,
            "category": d.category,
            "optional": d.optional,
            "used_by": d.used_by,
            "source_layer": layer,
            "present": present,
            "last_set_at": t.last_set_at,
            "last_test": t.last_test,
            "stale_suggested": stale,
        }));
    }

    // Provider API keys (from live catalog; titles come from provider display names)
    let skip_provider_rows = q.for_tool.as_deref() == Some("github_subtree_download");
    if let Ok(cat) = state.kernel.model_catalog.read() {
        if !skip_provider_rows {
        for p in cat.list_providers() {
            if p.api_key_env.is_empty() {
                continue;
            }
            let key = &p.api_key_env;
            if !agent_filter_passes(&state, &q, key) {
                continue;
            }
            let layer = state.kernel.credential_first_layer(key);
            let present = layer.is_some();
            let t = telem.keys.entry(key.clone()).or_default();
            t.rotation_suggested = t
                .last_set_at
                .map(|t0| now.saturating_sub(t0) > STALE_SECS)
                .unwrap_or(false);
            rows.push(serde_json::json!({
                "id": format!("provider-{}", p.id),
                "key": key,
                "title": format!("{} API key", p.display_name),
                "description": format!("Used by the `{}` provider for chat. Stored with the same pipeline as other Vault secrets (vault, secrets.env, process env).", p.id),
                "category": "llm",
                "optional": !p.key_required,
                "used_by": vec![p.id.as_str()],
                "source_layer": layer,
                "present": present,
                "last_set_at": t.last_set_at,
                "last_test": t.last_test,
                "stale_suggested": t.rotation_suggested,
            }));
        }
        }
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "request_id": rid.0,
            "home_dir": home.to_string_lossy(),
            "generated_at": now,
            "rows": rows
        })),
    )
}

fn tool_filter_passes(_q: &CatalogQuery, _d: &StaticSecretDef) -> bool {
    if let Some(ref tool) = _q.for_tool {
        if tool == "github_subtree_download" {
            return _d.key == "GITHUB_TOKEN" || _d.key == "GH_TOKEN";
        }
    }
    true
}

fn agent_filter_passes(
    state: &AppState,
    q: &CatalogQuery,
    key: &str,
) -> bool {
    let Some(agent_id) = q.for_agent.as_ref() else {
        return true;
    };
    let Ok(aid) = agent_id.parse::<openfang_types::agent::AgentId>() else {
        return true;
    };
    let Some(tools) = state.kernel.effective_llm_tool_definitions(aid) else {
        return true;
    };
    let has_github_tool = tools.iter().any(|t| t.name == "github_subtree_download");
    if key == "GITHUB_TOKEN" || key == "GH_TOKEN" {
        return has_github_tool;
    }
    true
}

#[derive(Deserialize)]
pub struct SetSecretBody {
    pub key: String,
    pub value: String,
}

/// POST /api/secrets
pub async fn post_secret(
    State(state): State<Arc<AppState>>,
    ext: Option<Extension<RequestId>>,
    Json(body): Json<SetSecretBody>,
) -> impl axum::response::IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/secrets";
    let key = body.key.trim();
    if key.is_empty() {
        return api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Missing key",
            "JSON `key` must be non-empty.".to_string(),
            None,
        );
    }
    if !is_allowed_key(&state, key) {
        return api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Key not in catalog",
            format!("`{key}` is not a managed secret key for this build."),
            Some("If you need a new integration key, add it to the static catalog in `secrets_api.rs` or extend provider definitions."),
        );
    }
    let value = body.value.trim();
    if value.is_empty() {
        return api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Empty value",
            "Secret value must be non-empty.".to_string(),
            None,
        );
    }

    state.kernel.store_credential(key, value);
    let secrets_path = state.kernel.config.home_dir.join("secrets.env");
    if let Err(e) = write_secret_env(&secrets_path, key, value) {
        return api_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &rid,
            PATH,
            "Failed to write secrets.env",
            format!("{e}"),
            Some("Check permissions on secrets.env in the home directory."),
        );
    }
    std::env::set_var(key, value);

    let mut telem = load_telemetry(&state.kernel.config.home_dir);
    let ent = telem.keys.entry(key.to_string()).or_default();
    ent.last_set_at = Some(now_unix());
    if let Err(e) = save_telemetry(&state.kernel.config.home_dir, telem) {
        tracing::warn!("secret telemetry write failed: {e}");
    }

    audit_credential_mutation(
        state.kernel.as_ref(),
        format!("vault:set key={key} request_id={}", rid.0),
        "ok",
    );

    // Reconnect MCP children if their env list includes this var (same as provider key save)
    {
        let kernel = state.kernel.clone();
        let key_owned = key.to_string();
        tokio::spawn(async move {
            let _ = kernel
                .reconnect_mcp_servers_with_env_var(&key_owned)
                .await;
        });
    }

    if let Ok(mut cat) = state.kernel.model_catalog.write() {
        cat.detect_auth();
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({ "ok": true, "request_id": rid.0, "key": key, "message": "saved" })),
    )
}

/// DELETE /api/secrets/{key}
pub async fn delete_secret(
    State(state): State<Arc<AppState>>,
    PathParam(key): PathParam<String>,
    ext: Option<Extension<RequestId>>,
) -> impl axum::response::IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/secrets/:key";
    let key = key.trim();
    if !is_allowed_key(&state, key) {
        return api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Key not in catalog",
            format!("`{key}` is not a managed key."),
            None,
        );
    }
    state.kernel.remove_credential(key);
    let secrets_path = state.kernel.config.home_dir.join("secrets.env");
    if let Err(e) = remove_secret_env(&secrets_path, key) {
        return api_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &rid,
            PATH,
            "Failed to update secrets.env",
            format!("{e}"),
            None,
        );
    }
    std::env::remove_var(key);

    let mut telem = load_telemetry(&state.kernel.config.home_dir);
    telem.keys.remove(key);
    if let Err(e) = save_telemetry(&state.kernel.config.home_dir, telem) {
        tracing::warn!("secret telemetry write failed: {e}");
    }

    audit_credential_mutation(
        state.kernel.as_ref(),
        format!("vault:remove key={key} request_id={}", rid.0),
        "ok",
    );

    if let Ok(mut cat) = state.kernel.model_catalog.write() {
        cat.detect_auth();
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({ "ok": true, "request_id": rid.0, "key": key, "message": "removed" })),
    )
}

/// POST /api/secrets/{key}/test
pub async fn post_secret_test(
    State(state): State<Arc<AppState>>,
    PathParam(key): PathParam<String>,
    ext: Option<Extension<RequestId>>,
) -> impl axum::response::IntoResponse {
    let rid = resolve_request_id(ext);
    const PATH: &str = "/api/secrets/:key/test";
    let key = key.trim();
    if !is_allowed_key(&state, key) {
        return api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Key not in catalog",
            format!("`{key}` is not a managed key."),
            None,
        );
    }
    let token = state.kernel.resolve_credential(key);
    if token.as_deref().map(|s| s.trim().is_empty()).unwrap_or(true) {
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "ok": false,
                "request_id": rid.0,
                "key": key,
                "detail": "not_set"
            })),
        );
    }
    let token = token.unwrap();
    let t0 = std::time::Instant::now();
    let provider_for_key = state
        .kernel
        .model_catalog
        .read()
        .ok()
        .and_then(|cat| {
            cat.list_providers()
                .iter()
                .find(|p| p.api_key_env == key)
                .map(|p| (p.id.clone(), p.display_name.clone()))
        });

    enum TestOutcome {
        Ran(bool, Option<String>),
        NotApplicable(String),
    }

    let test_flow = if key == "GITHUB_TOKEN" || key == "GH_TOKEN" {
        match test_github_user_api(&token).await {
            Ok(()) => TestOutcome::Ran(true, Some("GitHub API GET /user succeeded".to_string())),
            Err(s) => TestOutcome::Ran(false, Some(s)),
        }
    } else if let Some((pid, _disp)) = provider_for_key.as_ref() {
        TestOutcome::NotApplicable(format!(
            "No standalone API test for `{pid}` here. Use Settings → Providers → Test for live request validation."
        ))
    } else {
        TestOutcome::NotApplicable(
            "No automated test is wired for this key. Presence is verified above.".to_string(),
        )
    };

    let ms = t0.elapsed().as_millis() as u64;
    let (ok, detail, applicable) = match &test_flow {
        TestOutcome::Ran(o, d) => (*o, d.clone(), true),
        TestOutcome::NotApplicable(msg) => (false, Some(msg.clone()), false),
    };

    if let TestOutcome::Ran(probe_ok, _) = &test_flow {
        audit_credential_mutation(
            state.kernel.as_ref(),
            format!("vault:test key={key} probe=github_api_user request_id={}", rid.0),
            if *probe_ok { "ok" } else { "failed" },
        );
    }

    if applicable {
        let mut telem = load_telemetry(&state.kernel.config.home_dir);
        let ent = telem.keys.entry(key.to_string()).or_default();
        ent.last_test = Some(TestRecord {
            ok,
            at: now_unix(),
            ms: Some(ms),
            detail: detail.clone(),
        });
        if let Err(e) = save_telemetry(&state.kernel.config.home_dir, telem) {
            tracing::warn!("secret telemetry write failed: {e}");
        }
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "ok": ok,
            "applicable": applicable,
            "request_id": rid.0,
            "key": key,
            "ms": ms,
            "detail": detail
        })),
    )
}

async fn test_github_user_api(token: &str) -> Result<(), String> {
    let c = match reqwest::Client::builder()
        .user_agent("armaraos-secrets-vault/1.0")
        .timeout(std::time::Duration::from_secs(15))
        .build()
    {
        Ok(c) => c,
        Err(e) => return Err(e.to_string()),
    };
    let r = c
        .get("https://api.github.com/user")
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if r.status().is_success() {
        Ok(())
    } else {
        let s = r.status();
        let body = r.text().await.unwrap_or_default();
        Err(format!("GitHub returned {s}: {}", body.chars().take(200).collect::<String>()))
    }
}
