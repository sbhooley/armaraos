# Dashboard: Settings, Runtime, and shared page shell

The embedded dashboard **Settings** (`#settings`) and **Runtime** (`#runtime`) routes, plus several other top-level pages, share the same visual language: elevated headers, optional subtitles, radial page backgrounds, and (where applicable) toolbar-style tab strips.

This document maps **layout polish** to source files so changes stay consistent with **Get started** and **App Store** styling.

## Shared classes (Skills, Channels, Hands, Home folder, Analytics)

These routes reuse the same building blocks in `components.css`:

| Class | Role |
|-------|------|
| **`dashboard-page-body-polish`** | `page-body` — accent radial wash over `--bg-primary` |
| **`dashboard-page-header-polish`** | `page-header` — column layout, elevated bar |
| **`dashboard-page-header-sub`** | Subtitle paragraph under the title |
| **`dashboard-page-header-row`** | Title row (optional); pairs with **`dashboard-page-header-actions`** for toolbar buttons (**Home folder**) |
| **`dashboard-toolbar-tabs`** | Same rules as **`settings-page-tabs`** — rounded tab toolbar (used on **Skills**, **Hands**, **Analytics** tab rows) |
| **`dashboard-inline-filters`** | Channels category pills + search wrapped in one card |
| **`dashboard-stats-grid`** / **`dashboard-stat-card`** | Same grid/hover treatment as **`runtime-stats-grid`** / **`runtime-stat-card`** (**Analytics** hero stats) |
| **`dashboard-home-intro-panel`** | Home folder intro **`.card`** — top accent stripe, gradient fill |

**Markup:** `index_body.html` — `page === 'skills'`, `'channels'`, `'hands'`, `'home-files'`, `'analytics'`.

## Source files

| Page | Markup | Logic | Shared styles |
|------|--------|-------|----------------|
| **Settings** | `index_body.html` — `page === 'settings'` | `js/pages/settings.js` — `settingsPage()` (merged with `daemon_lifecycle.js`) | `components.css` — `settings-page-*` |
| **Runtime** | `index_body.html` — `page === 'runtime'` | `js/pages/runtime.js` — `runtimePage()` (merged with `daemon_lifecycle.js`) | `components.css` — `runtime-page-*`, `runtime-stats-grid`, `runtime-stat-card`, `runtime-panel*` |
| **Daemon lifecycle (shared)** | Same templates | `js/daemon_lifecycle.js` — `armaraosDaemonLifecycleControls()`; bundled in `webchat.rs` after `api.js` | Confirm modal opts: `js/api.js` — `OpenFangToast.confirm(..., opts)` |

Global primitives (**`.card`**, **`.tabs`**, **`.info-card`**, **`.table`**) are unchanged; page-scoped classes layer on top.

## Settings

- **Root:** `settings-page-root` on the outer `div` with `x-data="settingsPage"`.
- **Header:** `page-header settings-page-header` — column layout with title **Settings** and a short **subtitle** (`settings-page-header-sub`) describing providers, models, config, tools, and system preferences.
- **Body:** `page-body settings-page-body` — radial accent wash over `var(--bg-primary)` (same family as Get started / App Store).
- **Tab bar:** `tabs settings-page-tabs` — rounded toolbar with accent top stripe, inset shadow, pill-style tabs; active tab uses `accent-subtle` fill instead of only a bottom border. The **tabs separator** between primary and secondary tabs remains a subtle vertical rule (`tabs-separator`).

Tab labels and behavior (lazy loads for Security, Network, etc.) are unchanged; only presentation is scoped.

## Runtime

- **Root:** `runtime-page-root`.
- **Header:** `runtime-page-header` + `runtime-page-header-sub` — subtitle describes daemon status, platform, API listen, and providers.
- **Body:** `runtime-page-body` — layered radial gradients (`info-subtle` + `accent-subtle`) for a distinct but on-brand backdrop.
- **Stat tiles:** **`runtime-stats-grid`** replaces a fixed four-column grid with `repeat(auto-fill, minmax(148px, 1fr))` so tiles wrap on narrow widths. Each tile combines **`card stat-card runtime-stat-card`** (accent top border, gradient fill, hover lift).
- **Default model value:** class **`runtime-stat-value-sm`** for smaller type and `word-break` on long model IDs.
- **System / Providers blocks:** **`card runtime-panel`** with **`runtime-panel-title`** on the header (uppercase section label). Providers table adds **`runtime-panel-table`** for thead styling and row hover.
- **Footer actions:** **`runtime-page-footer`** wraps **Refresh** plus **Reload config**, **Reload channels**, **Reload integrations**, and **Shut down** (destructive styling on shutdown). Same actions appear under **Settings → System Info → Daemon / API runtime** with short help text and per-button spinners while a POST is in flight.

## Daemon / API runtime (hot reload & shutdown)

**Where:** **Settings → System Info** card **Daemon / API runtime**, and **Monitor → Runtime** footer.

**Actions (all use confirmation modals except implicit toast flow):**

| Button | API | Purpose |
|--------|-----|---------|
| **Reload config** | `POST /api/config/reload` | Reread `config.toml`, apply hot-reloadable fields; UI may warn if a full restart is still required |
| **Reload channels** | `POST /api/channels/reload` | Stop/restart messaging bridges from disk (includes refreshing `secrets.env` in the process) |
| **Reload integrations** | `POST /api/integrations/reload` | Reconnect extension MCP clients |
| **Shut down daemon** / **Shut down** | `POST /api/shutdown` | Graceful process exit; dashboard disconnects — restart via desktop app, `openfang start`, or a supervisor |

**Auth:** When `api_key` is set, reload POSTs use the same **Bearer** / session as the rest of the dashboard. **`POST /api/shutdown`** also allows **loopback** without Bearer (middleware), matching the diagnostics loopback pattern.

**Compare to GitHub:** **Check daemon vs GitHub** / **Check vs GitHub** uses **`GET /api/version`** plus **`GET /api/version/github-latest`** (server-side GitHub fetch so the browser does not call `api.github.com` directly).

## Manual verification

See **[dashboard-testing.md](dashboard-testing.md)** (*Settings and Runtime pages — layout polish*, *Daemon lifecycle & GitHub version check*) for quick browser checks after UI changes.
