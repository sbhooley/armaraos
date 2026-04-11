//! Production middleware for the OpenFang API server.
//!
//! Provides:
//! - Request ID generation and propagation
//! - Per-endpoint structured request logging
//! - In-memory rate limiting (per IP)

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, Response, StatusCode};
use axum::middleware::Next;
use std::net::SocketAddr;
use std::time::Instant;
use tracing::info;

/// Request ID header name (standard).
pub const REQUEST_ID_HEADER: &str = "x-request-id";

/// Per-request correlation ID (handlers may read via `Extension<RequestId>`; also sent as `x-request-id`).
#[derive(Clone, Debug)]
pub struct RequestId(pub String);

/// Middleware: inject a unique request ID and log the request/response.
pub async fn request_logging(mut request: Request<Body>, next: Next) -> Response<Body> {
    let request_id = uuid::Uuid::new_v4().to_string();
    request
        .extensions_mut()
        .insert(RequestId(request_id.clone()));
    let method = request.method().clone();
    let uri = request.uri().path().to_string();
    let start = Instant::now();

    let mut response = next.run(request).await;

    let elapsed = start.elapsed();
    let status = response.status().as_u16();

    info!(
        request_id = %request_id,
        method = %method,
        path = %uri,
        status = status,
        latency_ms = elapsed.as_millis() as u64,
        "API request"
    );

    // Inject the request ID into the response
    if let Ok(header_val) = request_id.parse() {
        response.headers_mut().insert(REQUEST_ID_HEADER, header_val);
    }

    response
}

/// Authentication state passed to the auth middleware.
#[derive(Clone)]
pub struct AuthState {
    pub api_key: String,
    pub auth_enabled: bool,
    pub session_secret: String,
}

#[inline]
fn connect_info_is_loopback(request: &Request<Body>) -> bool {
    request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip().is_loopback())
        .unwrap_or(false)
}

