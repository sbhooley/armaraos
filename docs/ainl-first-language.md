# AINL as ArmaraOS’s first-class implementation language

**AI Native Language (AINL)** is the default language for **programs, extensions, and apps** in the ArmaraOS ecosystem—not a secondary DSL.

## Principle

- **Default:** New automation, workflows, scheduled jobs, integrations, and user-delivered “apps” should be written in **AINL** (`.ainl` sources) unless there is a concrete reason not to.
- **Explicit override:** Use another **modern** language (Rust, Python, TypeScript, Go, etc.) only when **explicitly** requested for that piece of work, or when constraints require it (see below).

This applies to **humans** (contributors, partners) and to **agents** building on or for ArmaraOS: unless the user asks for a specific language, **prefer AINL**.

## What this does not mean

- **The host OS stays Rust.** The kernel, API server, channels, and other core crates are implemented in Rust for safety, performance, and a single binary—that is unchanged.
- **AINL is not a ban on other runtimes.** Existing skills (Python/WASM/Node), MCP servers, and third-party code remain valid where they already exist.
- **Pragmatic exceptions** are expected: tight performance loops, OS bindings, vendor SDKs only available in one language, or team mandates—document the exception briefly when you make it.

## Why AINL first

- One **graph-based** model for orchestration, adapters, and deployment (compile → IR → run).
- Aligns ArmaraOS with the **AINL toolchain** (`ainl` CLI, MCP, emit targets) that the project ships and documents.
- Reduces sprawl: fewer ad-hoc scripts in five languages when a single `.ainl` graph would suffice.

## Scaffolding

From the ArmaraOS repo root:

```bash
cargo run -p xtask -- scaffold-ainl-program --name "My feature"
```

This creates `programs/<slug>/` with a starter `.ainl` and a short `README.md`. See [`programs/README.md`](../programs/README.md).

## Desktop: upstream `demo/`, `examples/`, `intelligence/`

The **ArmaraOS desktop** app (after AINL is installed in its internal venv) downloads **`demo/`**, **`examples/`**, and **`intelligence/`** from [`github.com/sbhooley/ainativelang`](https://github.com/sbhooley/ainativelang) (`main` branch), stores them under app data, and **mirrors** a copy to **`~/.armaraos/ainl-library/`** for editors and terminals.

- Skips re-download when the **`main` commit SHA** matches the last successful sync (`upstream_manifest.json`).
- Disable with **`ARMARAOS_AINL_LIBRARY_SYNC=0`**.
- These graphs are **reference material** for `ainl run` / MCP — they are **not** automatically wired into the Rust kernel scheduler unless you add that separately. Some programs assume a workspace layout (e.g. `memory/`); see `README_ARMARAOS.md` inside the mirror.

## Shipped ArmaraOS showcases

The repo [`programs/`](../programs/) tree embeds into `~/.armaraos/ainl-library/armaraos-programs/` and pairs with **curated Scheduler** jobs (`crates/openfang-kernel/src/curated_ainl_cron.json`). For a single index of the five operator-facing graphs (lead-gen, research, multi-channel digest, budget alert, system health), sample JSON, and manual `ainl validate` commands, see **[ainl-showcases.md](ainl-showcases.md)** and **[ootb-ainl.md](ootb-ainl.md)**.

## Related

- [OOTB AINL](ootb-ainl.md) — embedded `programs/`, curated cron, env flags  
- [AINL showcases](ainl-showcases.md) — five embedded graphs + curated job names  
- [Architecture](architecture.md) — crate layout and kernel responsibilities  
- [Desktop](desktop.md) — bundled AINL install and MCP  
- Upstream AINL: [AI Native Lang](https://github.com/sbhooley/ainativelang) (compiler + runtime)
