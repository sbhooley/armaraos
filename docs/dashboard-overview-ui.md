# Dashboard: Command Center (Overview) page

The **Command Center** route is the default landing view in the embedded dashboard (hash still **`#overview`**). It aggregates fleet metrics + agent vitals (shared with **All Agents**), health, usage, setup checklist, token economics, operations snapshot (now including system health, security, channels, and MCP), graph memory, provider badges, and activity. This document describes layout, **Quick actions**, data sources, and where to change them.

> Design policy: all visual changes in this page must follow the canonical rules in [`dashboard-design-system.md`](dashboard-design-system.md).

## Source files

| Piece | Location |
|-------|----------|
| Markup (Alpine template) | `crates/openfang-api/static/index_body.html` — search `page === 'overview'` |
| Page logic (data loads, checklist, formatters, wizard CTA state) | `crates/openfang-api/static/js/pages/overview.js` — `overviewPage()` |
| Overview-specific styles | `crates/openfang-api/static/css/components.css` — classes prefixed `overview-` |
| Root app navigation (Command Center re-click) | `crates/openfang-api/static/js/app.js` — `navigateOverview()` |
| Shared fleet UI logic | `crates/openfang-api/static/js/fleet-vitals-mixin.js` — `armaraosFleetVitalsCore()` (used by overview + All Agents) |

The dashboard is a single-page Alpine app; new overview sections usually need HTML **and** any new fields or getters in `overview.js`.

## Section order (top to bottom)

Order matters for scanability and for the loading skeleton matching the real layout.

