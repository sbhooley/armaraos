# Skills workspace ↔ ArmaraOS (ClawHub / editor capture)

**You do not need OpenClaw.** ArmaraOS only needs a **folder** where skill capture lives (for example `.learnings/` from ClawHub-style skills). Point `workspace_path` at that folder, or use the defaults below.

**Agents:** You are on **ArmaraOS**, not OpenClaw. Do not install the OpenClaw CLI/npm package or follow OpenClaw bootstrap steps to “fix” MCP, AINL, or adapters — those are unrelated. Use OpenClaw migration docs only when the user wants to import data *from* an existing OpenClaw install.

## Division of labor

| Layer | Responsibility |
|-------|----------------|
| A folder you choose (e.g. `…/skills-workspace/.learnings/`) | Capture from skills (markdown, optional editor hooks) |
| `memory/` + long-term notes under that same root | Daily digest + promotion (see your workspace `INTEGRATION.md` if you use one) |
| `[openclaw_workspace]` / `[skills_workspace]` in `config.toml` | Tells ArmaraOS where that folder is |
| Embedded AINL programs + intelligence overlays | Executable behavior under `~/.armaraos/` — not automatic skill-log ingestion |

The kernel **does not** load `.learnings/` into SQLite memory. Digest export only **writes markdown** (and `.pipeline-state.json`) under the configured path.

## Configuration (`~/.armaraos/config.toml`)

Either table name works — same fields:

```toml
[skills_workspace]
enabled = true
# workspace_path = "/path/to/your/capture-root"   # optional; see resolution order below
run_export_on_startup = true
show_pending_in_tray = true
```

(Legacy name `[openclaw_workspace]` is still supported.)

| Field | Meaning |
|-------|---------|
| `enabled` | Master switch for startup export + tray actions. |
| `workspace_path` | Root containing `.learnings/` and `memory/`. Also accepted as `skills_workspace_path` in TOML. |
| `run_export_on_startup` | Run digest export once when the daemon starts (if the resolved path exists or can be created). |
| `show_pending_in_tray` | Desktop: tray tooltip shows pending total; refreshes about every 90s. |

### Path resolution order (same as the kernel)

1. `OPENCLAW_WORKSPACE` (legacy, still supported)
2. `ARMARAOS_SKILLS_WORKSPACE` (ArmaraOS-native alias — same meaning)
3. `workspace_path` / `skills_workspace_path` from config
4. Default: **`~/.armaraos/skills-workspace`** (under `openfang_home_dir()`)

So **ArmaraOS-only** users can ignore OpenClaw entirely and rely on **`ARMARAOS_SKILLS_WORKSPACE`** or the default under `~/.armaraos/`.

## What the desktop does

- Tray: pending line, **Open .learnings folder**, **Run learnings digest now**, tooltip when pending &gt; 0.
- Startup: same digest logic as `scripts/openclaw/export-learnings-to-daily-memory.sh` in the OpenClaw workspace repo (if you use that script elsewhere).

## AINL handoff

Promoted content should enter graphs as **explicit inputs**, not silent file watches. Example: `AI_Native_Lang/examples/compact/openclaw_learning_handoff.ainl`.
