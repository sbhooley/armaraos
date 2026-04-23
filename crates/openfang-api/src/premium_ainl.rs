//! $AINL Premium: browser wallet signature + Solana RPC balance check, then deep-link ticket for Tauri.
//!
//! Flow:
//! 1. User opens `/premium-ainl-verify.html` in a real browser (Phantom injects).
//! 2. `POST /api/premium/ainl/nonce` → sign message.
//! 3. `POST /api/premium/ainl/verify` → verify Ed25519 + SPL balance → one-time `ticket`.
//! 4. Browser navigates to `armaraos://premium-ainl?ticket=...` → desktop app redeems ticket for an HMAC session token.
//! 5. Dashboard sends `X-Armaraos-Premium-Ainl` on Premium / Hands mutations; server re-checks SPL balance for wallet-backed sessions.

use crate::middleware::RequestId;
use crate::routes::{api_json_error, AppState};
use crate::session_auth::{create_session_token, verify_session_token};
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse};
use axum::Json;
use base64::Engine;
use ed25519_dalek::{Signature, VerifyingKey};
use openfang_kernel::OpenFangKernel;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

pub const PASSWORD_UNLOCK_SUBJECT: &str = "__password_unlock__";
const PREMIUM_HEADER: &str = "x-armaraos-premium-ainl";
const DEFAULT_MINT: &str = "56hrCR3n7danhHNjWaU4VeUHpE1eRE9VRBWpHRPKpump";
const DEFAULT_MIN_UI: u128 = 1_000_000;
const DEFAULT_RPC: &str = "https://api.mainnet-beta.solana.com";
const NONCE_TTL_SECS: i64 = 600;
const TICKET_TTL_SECS: i64 = 120;
const SESSION_TTL_HOURS: u64 = 48;

static NONCES: OnceLock<dashmap::DashMap<String, i64>> = OnceLock::new();
/// Pending tickets: ticket id → (expiry unix, wallet pubkey base58)
static TICKETS: OnceLock<dashmap::DashMap<String, (i64, String)>> = OnceLock::new();

fn nonces() -> &'static dashmap::DashMap<String, i64> {
    NONCES.get_or_init(|| dashmap::DashMap::new())
}

fn tickets() -> &'static dashmap::DashMap<String, (i64, String)> {
    TICKETS.get_or_init(|| dashmap::DashMap::new())
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn prune_expired_nonces(map: &dashmap::DashMap<String, i64>, now: i64) {
    map.retain(|_, exp| *exp > now);
}

fn prune_expired_tickets(map: &dashmap::DashMap<String, (i64, String)>, now: i64) {
    map.retain(|_, (exp, _)| *exp > now);
}

pub fn premium_hmac_secret(kernel: &OpenFangKernel) -> String {
    let api_key = kernel.config.api_key.trim().to_string();
    if !api_key.is_empty() {
        return api_key;
    }
    if kernel.config.auth.enabled && !kernel.config.auth.password_hash.trim().is_empty() {
        return kernel.config.auth.password_hash.clone();
    }
    let mut h = sha2::Sha256::new();
    use sha2::Digest;
    h.update(b"armaraos-premium-ainl:");
    h.update(kernel.config.home_dir.to_string_lossy().as_bytes());
    hex::encode(h.finalize())
}

