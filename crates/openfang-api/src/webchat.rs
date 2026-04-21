//! Embedded WebChat UI served as static HTML.
//!
//! The production dashboard is assembled at compile time from separate
//! HTML/CSS/JS files under `static/` using `include_str!()`. This keeps
//! single-binary deployment while allowing organized source files.
//!
//! Features:
//! - Alpine.js SPA with hash-based routing (10 panels)
//! - Dark/light theme toggle with system preference detection
//! - Responsive layout with collapsible sidebar
//! - Markdown rendering + syntax highlighting (bundled locally)
//! - WebSocket real-time chat with HTTP fallback
//! - Agent management, workflows, memory browser, audit log, and more

use axum::http::header;
use axum::response::IntoResponse;

/// Nonce placeholder in compile-time HTML, replaced at request time.
const NONCE_PLACEHOLDER: &str = "__NONCE__";

/// PostHog dashboard config line — replaced at request time with a `window.__ARMARAOS_POSTHOG__ = …` assignment (see [`dashboard_posthog_config_js`]).
const POSTHOG_CONFIG_PLACEHOLDER: &str = "/*__ARMARAOS_POSTHOG_CONFIG__*/\n";

/// Builds `window.__ARMARAOS_POSTHOG__` for the embedded dashboard (compile-time env, same key family as desktop).
fn dashboard_posthog_config_js() -> String {
    let key = option_env!("ARMARAOS_DASHBOARD_POSTHOG_KEY")
        .or(option_env!("ARMARAOS_POSTHOG_KEY"))
        .or(option_env!("AINL_POSTHOG_KEY"))
        .unwrap_or("");
    let key = key.trim();
    if key.is_empty() {
        return "window.__ARMARAOS_POSTHOG__={configured:false};".to_string();
    }
    let host = option_env!("ARMARAOS_POSTHOG_HOST")
        .or(option_env!("AINL_POSTHOG_HOST"))
        .unwrap_or("https://us.i.posthog.com");
    let host = host.trim().trim_end_matches('/');
    let key_js = serde_json::to_string(key).unwrap_or_else(|_| "\"\"".to_string());
    let host_js =
        serde_json::to_string(host).unwrap_or_else(|_| "\"https://us.i.posthog.com\"".to_string());
    format!("window.__ARMARAOS_POSTHOG__={{configured:true,apiKey:{key_js},api_host:{host_js}}};")
}

/// Resolved PostHog `api_host` (compile-time env) — must stay in sync with [`dashboard_posthog_config_js`].
fn resolved_posthog_api_host() -> &'static str {
    option_env!("ARMARAOS_POSTHOG_HOST")
        .or(option_env!("AINL_POSTHOG_HOST"))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("https://us.i.posthog.com")
        .trim_end_matches('/')
}

fn origin_from_url(url: &str) -> Option<String> {
    let url = url.trim();
    let (scheme, rest) = if let Some(r) = url.strip_prefix("https://") {
        ("https", r)
    } else if let Some(r) = url.strip_prefix("http://") {
        ("http", r)
    } else {
        return None;
    };
    let authority = rest.split('/').next()?.split('?').next()?;
    if authority.is_empty() {
        return None;
    }
    Some(format!("{scheme}://{authority}"))
}

/// Space-separated `connect-src` origins for PostHog (cloud ingest + replay assets + custom compile-time host).
fn posthog_connect_src_origins() -> String {
    use std::collections::BTreeSet;
    let mut seen = BTreeSet::new();
    for o in [
        "https://us.i.posthog.com",
        "https://eu.i.posthog.com",
        "https://us-assets.i.posthog.com",
        "https://eu-assets.i.posthog.com",
        "https://app.posthog.com",
    ] {
        seen.insert(o.to_string());
    }
    if let Some(o) = origin_from_url(resolved_posthog_api_host()) {
        seen.insert(o);
    }
    seen.into_iter().collect::<Vec<_>>().join(" ")
}

/// Compile-time ETag based on the crate version.
/// Not used for the dashboard page (nonce prevents caching) but retained
/// for potential future use by static asset handlers.
#[allow(dead_code)]
const ETAG: &str = concat!("\"openfang-", env!("CARGO_PKG_VERSION"), "\"");

