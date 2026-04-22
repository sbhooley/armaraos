# Scheduled AINL graphs (cron)

Scheduled jobs whose action is **`ainl_run`** execute `ainl run …` as a subprocess of the ArmaraOS/ArmaraOS daemon. This page explains how **secrets**, **adapter policy**, and **manifest metadata** interact so you do not have to reinstall the app or re-enter keys for every schedule change.

## Where secrets live

- **Credential resolver chain** (same as chat / LLM): **vault** → **`~/.armaraos/.env`** → **process environment**.
- The daemon does **not** load `.env` into the global OS environment of the process; it keeps those values in the resolver. Scheduled `ainl run` therefore **injects** resolved values into the child process for a fixed set of env var names (API keys, DB URLs, OpenClaw paths, etc.).

**Practical rule:** put provider keys and shared paths in **`~/.armaraos/.env`** (or the dashboard / vault). Scheduled graphs see the **same** keys as interactive use, without a separate “cron env” file.

The kernel checks a **primary** list of env names (LLM keys, DB URLs, OpenClaw paths, etc.) plus a small **extension** tier for niche OpenClaw/CRM variables. At **debug**, each cron run logs **`keys`** (names only) and **`extension_resolved_keys`** for the extension tier. At **trace**, target **`openfang_kernel::ainl_cron_env`** logs each extension key that actually resolved so you can confirm what your install uses before dropping names from that tier.

## Adapter host allowlist (`AINL_HOST_ADAPTER_ALLOWLIST`)

The AINL runtime can **intersect** the graph’s IR-derived adapter list with a **host grant** (same idea as hosted runners). The subprocess may receive:

`AINL_HOST_ADAPTER_ALLOWLIST=core,http,web,queue,…`

When **`AINL_ALLOW_IR_DECLARED_ADAPTERS=1`** (the default for scheduled jobs below), the Python runtime **does not apply** this variable from the **environment** for intersection (CLI/`--host-adapter-allowlist` still wins). Use the next section for the full relax story.

### Default behavior (daemon)

For each cron job, the kernel looks at the job’s **target agent**:

- If manifest metadata contains **`ainl_host_adapter_allowlist`** (string), that value is passed verbatim. Use **`off`** or **`-`** to disable injecting this variable for that agent.
- Otherwise, if the agent looks “online-capable” (non-empty **network**, **tools**, **shell**, **agent_spawn**, or **ofp_connect** in the manifest), the daemon sets the **full** default list that matches the `ainl` CLI registry (equivalent to an unrestricted local `ainl run` for typical graphs).
- If the agent is offline-only on those axes, the variable is **not** set (no extra narrowing).

The dashboard **Agents → Info** panel (and **`GET /api/agents/{id}`**) includes read-only **`scheduled_ainl_host_adapter`**: `source` (`none` | `metadata` | `default_online`), a **`summary`** string, **`ainl_allow_ir_declared_adapters`** (`"1"` or `"0"`), and either **`allowlist`** (metadata) or **`adapter_count`** (default online list).

### CLI / manual runs

- **Environment:** `AINL_HOST_ADAPTER_ALLOWLIST=core,web,queue` (comma-separated).
- **CLI:** `ainl run --host-adapter-allowlist core,web,queue …` (overrides the env var for that process).

See the AI Native Lang runtime: `RuntimeEngine` reads the parameter first, then `AINL_HOST_ADAPTER_ALLOWLIST`.

## IR-declared adapters (`AINL_ALLOW_IR_DECLARED_ADAPTERS`)

The AINL Python runtime can **ignore `AINL_HOST_ADAPTER_ALLOWLIST` from the environment** when **`AINL_ALLOW_IR_DECLARED_ADAPTERS=1`**, so graphs may use any adapter referenced in the IR (`web`, `http`, …). **`AINL_HOST_ADAPTER_DENYLIST`**, **`AINL_SECURITY_PROFILE`**, and **`AINL_STRICT_MODE`** still apply.

### Default behavior (daemon + desktop)

- **Scheduled `ainl run`:** the kernel **always** sets **`AINL_ALLOW_IR_DECLARED_ADAPTERS`** on the child process — **`1`** by default, **`0`** only when the job’s target agent manifest sets **`ainl_allow_ir_declared_adapters`** to a falsey value (`"0"`, `"false"`, `"off"`, `"no"`, or JSON **`false`**). This avoids “capability gate: web” failures when the daemon was started from a shell that exported a narrow **`AINL_HOST_ADAPTER_ALLOWLIST`** (the kernel still **removes** inherited allowlist before optionally re-injecting its own; with relax **`1`**, Python ignores that env allowlist anyway).
- **Desktop app:** after loading **`~/.armaraos/.env`** (and **`secrets.env`**) into the process, if **`AINL_ALLOW_IR_DECLARED_ADAPTERS`** is still unset, the embedded server sets it to **`1`**. **Settings → AINL → Try** (`ainl validate` / `ainl run` on library files) also passes **`AINL_ALLOW_IR_DECLARED_ADAPTERS=1`** on that subprocess.

