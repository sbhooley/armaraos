# Wishlist host kit (ArmaraOS)

This directory is a **host integration kit**: a **smoke graph**, **example frames**, and a **runner** for upstream wishlist demos. It is not a placeholder—the smoke graph really executes (core-only), and the shell script really runs the full `examples/wishlist/` graphs when `AINL_ROOT` points at your [ainativelang](https://github.com/sbhooley/ainativelang) checkout.

**Why “kit” not “stub”:** in this repo, “stub” used to mean “minimal template,” but that reads like fake data. This folder is **real wiring** (validate, run, frames, script)—just not the canonical graphs themselves (those stay in AI_Native_Lang).

## Contents

| File | Purpose |
|------|---------|
| `wishlist_host_smoke.ainl` | **Core-only** smoke graph: proves `in:` frames reach AINL (`session_key`, `note`, `wishlist_id`). |
| `frame.example.json` | Example JSON for the smoke graph. |
| `frames/01.json` … `frames/08.json` | Minimal JSON frames for each upstream wishlist graph. |
| `run_upstream_wishlist.sh` | Runs `python -m cli.main run` against `AINL_ROOT` with env + flags per graph (ids **01–08** or **05b** for unified `llm` + offline config). |

## What stays in AI_Native_Lang

- All `.ainl` graphs, adapters, `ainl run` / `ainl serve`, and `examples/wishlist/README.md`.

## What belongs in ArmaraOS

- **When** to run a graph (message, heartbeat, tool invocation).
- **Working directory** (usually the workspace or repo root so `examples/wishlist` paths resolve).
- **Environment**: `AINL_MEMORY_DB`, `AINL_CACHE_JSON`, `AINL_VECTOR_MEMORY_PATH`, `AINL_EXT_ALLOW_EXEC`, `AINL_LLM_QUERY_*`, HTTP egress policy.
- **Adapter policy**: which adapters are enabled for which agent profile (mirror `--enable-adapter` flags).
- **Secrets**: API keys live in host config, not in graphs.

## Validate the smoke graph

From the ArmaraOS repo root (with `ainl` on `PATH` or `python -m cli.main` from an installed AINL checkout):

```bash
ainl validate programs/wishlist-host-kit/wishlist_host_smoke.ainl --strict
```

## Run the smoke graph

```bash
ainl run programs/wishlist-host-kit/wishlist_host_smoke.ainl --json \
  --frame "$(cat programs/wishlist-host-kit/frame.example.json)"
```

Expected result: a string starting with `armaraos_wishlist_host_ok|01|…`.

## Run upstream wishlist graphs (01–08, or 05b)

```bash
export AINL_ROOT=/path/to/AI_Native_Lang
./programs/wishlist-host-kit/run_upstream_wishlist.sh 01
```

Use **`05b`** to exercise the unified **`llm`** adapter with **`examples/wishlist/fixtures/llm_offline.yaml`** (no `llm_query`, no mock env — see upstream **`docs/LLM_ADAPTER_USAGE.md`**).

The script sets default `AINL_CACHE_JSON`, `AINL_MEMORY_DB`, and `AINL_VECTOR_MEMORY_PATH` under `$TMPDIR` when unset, and adds adapters per graph (`ext`, `llm_query`, `http`, `code_context` as needed).

Override frames:

```bash
./programs/wishlist-host-kit/run_upstream_wishlist.sh 04 /path/to/custom_frame.json
```

## Wire into Rust / ArmaraOS

Point automation at the same command (or at `POST /run` on `ainl serve`) with a JSON **frame** produced by the kernel. See [docs/learning-frame-v1.md](../../docs/learning-frame-v1.md).

### Cron / scheduled automation

- **Shell:** schedule `run_upstream_wishlist.sh` with `AINL_ROOT` set (same as manual runs). Use **`05b`** for unified **`llm`** + config without **`llm_query`** mock env.
- **HTTP:** run **`ainl serve`**; **`POST /run`** expects **`{ "source": "<full .ainl source>", "frame": { ... }, "strict"?: bool }`** (same contract as the CLI — embed or read file in your wrapper). Match **`--frame-json`** shape to **`docs/learning-frame-v1.md`**.
- **Kernel:** the Rust daemon’s job is **policy + scheduling + frame assembly**; AINL stays the executor. Start with a thin wrapper that shells to **`ainl run`** or calls **`/run`** until you need in-process IR.

## Related

- [programs/README.md](../README.md) — AINL program layout in this repo.
- Upstream index: `examples/wishlist/README.md` in the ainativelang repository.