/// Bearer token authentication middleware.
///
/// When `api_key` is non-empty (after trimming), requests to non-public
/// endpoints must include `Authorization: Bearer <api_key>`.
/// If the key is empty or whitespace-only, auth is disabled entirely
/// (public/local development mode).
///
/// When dashboard auth is enabled, session cookies are also accepted.
pub async fn auth(
    axum::extract::State(auth_state): axum::extract::State<AuthState>,
    request: Request<Body>,
    next: Next,
) -> Response<Body> {
    // SECURITY: Capture method early for method-aware public endpoint checks.
    let method = request.method().clone();

    // Shutdown is loopback-only (CLI on same machine) — skip token auth
    let path = request.uri().path();
    if path == "/api/shutdown" {
        let is_loopback_shutdown = connect_info_is_loopback(&request); // default-deny if no ConnectInfo
        if is_loopback_shutdown {
            return next.run(request).await;
        }
    }

    // Redacted support bundle: same machine only (desktop shell / local CLI).
    // Writes go under ~/.armaraos/support/; loopback prevents remote abuse when api_key is set.
    if path == "/api/support/diagnostics"
        && method == axum::http::Method::POST
        && connect_info_is_loopback(&request)
    {
        return next.run(request).await;
    }

    // Stream a generated zip from ~/support/ — same loopback-only rule as POST diagnostics.
    // Without this, the SPA fetch GET is rejected when api_key is set (Bearer not always
    // applied consistently in embedded WebViews), so users cannot save the bundle.
    if path == "/api/support/diagnostics/download"
        && method == axum::http::Method::GET
        && connect_info_is_loopback(&request)
    {
        return next.run(request).await;
    }

    // Stream arbitrary home files (e.g. support/*.zip) without forcing Bearer in embedded WebView.
    if path == "/api/armaraos-home/download"
        && method == axum::http::Method::GET
        && connect_info_is_loopback(&request)
    {
        return next.run(request).await;
    }

    // Kernel scheduler (wizard + overview): same-machine clients skip Bearer for mutations.
    // Remote callers must still send Authorization / ?token= when api_key is set.
    if connect_info_is_loopback(&request) {
        if path == "/api/schedules" && method == axum::http::Method::POST {
            return next.run(request).await;
        }
        if let Some(rest) = path.strip_prefix("/api/schedules/") {
            if method == axum::http::Method::POST {
                if let Some(id_part) = rest.strip_suffix("/run") {
                    if !id_part.is_empty() && !id_part.contains('/') {
                        return next.run(request).await;
                    }
                }
            }
            if !rest.contains('/')
                && (method == axum::http::Method::PUT || method == axum::http::Method::DELETE)
            {
                return next.run(request).await;
            }
        }
    }

    // Public endpoints that don't require auth (dashboard needs these).
    // SECURITY: /api/agents is GET-only (listing). POST (spawn) requires auth.
    // SECURITY: Public endpoints are GET-only unless explicitly noted.
    // POST/PUT/DELETE require auth for remote peers; loopback-only exceptions are
    // documented above (support bundle, home download, /api/schedules mutations).
    let is_get = method == axum::http::Method::GET;
    let is_loopback = connect_info_is_loopback(&request);

    // SSE: allow without credentials only from loopback (embedded dashboard). Remote clients
    // must send the same Bearer / ?token= as other protected routes when api_key is set.
    if is_get
        && (path == "/api/logs/stream"
            || path == "/api/logs/daemon/stream"
            || path == "/api/events/stream")
        && is_loopback
    {
        return next.run(request).await;
    }

    let is_public = path == "/"
        || path == "/assets/armaraos-logo.png"
        || path == "/logo.png"
        || path == "/favicon.ico"
        || (path == "/.well-known/agent.json" && is_get)
        || (path.starts_with("/a2a/") && is_get)
        || path == "/api/health"
        || path == "/api/health/detail"
        || path == "/api/status"
        || path == "/api/version"
        || path == "/api/version/github-latest"
        || (path == "/api/agents" && is_get)
        || (path == "/api/profiles" && is_get)
        || (path == "/api/config" && is_get)
        || (path == "/api/config/schema" && is_get)
        || (path.starts_with("/api/uploads/") && is_get)
        // Dashboard read endpoints — allow unauthenticated so the SPA can
        // render before the user enters their API key.
        || (path == "/api/models" && is_get)
        || (path == "/api/models/aliases" && is_get)
        || (path == "/api/providers" && is_get)
        || (path == "/api/budget" && is_get)
        || (path == "/api/budget/agents" && is_get)
        || (path.starts_with("/api/budget/agents/") && is_get)
        || (path == "/api/network/status" && is_get)
        || (path == "/api/system/network-hints" && is_get)
        || (path == "/api/a2a/agents" && is_get)
        || (path == "/api/approvals" && is_get)
        || (path.starts_with("/api/approvals/") && is_get)
        || (path == "/api/channels" && is_get)
        || (path == "/api/hands" && is_get)
        || (path == "/api/hands/active" && is_get)
        || (path.starts_with("/api/hands/") && is_get)
        || (path == "/api/skills" && is_get)
        || (path == "/api/sessions" && is_get)
        || (path == "/api/integrations" && is_get)
        || (path == "/api/integrations/available" && is_get)
        || (path == "/api/integrations/health" && is_get)
        || (path == "/api/workflows" && is_get)
        || (path == "/api/schedules" && is_get)
        // /api/logs/stream, /api/logs/daemon/stream, /api/events/stream: loopback bypass above; remote needs auth
        || (path.starts_with("/api/cron/") && is_get)
        || (path.starts_with("/api/ainl/library") && is_get)
        || path.starts_with("/api/providers/github-copilot/oauth/")
        || path == "/api/auth/login"
        || path == "/api/auth/logout"
        || (path == "/api/auth/check" && is_get);

    if is_public {
        return next.run(request).await;
    }

    // If no API key configured (empty, whitespace-only, or missing), skip auth
    // entirely. Users who don't set api_key accept that all endpoints are open.
    // To secure the dashboard, set a non-empty api_key in config.toml.
    let api_key_trimmed = auth_state.api_key.trim().to_string();
    if api_key_trimmed.is_empty() && !auth_state.auth_enabled {
        return next.run(request).await;
    }
    let api_key = api_key_trimmed.as_str();

    // Check Authorization: Bearer <token> header, then fallback to X-API-Key
    let bearer_token = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));

    let api_token = bearer_token.or_else(|| {
        request
            .headers()
            .get("x-api-key")
            .and_then(|v| v.to_str().ok())
    });

    // SECURITY: Use constant-time comparison to prevent timing attacks.
    let header_auth = api_token.map(|token| {
        use subtle::ConstantTimeEq;
        if token.len() != api_key.len() {
            return false;
        }
        token.as_bytes().ct_eq(api_key.as_bytes()).into()
    });

    // Also check ?token= query parameter (for EventSource/SSE clients that
    // cannot set custom headers, same approach as WebSocket auth).
    let query_token = request
        .uri()
        .query()
        .and_then(|q| q.split('&').find_map(|pair| pair.strip_prefix("token=")));

    // SECURITY: Use constant-time comparison to prevent timing attacks.
    let query_auth = query_token.map(|token| {
        use subtle::ConstantTimeEq;
        if token.len() != api_key.len() {
            return false;
        }
        token.as_bytes().ct_eq(api_key.as_bytes()).into()
    });

    // Accept if either auth method matches
    if header_auth == Some(true) || query_auth == Some(true) {
        return next.run(request).await;
    }

    // Check session cookie (dashboard login sessions)
    if auth_state.auth_enabled {
        if let Some(token) = extract_session_cookie(&request) {
            if crate::session_auth::verify_session_token(&token, &auth_state.session_secret)
                .is_some()
            {
                return next.run(request).await;
            }
        }
    }

    // Determine error message: was a credential provided but wrong, or missing entirely?
    let credential_provided = header_auth.is_some() || query_auth.is_some();
    let error_msg = if credential_provided {
        "Invalid API key"
    } else {
        "Missing Authorization: Bearer <api_key> header"
    };

    let request_id = uuid::Uuid::new_v4().to_string();
    let path = request.uri().path().to_string();
    let detail = if credential_provided {
        "Authentication failed for this request (API key mismatch)."
    } else {
        "No Bearer token was provided for a protected endpoint."
    };

    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .header("www-authenticate", "Bearer")
        .header(REQUEST_ID_HEADER, request_id.clone())
        .body(Body::from(
            serde_json::json!({
                "error": error_msg,
                "detail": detail,
                "path": path,
                "request_id": request_id,
                "hint": "Open Settings → Security and set your API key, or sign in with dashboard login."
            })
            .to_string(),
        ))
        .unwrap_or_default()
}