1. **Page header** — Title *Command Center*; **Setup Wizard** / **Run setup again** (see [Setup Wizard visibility](#setup-wizard-visibility)); health pill; manual refresh. Wrapper row: `overview-page-header-row` / `overview-page-header-actions`.
2. **Live strip** (conditional) — Last kernel SSE line + **Timeline** when `kernelEvents.last` is set.
3. **Quick actions** — See below; hidden on load error; replaced by a skeleton while `loading` is true (grid includes **seven** skeleton cells matching seven actions).
4. **Loading skeleton** — Shown only while `loading`; includes a Quick actions-shaped skeleton, a **fleet metric strip** placeholder (`command-center-fleet-block--skeleton`) + short agent area stub, a tall placeholder for the **Operations snapshot** card, then a **single** graph-memory panel placeholder.
5. **Error state** — Connection failure, retry, debug copy (no Quick actions).
6. **Setup checklist** — When `showSetupChecklist` (see `overview.js`); dismiss uses `localStorage` key `of-checklist-dismissed`.
7. **Onboarding banner** — When onboarding store says so and checklist was dismissed.
8. **Main content** (after successful load) — In order:
   - **Token Economics** (conditional) — `x-show` when `budget` has a limit and/or spend (`GET /api/budget`); **View Usage** / **Configure Budget** actions.
   - **Fleet metric strip + agent vitals** — same deck/cards as **All Agents** (`fleet-metric-deck`, `agent-vitals-grid`); data from shared `armaraosFleetVitalsCore` + `startCommandCenterFleet()` in `overview.js`.
   - **Operations snapshot** — single card: integration row, optional kernel/observability KPIs, **and** merged subsections: **System health**, **Security systems**, **Connected channels**, **MCP servers**, **MCP readiness** — see [Operations snapshot card](#operations-snapshot-card).
   - **Panel grid** — **Graph memory** only (when `graphMemorySignalsActive`); other former panels are folded into Operations snapshot.
   - **LLM Providers** — Badge row (`overview-provider-card`); **below** the panel grid, **above** recent activity.
   - **Recent activity** / empty state, then bottom **Quick** link cards (if present in the build).

The **Skills/MCP** page (`#skills` → **MCP Servers**) includes a **Host tools for google-workspace-mcp** explainer ( **`uv` / `uvx`**, link to **Settings → Tools** for OAuth), then **Add custom MCP server** and **Preset examples** — that flow complements the overview **MCP** / readiness chips.

## Measured vs estimated savings

- **Tokens used** and **Total cost** (e.g. on the **Usage** page and API consumers) are **actual rollups** from the persistent usage store (`GET /api/usage/summary`: `total_input_tokens`, `total_output_tokens`, `total_cost_usd` for **completed** LLM-metered calls).
- **Tokens saved (est.)** and **Cost saved (est.)** are **not** second meter readings from the provider. They combine:
  - **Compression + prompt-cache economics** from **`GET /api/status` → `eco_compression`** (7-day window in the status payload) — heuristics (e.g. estimated original vs compressed input, cache reads, model-catalog pricing) aggregated in SQLite.
  - **Quota / budget blocks** from **`status.quota_enforcement`** (7-day) and/or fallbacks from **`summary.quota_enforcement`** on the usage summary (all time in that API) — **estimated** input/output for **turns that were blocked** before the LLM ran (no literal completion tokens exist for those events).

Do not treat the “saved” columns as invoice-grade counterfactuals; they are **engineering estimates** for trends and policy impact. Tooltips on the cards in `index_body.html` spell this out for end users.

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

**Economics row:** `overview-economics-stats`, `overview-economics-stats--single-row` (CSS grid, four columns on wide viewports, wraps on small screens). Icons: `overview-stat-icon--blue`, `--green`, `--amber`, `--teal`.

## Operations snapshot card

Single card: **`overview-snapshot-card`** (`role="region"`, `aria-label="Operations snapshot"`). Shown when **`!loading && !loadError`**.

**Header** — `overview-snapshot-top`: eyebrow *Live*, title *Operations snapshot*, lede (integration + kernel copy).

**Integration summary** — `overview-snapshot-integrations` (`aria-label="Integration summary"`). Five compact tiles:

| Tile | Source (Alpine) | Notes |
|------|-----------------|--------|
| Channels | `channels.length` | **Button** — `location.hash='channels'` |
| Skills | `skillCount` | **Button** — `location.hash='skills'` |
| MCP servers | `connectedMcp.length` | Non-link |
| Tool calls | `formatNumber(usageSummary.total_tools)` | From usage summary |
| Providers | `configuredProviders.length` | Success tint when &gt; 0 |

**Kernel / observability block** — Shown when **`observability && observability.agent_count !== undefined`** (**`GET /api/observability/snapshot`**). Four **KPI** cells (`overview-snapshot-kpi*`: agents running/total, daemon uptime, pending approvals, cron jobs) and **foot** rows (channels ready/configured, last scheduler tick). Agent counts and uptime here supersedes a duplicate hero row (there is no separate *Agents* / *Daemon uptime* stat card above the snapshot).

**Compression billing breakdown** — Second `overview-snapshot-foot` block, shown only when **`overviewBilledInputTokensTotal > 0`** (i.e. at least one `eco_compression_events` row was persisted on schema **v15+**, which captures provider-reported `billed_input_tokens` per turn). Three rows:

| Row | Source | Tooltip / meaning |
|-----|--------|------------------|
| Pre-compression input (cum.) | `overviewOriginalInputTokensTotal` | Sum of pre-compression input tokens across all persisted compression turns (audit baseline). |
| Provider-billed input (cum.) | `overviewBilledInputTokensTotal` + `formatCost(overviewBilledInputCostUsdTotal)` | Sum of provider-reported input tokens after compression — what was actually billed — with catalog-priced USD. |
| Saved by compression | `overviewOriginalInputTokensTotal − overviewBilledInputTokensTotal` + `formatCost(overviewCostSavedUsd)` | Counterfactual: input tokens not billed × catalog input $/1M at time of each compression. |

These getters all read **`usageSummary.compression_savings`** (from **`GET /api/usage/summary`**). The same response also exposes **`compression_savings.by_provider_model[]`** for per-(provider, model) breakdowns (`overviewCompressionByProviderModel` in `overview.js`); future iterations may render that as a sub-table inside this block.

**CSS** (in `components.css`): `overview-snapshot-kpis`, `overview-snapshot-kpi*`, `overview-snapshot-foot*`, optional skeleton helpers. Use **theme tokens** per the [design system](dashboard-design-system.md).

## Setup Wizard visibility

**Behavior reference (steps, provider rules, manifest TOML, rebuild):** [dashboard-setup-wizard.md](dashboard-setup-wizard.md).

After the guided **Setup Wizard** completes, the dashboard sets `localStorage` **`openfang-onboarded`** to **`true`** (see `wizard.js`). The Get started page uses that flag to avoid cluttering the header and checklist for users who already finished onboarding.

| State | Header | Checklist **Setup Wizard** button |
|-------|--------|-----------------------------------|
| Not onboarded (`openfang-onboarded` ≠ `true`) | Primary **Setup Wizard** → `#wizard` | Shown (when checklist is visible) |
| Onboarded, wizard CTA collapsed | Ghost **Run setup again** → reveals wizard UI | Hidden until CTA revealed |
| Onboarded, CTA revealed | Primary **Setup Wizard** → `#wizard` | Shown (when checklist is visible) |

**Alpine state** (`overview.js`): `onboarded` (from `localStorage`), `overviewWizardCtaVisible` (initialized to `!onboarded`). **Run setup again** calls `revealSetupWizardCta()`. **`refreshOnboardingFlags()`** runs after successful `loadOverview` and `silentRefresh` so completing the wizard in-session collapses the CTA again.

**Sidebar:** The **Get started** nav item calls **`navigateOverview()`** instead of `navigate('overview')`. If the user is **already** on Get started and clicks **Get started** again, the app dispatches **`openfang-overview-nav-same-page`**; the overview root listens with `@openfang-overview-nav-same-page.window="onOverviewNavSamePage()"`, which reveals the wizard CTA for onboarded users (same effect as **Run setup again**).

**Init order:** `x-init="initOverviewWizardCta(); loadOverview().then(() => startAutoRefresh())"`.

## Setup checklist (same page)

- **Core (3):** provider configured, at least one agent, at least one enabled schedule — card title **Getting Started**.
- **Optional section:** **channel** row can complete (progress bar after core is 0–100% for this row only). **Chat** and **Skills/MCP** rows are **perpetual shortcuts** (`perpetual: true` in `overview.js`): always ○, always **Go**, never strikethrough; they do not use `localStorage` completion flags.
- **Dismiss:** `localStorage` key `of-checklist-dismissed` hides the entire card.
- **Setup Wizard** button in the checklist card is shown only when `overviewWizardCtaVisible` is true (same rule as the header primary button); onboarded users who have not clicked **Run setup again** (or the sidebar re-click) will not see it there either.

## Related behavior (same page)

- **Auto-refresh:** `silentRefresh()` on a 30s interval; debounced refresh on `armaraos-kernel-event` for lifecycle/system events (`overviewShouldRefreshOnKernelEvent` in `overview.js`).
- **Teardown:** `@page-leave.window="stopAutoRefresh()"` clears the interval and kernel listener.
- **Usage + cost:** `loadUsage()` reads **`GET /api/usage/summary`** (SQLite-backed **actual** usage for completed calls). It also stores **`quota_enforcement`** from that response (aggregate quota-block estimates).
- **Status for savings columns:** `loadStatus()` provides **`eco_compression`**, **`quota_enforcement`**, and related fields on **`GET /api/status`**. The getters **`overviewTokensSaved`** / **`overviewCostSavedUsd`** merge compression (7d) + quota avoidance (7d from status when available).
- **Implementation** of titles, progress, and `showSetupChecklist` is in `overview.js` getters; keep **`docs/dashboard-testing.md`** (*Get started page* sections) in sync when that logic changes.

## Sidebar navigation

The dashboard sidebar exposes **Get started** as its **own section above Chat**. **Comms** lives under **Monitor** (with **Timeline**, **Logs**, **Runtime**, etc.), not under **Agents**. The hash route remains **`#overview`** for deep links and wizard redirects. **Get started** uses **`navigateOverview()`** so a second click while already on the page can reveal the Setup Wizard for onboarded users (see [Setup Wizard visibility](#setup-wizard-visibility)).

## App Store page (related)

The **App Store** route is **`#ainl-library`**. Deeper layout: `js/pages/ainl-library.js` and `components.css` (`app-store-*` classes). OOTB disk layout: [ootb-ainl.md](ootb-ainl.md).

## Global notification bell (all routes)

The **notification center** is not overview-specific: the bell lives in the root shell so it appears on every hash route. Chat-related rows (**`chat-unread-*`**) are kept in **sync** with the **All Agents** sidebar and **Agents → Fleet Status** unread badge (same `chatUnreadCounts` source). Behavior and QA: [dashboard-testing.md](dashboard-testing.md#notification-center-bell).

## Manual verification

See **docs/dashboard-testing.md**: Quick actions (seven tiles), **Operations snapshot** (five integration tiles + optional kernel KPIs), the **four-tile** economics row, **LLM Providers** position (below panel grid, above activity), and loading skeleton behavior.
