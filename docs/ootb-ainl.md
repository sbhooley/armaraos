# Out-of-the-box AINL integration

ArmaraOS ships **embedded** AINL graphs from the repo `programs/` tree. On kernel boot (and when calling `POST /api/ainl/library/register-curated`), files are written to:

`~/.armaraos/ainl-library/armaraos-programs/`

This directory is **separate** from the desktop/GitHub mirror of upstream `demo/`, `examples/`, and `intelligence/` (which live alongside it under `ainl-library/`). That separation avoids filename clashes and makes it obvious which graphs are maintained with the ArmaraOS release.

## Embedded revision

The file `armaraos-programs/.embedded-revision.txt` records the embedded content revision (see `EMBEDDED_PROGRAMS_REVISION` in `crates/openfang-kernel/src/embedded_ainl_programs.rs`).

**When to bump `EMBEDDED_PROGRAMS_REVISION`:** increment the string whenever you add, remove, or meaningfully change files under `programs/` that ship in the binary, so operators comparing `.embedded-revision.txt` across upgrades can see that the on-disk mirror should refresh. Pure comment or README tweaks can skip a bump if you prefer less churn.

**CI:** `.github/workflows/ci.yml` runs `cargo build -p openfang-kernel`, `cargo test -p openfang-kernel`, and `ainl validate --strict` on every `programs/**/*.ainl` when PyPI `ainativelang` is installed.

**Build:** `crates/openfang-kernel/build.rs` copies `../../programs` into `$OUT_DIR/embedded_programs_src/` and generates `embedded_programs.rs` with `include_bytes!(concat!(env!("OUT_DIR"), …))` so every file ships in the binary deterministically.

## Environment overrides

| Variable | Effect |
|----------|--------|
| `ARMARAOS_SKIP_EMBEDDED_AINL_PROGRAMS=1` | Skip writing embedded programs (tests, debugging). |
| `ARMARAOS_DISABLE_CURATED_AINL_CRON=1` | Do not register curated `ainl run` cron jobs at boot. |
| `ARMARAOS_AINL_BIN` | `ainl` binary used for scheduled runs. |

## Curated cron jobs

Curated jobs are defined in `crates/openfang-kernel/src/curated_ainl_cron.json`. They register **idempotently** (existing job names are skipped). Paths are relative to `ainl-library/`.

### Enabled by default