/// Embedded logo PNG for single-binary deployment.
const LOGO_PNG: &[u8] = include_bytes!("../static/logo.png");

/// ArmaraOS mark (chat avatars, branding).
const ARMARAOS_LOGO_PNG: &[u8] = include_bytes!("../static/assets/armaraos-logo.png");

/// Embedded favicon ICO for browser tabs.
const FAVICON_ICO: &[u8] = include_bytes!("../static/favicon.ico");

/// GET /assets/armaraos-logo.png — ArmaraOS mark for chat UI.
pub async fn armaraos_logo_png() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "public, max-age=86400, immutable"),
        ],
        ARMARAOS_LOGO_PNG,
    )
}

/// GET /logo.png — App header / PWA icon.
pub async fn logo_png() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "public, max-age=86400, immutable"),
        ],
        LOGO_PNG,
    )
}

/// GET /favicon.ico — Serve the OpenFang favicon.
pub async fn favicon_ico() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "image/x-icon"),
            (header::CACHE_CONTROL, "public, max-age=86400, immutable"),
        ],
        FAVICON_ICO,
    )
}

/// Embedded PWA manifest for installable web app support.
const MANIFEST_JSON: &str = include_str!("../static/manifest.json");

/// Embedded service worker for PWA support.
const SW_JS: &str = include_str!("../static/sw.js");

/// GET /manifest.json — Serve the PWA web app manifest.
pub async fn manifest_json() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "application/manifest+json"),
            (header::CACHE_CONTROL, "public, max-age=86400, immutable"),
        ],
        MANIFEST_JSON,
    )
}

/// GET /sw.js — Serve the PWA service worker.
pub async fn sw_js() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "application/javascript"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        SW_JS,
    )
}

/// GET / — Serve the OpenFang Dashboard single-page application.
///
/// Generates a unique CSP nonce on every request and injects it into both
/// the `<script>` tags and the `Content-Security-Policy` header. This
/// replaces `'unsafe-inline'` so only our own scripts execute.
pub async fn webchat_page() -> impl IntoResponse {
    let nonce = uuid::Uuid::new_v4().to_string();
    let posthog_cfg = dashboard_posthog_config_js();
    let html = WEBCHAT_HTML
        .replace(NONCE_PLACEHOLDER, &nonce)
        .replace(POSTHOG_CONFIG_PLACEHOLDER, &posthog_cfg);
    let ph_connect = posthog_connect_src_origins();
    let csp = format!(
        "default-src 'self'; \
         script-src 'self' 'nonce-{nonce}' 'unsafe-eval'; \
         style-src 'self' 'unsafe-inline' https://fonts.googleapis.com https://fonts.gstatic.com; \
         img-src 'self' data: blob:; \
         connect-src 'self' ws://localhost:* ws://127.0.0.1:* wss://localhost:* wss://127.0.0.1:* \
           {ph_connect}; \
         font-src 'self' https://fonts.gstatic.com; \
         media-src 'self' blob:; \
         frame-src 'self' blob:; \
         object-src 'none'; \
         base-uri 'self'; \
         form-action 'self'"
    );
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8".to_string()),
            (
                header::HeaderName::from_static("content-security-policy"),
                csp,
            ),
            (header::CACHE_CONTROL, "no-store".to_string()),
        ],
        html,
    )
}

