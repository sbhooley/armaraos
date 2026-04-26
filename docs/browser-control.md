# Browser Control (Chrome / Chromium via CDP)

ArmaraOS includes a built-in browser-control stack: the agent runtime can drive a real Chromium / Chrome instance over the [Chrome DevTools Protocol (CDP)](https://chromedevtools.github.io/devtools-protocol/) — either headless (no window), headed (visible window), or attached to a Chrome the user already started themselves.

The same stack is also exposed to AINL programs through a thin **`browser` adapter** that lives in the `AI_Native_Lang` repo (`adapters/browser.py`). That adapter does not speak CDP itself; it proxies every call to this daemon's MCP HTTP endpoint, so there is exactly one implementation across the desktop, the daemon, AINL programs, and scheduled `ainl run` jobs.

## Table of contents

- [Architecture](#architecture)
- [Modes](#modes)
- [Configuration](#configuration)
- [Built-in tools](#built-in-tools)
- [Bundled `browser` hand](#bundled-browser-hand)
- [AINL `browser` adapter](#ainl-browser-adapter)
- [Diagnostics](#diagnostics)
- [Security notes](#security-notes)

---

## Architecture

```
┌──────────────────────────┐         ┌──────────────────────────┐
│  agent loop / chat       │         │  AINL `R browser.*`      │
│  (any built-in agent)    │         │  ainl run / scheduled    │
└────────────┬─────────────┘         └────────────┬─────────────┘
             │ tool_runner.execute_tool           │ HTTP POST /mcp
             │ ("browser_navigate", …)            │ (JSON-RPC tools/call)
             ▼                                    ▼
   ┌───────────────────────────────────────────────────────┐
   │  openfang-runtime::browser                            │
   │  • BrowserManager (per-agent sessions)                │
   │  • BrowserSession (CDP WebSocket, lifecycle)          │
   │  • BrowserMode (Headless | Headed | Attach)           │
   └───────────────────────────────────────────────────────┘
                              │
                              ▼ Chrome DevTools Protocol
                ┌──────────────────────────────┐
                │  Chromium / Chrome (local)   │
                │  spawned or user-launched    │
                └──────────────────────────────┘
```

Source files of interest:

- `crates/openfang-runtime/src/browser.rs` — `BrowserManager`, `BrowserSession`, `BrowserMode`, every `tool_browser_*` handler.
- `crates/openfang-runtime/src/tool_runner.rs` — dispatch + `ToolDefinition`s.
- `crates/openfang-types/src/config.rs` → `BrowserConfig` — global defaults.
- `crates/openfang-hands/bundled/browser/HAND.toml` — the bundled "Browser" hand.
- `crates/openfang-api/src/routes.rs` — exposes browser tools through `POST /mcp` (`tools/call`) and the per-agent event endpoints.

## Modes

| Mode | What it does | Use when |
|------|---|---|
| `headless` (default) | Spawns a fresh Chromium with `--headless=new`. No visible window. | Background scrapes, CI, anything where you want speed and no UI. |
| `headed` | Spawns a fresh Chromium with a visible window. | The site detects headless; the user wants to watch the agent; debugging. |
| `attach` | Connects over CDP to a Chrome the user launched with `--remote-debugging-port=<port>`. **Does not spawn a process.** | The agent must drive the user's *real* browser — keeping their cookies, sign-ins, profile, and the actual native window. |

Switching modes mid-session **closes** the current session and opens a new one. Page state, cookies, and any open tabs from the previous mode are lost. `attach` mode preserves the user's running tabs because no spawn / kill happens.

## Configuration

Configured in `~/.armaraos/config.toml`:

```toml
[browser]
enabled = true                 # enable the built-in browser_* tools
default_mode = "headless"      # "headless" | "headed" | "attach"
attach_port = 9222             # port used for `attach` mode
viewport_width = 1280
viewport_height = 720
timeout_secs = 30              # per-action CDP timeout
idle_timeout_secs = 300        # auto-close session after this many idle seconds
max_sessions = 5
# chromium_path = "/path/to/chrome"  # auto-detected by default
```

`headless` (boolean) is preserved for backwards compatibility. New configs should use `default_mode`. When `default_mode` is unset, `headless = true` ⇒ `Headless`, `false` ⇒ `Headed`. Agents can override the mode per call (`browser_navigate { url, mode = "headed" }` or `browser_session_start { mode = "attach" }`).

To use `attach` mode, the user runs Chrome themselves with the matching debug port:

```bash
# macOS
"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" \
  --remote-debugging-port=9222 \
  --user-data-dir="$HOME/Library/Application Support/Google/Chrome"

# Linux
google-chrome --remote-debugging-port=9222

# Windows
"C:\Program Files\Google\Chrome\Application\chrome.exe" --remote-debugging-port=9222
```

## Built-in tools

All registered through `crates/openfang-runtime/src/tool_runner.rs::builtin_tool_definitions()`.

| Tool | Purpose | Key args |
|---|---|---|
| `browser_navigate` | Open a URL. Optionally pick a `mode`. | `url`, `mode?` |
| `browser_click` | Click by CSS selector (falls back to text). On failure returns URL, readyState, match count, and interactive element count. | `selector` |
| `browser_click_text` | Click by visible text (case-insensitive partial match). More robust than CSS for SPAs. On failure lists available clickable elements. | `text` |
| `browser_type` | Type into an input by CSS/name/placeholder/label. Dispatches React-compatible events. | `selector`, `text` |
| `browser_fill` | Fill a field by name, label, placeholder, data-testid, aria-label, id, or CSS. Tries 7 strategies. On failure lists all form fields on the page. **Best for React/SPA forms.** | `field`, `value` |
| `browser_screenshot` | Return a PNG (base64). | — |
| `browser_read_page` | Return the current rendered text content as markdown. | — |
| `browser_snapshot` | Return a structured accessibility snapshot: every interactive element with role, name, selector hint, and value. **Use before clicking/typing to see available targets.** | — |
| `browser_scroll` | Scroll the viewport. | `direction?`, `amount?` |
| `browser_wait` | Wait for a selector to appear. Default 15s (was 5s). On timeout returns page URL, readyState, element counts. Max 60s. | `selector`, `timeout_ms?` |
| `browser_run_js` | Evaluate a JS expression and return the result. | `expression` |
| `browser_back` | Go back one entry in the history stack. | — |
| `browser_close` | Close the current session. | — |
| `browser_session_start` | Open / reset a session in a specific mode. | `mode?` |
| `browser_session_status` | Return diagnostics: `agent_id`, `active`, `mode`, `chromium_available`, `chromium_path`, `available_modes`. | — |

Mode strings accepted everywhere: `headless`, `headed` (aliases: `headful`, `visible`, `windowed`, `gui`), `attach` (aliases: `connect`, `existing`, `user_chrome`).

### Recommended workflow for dynamic / SPA pages

For React, Next.js, and other SPA sites where CSS selectors are fragile:

1. **`browser_navigate`** — open the URL
2. **`browser_snapshot`** — structured list of all interactive elements with selector hints
3. **`browser_fill`** or **`browser_click_text`** — interact by field name/label/text
4. **`browser_read_page`** — verify result

This avoids CSS selector guessing. `browser_snapshot` returns each element's `name`, `id`, `data-testid`, `aria-label`, and `placeholder` so the agent can pick the best targeting strategy.

### Error diagnostics

All browser tools now return **rich error context** on failure:

- **`browser_click` / `browser_click_text`**: page URL, `readyState`, count of interactive elements, available clickable texts
- **`browser_type` / `browser_fill`**: page URL, input count, list of available fields with name/id/placeholder
- **`browser_wait`**: page URL, `readyState`, total elements, interactive element count, selector match count
- **`browser_read_page`**: session state hint and recovery suggestion

This helps the model self-correct on the next attempt instead of retrying blindly.

## Bundled `browser` hand

`crates/openfang-hands/bundled/browser/HAND.toml` packages every `browser_*` tool plus a system prompt that teaches the agent to **pick a mode based on the user's intent** (headless for background work, headed for visible debugging or anti-headless sites, attach for "use my real Chrome"). It also exposes `default_mode`, `attach_port`, and a legacy `headless` toggle as user-tunable settings on the hand.

Install via the App Store / Hands UI or via CLI:

```bash
openfang hands install browser
openfang hands enable browser --agent <agent-id>
```

## AINL `browser` adapter

The AINL runtime (Python) does not speak CDP. It ships a thin proxy in [`AI_Native_Lang/adapters/browser.py`](../../AI_Native_Lang/adapters/browser.py) that turns `R browser.<VERB>` calls into JSON-RPC `tools/call` requests against this daemon's `POST /mcp` endpoint. This means:

- `ainl run`, `ainl serve`, scheduled `ainl run`, and the desktop's embedded AINL all drive the **same** `BrowserSession` you'd get from the chat agent.
- There is no second CDP implementation to keep in sync.
- Standalone `ainl run` against a host without ArmaraOS will fail fast with an actionable error pointing at `openfang start`.

Verb mapping (case-insensitive):

| AINL `R browser.*` | ArmaraOS tool |
|---|---|
| `NAVIGATE url [mode]` | `browser_navigate` |
| `CLICK selector` | `browser_click` |
| `CLICK_TEXT text` | `browser_click_text` |
| `TYPE selector text` | `browser_type` |
| `FILL field value` | `browser_fill` |
| `READ_PAGE` (alias `read`) | `browser_read_page` |
| `SNAPSHOT` | `browser_snapshot` |
| `SCREENSHOT` | `browser_screenshot` |
| `SCROLL [direction] [amount]` | `browser_scroll` |
| `WAIT selector [timeout_ms]` | `browser_wait` (default 15s) |
| `RUN_JS expression` | `browser_run_js` |
| `BACK` | `browser_back` |
| `SESSION_START [mode]` (alias `start`) | `browser_session_start` |
| `SESSION_STATUS` (alias `status`) | `browser_session_status` (returns parsed JSON) |
| `CLOSE` | `browser_close` |

Adapter env vars:

- `ARMARAOS_API_BASE` — daemon URL (default `http://127.0.0.1:4200`)
- `ARMARAOS_API_KEY` — bearer token, if the daemon enforces auth
- `AINL_BROWSER_AGENT_ID` — session tag (default `ainl-default`); two AINL programs sharing this id share the browser session
- `AINL_BROWSER_TIMEOUT_S` — per-request HTTP timeout (default `60`)

The strict-valid example lives at `AI_Native_Lang/examples/browser/browser_visit_minimal.ainl` and is registered in `tooling/artifact_profiles.json` → `strict-valid` plus the canonical curriculum/training pack manifests.

## Diagnostics

Two cheap checks for "is the browser stack actually wired up?":

```bash
# 1. From the agent / chat: ask for the diagnostic tool
"Run browser_session_status"
```

```rust
// 2. From Rust:
let info = browser::discover_chromium_path();   // Option<PathBuf>
```

`browser_session_status` includes:

```json
{
  "agent_id": "…",
  "active": true,
  "mode": "headless",
  "chromium_available": true,
  "chromium_path": "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
  "available_modes": ["headless", "headed", "attach"]
}
```

`available_modes` reflects what's reachable *right now*: `headless`/`headed` require Chromium on the host, `attach` requires a Chrome listening on `127.0.0.1:<attach_port>`.

## Security notes

- Browser sessions inherit the host's network access — there is no host allowlist on the CDP path. If you need to confine where an agent can browse, run the daemon under a network-restricted user, behind a firewall, or under a sandbox (Docker, `firejail`, macOS Sandbox-Exec).
- `attach` mode connects to whatever Chrome happens to be listening on the configured port. Only use it on trusted local hosts.
- The AINL `browser` adapter is `privilege_tier: network` in `tooling/security_profiles.json` — it is gated by the same allowlist as `web` / `http` and is *not* enabled in the `local_minimal` or `sandbox_compute_and_store` profiles.
- Per-agent sessions are isolated; mode switches close the current session before starting a new one to avoid mixing states.
