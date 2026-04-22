# AINL crates — publish matrix (ArmaraOS workspace)

Crates under `crates/ainl-*` that participate in the self-learning stack are intended to ship **without** `openfang-*` dependencies so they can be reused from `ainl-inference-server`, scheduled `ainl run`, and other hosts. This file is a **release checklist** / ordering guide, not a hard CI gate.

| Crate | crates.io | `openfang` in deps? | Notes |
|-------|------------|----------------------|--------|
| `ainl-contracts` | yes (workspace version) | **no** | Shared `CognitiveVitals`, `ContextFreshness`, `TrajectoryStep`, `ProposalEnvelope`, telemetry keys. |
| `ainl-memory` | yes | **no** | Graph store; depends on `ainl-contracts` for shared vocabulary where applicable. |
| `ainl-compression` | yes | **no** | Eco-mode compression + profiles / adaptive / cache. |
| `ainl-trajectory` | workspace / future publish | **no** | JSONL replay helpers; optional `in-memory` feature for slim hosts. |
| `ainl-failure-learning` | workspace / future publish | **no** | FTS + `should_emit_failure_suggestion` gate + prevention string helpers. |
| `ainl-improvement-proposals` | workspace / future publish | **no** | SQLite ledger; validation calls into `ainl-runtime` where used. |
| `ainl-context-compiler` | workspace / future publish | **no** | Segment + budget scoring; default features documented in `Cargo.toml` and `scripts/verify-ainl-context-compiler-feature-matrix.sh`. |
| `ainl-runtime` | yes | **no** | Graph execution; not an “openfang” crate. |

**Slim consumer build** (from [SELF_LEARNING_INTEGRATION_MAP.md](SELF_LEARNING_INTEGRATION_MAP.md) §15, `ainl-inference-server` docs):

```bash
cargo build -p ainl-trajectory --no-default-features --features in-memory,vitals,graph-export
```

Bump versions and `CHANGELOG` per crate when publishing; `ainl-contracts` semver is the main compatibility anchor for host embedders.
