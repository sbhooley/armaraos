# Dashboard UI Design System (Canonical)

This is the **single source of truth** for dashboard visual design in ArmaraOS.

If you are a human contributor or AI agent making UI changes, follow this file first.

## Scope

### Paths (canonical)

The **entire embedded dashboard** under **`crates/openfang-api/static/`** is in scope, including:

- `css/theme.css`, `css/layout.css`, `css/components.css`
- `index_head.html`, `index_body.html`
- **`js/**`** — `app.js`, `js/pages/*.js`, loaders such as **`page-load-error.js`**, and all bundled dashboard scripts

### Coverage target (“everywhere” in `static/`)

**Policy:** keep tightening this tree so **all product surfaces** — chat, agents, settings, runtime, modals, overlays (e.g. command palette), load/error flows, and any other `#hash` pages — use **shared classes** and **`theme.css` variables** for layout and appearance. Prefer removing or replacing **inline `style="..."`** and **ad-hoc hex neutrals** when touching a file; large cleanups may be incremental.

**Out of scope (explicit carve-outs):**

- **`crates/openfang-api/static/vendor/**`** — third-party bundles (e.g. syntax/highlight CSS). Do not “theme” them by scattering product `#hex` in random places; if they must be adjusted, do it in a **controlled** way (documented override or alternative vendor theme). **Build-time or test harness HTML** outside `static/` is out of scope unless it becomes shipped UI.
- **Desktop shell** (`crates/openfang-desktop/`) — native menus, tray, OS dialogs, icons: follow **brand/copy/icon** consistency; **not** `theme.css` (no parallel CSS skin).

### Exceptions (practical)

**Canvas, SVG, and chart libraries** (Chart.js, D3, workflow graph colors, usage palettes) may use **programmatic hex/rgba** for **series discrimination** or **drawing APIs** that do not read CSS variables. Prefer **semantic alignment** with dashboard status/accent colors where reasonable; do not use that as an excuse for **layout chrome** (panels, nav, forms) to bypass tokens.

## Non-Negotiable Rules

1. **Token-first edits only**
   - Use existing CSS variables and component classes.
   - Do not introduce arbitrary one-off values unless a new reusable token/pattern is required.

2. **No inline visual styling**
   - Do not add inline `style="..."` for colors, spacing, typography, or shadows.
   - Convert inline styles into named classes in `components.css` / `layout.css`.

3. **Extend before replace**
   - Preserve existing UI patterns and layer improvements through shared classes.
   - Avoid rewriting entire sections when a scoped class override can solve the issue.

4. **Keep visual hierarchy consistent**
   - Typography, spacing, radii, border strength, and shadows must follow the scales below.

5. **Accessibility and interaction parity**
   - Preserve keyboard focus states, hover states, contrast, and motion clarity.
   - New controls must include usable focus-visible styles.

## Color palette and surfaces (conformance — “2026-tier” product UI)

These rules keep the dashboard feeling **premium, intentional, and cohesive** with the **gold/amber brand** while preserving **near-white** light canvases and **true / near-black** dark canvases.

### Principles (mandatory)

1. **Chromatic neutrals, not flat “web gray”**  
   Surfaces and borders use **warm-tinted neutrals** (slight brown/amber bias) so panels read as *designed*, not default gray boxes. Gold is the product accent; neutrals should harmonize with it.

2. **Few named layers; clear steps**  
   Do not introduce many similar mid-grays. Use the **semantic roles** below and change only via `crates/openfang-api/static/css/theme.css` tokens unless a new reusable token is warranted.

3. **Gold is for signal, not wallpaper**  
   Reserve `--accent` and amber mixes for **selection, primary actions, focus, key metrics, and brand emphasis**. Everyday chrome (borders, quiet panels, disabled states) stays in the **neutral ramp**.

4. **Depth from structure, not heavy fills**  
   Prefer **hairline borders** (`--border`, `--border-subtle`) and existing **shadow tokens** over large jumps in gray L\* for hierarchy. Cards should feel *lifted* (border + subtle shadow/inset already in system), not *mud-flats*.

5. **Semantic status colors stay separate**  
   Success/error/warning/info use their tokens; do not tint whole cards with brand amber unless the pattern already exists for that component.

6. **Light mode warmth**  
   Page/canvas may be **warm off-white**; cards may be **white** or warm-tinted neutrals. Primary text stays **warm ink** (`--text`), not cold pure `#000000`, unless contrast requires it.

7. **Dark mode depth**  
   Default dark canvas may be **true black** for OLED-style depth; surfaces step up in **warm charcoal** layers (never a single muddy gray).

### Semantic surface roles (token map)

| Role | Variable | Use |
|------|----------|-----|
| App canvas | `--bg` | Outermost background (light: warm off-white; dark: black/near-black). |
| Chrome / sidebars / large panels | `--bg-primary` | Structural regions below modals, distinct from canvas. |
| Elevated shell | `--bg-elevated` | Sticky headers, popover-ish regions — between primary and cards. |
| Primary card / panel fill | `--surface` | Default card and panel body. |
| Secondary inset / striped rows | `--surface2` | Secondary blocks, dense lists, input well. |
| Tertiary / sunken | `--surface3` | Muted wells, subtle differentiation — use sparingly. |
| Hairline / medium / strong borders | `--border`, `--border-subtle`, `--border-light`, `--border-strong` | Dividers, card strokes, separators. |