/// The embedded HTML/CSS/JS for the OpenFang Dashboard.
///
/// Assembled at compile time from organized static files.
/// All vendor libraries (Alpine.js, marked.js, highlight.js) are bundled
/// locally — no CDN dependency. Alpine.js is included LAST because it
/// immediately processes x-data directives and fires alpine:init on load.
const WEBCHAT_HTML: &str = concat!(
    include_str!("../static/index_head.html"),
    "<style>\n",
    include_str!("../static/css/theme.css"),
    "\n",
    include_str!("../static/css/layout.css"),
    "\n",
    include_str!("../static/css/components.css"),
    "\n",
    include_str!("../static/vendor/github-dark.min.css"),
    "\n</style>\n",
    include_str!("../static/index_body.html"),
    // PostHog: compile-time public project key + vendored SDK + dashboard analytics (before app code).
    "<script nonce=\"__NONCE__\">\n",
    "/*__ARMARAOS_POSTHOG_CONFIG__*/\n",
    "</script>\n",
    "<script nonce=\"__NONCE__\">\n",
    include_str!("../static/vendor/posthog.array.full.es5.js"),
    "\n</script>\n",
    "<script nonce=\"__NONCE__\">\n",
    include_str!("../static/js/analytics.js"),
    "\n</script>\n",
    // Vendor libs: marked + highlight first (used by app.js), then Chart.js
    "<script nonce=\"__NONCE__\">\n",
    include_str!("../static/vendor/marked.min.js"),
    "\n</script>\n",
    "<script nonce=\"__NONCE__\">\n",
    include_str!("../static/vendor/highlight.min.js"),
    "\n</script>\n",
    "<script nonce=\"__NONCE__\">\n",
    include_str!("../static/vendor/chart.umd.min.js"),
    "\n</script>\n",
    "<script nonce=\"__NONCE__\">\n",
    include_str!("../static/vendor/d3.min.js"),
    "\n</script>\n",
    // App code
    "<script nonce=\"__NONCE__\">\n",
    include_str!("../static/js/api.js"),
    "\n",
    include_str!("../static/js/daemon_lifecycle.js"),
    "\n",
    include_str!("../static/js/page-load-error.js"),
    "\n",
    include_str!("../static/js/app.js"),
    "\n",
    include_str!("../static/js/pages/overview.js"),
    "\n",
    include_str!("../static/js/pages/command-palette.js"),
    "\n",
    include_str!("../static/js/katex.js"),
    "\n",
    include_str!("../static/js/pages/bookmarks.js"),
    "\n",
    include_str!("../static/js/pages/chat.js"),
    "\n",
    include_str!("../static/js/pages/agents.js"),
    "\n",
    include_str!("../static/js/pages/workflows.js"),
    "\n",
    include_str!("../static/js/pages/workflow-builder.js"),
    "\n",
    include_str!("../static/js/pages/channels.js"),
    "\n",
    include_str!("../static/js/pages/skills.js"),
    "\n",
    include_str!("../static/js/pages/ainl-library.js"),
    "\n",
    include_str!("../static/js/pages/home-files.js"),
    "\n",
    include_str!("../static/js/pages/hands.js"),
    "\n",
    include_str!("../static/js/pages/scheduler.js"),
    "\n",
    include_str!("../static/js/pages/settings.js"),
    "\n",
    include_str!("../static/js/pages/usage.js"),
    "\n",
    include_str!("../static/js/pages/sessions.js"),
    "\n",
    include_str!("../static/js/pages/logs.js"),
    "\n",
    include_str!("../static/js/pages/timeline.js"),
    "\n",
    include_str!("../static/js/pages/wizard.js"),
    "\n",
    include_str!("../static/js/pages/approvals.js"),
    "\n",
    include_str!("../static/js/pages/comms.js"),
    "\n",
    include_str!("../static/js/pages/network.js"),
    "\n",
    include_str!("../static/js/pages/runtime.js"),
    include_str!("../static/js/pages/orchestration-traces.js"),
    "\n",
    include_str!("../static/js/pages/graph-memory.js"),
    "\n</script>\n",
    // Alpine.js MUST be last — it processes x-data and fires alpine:init
    "<script nonce=\"__NONCE__\">\n",
    include_str!("../static/vendor/alpine.min.js"),
    "\n</script>\n",
    "</body></html>"
);

#[cfg(test)]
mod posthog_csp_tests {
    use super::origin_from_url;

    #[test]
    fn origin_from_posthog_url() {
        assert_eq!(
            origin_from_url("https://eu.i.posthog.com").as_deref(),
            Some("https://eu.i.posthog.com")
        );
        assert_eq!(
            origin_from_url("https://ph.example.com/ingest/").as_deref(),
            Some("https://ph.example.com")
        );
    }
}