| Job name | Schedule | Program | Purpose |
|----------|----------|---------|---------|
| `armaraos-agent-health-monitor` | `*/15 * * * *` | `agent_health_monitor.ainl` | Polls `/api/health` + `/api/agents` every 15 min; **on success** stdout is **not** written into Chat (see [scheduled-ainl.md — session transcript](scheduled-ainl.md#session-transcript-notifications-and-routine-monitors)); **failures** still surface as usual. |
| `armaraos-daily-budget-digest` | `0 8 * * *` | `daily_budget_digest.ainl` | Fetches token spend at 08:00 daily; **on success** **no** session append / success toast (same “quiet success” policy as the row above). |
| `armaraos-budget-threshold-alert` | `0 * * * *` | `budget_threshold_alert.ainl` | Hourly spend vs 80% of limit; **on success** **no** session append / success toast (failures and threshold issues that fail the run still alert). |
| `armaraos-new-version-checker` | `0 10 * * 6` | `new_version_checker.ainl` | Weekly Saturday: GitHub + PyPI vs local version; **JSON appended to the target agent session**. |
| `armaraos-channel-session-digest` | `0 */6 * * *` | `channel_session_digest.ainl` | Every 6h: digest of daemon, agents, channels catalog, workflows; **JSON appended to the target agent session**. |
| `armaraos-ainl-health-weekly` | `0 9 * * 7` | `armaraos_health_ping.ainl` | Sunday: minimal `core` smoke test confirming AINL runtime is operational; **on success** **no** session append / success toast (see [session transcript](scheduled-ainl.md#session-transcript-notifications-and-routine-monitors)). |
| `armaraos-learning-frame-echo-quarterly` | `0 12 1 */3 *` | `learning_frame_echo.ainl` | Quarterly: validates learning-frame handling end-to-end. |

### Disabled (opt-in from Scheduler UI)

| Job name | Program | Notes |
|----------|---------|-------|
| `armaraos-system-health-monitor` | `system_health_monitor.ainl` | Combined local health + upstream versions (does not replace `agent_health_monitor` / `new_version_checker`); when enabled, **on success** uses the same **quiet success** policy as the health/budget table rows above. |
| `armaraos-lead-gen-pipeline` | `lead_gen_pipeline.ainl` | Monday lead-gen showcase; LearningFrame `extra` carries `seed_company` / `use_llm`. See [ainl-showcases.md](ainl-showcases.md). |
| `armaraos-research-pipeline` | `research_pipeline.ainl` | Tuesday research showcase; `extra.research_query` must be URL-safe. |
| `armaraos-weekly-usage-report` | `weekly_usage_report.ainl` | LLM-generated Sunday summary — requires LLM adapter enabled. |
| `armaraos-skill-mint-stub-monthly` | `skill_mint_stub.ainl` | **Opt-in** monthly template: schedule `0 10 2 * *` (10:00 2nd of month). Passes a learning frame with `op: skill_mint` via `--frame-json`. The graph emits a minimal Markdown body (`# intent` + Episode); full SKILL Meta for interactive flows is added by the host (`skills_staging::render_skill_draft_markdown`). See [agent-automation-hardening.md](agent-automation-hardening.md#curated-ainl-skill-mint-stub-reference). |

**Where output goes:** scheduled `ainl run` with `json_output: true` captures **stdout** and, for most jobs, appends it to the **owning agent’s session** (see kernel `append_cron_output_to_agent_session`). The **curated health / budget / weekly AINL-smoke** jobs in the tables above are **exceptions on success** — they do not append to Chat and do not raise success toasts; see **[scheduled-ainl.md#session-transcript-notifications-and-routine-monitors](scheduled-ainl.md#session-transcript-notifications-and-routine-monitors)**. Graphs do **not** automatically call the AINL `memory` adapter or `PUT /api/memory/...` unless you add that in the `.ainl` source. For structured KV, use the HTTP memory API or agent tools from an interactive session.

**Showcase index:** [ainl-showcases.md](ainl-showcases.md) lists all five operator-facing programs and sample JSON.

Disable all curated registration with `ARMARAOS_DISABLE_CURATED_AINL_CRON=1`.

## Learning frames in cron

Entries may include a validated **`frame`** object (see [learning-frame-v1.md](learning-frame-v1.md)). At registration time the kernel deserializes and validates [`LearningFrameV1`](https://github.com/sbhooley/armaraos/blob/main/crates/openfang-types/src/learning_frame.rs) before attaching the job.

## Emit targets (toolchain)

The `ainl` CLI supports `ainl emit --target <t>` (e.g. `cron`, `ir`, `langgraph`). Use it to generate sidecar artifacts from `.ainl` sources when you want manifests or exports **outside** the kernel’s built-in cron JSON. ArmaraOS does not auto-import emit output; it is a developer workflow documented here for parity with the AINL toolchain.

## Scheduled runs and adapter policy

Scheduled **`ainl run`** jobs and the **desktop** embedded server set **`AINL_ALLOW_IR_DECLARED_ADAPTERS=1`** by default so typical graphs (`web`, `http`, …) work without users exporting host-adapter env vars. Per-agent opt-out and interaction with **`AINL_HOST_ADAPTER_ALLOWLIST`** are documented in **[scheduled-ainl.md](scheduled-ainl.md)**.

## MCP and agents

When the desktop app or `ainl install armaraos` configures **`ainl-mcp`**, agents with MCP access can validate, compile, and run graphs against `~/.armaraos/ainl-library/`. Prefer **AINL** for new automation unless a concrete constraint requires another language ([ainl-first-language.md](ainl-first-language.md)).

**Defaults (kernel):** If you set a **non-empty** per-agent tool allowlist, the kernel still merges core file/web/channel tools plus **`mcp_ainl_*`** patterns and explicit names such as **`mcp_ainl_ainl_validate`**, **`mcp_ainl_ainl_compile`**, **`mcp_ainl_ainl_run`**, and **`mcp_resource_read`** (read **`ainl://…`** and other URIs from the connected AINL MCP server via host **`resources/read`**; default **byte-capped** reads — **`docs/mcp-a2a.md`**) so dashboards do not have to list every AINL MCP tool by hand. If you set a **non-empty** MCP **server** allowlist, **`ainl`** is merged in when absent so the AINL MCP namespace stays connected. Agents should call **`mcp_ainl_*`** tools directly when present; they do **not** need to `pip install ainativelang` first — the host exposes the toolchain via MCP. See **GET `/api/agents/{id}/tools`** in [api-reference.md](api-reference.md).

## Dashboard

`GET /api/ainl/library` lists `.ainl` / `.lang` files under `ainl-library`, with **`armaraos-programs`** grouped first when present.

In the embedded dashboard, the **App Store** page (`#ainl-library`) surfaces the same tree; the collapsible on-disk catalog section is labeled **AI Native Lang Programs Available**. UI layout notes: [dashboard-overview-ui.md](dashboard-overview-ui.md) (Get started quick action + cross-link), [dashboard-testing.md](dashboard-testing.md) (manual check).

### Excluding directories from the App Store (`.ainl-library-skip`)

The `ainl-library` walker (`crates/openfang-kernel/src/ainl_library.rs`) respects a **`.ainl-library-skip`** marker file. Any directory tree that contains this file is silently skipped during the recursive walk — its `.ainl` / `.lang` files will not appear in the App Store or `GET /api/ainl/library`.

**When to use it:** place `.ainl-library-skip` in directories that hold development demos, integration tests, or experimental programs not intended for end-user discovery. The upstream `AI_Native_Lang` repo ships `demo/.ainl-library-skip` for this reason.

**Format:** the file may contain a human-readable description (the content is ignored by the walker; only presence matters):

```
This directory is excluded from the ArmaraOS App Store listing.
Files here are development demos that may use experimental syntax.
To include a file, move it to examples/ and ensure it passes:
  ainl validate <file> --strict
```

The marker applies to the directory it lives in **and all subdirectories** beneath it. Sibling directories are unaffected.

Manual checklist: [ootb-ainl-smoke.md](ootb-ainl-smoke.md).
