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

- **`armaraos-ainl-health-weekly`** — **Enabled** by default. Runs `armaraos-programs/armaraos_health_ping/armaraos_health_ping.ainl` weekly (`0 9 * * 7` — Sunday 09:00 UTC; day-of-week uses **1=Mon … 7=Sun** in the scheduler). Pure `core` ops only (no network). Uses `ainl run --json`.
- **`armaraos-automation-stub-weekly`** — **Disabled** by default. Template graph at `armaraos-programs/armaraos_automation_stub/armaraos_automation_stub.ainl` for extending with adapters; opt in from the scheduler.
- Other entries (upstream examples, learning-frame echo, skill mint stub) ship **disabled** so users can opt in from the scheduler UI.

Disable all curated registration with `ARMARAOS_DISABLE_CURATED_AINL_CRON=1`.

## Learning frames in cron

Entries may include a validated **`frame`** object (see [learning-frame-v1.md](learning-frame-v1.md)). At registration time the kernel deserializes and validates [`LearningFrameV1`](https://github.com/sbhooley/armaraos/blob/main/crates/openfang-types/src/learning_frame.rs) before attaching the job.

## Emit targets (toolchain)

The `ainl` CLI supports `ainl emit --target <t>` (e.g. `cron`, `ir`, `langgraph`). Use it to generate sidecar artifacts from `.ainl` sources when you want manifests or exports **outside** the kernel’s built-in cron JSON. ArmaraOS does not auto-import emit output; it is a developer workflow documented here for parity with the AINL toolchain.

## MCP and agents

When the desktop app or `ainl install armaraos` configures **`ainl-mcp`**, agents with MCP access can validate, compile, and run graphs against `~/.armaraos/ainl-library/`. Prefer **AINL** for new automation unless a concrete constraint requires another language ([ainl-first-language.md](ainl-first-language.md)).

## Dashboard

`GET /api/ainl/library` lists `.ainl` / `.lang` files under `ainl-library`, with **`armaraos-programs`** grouped first when present.

Manual checklist: [ootb-ainl-smoke.md](ootb-ainl-smoke.md).
