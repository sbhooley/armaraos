# Scheduled AINL graphs (cron)

Scheduled jobs whose action is **`ainl_run`** execute `ainl run …` as a subprocess of the ArmaraOS/OpenFang daemon. This page explains how **secrets**, **adapter policy**, and **manifest metadata** interact so you do not have to reinstall the app or re-enter keys for every schedule change.

## Where secrets live

- **Credential resolver chain** (same as chat / LLM): **vault** → **`~/.armaraos/.env`** → **process environment**.
- The daemon does **not** load `.env` into the global OS environment of the process; it keeps those values in the resolver. Scheduled `ainl run` therefore **injects** resolved values into the child process for a fixed set of env var names (API keys, DB URLs, OpenClaw paths, etc.).

**Practical rule:** put provider keys and shared paths in **`~/.armaraos/.env`** (or the dashboard / vault). Scheduled graphs see the **same** keys as interactive use, without a separate “cron env” file.

The kernel checks a **primary** list of env names (LLM keys, DB URLs, OpenClaw paths, etc.) plus a small **extension** tier for niche OpenClaw/CRM variables. At **debug**, each cron run logs **`keys`** (names only) and **`extension_resolved_keys`** for the extension tier. At **trace**, target **`openfang_kernel::ainl_cron_env`** logs each extension key that actually resolved so you can confirm what your install uses before dropping names from that tier.

## Adapter host allowlist (`AINL_HOST_ADAPTER_ALLOWLIST`)

The AINL runtime can **intersect** the graph’s IR-derived adapter list with a **host grant** (same idea as hosted runners). The subprocess may receive:

`AINL_HOST_ADAPTER_ALLOWLIST=core,http,web,queue,…`

### Default behavior (daemon)

For each cron job, the kernel looks at the job’s **target agent**:

- If manifest metadata contains **`ainl_host_adapter_allowlist`** (string), that value is passed verbatim. Use **`off`** or **`-`** to disable injecting this variable for that agent.
- Otherwise, if the agent looks “online-capable” (non-empty **network**, **tools**, **shell**, **agent_spawn**, or **ofp_connect** in the manifest), the daemon sets the **full** default list that matches the `ainl` CLI registry (equivalent to an unrestricted local `ainl run` for typical graphs).
- If the agent is offline-only on those axes, the variable is **not** set (no extra narrowing).

The dashboard **Agents → Info** panel (and **`GET /api/agents/{id}`**) includes read-only **`scheduled_ainl_host_adapter`**: `source` (`none` | `metadata` | `default_online`), a **`summary`** string, and either **`allowlist`** (metadata) or **`adapter_count`** (default online list).

### CLI / manual runs

- **Environment:** `AINL_HOST_ADAPTER_ALLOWLIST=core,web,queue` (comma-separated).
- **CLI:** `ainl run --host-adapter-allowlist core,web,queue …` (overrides the env var for that process).

See the AI Native Lang runtime: `RuntimeEngine` reads the parameter first, then `AINL_HOST_ADAPTER_ALLOWLIST`.

## Editing cron jobs

The API supports **updating** a job in place: **`PUT /api/cron/jobs/{id}`** with the same JSON shape as **`POST /api/cron/jobs`** (including `agent_id`, `name`, `schedule`, `action`, `delivery`, `enabled`, optional `one_shot`). The Dashboard **Scheduler** page can **Edit** or **Duplicate** a job so you do not have to delete and recreate manually.

## Related docs

- [Data directory](data-directory.md) — `~/.armaraos/` layout  
- [AINL first (default language)](ainl-first-language.md)  
- [OOTB AINL](ootb-ainl.md)  
