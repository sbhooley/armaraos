# Planner + native infer — manual smoke

This path uses `POST /armara/v1/infer` on **`ainl-inference-server`** (or Wiremock in tests) when all of the following hold:

- **Planner mode** is on (`planner_mode` in agent manifest metadata, or env — see `resolve_planner_mode` in `openfang-runtime`).
- **`ARMARA_NATIVE_INFER_URL`** is set to the infer server base (no trailing path; the client calls `/armara/v1/infer`).
- The agent has **graph memory** available (seeded DB under the agent’s home).

## Environment variables

| Variable | Role |
|----------|------|
| `ARMARA_NATIVE_INFER_URL` | Base URL for native infer (e.g. `http://127.0.0.1:8787`). Required for the planner HTTP path. |
| `ARMARA_NATIVE_INFER_API_KEY` | Optional `Authorization: Bearer …` for the infer server. |
| `ARMARA_PLANNER_MODE` | Optional override to force planner eligibility (`1` / `true` / `on` — see `planner_mode.rs`). |
| `ARMARA_AGENT_SNAPSHOT_ENABLED` | **Infer server**: when building prompts/validation on the server, enables agent snapshot injection (see `armara-infer-core`). |
| `ARMARAOS_HOME` | **Client (ArmaraOS / tests)**: must match the kernel home used for **`~/.armaraos/agents/<id>/ainl_memory.db`**. If the test kernel uses a temp dir, set `ARMARAOS_HOME` to that same path before the turn; mismatch breaks graph memory + planner snapshot. |

`GET /metrics` on the ArmaraOS API includes planner counters prefixed with `openfang_planner_*` (native infer attempts, HTTP errors, plan validation, executor outcomes).

## Quick smoke (infer server running)

1. Start **`ainl-inference-server`** (see that crate’s README) on a known port.
2. Export `ARMARA_NATIVE_INFER_URL=http://127.0.0.1:<port>` (adjust host/port).
3. Ensure an agent has **`planner_mode = "on"`** in `[metadata]` and graph memory has at least one episode (normal use) or seed for testing.
4. Send a chat message; check **`#orchestration-traces`** for `plan_started` / `plan_step_*` events, or `GET /api/orchestration/traces?limit=20`.
5. Optional: `curl -s http://127.0.0.1:4200/metrics | grep openfang_planner` (API port from your daemon).

A minimal scripted flow is in **`scripts/planner-native-infer-smoke.sh`** (adjust URLs and bearer token if used).
