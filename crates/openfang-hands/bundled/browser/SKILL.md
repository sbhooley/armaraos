---
name: browser-automation
version: "2.0.0"
description: Chrome DevTools Protocol (CDP) browser automation for autonomous web interaction
author: OpenFang
tags: [browser, automation, cdp, web, scraping]
tools: [browser_navigate, browser_click, browser_click_text, browser_type, browser_fill, browser_screenshot, browser_read_page, browser_snapshot, browser_close, browser_scroll, browser_wait, browser_run_js, browser_back, browser_session_start, browser_session_status]
runtime: prompt_only
---

# Browser Automation Skill (CDP)

ArmaraOS drives a real Chrome/Chromium via the Chrome DevTools Protocol.
No Playwright, no Puppeteer — direct WebSocket CDP.

## Recommended Workflow for Dynamic / SPA Pages

1. **`browser_navigate`** → open the URL
2. **`browser_snapshot`** → see all interactive elements with selector hints
3. **`browser_fill`** or **`browser_click_text`** → interact by name/label/text
4. **`browser_read_page`** → verify result

This workflow avoids brittle CSS selectors entirely.

## Tool Selection Guide

| Task | Best tool | Why |
|------|-----------|-----|
| Click a button/link/tab | `browser_click_text` | Matches by visible text, ignores generated CSS classes |
| Click by known CSS/ID | `browser_click` | Falls back to text match if CSS fails |
| Fill a form field | `browser_fill` | Tries name, label, placeholder, testId, aria-label, id, CSS — 7 strategies |
| Type into known input | `browser_type` | Also tries name/placeholder/label fallback |
| See what's on the page | `browser_snapshot` | Structured list of all interactive elements with selectors |
| Read page content | `browser_read_page` | Markdown text content |
| Wait for element | `browser_wait` | 15s default, returns diagnostic on timeout |
| Advanced DOM work | `browser_run_js` | Arbitrary JavaScript |
| Switch browser mode | `browser_session_start` | headless → headed → attach |

## Selector Strategy (when you must use CSS)

Prefer these selectors in order:
1. `[name="email"]` — stable across deploys
2. `[data-testid="submit"]` — designed for automation
3. `#unique-id` — stable if set by developers
4. `[aria-label="Close"]` — accessibility attribute
5. `[placeholder="Enter email"]` — usually stable
6. `.class-name` — **last resort**, often generated

## Browser Modes

| Mode | Use when |
|------|----------|
| `headless` (default) | Background tasks, scraping, speed |
| `headed` | Site detects headless, user wants to watch, debugging |
| `attach` | Drive user's real Chrome (their cookies, sign-ins, profile) |

Switch: `browser_session_start { mode: "headed" }`

## Form Filling (React / SPA)

React controlled inputs intercept `el.value = x` and ignore it.
`browser_fill` and `browser_type` use the native value setter trick
to bypass React's synthetic event system:

```
browser_fill { field: "email", value: "user@example.com" }
browser_fill { field: "password", value: "MyPass123!" }
browser_fill { field: "displayName", value: "Agent Bot" }
browser_click_text { text: "Register" }
```

## Common Workflows

### Account Login
```
1. browser_navigate → login page
2. browser_snapshot → see available fields
3. browser_fill → email field
4. browser_fill → password field
5. browser_click_text → "Sign In" or "Log In"
6. browser_read_page → verify success
```

### Form Submission
```
1. browser_navigate → form page
2. browser_snapshot → map all fields and buttons
3. browser_fill → fill each field by name/label
4. browser_click_text → submit button text
5. browser_read_page → verify confirmation
```

### Tab / Panel Navigation
```
1. browser_snapshot → see available tabs
2. browser_click_text → tab label text (e.g., "Register")
3. browser_wait → wait for panel content selector
4. browser_snapshot → see fields in new panel
```

## Error Recovery

| Error | Recovery |
|-------|----------|
| Element not found | Use `browser_snapshot` to see available elements |
| Selector timeout | Increase `timeout_ms`, check `readyState` in error |
| Page navigating | Use `browser_wait` for a stable element first |
| React input ignored | Use `browser_fill` instead of `browser_type` |
| Click missed | Use `browser_click_text` with visible text |
| Session dropped | Use `browser_session_status` to check, re-navigate |
| Headless blocked | Switch to `headed` or `attach` mode |

## Security

- Verify domain before entering credentials
- Never store passwords in memory_store
- Check for HTTPS before submitting sensitive data
- Never auto-approve financial transactions