fn env_trim(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn solana_rpc_url() -> String {
    env_trim("ARMARAOS_SOLANA_RPC_URL")
        .or_else(|| env_trim("ARMARAOS_AINL_SOLANA_RPC"))
        .unwrap_or_else(|| DEFAULT_RPC.to_string())
}

fn ainl_mint() -> String {
    env_trim("ARMARAOS_AINL_SPL_MINT").unwrap_or_else(|| DEFAULT_MINT.to_string())
}

fn min_ui_tokens() -> u128 {
    env_trim("ARMARAOS_AINL_MIN_UI")
        .and_then(|s| s.parse::<u128>().ok())
        .unwrap_or(DEFAULT_MIN_UI)
}

fn deep_link_for_ticket(ticket: &str) -> String {
    format!("armaraos://premium-ainl?ticket={ticket}")
}

/// GET /premium-ainl-verify.html — minimal page opened in the system browser (Tauri).
pub async fn verify_page() -> impl IntoResponse {
    let html = include_str!("../static/premium-ainl-verify.html");
    // This page intentionally uses inline CSS/JS for a single-file browser handoff.
    // Override the API default CSP (`default-src 'none'`) so wallet connect works.
    let csp = "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; connect-src 'self'; img-src 'self' data:; object-src 'none'; base-uri 'none'; frame-ancestors 'none'";
    (
        [(
            header::CACHE_CONTROL,
            "no-store, no-cache, must-revalidate, max-age=0",
        ), (
            header::CONTENT_SECURITY_POLICY,
            csp,
        )],
        Html(html.to_string()),
    )
}

/// POST /api/premium/ainl/nonce
pub async fn post_nonce(State(_state): State<Arc<AppState>>) -> impl IntoResponse {
    let now = now_unix();
    prune_expired_nonces(nonces(), now);
    let nonce = uuid::Uuid::new_v4().to_string();
    let mint = ainl_mint();
    let issued = chrono::Utc::now().to_rfc3339();
    let message = format!(
        "ArmaraOS Premium ($AINL) wallet verification\n\
Nonce: {nonce}\n\
Issued (UTC): {issued}\n\
SPL mint: {mint}\n\
Statement: I control this wallet and request a Premium verification ticket."
    );
    nonces().insert(nonce.clone(), now + NONCE_TTL_SECS);
    (
        StatusCode::OK,
        Json(json!({
            "nonce": nonce,
            "message": message,
            "mint": mint,
            "min_ui_tokens": min_ui_tokens().to_string(),
        })),
    )
}

#[derive(Debug, Deserialize)]
pub struct VerifyBody {
    pub public_key: String,
    pub message: String,
    pub signature_b64: String,
}

/// POST /api/premium/ainl/verify
pub async fn post_verify(
    State(_state): State<Arc<AppState>>,
    ext: Option<axum::extract::Extension<RequestId>>,
    Json(body): Json<VerifyBody>,
) -> impl IntoResponse {
    let rid = ext
        .map(|e| e.0)
        .unwrap_or_else(|| RequestId("unknown".to_string()));
    const PATH: &str = "/api/premium/ainl/verify";
    let now = now_unix();
    prune_expired_nonces(nonces(), now);

    let pk = body.public_key.trim().to_string();
    if pk.is_empty() {
        return api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Missing public_key",
            "JSON body must include a Solana wallet public key (base58).".to_string(),
            None,
        );
    }

    let nonce_line = body
        .message
        .lines()
        .find_map(|l| l.strip_prefix("Nonce: "))
        .map(str::trim)
        .unwrap_or("");
    if nonce_line.is_empty() {
        return api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Invalid message",
            "Signed message must include a `Nonce:` line.".to_string(),
            None,
        );
    }
    if nonces().remove(nonce_line).is_none() {
        return api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Invalid or expired nonce",
            "Request a fresh nonce and sign the new message.".to_string(),
            None,
        );
    }

    let sig_bytes = match base64::engine::general_purpose::STANDARD.decode(body.signature_b64.trim())
    {
        Ok(b) if b.len() == 64 => b,
        _ => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid signature",
                "signature_b64 must be standard base64 of 64 raw bytes.".to_string(),
                None,
            );
        }
    };

    let pk_bytes = match bs58::decode(&pk).into_vec() {
        Ok(v) if v.len() == 32 => v,
        _ => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid public_key",
                "public_key must be base58-encoded 32 bytes.".to_string(),
                None,
            );
        }
    };

    let pk_arr: [u8; 32] = match pk_bytes.as_slice().try_into() {
        Ok(a) => a,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid public_key",
                "public_key must decode to 32 bytes.".to_string(),
                None,
            );
        }
    };

    let vk = match VerifyingKey::from_bytes(&pk_arr) {
        Ok(v) => v,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid public_key",
                "Could not parse Ed25519 verifying key.".to_string(),
                None,
            );
        }
    };

    let sig_arr: [u8; 64] = match sig_bytes.as_slice().try_into() {
        Ok(a) => a,
        Err(_) => {
            return api_json_error(
                StatusCode::BAD_REQUEST,
                &rid,
                PATH,
                "Invalid signature",
                "Signature must be 64 bytes.".to_string(),
                None,
            );
        }
    };
    let sig = Signature::from_bytes(&sig_arr);
    let msg_bytes = body.message.as_bytes();
    if vk.verify_strict(msg_bytes, &sig).is_err() {
        return api_json_error(
            StatusCode::FORBIDDEN,
            &rid,
            PATH,
            "Signature verification failed",
            "The wallet signature did not match the message.".to_string(),
            None,
        );
    }

    let mint = ainl_mint();
    let min_ui = min_ui_tokens();
    match fetch_spl_ui_amount(&pk, &mint).await {
        Ok(ui) if ui >= min_ui => {}
        Ok(ui) => {
            return api_json_error(
                StatusCode::FORBIDDEN,
                &rid,
                PATH,
                "Insufficient $AINL balance",
                format!(
                    "Wallet holds {ui} UI tokens; required minimum is {min_ui} for mint {mint}."
                ),
                Some("Acquire more $AINL on the configured mint, then retry."),
            );
        }
        Err(e) => {
            return api_json_error(
                StatusCode::BAD_GATEWAY,
                &rid,
                PATH,
                "Solana RPC error",
                e,
                Some("Set ARMARAOS_SOLANA_RPC_URL to a reliable RPC endpoint if this persists."),
            );
        }
    }

    let ticket = uuid::Uuid::new_v4().to_string();
    prune_expired_tickets(tickets(), now);
    tickets().insert(
        ticket.clone(),
        (now + TICKET_TTL_SECS, pk.clone()),
    );

    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "ticket": ticket,
            "deep_link": deep_link_for_ticket(&ticket),
        })),
    )
}

