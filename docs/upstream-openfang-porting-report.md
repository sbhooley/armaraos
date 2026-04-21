# Upstream OpenFang porting — future consideration

This note captures a snapshot analysis of **rightnow-ai/openfang** `main` versus **ArmaraOS** `main`, for deciding what to cherry-pick or manually port without losing fork-specific work (dashboard, graph memory, planner/MCP, branding, etc.).

**Generated for planning only** — re-run the commands in [How to reproduce](#how-to-reproduce) before acting; upstream moves continuously.

## Snapshot (see repro section for current SHAs)

| Item | Value |
|------|--------|
| **Upstream repo** | `https://github.com/rightnow-ai/openfang` |
| **Merge-base** (common ancestor with ArmaraOS `main`) | `a78299e` — *Merge PR #753: Argon2id for dashboard auth* |
| **Upstream tip at analysis** | `e6bab99` — *bump v0.6.0* |
| **Approx. divergence** | ArmaraOS `main` and upstream `main` each have **on the order of ~190 / ~97** unique commits after the merge-base (symmetric difference; exact numbers change as either branch moves). |

**Interpretation:** there are **~97 commits** on upstream that are not ancestors of ArmaraOS `main`. Blindly merging upstream `main` is unlikely to be low-conflict; prefer **ordered cherry-picks** or **per-hunk manual ports**, with tests after each batch.

## Merge commits

Many fixes appear twice: as a **normal commit** and again as a **merge commit** recording the same GitHub PR. When cherry-picking, prefer **non-merge** commits (or inspect `git show -m <merge>` / second parent) to avoid double-applying the same change.

## Tier A — Generally worth porting (review for conflicts)

High value for correctness, supply chain, or security; usually touch runtime/kernel/types more than the embedded dashboard.

| Focus | Example upstream commits / areas |
|--------|----------------------------------|
| **CVE / deps** | `528a7b9` — wasmtime / rumqttc (RUSTSEC-2026-0049, 0085–0096, etc.) |
| **Security-deps merge** | `983519c` — large; includes lockfile + `copilot` / sandbox / API touches — **high conflict risk**, high value |
| **API exposure** | `6f519b9` — reject unauthenticated requests from non-loopback by default — **verify** reverse-proxy / tunnel setups |
| **Crypto init** | `e1790b5` — explicit crypto provider |
| **Gemini / message shape** | `3c221dc`, `3b21494`; PR merges for function-call ordering / array items (`a603dc9`, `1ca86c4`) |
| **UTF-8 safety** | `4565988`, `34a27de`, merge `80359b4` |
| **Copilot / proxy pipeline** | `c1a14e8` and the assistant-message stripping series (e.g. through `be1c4db`); merge `f992178` may duplicate — **apply as an ordered series**, test Copilot/Gemini paths |
| **Runtime behavior** | `6ab07d1` — re-read agent `context.md` per turn; `ce89d05` — multimodal user message blocks |
| **Cron / scheduler** | `df22a3d` — preserve cron across hand reactivation; cron-loss PR (`8a21971`); `ff44cfb` — route `schedule_*` tools and `/api/schedules` through kernel cron — **compare to fork scheduler** before trusting a straight cherry-pick |
| **Cron delivery (backend-heavy)** | `3db5d3a` — adds `cron_delivery` kernel module + types — optional; mostly not the UI commit |
| **Large cross-cutting** | `9323edc` — WS, Feishu/Revolt, sandbox, kernel, routes — **very high conflict risk** with custom API; consider **per-file / per-hunk** port |

**lettre / prost / cron crate bumps** (`185ebc4`, `c47e15c`, `6f86f7a`, and related merges) — take with the CVE/security wave; resolve `Cargo.lock` conflicts in your workspace.

## Tier B — Optional / situational

Port only if you want parity with upstream product or release matrix.

| Topic | Notes |
|--------|--------|
| **New LLM providers** | AWS Bedrock (`2fe926c` + merge `2daaf92`), Novita (`365bec8`, `4714efb`, merge `3fa8c6c`) |
| **armv7 / Nix / CI / install** | `449a294`, `8b925d8`, `28d01ac`, `be14959`, `0796377` |
| **Channels** | Signal default formatter (`80af18a`), Discord free-response (`d94508e`), optional outbound prefix (`00c0ff6`) |
| **CLI** | Hand config subcommand (`e2b0a54`), `config get` base URL (`07af248`) |
| **Runtime niceties** | Intermediate tool text (`0408d65`), skip empty tool id/name (`abeaaf5`) |
| **Bundle** | `605ce74` — “6 bugs”; **do not** cherry-pick wholesale — inspect per hunk |

## Tier C — Usually skip, or port minimal hunks by hand

These tend to **fight ArmaraOS customizations** or add **upstream product UX** you may not want.

- **Dashboard shell / layout / theming / i18n** under `crates/openfang-api/static/` (e.g. sidebar refactor, CSS theme tokens, Russian i18n, comms/analytics/modal styling, logo/manifest tweaks). ArmaraOS already customizes navigation (Graph Memory, Orchestration, notifications, branding).
- **Version-only / release bookkeeping** commits (`bump v…`, `release: v…`) — keep ArmaraOS versioning.
- **Pure formatting** (`style: apply cargo fmt`) unless normalizing the whole tree.
- **Debug logging** — `73d50c0` adds noise; `3854e3e` removes it upstream (take behavior, not debug).
- **Clippy-suppression / CI-unblock** batches — only if your CI policy matches upstream.

**High-risk product surface (evaluate before porting whole commits):**

- **`a26f762`** / broad UX merge — multi-instance hands + many fixes; overlaps areas the fork likely evolved differently.
- **`5a1f372`** — large `openfang-types` command registry + `ws` / bridge / TUI — strong chance of **semantic conflict** with fork command/event story.
- **`a9f15b2` / `a39a675` / `88eeaa6`** — skill config + command UI — optional; may duplicate or clash with fork MCP/skills direction.
- **`0ce390e`** — cron delivery UI: **large** `routes.rs`, `index_body.html`, `scheduler.js` — expect **heavy conflicts**; port behavior selectively if needed.

## Suggested workflow (when someone picks this up)

1. **First wave:** CVE/supply-chain (`528a7b9`), crypto init (`e1790b5`), then **carefully** integrate **`983519c`** or equivalent dep/runtime diffs (resolve vs. fork `copilot` / sandbox).
2. **Second wave:** Gemini/UTF-8/Copilot series in dependency order; then **`6f519b9`** with documentation for proxied installs.
3. **Third wave:** kernel/cron items after diffing against current ArmaraOS cron/hand code (`df22a3d`, cron-loss fix, `ff44cfb`, optionally `3db5d3a` without UI).
4. **Dashboard:** avoid wholesale upstream UI merges; cherry-pick **minimal diffs** into ArmaraOS `static/` only where a bug still reproduces on your tree.

## How to reproduce

```bash
cd /path/to/armaraos
git fetch https://github.com/rightnow-ai/openfang.git main:refs/remotes/upstream-openfang/main

git merge-base main upstream-openfang/main
git rev-list --left-right --count main...upstream-openfang/main
git log --oneline --reverse $(git merge-base main upstream-openfang/main)..upstream-openfang/main
```

## References

- ArmaraOS positions itself as a fork/rebrand of OpenFang; upstream attribution remains where applicable (`README.md`).
- This document does not track every file path per commit; use `git show <sha> --stat` when planning a port.
