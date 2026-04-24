//! $AINL Premium: browser wallet signature + Solana RPC balance check, then deep-link ticket for Tauri.
//!
//! Flow:
//! 1. User opens `/premium-ainl-verify.html` in a real browser (Phantom injects).
//! 2. `POST /api/premium/ainl/nonce` → sign message.
//! 3. `POST /api/premium/ainl/verify` → verify Ed25519 + SPL balance → one-time `ticket`.
//! 4. Browser navigates to `armaraos://premium-ainl?ticket=...` → desktop app redeems ticket for an HMAC session token.
//! 5. Dashboard sends `X-Armaraos-Premium-Ainl` on Premium / Hands mutations; server re-checks SPL balance for wallet-backed sessions.
//! 6. On wallet redeem, the API also sets an **HttpOnly** `armaraos_premium_wallet_session` cookie so gated routes
//!    enforce SPL minimum even if the client omits `X-Armaraos-Premium-Ainl` (same-origin requests include the cookie).

use crate::middleware::RequestId;
use crate::routes::{api_json_error, AppState};
use crate::session_auth::{create_session_token, verify_session_token};
use axum::body::Body;
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
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
/// HttpOnly cookie: wallet pubkey session (separate secret from `X-Armaraos-Premium-Ainl` token).
pub const PREMIUM_WALLET_BIND_COOKIE: &str = "armaraos_premium_wallet_session";
const DEFAULT_MINT: &str = "56hrCR3n7danhHNjWaU4VeUHpE1eRE9VRBWpHRPKpump";
const DEFAULT_MIN_UI: u128 = 1_000_000;
const DEFAULT_RPC: &str = "https://api.mainnet-beta.solana.com";
const NONCE_TTL_SECS: i64 = 600;
const TICKET_TTL_SECS: i64 = 120;
const SESSION_TTL_HOURS: u64 = 48;

static NONCES: OnceLock<dashmap::DashMap<String, i64>> = OnceLock::new();
/// Pending tickets: ticket id → (expiry unix, wallet pubkey base58)
static TICKETS: OnceLock<dashmap::DashMap<String, (i64, String)>> = OnceLock::new();
/// Per-process random nonce mixed into [`premium_hmac_secret`] so every kernel
/// boot rotates the signing key. Effect: any previously-issued Premium token
/// (sessionStorage, leaked copy, etc.) becomes invalid the moment ArmaraOS
/// restarts or is reinstalled. Combined with the in-token 48 h TTL this
/// satisfies "lock expires every 48 h or when the app closes/reinstalls".
static BOOT_NONCE: OnceLock<String> = OnceLock::new();

fn boot_nonce() -> &'static str {
    BOOT_NONCE.get_or_init(|| uuid::Uuid::new_v4().to_string())
}
/// Per-wallet SPL holdings cache: wallet pubkey → (expiry unix, holds_minimum)
///
/// Without this cache every gated request (Hands activate, Hands chat, WS upgrade,
/// status probes…) would trigger a fresh `getTokenAccountsByOwner` against the
/// public Solana RPC. The free endpoint rate-limits aggressively, so back-to-back
/// requests after a successful verify would intermittently 403 with
/// "Premium holdings requirement not met" even though the wallet clearly holds
/// the minimum. We cache positive results for ~5 minutes and keep the last known
/// value as a fallback when RPC transiently fails.
static MEETS_MIN_CACHE: OnceLock<dashmap::DashMap<String, (i64, bool)>> = OnceLock::new();
const MEETS_MIN_CACHE_TTL_SECS: i64 = 300;

fn nonces() -> &'static dashmap::DashMap<String, i64> {
    NONCES.get_or_init(|| dashmap::DashMap::new())
}

fn tickets() -> &'static dashmap::DashMap<String, (i64, String)> {
    TICKETS.get_or_init(|| dashmap::DashMap::new())
}

fn meets_min_cache() -> &'static dashmap::DashMap<String, (i64, bool)> {
    MEETS_MIN_CACHE.get_or_init(|| dashmap::DashMap::new())
}

fn cache_key(wallet: &str, mint: &str, min_ui: u128) -> String {
    format!("{wallet}|{mint}|{min_ui}")
}