**Dashboard / API:** **`GET /api/agents/{id}`** includes **`scheduled_ainl_host_adapter.ainl_allow_ir_declared_adapters`** (`"1"` or `"0"`) next to the allowlist summary fields.

### Headless CLI daemon

If you run **`openfang start`** from a terminal **without** going through the desktop shell, the process does **not** get the desktop’s default; scheduled jobs **still** inject **`AINL_ALLOW_IR_DECLARED_ADAPTERS=1`** per the kernel rule above. For **manual** `ainl run` in the same terminal, set the variable yourself or rely on upstream AINL behavior (e.g. **`intelligence/`** paths); see **`AI_Native_Lang/AGENTS.md`**.

## Intelligence digest graphs (`intelligence/*.lang`)

Programs like **`intelligence_digest.lang`** call **`web`**, **`tiktok`**, **`cache`**, **`queue`**, and **`memory`** (via `genmem`). With current kernel defaults, scheduled runs should **not** fail on **`adapter blocked by capability gate: web`** solely because the user never set environment variables.

**If it still fails**, typical causes are: an **old** ArmaraOS/AINL pair, manifest **`ainl_allow_ir_declared_adapters`** set to **off**, **`AINL_INTELLIGENCE_FORCE_HOST_POLICY=1`** on the AINL side, or running **`ainl`** **outside** ArmaraOS without relax. **Fix (pick one):**

1. **Upgrade** ArmaraOS and PyPI **`ainativelang`** so scheduled injection and intelligence-path relax are present.

2. **Manifest:** ensure **`ainl_allow_ir_declared_adapters`** is not **`"0"`** / **`false`** unless you intend strict host intersection. To force relax explicitly:

   ```toml
   [metadata]
   ainl_allow_ir_declared_adapters = "1"
   ```

3. **Explicit allowlist:** set **`ainl_host_adapter_allowlist`** to a CSV that includes at least **`core,web,tiktok,cache,queue,memory`** (only needed if relax is **`0`** and you enumerate adapters). Copy the full default line from **[`docs/snippets/agent-metadata-intelligence-cron.toml`](snippets/agent-metadata-intelligence-cron.toml)**.

4. **Upstream AINL:** sources under an **`intelligence/`** path also set **`AINL_ALLOW_IR_DECLARED_ADAPTERS=1`** when unset (unless **`AINL_INTELLIGENCE_FORCE_HOST_POLICY=1`**).

After editing the agent manifest, restart the agent or reload configuration so the kernel picks up metadata — e.g. **Settings → System Info → Daemon / API** or **Monitor → Runtime** → **Reload config**, or a full daemon restart. Check **Agents → agent → Info** for **`scheduled_ainl_host_adapter`** (summary, **`ainl_allow_ir_declared_adapters`**, and allowlist fields). Narrative: **`AI_Native_Lang/docs/INTELLIGENCE_PROGRAMS.md`**; UI map: **[dashboard-settings-runtime-ui.md](dashboard-settings-runtime-ui.md)** (*Daemon / API runtime*).

## AINL bundle + graph memory (stateful scheduled runs)

Scheduled jobs with action **`ainl_run`** are executed by **`OpenFangKernel::cron_run_job`** (`crates/openfang-kernel/src/kernel.rs`): it resolves the program under **`~/.armaraos/ainl-library/`**, builds a `tokio::process::Command` for **`ainl run`**, then applies host env injection (secrets, adapter policy — sections above). Optional helpers live in **`openfang_runtime::ainl_bundle_cron`** (`crates/openfang-runtime/src/ainl_bundle_cron.rs`).

| Path | Role |
|------|------|
| **`~/.armaraos/agents/<agent_id>/bundle.ainlbundle`** | Optional **`AINLBundle`** JSON (workflow IR + memory snapshot + persona list + tools). Not required for every graph; created or updated by the host export path. |

**Pre-run (bundle → Python):** If **`~/.armaraos/agents/<agent_id>/bundle.ainlbundle`** exists, the kernel sets on the child process:

- **`AINL_BUNDLE_PATH`** — absolute path to that file  
- **`AINL_AGENT_ID`** — the cron job’s target agent id (string)

The AINL subprocess should have **`~/.armaraos/ainl-library`** on **`PYTHONPATH`** (normal ArmaraOS layout). **`AINLGraphMemoryBridge.boot()`** in the **ainativelang** repo (`armaraos/bridge/ainl_graph_memory.py`) reads **`AINL_BUNDLE_PATH`**, loads **`AINLBundle`**, and best-effort replays **`bundle.persona`** via **`persona.update`**, then **`bundle.memory`** (serialized non-persona **`MemoryNode`** dicts from the last export — episodic, semantic, procedural, patch) into the JSON graph store **only for ids not already present** so scheduled runs restore semantic / episodic / procedural state as well as traits. Malformed bundle rows are skipped; the live store wins on id collisions.