#[derive(Debug, Deserialize)]
pub struct RedeemBody {
    pub ticket: String,
}

/// POST /api/premium/ainl/redeem — consume one-time ticket, return session token for the dashboard.
pub async fn post_redeem(
    State(state): State<Arc<AppState>>,
    ext: Option<axum::extract::Extension<RequestId>>,
    Json(body): Json<RedeemBody>,
) -> impl IntoResponse {
    let rid = ext
        .map(|e| e.0)
        .unwrap_or_else(|| RequestId("unknown".to_string()));
    const PATH: &str = "/api/premium/ainl/redeem";
    let ticket = body.ticket.trim().to_string();
    if ticket.is_empty() {
        return api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Missing ticket",
            "JSON body must include a non-empty 'ticket'.".to_string(),
            None,
        );
    }
    let now = now_unix();
    prune_expired_tickets(tickets(), now);
    let Some((_k, (exp, wallet))) = tickets().remove(&ticket) else {
        return api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Invalid or expired ticket",
            "Tickets are single-use and short-lived. Verify again in your browser.".to_string(),
            None,
        );
    };
    if exp <= now {
        return api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Expired ticket",
            "Retry verification from the browser page.".to_string(),
            None,
        );
    }

    let secret = premium_hmac_secret(&state.kernel);
    let token = create_session_token(&wallet, &secret, SESSION_TTL_HOURS);
    let exp_ms = chrono::Utc::now().timestamp_millis() + (SESSION_TTL_HOURS as i64 * 3600 * 1000);

    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "token": token,
            "wallet": wallet,
            "expires_at_ms": exp_ms,
        })),
    )
}

fn premium_token_from_headers(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(PREMIUM_HEADER)?.to_str().ok()?.trim();
    if raw.is_empty() {
        return None;
    }
    Some(raw.to_string())
}

async fn assert_wallet_meets_min(
    wallet: &str,
    mint: &str,
    min_ui: u128,
) -> Result<(), String> {
    let ui = fetch_spl_ui_amount(wallet, mint).await?;
    if ui < min_ui {
        return Err(format!(
            "Wallet no longer meets the Premium minimum ({min_ui} UI tokens) for mint {mint}. Current: {ui}."
        ));
    }
    Ok(())
}

