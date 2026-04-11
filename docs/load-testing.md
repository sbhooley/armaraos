# Load testing (manual / operator)

This repository includes an **opt-in** HTTP harness that drives a **running** ArmaraOS / OpenFang API daemon. It is **not** run in CI and **must not** be pointed at production without an explicit risk decision.

## Safety

1. **Hard gate:** the harness refuses to start unless `ARMARAOS_LOAD_TEST=1`.
2. **Costs:** `POST /api/agents/:id/message` and `POST /api/workflows/:id/run` invoke real model traffic (provider keys are whatever the **daemon** is configured with — the harness does not inject cloud API keys).
3. **Workflows:** `ARMARAOS_TEST_WORKFLOW_RUNS` defaults to **0**. Each non-zero run typically executes workflow steps (often **one LLM call per run**). Set to `100`–`300` only when you intend to stress workflow retention / eviction.
4. **Rate limits:** use `ARMARAOS_TEST_INTER_BATCH_MS` to stagger LLM workers (worker `i` sleeps `i * ms` before its first message). Lower concurrency if the provider throttles.
5. **Wall clock:** `ARMARAOS_TEST_MAX_WALL_SECS` (default `900`) aborts the whole run.

## Prerequisites

- Daemon listening (default `http://127.0.0.1:4200`).
- A **valid agent UUID** from `GET /api/agents` (agent should be `Running` and auth-ready).
- If `api_key` is set in `config.toml`, set `ARMARAOS_TEST_BEARER` to the same value for `Authorization: Bearer …`.

## AINL showcases after load test

The xtask harness does **not** execute embedded `programs/` graphs. After a load-test run (or any long soak), it is useful to confirm the daemon and `ainl` toolchain still behave end-to-end:

1. Materialize or sync `~/.armaraos/ainl-library/armaraos-programs/` (kernel boot or `POST /api/ainl/library/register-curated`).
2. From a shell with `ainl` on `PATH` and the same **HTTP allowlist** policy your scheduled jobs use, run `ainl validate … --strict` on the graphs listed in [ainl-showcases.md](ainl-showcases.md).
3. Optionally run `ainl run … --json` for **`channel_session_digest`** or **`budget_threshold_alert`** (loopback-only) while the API is still up — you should see structured JSON on stdout matching the READMEs.

This catches regressions in adapter registration, host allowlists, and cron capture separate from HTTP stress metrics.

## Runtime limits / unbounded mode

High `runtime_max_*` or `allow_unbounded_agent_loop` are **daemon-side** settings in `config.toml` / agent manifests plus `ARMARAOS_UNBOUNDED=1` where applicable (see `docs/architecture.md` § runtime limits). The harness only issues HTTP; it does not change limits.

## What it exercises

| Phase | Behavior |
|--------|-----------|
| Preflight | `GET /api/health`, `GET /api/agents` (validates target id) |
| Metrics | `GET /api/metrics` — prints lines containing `llm_*` before and after |
| Memory | Concurrent `PUT` + `GET` on `/api/memory/agents/:id/kv/:key` (shared structured namespace; URL agent id is ignored by the server — keys are unique per worker) |
| LLM | Concurrent `POST /api/agents/:id/message` with a short prompt |
| Workflow (optional) | `POST /api/workflows` once, then N × `POST /api/workflows/:id/run` (bounded in-flight) |
| Optional probe | If `ARMARAOS_LOAD_TEST_AGENT_SEND_CHAIN` is set (comma-separated UUIDs), the message body appends a **best-effort** note for manual `agent_send` experiments — not deterministic. |

## `[llm]` shared vs isolated

The harness prints `driver_isolation` from the daemon’s `config.toml` when readable (`ARMARAOS_HOME` / `OPENFANG_HOME` / `~/.armaraos/config.toml` or `ARMARAOS_TEST_CONFIG_PATH`). It does **not** expose LRU hit counts over HTTP; use `llm_requests_total`, `llm_errors_total`, latency sum/count, and `llm_in_flight` on `/api/metrics` while varying concurrency.

## Commands

Dry run (preflight + metrics excerpt only):

```bash
ARMARAOS_LOAD_TEST=1 \
ARMARAOS_TEST_AGENT_ID='<uuid-from-GET-/api/agents>' \
ARMARAOS_TEST_BEARER='<same-as-daemon-api_key-if-set>' \
cargo run -p xtask -- load-test --dry-run
```

Full stress (memory + LLM; workflows off by default):

```bash
ARMARAOS_LOAD_TEST=1 \
ARMARAOS_TEST_AGENT_ID='<uuid>' \
ARMARAOS_TEST_BEARER='<token-if-needed>' \
cargo run -p xtask -- load-test \
  --base-url 'http://127.0.0.1:4200' \
  --concurrency 24
```

Explicit workflow retention stress (⚠ bills N LLM calls):

```bash
ARMARAOS_LOAD_TEST=1 \
ARMARAOS_TEST_AGENT_ID='<uuid>' \
ARMARAOS_TEST_WORKFLOW_RUNS=200 \
cargo run -p xtask -- load-test
```

## Environment reference

| Variable | Meaning |
|----------|---------|
| `ARMARAOS_LOAD_TEST` | Must be `1` or the harness exits |
| `ARMARAOS_TEST_BASE_URL` | API root (default `http://127.0.0.1:4200`) |
| `ARMARAOS_TEST_AGENT_ID` | Target agent UUID |
| `ARMARAOS_TEST_BEARER` | Bearer token for `Authorization` when daemon `api_key` is set |
| `ARMARAOS_TEST_CONCURRENCY` | Parallel workers (default `24`, clamped `1`–`128`) |
| `ARMARAOS_TEST_KV_OPS` | PUT+GET iterations per KV worker (default `50`) |
| `ARMARAOS_TEST_MESSAGE_ROUNDS` | `POST /message` per LLM worker (default `1`) |
| `ARMARAOS_TEST_MESSAGE` | Message body override |
| `ARMARAOS_TEST_WORKFLOW_RUNS` | `POST .../run` count after one `POST /api/workflows` (default **`0`**) |
| `ARMARAOS_TEST_INTER_BATCH_MS` | Stagger LLM starts: worker `k` sleeps `k * ms` |
| `ARMARAOS_TEST_MAX_WALL_SECS` | Global timeout (default `900`) |
| `ARMARAOS_TEST_CONFIG_PATH` | Override path to `config.toml` for isolation hint |
| `ARMARAOS_LOAD_TEST_AGENT_SEND_CHAIN` | Optional comma-separated UUIDs (message appendix only) |

CLI flags (`--base-url`, `--agent-id`, etc.) override the corresponding env vars when passed.