/// Seed the SPL minimum cache after a fresh verify so subsequent gated requests
/// (within the cache TTL) can short-circuit the Solana RPC round-trip.
fn record_meets_min(wallet: &str, mint: &str, min_ui: u128, ok: bool) {
    let now = now_unix();
    meets_min_cache().insert(cache_key(wallet, mint, min_ui), (now + MEETS_MIN_CACHE_TTL_SECS, ok));
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
    use sha2::Digest;
    // Always derive via Sha256 over a stable per-install component AND a
    // per-process [`boot_nonce`]. The nonce is what guarantees Premium tokens
    // do not survive an app restart or reinstall. The stable component
    // (api_key → password_hash → home_dir) is mixed in so anyone who steals
    // the boot nonce alone cannot forge a token without also knowing the
    // local install's secret material.
    let stable: String = {
        let api_key = kernel.config.api_key.trim().to_string();
        if !api_key.is_empty() {
            api_key
        } else if kernel.config.auth.enabled
            && !kernel.config.auth.password_hash.trim().is_empty()
        {
            kernel.config.auth.password_hash.clone()
        } else {
            kernel.config.home_dir.to_string_lossy().to_string()
        }
    };
    let mut h = sha2::Sha256::new();
    h.update(b"armaraos-premium-ainl-v2:");
    h.update(stable.as_bytes());
    h.update(b"|boot:");
    h.update(boot_nonce().as_bytes());
    hex::encode(h.finalize())
}

fn premium_wallet_bind_secret(kernel: &OpenFangKernel) -> String {
    let mut h = sha2::Sha256::new();
    use sha2::Digest;
    h.update(b"armaraos-premium-wallet-bind-v1:");
    h.update(premium_hmac_secret(kernel).as_bytes());
    hex::encode(h.finalize())
}