/// GET /api/premium/ainl/status — validates the premium token and (for wallet sessions) re-checks SPL balance.
pub async fn get_status(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let Some(tok) = premium_token_from_headers(&headers) else {
        return (StatusCode::OK, Json(json!({ "ok": false })));
    };
    let secret = premium_hmac_secret(&state.kernel);
    let Some(subject) = verify_session_token(&tok, &secret) else {
        return (StatusCode::OK, Json(json!({ "ok": false })));
    };

    if subject == PASSWORD_UNLOCK_SUBJECT {
        return (
            StatusCode::OK,
            Json(json!({
                "ok": true,
                "mode": "password_unlock",
            })),
        );
    }

    let mint = ainl_mint();
    let min_ui = min_ui_tokens();
    match assert_wallet_meets_min(&subject, &mint, min_ui).await {
        Ok(()) => (
            StatusCode::OK,
            Json(json!({
                "ok": true,
                "mode": "wallet",
                "wallet": subject,
                "mint": mint,
                "min_ui_tokens": min_ui.to_string(),
            })),
        ),
        Err(detail) => (
            StatusCode::OK,
            Json(json!({
                "ok": false,
                "reason": "insufficient_balance",
                "detail": detail,
            })),
        ),
    }
}

/// Enforces Premium authorization for sensitive Hands mutations.
pub async fn require_premium_for_hand_mutation(
    state: &Arc<AppState>,
    headers: &HeaderMap,
    rid: &RequestId,
    path: &str,
) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    let Some(tok) = premium_token_from_headers(headers) else {
        return Err(api_json_error(
            StatusCode::FORBIDDEN,
            rid,
            path,
            "Premium verification required",
            "Verify $AINL in your desktop browser (Premium screen) or use the configured admin unlock token.".to_string(),
            Some("Open Premium, click \"Verify in browser\", complete Phantom signing, then return to ArmaraOS."),
        ));
    };

    let secret = premium_hmac_secret(&state.kernel);
    let Some(subject) = verify_session_token(&tok, &secret) else {
        return Err(api_json_error(
            StatusCode::FORBIDDEN,
            rid,
            path,
            "Invalid premium session",
            "Premium session token is missing, expired, or invalid.".to_string(),
            Some("Re-verify $AINL from the Premium screen."),
        ));
    };

    if subject == PASSWORD_UNLOCK_SUBJECT {
        return Ok(());
    }

    let mint = ainl_mint();
    let min_ui = min_ui_tokens();
    if let Err(detail) = assert_wallet_meets_min(&subject, &mint, min_ui).await {
        return Err(api_json_error(
            StatusCode::FORBIDDEN,
            rid,
            path,
            "Premium holdings requirement not met",
            detail,
            Some("Your wallet must hold at least 1,000,000 $AINL (UI amount) on the configured mint."),
        ));
    }

    Ok(())
}

async fn solana_rpc_json(body: serde_json::Value) -> Result<serde_json::Value, String> {
    let client = reqwest::Client::new();
    let url = solana_rpc_url();
    let resp = client
        .post(url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("HTTP error: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let v: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Invalid JSON: {e}"))?;
    if let Some(err) = v.get("error") {
        return Err(err.to_string());
    }
    Ok(v)
}

/// Returns the summed UI token amount for `mint` owned by `owner` (base58 pubkey).
async fn fetch_spl_ui_amount(owner: &str, mint: &str) -> Result<u128, String> {
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getTokenAccountsByOwner",
        "params": [
            owner,
            { "mint": mint },
            { "encoding": "jsonParsed" }
        ]
    });

    let v = solana_rpc_json(body).await?;
    let Some(values) = v.pointer("/result/value").and_then(|x| x.as_array()) else {
        return Ok(0);
    };

    let mut sum_raw: u128 = 0;
    let mut decimals: u8 = 0;
    for acc in values {
        let token_amount = acc
            .pointer("/account/data/parsed/info/tokenAmount")
            .cloned()
            .unwrap_or_else(|| json!({}));
        if let Some(amount_str) = token_amount.get("amount").and_then(|x| x.as_str()) {
            if let Ok(raw) = amount_str.parse::<u128>() {
                sum_raw = sum_raw.saturating_add(raw);
            }
        }
        if decimals == 0 {
            if let Some(d) = token_amount.get("decimals").and_then(|x| x.as_u64()) {
                if let Ok(d8) = u8::try_from(d) {
                    decimals = d8;
                }
            }
        }
    }

    if decimals == 0 {
        return Ok(0);
    }

    let scale = 10u128.saturating_pow(decimals as u32);
    if scale == 0 {
        return Ok(0);
    }
    Ok(sum_raw / scale)
}
