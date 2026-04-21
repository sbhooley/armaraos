# Dashboard: Get started (Overview) page

The **Get started** route is the default landing view in the embedded dashboard (`#overview`). It aggregates health, usage, setup checklist, provider badges, and activity. This document describes layout, **Quick actions**, and where to change them.

> Design policy: all visual changes in this page must follow the canonical rules in [`dashboard-design-system.md`](dashboard-design-system.md).

## Source files

| Piece | Location |
|-------|----------|
| Markup (Alpine template) | `crates/openfang-api/static/index_body.html` — search `page === 'overview'` |
| Page logic (data loads, checklist, formatters, wizard CTA state) | `crates/openfang-api/static/js/pages/overview.js` — `overviewPage()` |
| Overview-specific styles | `crates/openfang-api/static/css/components.css` — classes prefixed `overview-` |
| Root app navigation (Get started re-click) | `crates/openfang-api/static/js/app.js` — `navigateOverview()` |

The dashboard is a single-page Alpine app; new overview sections usually need HTML **and** any new fields or getters in `overview.js`.

## Section order (top to bottom)

Order matters for scanability and for the loading skeleton matching the real layout.

1. **Page header** — Title *Get started*; **Setup Wizard** / **Run setup again** (see [Setup Wizard visibility](#setup-wizard-visibility)); health pill; manual refresh. Wrapper row: `overview-page-header-row` / `overview-page-header-actions`.
2. **Live strip** (conditional) — Last kernel SSE line + **Timeline** when `kernelEvents.last` is set.
3. **Quick actions** — See below; hidden on load error; replaced by a skeleton while `loading` is true (grid includes **seven** skeleton cells matching seven actions).
4. **Loading skeleton** — Shown only while `loading`; includes a Quick actions-shaped skeleton, hero stat placeholders, then panel grid placeholders.
5. **Error state** — Connection failure, retry, debug copy (no Quick actions).
6. **Setup checklist** — When `showSetupChecklist` (see `overview.js`); dismiss uses `localStorage` key `of-checklist-dismissed`.
7. **Onboarding banner** — When onboarding store says so and checklist was dismissed.
8. **Main content** (after successful load) — Hero stats, compact stats, observability snapshot, provider badges, panel grid (System health, Security, Channels, **MCP Servers**, **MCP readiness** chips from `GET /api/mcp/servers` → `readiness.checks`), Recent activity, empty states.

The **Skills/MCP** page (`#skills` → **MCP Servers**) leads with **Add custom MCP server** (`POST /api/integrations/custom/*`) and a collapsible **preset examples** section (`POST /api/integrations/*`) for curated templates — both install without editing TOML; that flow complements the overview **MCP Servers** / readiness chips.

The duplicate **Quick actions** block that previously appeared after the panel grid was removed; actions exist only in the top region.

## Quick actions

**Purpose:** One tap to common hash routes without opening the sidebar.

**Visibility:**

- Shown when `!loadError && !loading` (real buttons).
- While `loading`, a **skeleton** card with class `overview-quick-actions--skeleton` occupies the same vertical slot so the page does not jump when data arrives.
- Hidden entirely when the overview load failed (`loadError`), since navigation may be misleading until retry.

**Actions and targets:**

| Label | Hash route |
|-------|------------|
| New Agent | `#agents` |
| Browse Skills/MCP | `#skills` |
| App Store | `#ainl-library` (internal page id `ainl-library`; alias `#app-store` may redirect in `app.js`) |
| Add Channel | `#channels` |
| Create Workflow | `#workflows` |
| Settings | `#settings` |
| Daemon & runtime | `#runtime` — reload config/channels/integrations or shut down (see [dashboard-settings-runtime-ui.md](dashboard-settings-runtime-ui.md)) |

**Markup roles:** The container is `role="region"` with `aria-label="Quick actions"`. Each control is a `<button type="button">` (not a link) that sets `location.hash`.

## CSS reference (overview quick actions)

Classes live in `components.css`:

- **`overview-quick-actions`** — Card container; gradient background, accent top border, spacing below the live strip.
- **`overview-quick-actions-head`** — Title row (*Quick actions* + subtitle *Common tasks*).
- **`overview-quick-actions-grid`** — Responsive CSS grid (`minmax(148px, 1fr)`).
- **`overview-quick-action-tile`** — Full-width-in-cell button, icon + label row, hover/focus styles.
- **`overview-quick-action-icon`** / **`overview-quick-action-label`** — Icon chip and text.
- **`overview-quick-actions--skeleton`** — Loading placeholder; `pointer-events: none`.
- **`overview-quick-action-skel`** — Skeleton cell height inside the grid.

Older class **`overview-inline-actions`** was removed with the bottom quick-actions card; do not reintroduce it unless a second strip is needed elsewhere.

## Setup Wizard visibility

**Behavior reference (steps, provider rules, manifest TOML, rebuild):** [dashboard-setup-wizard.md](dashboard-setup-wizard.md).

After the guided **Setup Wizard** completes, the dashboard sets `localStorage` **`openfang-onboarded`** to **`true`** (see `wizard.js`). The Get started page uses that flag to avoid cluttering the header and checklist for users who already finished onboarding.

| State | Header | Checklist **Setup Wizard** button |
|-------|--------|-----------------------------------|
| Not onboarded (`openfang-onboarded` ≠ `true`) | Primary **Setup Wizard** → `#wizard` | Shown (when checklist is visible) |
| Onboarded, wizard CTA collapsed | Ghost **Run setup again** → reveals wizard UI | Hidden until CTA revealed |
| Onboarded, CTA revealed | Primary **Setup Wizard** → `#wizard` | Shown (when checklist is visible) |

**Alpine state** (`overview.js`): `onboarded` (from `localStorage`), `overviewWizardCtaVisible` (initialized to `!onboarded`). **Run setup again** calls `revealSetupWizardCta()`. **`refreshOnboardingFlags()`** runs after successful `loadOverview` and `silentRefresh` so completing the wizard in-session collapses the CTA again.

**Sidebar:** The **Get started** nav item calls **`navigateOverview()`** instead of `navigate('overview')`. If the user is **already** on Get started and clicks **Get started** again, the app dispatches a window event **`openfang-overview-nav-same-page`**; the overview root listens with `@openfang-overview-nav-same-page.window="onOverviewNavSamePage()"`, which reveals the wizard CTA for onboarded users (same effect as **Run setup again**).

**Init order:** `x-init="initOverviewWizardCta(); loadOverview().then(() => startAutoRefresh())"`.

## Setup checklist (same page)

- **Core (3):** provider configured, at least one agent, at least one enabled schedule — card title **Getting Started**.
- **Optional section:** **channel** row can complete (progress bar after core is 0–100% for this row only). **Chat** and **Skills/MCP** rows are **perpetual shortcuts** (`perpetual: true` in `overview.js`): always ○, always **Go**, never strikethrough; they do not use `localStorage` completion flags.
- **Dismiss:** `localStorage` key `of-checklist-dismissed` hides the entire card.
- **Setup Wizard** button in the checklist card is shown only when `overviewWizardCtaVisible` is true (same rule as the header primary button); onboarded users who have not clicked **Run setup again** (or the sidebar re-click) will not see it there either.

## Related behavior (same page)

- **Auto-refresh:** `silentRefresh()` on a 30s interval; debounced refresh on `armaraos-kernel-event` for lifecycle/system events (`overviewShouldRefreshOnKernelEvent` in `overview.js`).
- **Teardown:** `@page-leave.window="stopAutoRefresh()"` clears the interval and kernel listener.
- **Usage + cost hero:** `loadUsage()` in `overview.js` reads **`GET /api/usage/summary`** (SQLite-backed totals) so headline token and USD figures stay meaningful across **daemon restarts** and desktop upgrades when the ArmaraOS data directory is intact.
- **Implementation** of titles, progress, and `showSetupChecklist` is in `overview.js` getters; keep **`docs/dashboard-testing.md`** (*Get started page — setup checklist*) in sync when that logic changes. Manual QA steps (hashes, `localStorage`, removed onboarding keys) live there. First-run **Setup Wizard** contract: [dashboard-setup-wizard.md](dashboard-setup-wizard.md).

## Sidebar navigation

The dashboard sidebar exposes **Get started** as its **own section above Chat**. **Comms** lives under **Monitor** (with **Timeline**, **Logs**, **Runtime**, etc.), not under **Agents**. The hash route remains **`#overview`** for deep links and wizard redirects. **Get started** uses **`navigateOverview()`** so a second click while already on the page can reveal the Setup Wizard for onboarded users (see [Setup Wizard visibility](#setup-wizard-visibility)).

## App Store page (related)

The **App Store** route is **`#ainl-library`**. In the library UI, the collapsible section that lists synced programs on disk is titled **AI Native Lang Programs Available** (user-facing copy; implementation in `index_body.html` near `app-store-section-toggle-title`). Deeper layout for that page lives in `js/pages/ainl-library.js` and `components.css` (`app-store-*` classes). OOTB disk layout: [ootb-ainl.md](ootb-ainl.md).

## Global notification bell (all routes)

The **notification center** is not overview-specific: the bell lives in the root shell (`index_body.html` / `app.js` / `layout.css`) so it appears on every hash route. **`--notify-bell-reserve`** + **`.main-content`** right padding keep page chrome from sliding under the fixed bell; focus mode hides the bell and clears that padding. Behavior, API wiring, and QA checklist: [dashboard-testing.md](dashboard-testing.md#notification-center-bell).

## Manual verification

See **docs/dashboard-testing.md** (manual browser checklist): confirm Quick actions appear after load (**seven** tiles), navigate to the correct tabs (including **Daemon & runtime** → `#runtime`), and that the loading state shows the skeleton then swaps to buttons without large layout shift.