**Post-run (Python → bundle):** After **`ainl` exits successfully**, the kernel schedules a **non-blocking** export (`tokio::task::spawn_blocking` → **`openfang_runtime::ainl_bundle_cron::export_ainl_bundle_after_ainl_run_best_effort`** in `crates/openfang-runtime/src/ainl_bundle_cron.rs`). That spawns **`python3`** with **`AINL_EXPORT_AGENT_ID`**, re-boots the bridge for that agent, merges the live graph with the previous bundle source (or a tiny default **`.ainl`** stub if no bundle existed yet), and **`AINLBundleBuilder.save`** over **`bundle.ainlbundle`**. Failures are logged only; they do not fail the cron job.

## Session transcript, notifications, and routine monitors

On a **successful** scheduled **`ainl run`**, the kernel normally appends a scheduler envelope (**`<<<ARMARAOS_SCHEDULER_V2>>>`** + stdout) to the **target agent’s session** as an assistant line (`append_cron_output_to_agent_session` in `OpenFangKernel`), which appears in the embedded **Chat** as a scheduled bubble. The kernel also publishes **`CronJobCompleted`** (audit, SSE, and desktop notification wiring).

**Quiet success for embedded health / budget monitors:** for the **curated** job names in **[ootb-ainl.md](ootb-ainl.md)** (*agent health, system health when enabled, daily + threshold budget, weekly AINL smoke*) and the same programs when referenced by path, the host **skips** appending that assistant line on success. **Failures** (nonzero exit, delivery errors, time-outs) still follow the error path: **`CronJobFailed`**, dashboard toasts, and notification-center rows; they are **not** “success-suppressed” into the session.

**Dashboard toasts + bell:** the embedded app treats those same **successful** `CronJobCompleted` events as **low-signal** and does not show an info toast or notification-center row for them, so the bell stays meaningful for work you care to react to. Other scheduled jobs keep the previous success toast + row behavior.

Implementation pointers: **`cron_success_suppresses_session_append`** in **`crates/openfang-kernel/src/kernel.rs`**, and **`armaraosRoutineMonitorCronJobName()`** in **`crates/openfang-api/static/js/app.js`** (must stay aligned with the curated job names).

**Related (chat LLM, not the AINL subprocess):** **`openfang-runtime`** opens per-agent SQLite **`~/.armaraos/agents/<id>/ainl_memory.db`** via **`GraphMemoryWriter`** and, in **`run_agent_loop` / `run_agent_loop_streaming`**, appends recent **Persona** nodes (strength ≥ **0.1**, last **90** days) to the **system prompt** as **`[Persona traits active: …]`**. The same loop optionally writes a **per-agent JSON export** for **ainativelang** (shared directory via **`AINL_GRAPH_MEMORY_ARMARAOS_EXPORT`** or default **`ainl_graph_memory_export.json`** — see **[graph-memory.md](graph-memory.md)** *On-disk layout*). That path uses the **`ainl-memory`** crate (Rust), separate from the JSON **`ainl_graph_memory`** file used inside **`ainl run`**. See **[data-directory.md](data-directory.md)**, **[graph-memory.md](graph-memory.md)** (Rust runtime hub), and **`AI_Native_Lang/docs/adapters/AINL_GRAPH_MEMORY.md`**.

## Editing cron jobs

The API supports **updating** a job in place: **`PUT /api/cron/jobs/{id}`** with the same JSON shape as **`POST /api/cron/jobs`** (including `agent_id`, `name`, `schedule`, `action`, `delivery`, `enabled`, optional `one_shot`). The Dashboard **Scheduler** page can **Edit** or **Duplicate** a job so you do not have to delete and recreate manually.

## Related docs

- [Data directory](data-directory.md) — `~/.armaraos/` layout (per-agent **`ainl_memory.db`**, **`bundle.ainlbundle`**)  
- [AINL graph memory (runtime)](graph-memory.md) — **`GraphMemoryWriter`**, persona **system prompt** hook  
- [Architecture](architecture.md) — **`openfang-runtime`**, **`ainl-memory`**, graph memory  
- [AINL first (default language)](ainl-first-language.md)  
- [OOTB AINL](ootb-ainl.md)  
- **AINL:** [ArmaraOS integration](https://github.com/sbhooley/ainativelang/blob/main/docs/ARMARAOS_INTEGRATION.md), [AINL graph memory adapter](https://github.com/sbhooley/ainativelang/blob/main/docs/adapters/AINL_GRAPH_MEMORY.md)  
- [Publishing `ainl-*` crates](ainl-crates-publish.md) — crates.io order and dry-run caveats  
- [ainl-runtime GraphPatch](ainl-runtime-graph-patch.md) — Rust patch adapter vs Python GraphPatch  
