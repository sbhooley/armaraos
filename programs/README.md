# AINL programs (ArmaraOS)

First-class [**AI Native Language (AINL)**](https://github.com/sbhooley/ainativelang) graphs and related assets for ArmaraOS-adjacent automation—**not** a substitute for the Rust kernel (`crates/*`).

## Layout

Program folder names sometimes end in `-stub` for **minimal templates** (still valid, runnable graphs)—not “fake” implementations. Prefer a concrete name when the bundle is a full integration kit (e.g. [wishlist-host-kit/](wishlist-host-kit/)).

Each program lives in its own directory:

```
programs/
  README.md           ← you are here
  <slug>/
    <slug>.ainl       ← source graph (compact syntax recommended)
    README.md         ← optional notes
```

## Scaffold a new program

From the repo root:

```bash
cargo run -p xtask -- scaffold-ainl-program --name "my feature"
```

Then validate with a local `ainl` install:

```bash
ainl validate programs/<slug>/<slug>.ainl --strict
```

Policy: [docs/ainl-first-language.md](../docs/ainl-first-language.md).

**Learning frame (v1):** host → graph JSON contract for skill / memory pipelines is documented in [docs/learning-frame-v1.md](../docs/learning-frame-v1.md). See [learning-frame-echo/](learning-frame-echo/) for frame wiring smoke test and [skill-mint-stub/](skill-mint-stub/) for a minimal skill body graph. Staging path: `~/.armaraos/skills/staging/` (see same doc for `POST /api/learning/skill-draft`).

**Desktop:** the app also syncs upstream `demo/`, `examples/`, and `intelligence/` from [sbhooley/ainativelang](https://github.com/sbhooley/ainativelang) into `~/.armaraos/ainl-library/` after AINL bootstrap — see the same policy doc.

**Runtime mirror:** the kernel embeds this `programs/` tree at build time and materializes it to `~/.armaraos/ainl-library/armaraos-programs/` on boot (see [docs/ootb-ainl.md](../docs/ootb-ainl.md)). Repo paths like `programs/learning-frame-echo/` appear on disk as `armaraos-programs/learning-frame-echo/` next to upstream `examples/`, etc. Includes `armaraos_health_ping/`, `armaraos_automation_stub/` (disabled curated template), and shared learning-frame samples.

**Orchestration wishlist (1–8):** the canonical AINL graphs live upstream in `examples/wishlist/` (cache, memory, vector_memory, fanout, ext, llm_query, http, code_context). This repo adds [wishlist-host-kit/](wishlist-host-kit/) — `wishlist_host_smoke.ainl` (core-only host wiring test), `frames/*.json`, and `run_upstream_wishlist.sh`. See [wishlist-host-kit/README.md](wishlist-host-kit/README.md).