**Implementation:** all semantic colors for the dashboard ship from **`theme.css`** (`[data-theme="light"]` and `:root` dark defaults). Agents must **not** scatter one-off `#rgb` neutrals in new CSS.

### Checklist before merging UI changes

- [ ] New neutrals go through **tokens** in `theme.css` (or a documented new token in this file).
- [ ] Light mode stays **warm off-white / white** family; dark mode stays **black canvas + warm charcoal** surfaces.
- [ ] **Gold** use is **hierarchical** (signal vs chrome).
- [ ] Focus/hover/contrast and reduced-motion behavior remain intact.

## Canonical Typography Scale

Use this scale for dashboard UI text:

- `9px`: ultra-compact metadata badges (`.runtime-badge`, `.tier-badge`, `.sec-badge`, `.model-switcher-tier`)
- `10px`: labels + dense metadata (`.form-group label`, `.log-level`, `.log-timestamp`, compact chips)
- `11px`: body microcopy and operational indicators (`.log-entry`, `.live-indicator`, small pills)
- `12px`: secondary body + compact controls (`.btn-sm`, table body/meta, panel micro-headings)
- `13px`: primary control/body text (`.btn`, `.form-input`, core dashboard body)
- `14px`: section/page header titles and high-salience UI labels

Line-height guidance:

- 1.25–1.35 for uppercase labels/tight metadata
- 1.4–1.5 for body/control text
- 1.5+ for long-form text blocks

## Canonical Spacing Rhythm

Use an 8px system with compact derivatives:

- Primary spacing: `8, 16, 24`
- Compact spacing: `4, 6, 10, 12, 14`
- Section rhythm:
  - page shell: 16/24 gutters depending on breakpoint
  - card internals: 12–16
  - dense rows (logs/tables): 8

Do not introduce irregular spacing jumps (for example 3, 5, 7, 11, 13, 15) unless tied to an existing pattern.

## Radius, Border, Shadow, Motion

- Radius: use existing tokens (`--radius-sm`, `--radius-md`, `--radius-lg`, `--radius-xl`)
- Border: default `var(--border)`; accent borders should use `color-mix` against accent + border
- Shadow: prefer token shadows (`--shadow-xs/sm/md/lg/xl`, `--shadow-inset`, `--shadow-accent`)
- Motion:
  - quick micro transitions: `0.12s` to `0.16s`
  - larger transforms/entry: `0.18s` to `0.25s`
  - use existing easing tokens (`--ease-smooth`, `--ease-out`, `--ease-spring`)

## Component Rules

### Buttons
- Keep button text in the typography ladder (13 primary, 12 small).
- Preserve hover lift and focus affordance; do not remove focus-visible behavior.

### Badges and pills
- Use the 9/10/11 hierarchy by semantic density.
- Keep uppercase badge letter-spacing consistent (`0.06em` range for compact labels).

### Dropdowns, popovers, command surfaces
- Use consistent shell treatment: border + elevated shadow + subtle backdrop blur where already established.
- Hover behavior should be subtle (`translateX(1px)` class of motion max).

### Data surfaces (tables/logs/timelines)
- Preserve sticky headers where implemented.
- Use tabular numerics for timestamp/value scanability where relevant.
- Prefer subtle row highlight/contrast improvements over dramatic color changes.

### Mobile
- At `<=768px` and `<=480px`, preserve rhythm by reducing density, not removing hierarchy.
- Keep bell/header safe-area behavior intact (`--notify-bell-reserve` interactions).

### Get started (Overview) — Operations snapshot
- The **Operations snapshot** card uses **`overview-snapshot-card`** plus an **integration** strip (**`overview-snapshot-integrations`**, **`overview-snapshot-int`**, link modifier **`overview-snapshot-int--link`** for hash navigation) and, when observability is available, the existing KPI grid and foot (**`overview-snapshot-kpi*`**, **`overview-snapshot-foot*`**). Keep gold/accent for **focus and interactive** tiles, not full-card fills.
- Full markup, data bindings, and visibility rules: **[`dashboard-overview-ui.md`](dashboard-overview-ui.md)** (*Operations snapshot card*).

## Required Workflow for Design Changes

1. Read this file and related page docs before editing.
2. Implement changes with shared classes/tokens.
3. Verify:
   - no broken layout across desktop + mobile breakpoints
   - no regressions in focus/hover states
4. Run lint/diagnostics on touched files.
5. Document any new tokens/patterns here in the same PR.

## Agent Enforcement Policy

Future AI agents should treat this file as **mandatory policy** for dashboard design edits.

- If a requested design change conflicts with these rules, propose a tokenized/system-consistent alternative.
- Do not ship ad-hoc visual styling that bypasses this system.

## Related Docs

- `docs/dashboard-overview-ui.md`
- `docs/dashboard-settings-runtime-ui.md`
- `docs/dashboard-testing.md`
- `CLAUDE.md` (agent instructions)
- `CONTRIBUTING.md` (contributor policy)