/// Extract the `openfang_session` cookie value from a request.
fn extract_session_cookie(request: &Request<Body>) -> Option<String> {
    request
        .headers()
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(|cookies| {
            cookies.split(';').find_map(|c| {
                c.trim()
                    .strip_prefix("openfang_session=")
                    .map(|v| v.to_string())
            })
        })
}

/// Security headers middleware — applied to ALL API responses.
pub async fn security_headers(request: Request<Body>, next: Next) -> Response<Body> {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert("x-content-type-options", "nosniff".parse().unwrap());
    headers.insert("x-frame-options", "DENY".parse().unwrap());
    headers.insert("x-xss-protection", "1; mode=block".parse().unwrap());
    // The dashboard handler (webchat_page) sets its own nonce-based CSP.
    // For all other responses (API endpoints), apply a strict default.
    if !headers.contains_key("content-security-policy") {
        headers.insert(
            "content-security-policy",
            "default-src 'none'; frame-ancestors 'none'"
                .parse()
                .unwrap(),
        );
    }
    headers.insert(
        "referrer-policy",
        "strict-origin-when-cross-origin".parse().unwrap(),
    );
    headers.insert(
        "cache-control",
        "no-store, no-cache, must-revalidate".parse().unwrap(),
    );
    headers.insert(
        "strict-transport-security",
        "max-age=63072000; includeSubDomains".parse().unwrap(),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_id_header_constant() {
        assert_eq!(REQUEST_ID_HEADER, "x-request-id");
    }
}
