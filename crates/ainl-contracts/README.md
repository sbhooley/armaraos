# ainl-contracts

- **Repository:** <https://github.com/sbhooley/armaraos>
- **API reference:** <https://docs.rs/ainl-contracts>

Serde-stable types shared by **`ainl-repo-intel`**, **`ainl-context-freshness`**, **`ainl-impact-policy`**, AINL MCP (`tooling/ainl_policy_contract.json` in the AI_Native_Lang repo), and optional ArmaraOS / inference telemetry.

No `openfang_*` dependencies.

See workspace `docs/POLICY_BOUNDARIES.md` (ArmaraOS repo) and `docs/SELF_LEARNING_INTEGRATION_MAP.md` (Phase 0, §1, and §15).

## Schema / version numbers

- **`CONTRACT_SCHEMA_VERSION`** — version for **policy** payloads (MCP / repo-intel / impact enums). Bump when you make a **breaking** serde change to a type that is **not** covered by `LEARNER_SCHEMA_VERSION` below; update `ainl_policy_contract.json` and any Python/Rust co-consumers in lockstep.
- **`LEARNER_SCHEMA_VERSION`** — version for the **self-learning** wire set: [`ProposalEnvelope`](./src/learner.rs) and any future learner-blocked structs that are explicitly versioned in-field (`schema_version` on the envelope) or via this constant when the crate does not yet embed a field.

`cargo test` in this crate includes JSON round-trips; treat failing tests as a **breaking wire change** and bump the right version + map note in `SELF_LEARNING_INTEGRATION_MAP.md` §15.

## Learner vocabulary (shared across hosts)

Re-exported from the crate root and defined in `src/learner.rs` / `src/vitals.rs`:

- **`CognitiveVitals`**, `CognitivePhase`, `VitalsGate` — cognitive budget / gating; see also `openfang-types::vitals` (re-export shim).
- **`TrajectoryStep`**, **`TrajectoryOutcome`**, **`FailureKind`**, **`ProposalEnvelope`** — used by `ainl-trajectory`, `ainl-failure-learning`, `ainl-memory` payloads, and hosts that assemble prompts or graph rows.

`openfang-types::vitals` re-exports the contract vitals types for legacy call sites; **prefer** `ainl-contracts` in new `ainl-*` crates.

## Telemetry and context-compiler string keys

- **`telemetry`** — stable field/label names (`TRAJECTORY_RECORDED`, `PROPOSAL_ADOPTED`, `context_compiler_compose`, …) so different hosts (Rust daemon, Python MCP, inference server) can emit the same keys.
- **`context_compiler`** — small **string vocabularies** for segment kinds and tiers; lets dashboards read `compose` telemetry without depending on the `ainl-context-compiler` binary crate.

When you add a new public constant, document whether it is **field-rate**, **counter**, or **histogram**-oriented in the `SELF_LEARNING` map or `POLICY_BOUNDARIES` as appropriate.