fn extract_premium_wallet_bind_cookie_raw(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    for part in raw.split(';') {
        let p = part.trim();
        let Some((name, val)) = p.split_once('=') else {
            continue;
        };
        if name.trim() == PREMIUM_WALLET_BIND_COOKIE {
            let v = val.trim();
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// Wallet pubkey to enforce for SPL minimum, or `None` if no wallet gate applies.
/// `Err(())` = bind cookie wallet disagrees with `X-Armaraos-Premium-Ainl` wallet.
fn resolve_wallet_for_holdings_gate(
    headers: &HeaderMap,
    kernel: &OpenFangKernel,
) -> Result<Option<String>, ()> {
    let prem_secret = premium_hmac_secret(kernel);
    let bind_secret = premium_wallet_bind_secret(kernel);

    let header_wallet = premium_token_from_headers(headers)
        .and_then(|t| verify_session_token(&t, &prem_secret))
        .filter(|s| *s != PASSWORD_UNLOCK_SUBJECT);

    let cookie_wallet = extract_premium_wallet_bind_cookie_raw(headers)
        .and_then(|t| verify_session_token(&t, &bind_secret));

    match (&cookie_wallet, &header_wallet) {
        (Some(a), Some(b)) if a != b => Err(()),
        (Some(a), _) => Ok(Some(a.clone())),
        (None, Some(b)) => Ok(Some(b.clone())),
        (None, None) => Ok(None),
    }
}

pub fn clear_premium_wallet_bind_set_cookie_value() -> &'static str {
    "armaraos_premium_wallet_session=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0"
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
        Ok(ui) if ui >= min_ui => {
            // Seed cache so subsequent gated requests (Hands activate, WS, status…)
            // skip the Solana RPC round-trip while this verify is fresh.
            record_meets_min(&pk, &mint, min_ui, true);
        }
        Ok(ui) => {
            record_meets_min(&pk, &mint, min_ui, false);
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

fn json_response_with_optional_set_cookie(
    status: StatusCode,
    body: serde_json::Value,
    set_cookie: Option<String>,
) -> Response {
    let mut b = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(c) = set_cookie {
        b = b.header(header::SET_COOKIE, c);
    }
    b.body(Body::from(body.to_string())).unwrap()
}

fn json_error_response(err: (StatusCode, Json<serde_json::Value>)) -> Response {
    let (st, Json(body)) = err;
    json_response_with_optional_set_cookie(st, body, None)
}

/// POST /api/premium/ainl/redeem — consume one-time ticket, return session token for the dashboard.
pub async fn post_redeem(
    State(state): State<Arc<AppState>>,
    ext: Option<axum::extract::Extension<RequestId>>,
    Json(body): Json<RedeemBody>,
) -> Response {
    let rid = ext
        .map(|e| e.0)
        .unwrap_or_else(|| RequestId("unknown".to_string()));
    const PATH: &str = "/api/premium/ainl/redeem";
    let ticket = body.ticket.trim().to_string();
    if ticket.is_empty() {
        return json_error_response(api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Missing ticket",
            "JSON body must include a non-empty 'ticket'.".to_string(),
            None,
        ));
    }
    let now = now_unix();
    prune_expired_tickets(tickets(), now);
    let Some((_k, (exp, wallet))) = tickets().remove(&ticket) else {
        return json_error_response(api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Invalid or expired ticket",
            "Tickets are single-use and short-lived. Verify again in your browser.".to_string(),
            None,
        ));
    };
    if exp <= now {
        return json_error_response(api_json_error(
            StatusCode::BAD_REQUEST,
            &rid,
            PATH,
            "Expired ticket",
            "Retry verification from the browser page.".to_string(),
            None,
        ));
    }

    let secret = premium_hmac_secret(&state.kernel);
    let token = create_session_token(&wallet, &secret, SESSION_TTL_HOURS);
    let exp_ms = chrono::Utc::now().timestamp_millis() + (SESSION_TTL_HOURS as i64 * 3600 * 1000);

    json_response_with_optional_set_cookie(
        StatusCode::OK,
        json!({
            "ok": true,
            "token": token,
            "wallet": wallet,
            "expires_at_ms": exp_ms,
        }),
        None,
    )
}

fn premium_token_from_headers(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(PREMIUM_HEADER)?.to_str().ok()?.trim();
    if raw.is_empty() {
        return None;
    }
    Some(raw.to_string())
}

/// Premium token from `X-Armaraos-Premium-Ainl` or `?premium_ainl=` (WebSocket upgrades cannot set custom headers).
pub fn premium_token_from_headers_or_query(
    headers: &HeaderMap,
    uri: Option<&axum::http::Uri>,
) -> Option<String> {
    if let Some(t) = premium_token_from_headers(headers) {
        return Some(t);
    }
    let q = uri?.query()?;
    let joined = format!("http://_/invalid?{q}");
    let u = match reqwest::Url::parse(&joined) {
        Ok(v) => v,
        Err(_) => return None,
    };
    u.query_pairs()
        .find(|(k, _)| k == "premium_ainl")
        .map(|(_, v)| v.into_owned())
        .filter(|s| !s.trim().is_empty())
}

/// Require the configured SPL minimum when a wallet obligation is present:
/// valid `X-Armaraos-Premium-Ainl` **wallet** session and/or HttpOnly `armaraos_premium_wallet_session` bind cookie.
///
/// - **Password-unlock** premium header: **allow** (no SPL gate).
/// - **No** bind cookie and **no** valid wallet header: **allow**.
/// - **Mismatch** (cookie wallet ≠ header wallet): **403**.
/// - Wallet below minimum: **403**.
pub async fn require_premium_wallet_holdings_when_wallet_session(
    state: &Arc<AppState>,
    headers: &HeaderMap,
    rid: &RequestId,
    path: &str,
) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    let kernel = &state.kernel;
    let prem_secret = premium_hmac_secret(kernel);
    if let Some(tok) = premium_token_from_headers(headers) {
        if let Some(subj) = verify_session_token(&tok, &prem_secret) {
            if subj == PASSWORD_UNLOCK_SUBJECT {
                return Ok(());
            }
        }
    }

    let wallet = match resolve_wallet_for_holdings_gate(headers, kernel) {
        Ok(w) => w,
        Err(()) => {
            return Err(api_json_error(
                StatusCode::FORBIDDEN,
                rid,
                path,
                "Premium wallet session mismatch",
                "The wallet binding cookie does not match X-Armaraos-Premium-Ainl. Re-verify from the Premium screen."
                    .to_string(),
                Some("Clear cookies for this host or complete Premium verification again."),
            ));
        }
    };

    let Some(wallet) = wallet else {
        return Ok(());
    };

    let mint = ainl_mint();
    let min_ui = min_ui_tokens();
    if let Err(detail) = assert_wallet_meets_min(&wallet, &mint, min_ui).await {
        return Err(api_json_error(
            StatusCode::FORBIDDEN,
            rid,
            path,
            "Premium holdings requirement not met",
            detail,
            Some("Your wallet must hold the configured minimum $AINL (UI amount) on the configured mint."),
        ));
    }
    Ok(())
}

async fn assert_wallet_meets_min(
    wallet: &str,
    mint: &str,
    min_ui: u128,
) -> Result<(), String> {
    let key = cache_key(wallet, mint, min_ui);
    let now = now_unix();
    if let Some(entry) = meets_min_cache().get(&key) {
        let (exp, ok) = *entry;
        if exp > now {
            return if ok {
                Ok(())
            } else {
                Err(format!(
                    "Wallet no longer meets the Premium minimum ({min_ui} UI tokens) for mint {mint}."
                ))
            };
        }
    }

    match fetch_spl_ui_amount(wallet, mint).await {
        Ok(ui) => {
            let ok = ui >= min_ui;
            record_meets_min(wallet, mint, min_ui, ok);
            if ok {
                Ok(())
            } else {
                Err(format!(
                    "Wallet no longer meets the Premium minimum ({min_ui} UI tokens) for mint {mint}. Current: {ui}."
                ))
            }
        }
        Err(rpc_err) => {
            // Tolerate transient public-RPC failures by falling back to the most
            // recent positive cache entry. Without this, a single 429 from
            // mainnet-beta would lock a freshly verified user out of their
            // Premium agents.
            if let Some(entry) = meets_min_cache().get(&key) {
                if entry.1 {
                    return Ok(());
                }
            }
            Err(format!(
                "Solana RPC unavailable while re-checking Premium balance: {rpc_err}"
            ))
        }
    }
}

/// GET /api/premium/ainl/status — validates premium header and/or wallet bind cookie; re-checks SPL for wallet mode.
pub async fn get_status(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let kernel = &state.kernel;
    let prem_secret = premium_hmac_secret(kernel);

    if let Some(tok) = premium_token_from_headers(&headers) {
        if let Some(subject) = verify_session_token(&tok, &prem_secret) {
            if subject == PASSWORD_UNLOCK_SUBJECT {
                return (
                    StatusCode::OK,
                    Json(json!({
                        "ok": true,
                        "mode": "password_unlock",
                    })),
                );
            }
        }
    }

    match resolve_wallet_for_holdings_gate(&headers, kernel) {
        Err(()) => (
            StatusCode::OK,
            Json(json!({
                "ok": false,
                "reason": "wallet_mismatch",
                "detail": "Premium wallet binding cookie does not match X-Armaraos-Premium-Ainl wallet.",
            })),
        ),
        Ok(None) => (
            StatusCode::OK,
            Json(json!({
                "ok": false,
                "reason": "no_session",
            })),
        ),
        Ok(Some(wallet)) => {
            let mint = ainl_mint();
            let min_ui = min_ui_tokens();
            match assert_wallet_meets_min(&wallet, &mint, min_ui).await {
                Ok(()) => (
                    StatusCode::OK,
                    Json(json!({
                        "ok": true,
                        "mode": "wallet",
                        "wallet": wallet,
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
        tracing::warn!(
            request_id = %rid.0,
            target_path = path,
            "premium gate denied: missing X-Armaraos-Premium-Ainl header"
        );
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
        tracing::warn!(
            request_id = %rid.0,
            target_path = path,
            "premium gate denied: token failed HMAC/expiry verification \
             (likely issued by a previous kernel boot or expired after 48h)"
        );
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
        tracing::debug!(
            request_id = %rid.0,
            target_path = path,
            "premium gate allowed: password unlock subject"
        );
        return Ok(());
    }

    let mint = ainl_mint();
    let min_ui = min_ui_tokens();
    if let Err(detail) = assert_wallet_meets_min(&subject, &mint, min_ui).await {
        tracing::warn!(
            request_id = %rid.0,
            target_path = path,
            wallet = %subject,
            mint = %mint,
            min_ui = %min_ui,
            error = %detail,
            "premium gate denied: SPL holdings re-check failed"
        );
        return Err(api_json_error(
            StatusCode::FORBIDDEN,
            rid,
            path,
            "Premium holdings requirement not met",
            detail,
            Some("Your wallet must hold at least 1,000,000 $AINL (UI amount) on the configured mint."),
        ));
    }

    tracing::debug!(
        request_id = %rid.0,
        target_path = path,
        wallet = %subject,
        "premium gate allowed: wallet meets SPL minimum"
    );
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
