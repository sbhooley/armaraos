# `ainl-kernel` — vision and architecture

**Status:** north-star design document. **Phase 1b landed** (façade traits + hermetic conformance harness against `OpenFangKernelAdapter`). **Phase 2.5 schema landed** (typed graph handles in `crate::graph` — `AgentNodeRef`, `WardNodeRef`, `McpServerNodeRef`, `TesseraId`, `PolicyEvalId` — and the rate-limit contracts retrofitted to use them; wire format unchanged). **Phase 2 step 1 landed for the wedge slot** (MCP-reconnect contracts: `McpReconnectFrame`, `McpReconnectDecision`, `McpReconnectOutcome`, `McpServerStatus`, `ReconnectCaps`, `PolicySlot::McpReconnect`). **Phase 2 step 2 landed for the wedge slot** (`mcp_reconnect_default_decision` — bit-for-bit Rust parity with today's `HealthMonitor::backoff_duration` + `should_reconnect`, pinned by property tests). **Phase 2 step 3a landed for the wedge slot** (kernel-side policy seam: `McpReconnectPolicy` trait, `DefaultMcpReconnectPolicy` parity wrapper, `FallbackMcpReconnectPolicy<P>` kill-switch wrapper, `McpReconnectPolicyError`, `SharedMcpReconnectPolicy = Arc<dyn ...>`). **Not yet landed:** Phase 2 step 3b (call-site rewire in `openfang-kernel`'s `auto_reconnect_loop`) and steps 4–6 (no AINL evaluator on the path, no conformance/parity/latency tests at the call-site level, no observability projections, no empirical writeup). The kernel itself is not yet AI-native; this document describes the architecture we are building toward and the discipline that keeps it from drifting into hype.

> Hub docs to read alongside this one: **[architecture.md](architecture.md)** (current ArmaraOS architecture), **[ainl-runtime.md](ainl-runtime.md)** (embedded AINL runtime), **[ainl-runtime-graph-patch.md](ainl-runtime-graph-patch.md)** (current GraphPatch integration), **[ainl-first-language.md](ainl-first-language.md)** (AINL-first policy), **[POLICY_BOUNDARIES.md](POLICY_BOUNDARIES.md)** (layering rules), **[SELF_LEARNING_INTEGRATION_MAP.md](SELF_LEARNING_INTEGRATION_MAP.md)** (where each learning capability lives).

---

## Table of contents

- [Goal in one sentence](#goal-in-one-sentence)
- [Scope: what this kernel is and is not](#scope-what-this-kernel-is-and-is-not)
- [Why this is structurally first-of-kind (five properties)](#why-this-is-structurally-first-of-kind-five-properties)
- [Kernel vocabulary: Tessera and Ward](#kernel-vocabulary-tessera-and-ward)
- [The four-layer architecture](#the-four-layer-architecture)
- [Substrate primitives (live in the kernel)](#substrate-primitives-live-in-the-kernel)
- [Kernel state IS the graph](#kernel-state-is-the-graph)
- [Policy primitives (AINL programs evaluated by the kernel)](#policy-primitives-ainl-programs-evaluated-by-the-kernel)
- [The kernel-inclusion litmus test](#the-kernel-inclusion-litmus-test)
- [Deliberately not in the kernel](#deliberately-not-in-the-kernel)
- [Non-negotiable mitigations](#non-negotiable-mitigations)
- [The GraphPatch lift: from runtime-scoped to kernel-scoped](#the-graphpatch-lift-from-runtime-scoped-to-kernel-scoped)
- [Phase roadmap](#phase-roadmap)
- [Marketing discipline](#marketing-discipline)
- [Open questions](#open-questions)
- [Glossary](#glossary)

---

## Goal in one sentence

`ainl-kernel` is the smallest possible verified Rust substrate that makes the **kernel's own decision logic** expressible as **AINL policy programs**, evolvable at runtime under the same `GraphPatch` discipline that already governs procedural memory in `ainl-runtime`, with every modification dataflow-verified, lineage-tracked, fitness-monitored, and reversible.

It is **not** a new operating system, **not** a self-aware kernel, **not** a kernel that "thinks." It is a kernel whose policies are programs in a typed deterministic IR, where the *programs* can learn under verification while the *kernel itself* stays fixed.

---

## Scope: what this kernel is and is not

This is the load-bearing scope clarification that turns several large risks into manageable ones. Future contributors should re-read this section before any architectural debate, because most "is this realistic?" arguments dissolve once the scope is held in mind.

### What `ainl-kernel` is

- **The kernel of an *agent operating system* that ships as software** (binary, container, or installer) on top of Windows, macOS, or Linux — locally on a user's machine, or in the cloud as a per-tenant deployment.
- **A peer of products like Cursor, Claude Desktop, ChatGPT Desktop, or n8n self-hosted** in distribution model: install it, run it, update it through normal software channels.
- **Optionally bootable from hardware in the long run**, but this is a "maybe later" niche, not the design center. Nothing in the architecture forecloses it; nothing in the architecture depends on it.

### What `ainl-kernel` is NOT

- **Not a hardware-bootable general-purpose OS kernel.** We are not in the business of competing with Linux, Windows, or macOS for hardware boot seats. We are not asking anyone to give up their existing OS to install us. Every prior failed attempt at "rewrite the substrate" (Plan 9, Singularity/Midori, Lisp Machines, Smalltalk image, semantic-web-as-OS) died because it tried to displace an incumbent OS. That fight is not our fight.
- **Not a microkernel, hypervisor, or container runtime.** Process isolation, memory protection, file systems, and device drivers are the host OS's job. We use what's already there.
- **Not a replacement for `ainl-runtime` or `ainl-memory`.** The runtime is embedded inside us; per-agent memory shards continue to live where they live today.
- **Not a competitor to general-purpose graph databases.** The kernel-scoped graph is *the kernel's own state*, not a product offered to applications.

### Why this scope changes the risk picture

Several failure modes that would be existential for a hardware-boot OS shrink to operational concerns at this scope. The full mapping lives in [Non-negotiable mitigations](#non-negotiable-mitigations); the headline:

| Risk that killed prior substrate rewrites | Applies to us? | Why |
|---|---|---|
| Compatibility gravity (Win32, POSIX, ABI) | No | We don't replace the host OS; we ship alongside it. |
| Driver / device support | No | The host OS provides drivers; we never see hardware. |
| Hardware boot story | No | We boot like an app. `systemd`, `launchd`, or the user double-clicks an icon. |
| Performance ceiling on microsecond hot paths | Reduced | Local single-user kernel sees ~50–200 RPS peak. Most decisions are LLM calls (200–2000 ms), so AINL eval overhead at the millisecond scale is rounding error on the user-visible latency. Cloud multi-tenant deployments are still nowhere near hardware-OS load. |
| Verifier escape catastrophic blast radius | Reduced | An escape on a user's machine is the same threat model as Cursor or Claude Desktop running a malicious extension or MCP server: bad, recoverable, scoped to one user/tenant — not "global Linux kernel takeover." |
| Ecosystem cold start (no logging/monitoring tools speak our dialect) | Real but solvable | Hard requirement: standard observability protocols (Prometheus, OTLP, structured JSON logs) emit projections of the graph from day one. The graph is the canonical record; observability protocols are projections. Captured as a binding commitment in [Non-negotiable mitigations](#non-negotiable-mitigations). |
| Operator skill drag (graph debugging is unfamiliar) | Real but solvable | Most users are end-users behind a dashboard. Operators get the standard debug surfaces *plus* the optional graph-lineage power feature. Graph debugging is a power-user benefit, never the only debug story. |
| Hype contamination ("AI-native everything" vapor) | Real | Mitigated by [marketing discipline](#marketing-discipline) — proof-not-promise framing, lead with concrete capabilities, never lead with the substrate claim. |

### Capability preservation (this kernel must not subtract)

The graph-canonical kernel is committed to preserving every capability ArmaraOS ships today, with several getting strictly better through structural lineage. Anything in this list that is broken or measurably slowed by an architectural decision is a regression that blocks the decision:

- Daemon stays fast and light (~40 MB idle, sub-second cold start).
- Chat with agents, sub-agent delegation, multi-agent swarms.
- Skill / app store install + uninstall + reload.
- Web research, code generation, full-stack app scaffolding via AINL.
- MCP tool discovery, invocation, reconnection.
- CLI, A2A messaging, typed connectors.
- Spreadsheet / document analysis, Python / TypeScript / shell process execution.
- Self-learning (GraphPatch in `ainl-runtime`, persona evolution in `ainl-persona`).
- Embedded clawhub skills.
- Prompt compression and eco-mode token savings.
- Cron, triggers, scheduled jobs, channel inbox handling.

This list is the regression baseline. Phase-gate exit criteria check it explicitly.

---

## Why this is structurally first-of-kind (five properties)

These five properties together do not exist in any shipped system today. Linux + eBPF has #1–#3 partially. LangGraph has graph shape but no kernel and no patching. Letta has memory but no IR and no verification. AutoGPT-style self-improvement has none of the verification, provenance, or fitness machinery. The combination is the novel claim.

### 1. AINL-native

The kernel's own decision logic — scheduling, dispatch, capability enforcement, retries, escalations, trigger evaluation — is expressed as **AINL programs** evaluated by an embedded `ainl-runtime`, not hardcoded in Rust. AINL has a privileged role in `ainl-kernel` analogous to C in Linux or shell in Unix: it is the language the system uses to talk to itself.

> The `ainl-runtime` Rust crate exists today (`crates/ainl-runtime/`) with `run_turn` / `run_turn_async`, `MemoryContext`, `PatchAdapter`, `AdapterRegistry`. Embedding it inside the kernel boundary at *decision points* is the work.

### 2. Graph-native

Every kernel object — agents, capabilities, workflows, traces, policies, patches, sessions, hands, MCP connections — is a typed node in one causal graph. Edges are first-class. There is no parallel "events / logs / metrics / traces" pipeline; they all reduce to graph projections of the same substrate.

> `ainl-memory` already provides the typed graph store (`AinlMemoryNode`, `AinlNodeType::{Episode,Semantic,Procedural,Persona,RuntimeState,...}`). The work is making it the kernel's *primary* state representation rather than a per-agent SQLite shard.

### 3. Memory-native

Kernel state is a graph memory. Every kernel operation auto-emits a typed event node (cause) and edges (consequence) into that memory. The trajectory is a queryable kernel primitive, not a bolt-on telemetry pipeline. AINL policies executing inside the kernel can read trajectory through standard graph queries (`GraphQuery::active_patches` etc.) without crossing a process boundary.

### 4. Self-evolving under verification

Kernel policies (AINL programs at decision points) can be modified at runtime via `GraphPatch`. Every patch goes through the same discipline that already protects `ainl-runtime`:

- **Compile-time:** strict literal checks (AINL `--strict` mode).
- **Pre-install:** dataflow validation — every `read` is checked against frame + prior writes (see Python `_runtime_validate_patch_dataflow` in `runtime/engine.py`, Rust `GraphPatchAdapter::execute_patch` declared-reads check in `crates/ainl-runtime/src/adapters/graph_patch.rs`).
- **Install boundary:** `OverwriteGuardError` blocks patches over compiled (non-`__patched__`) labels. Kernel-owned policies have an analogous "compiled" tier that GraphPatch cannot replace.
- **Provenance:** every patch carries a `PatchRecord` (`node_id`, `label_name`, `source_pattern_node_id`, `source_episode_ids`, `parent_patch_id`, `patch_version`, `patched_at`).
- **Quality:** per-label `__fitness__` EMA; bad patches measurably degrade and auto-retire.
- **Lifecycle:** `retire_patch(reason)`, `parent_patch_id` lineage chain, `_reinstall_patches` on boot for crash recovery.
- **Effect typing:** AINL's `pure | io` effect system bounds what a patched policy can do.

The kernel itself — Rust crates, cryptographic roots, capability checks, IO drivers — is **fixed and never patched**. Only AINL policy programs are evolvable.

### 5. Persona-aware

Behavior boundaries (truthfulness, scope, escalation triggers, value constraints) are kernel-enforced invariants attached to agents as first-class graph nodes (`AinlNodeType::Persona`), not prompt-engineering tricks. Persona evolution flows through the same `GraphPatch` discipline as procedural memory.

> `ainl-persona` exists today and the runtime already supports `run_persona_evolution_pass`. The work is enforcing persona constraints at the *kernel* boundary rather than the agent loop.

---

## Kernel vocabulary: Tessera and Ward

`ainl-kernel` introduces exactly two kernel-distinct primitive names beyond what the workspace already uses: **Tessera** and **Ward**. Both name architectural slots that the kernel needs and that no existing primitive covers. Adding novel vocabulary to a kernel has real cost (newcomer onboarding, doc surface, search-engine prior); two well-defined primitives is in the safe zone, three or more would not be. This section is the canonical definition for both.

> Discipline: the names are *generative* (they gave the design its shape), not *load-bearing* (no code review should ever argue from "but in actual…"). Definitions below are technical and stand on their own.

### Tessera

A **Tessera** is a kernel-scoped, typed knowledge fragment with provenance. It is the unit of cross-agent shared knowledge in the kernel. Tesserae are individually small and contextual; useful patterns emerge from arrangement by AINL policies.

A Tessera is distinct from a per-agent `AinlMemoryNode` (Episode / Semantic / Procedural / Persona) in three ways:

1. **Ownership:** kernel-owned, not agent-owned. Survives agent death, agent rename, and agent uninstall.
2. **Visibility:** readable across agent boundaries (subject to ward visibility rules), not gated to the originating agent.
3. **Composition:** designed to be assembled. Every Tessera carries enough provenance (source agents, episodes, confidence, effect class) for an AINL policy to weigh and combine without re-deriving context.

Conceptual shape (final types defined in Phase 3a):

```rust
pub struct Tessera {
    pub id: TesseraId,
    pub kind: TesseraKind,                  // Observation | Inference | Pattern | Constraint
    pub payload: serde_json::Value,         // typed by `kind`
    pub provenance: Vec<TesseraSource>,     // contributing agents + episodes + wards
    pub confidence: f32,                    // 0.0–1.0 with calibrated meaning
    pub effect: AinlEffect,                 // pure | io
    pub created_at: u64,
    pub patched_by: Option<PatchId>,        // if produced by a self-learning policy
}
```

Plural: **tesserae**. Used consistently throughout this doc and in code/tests.

### Ward

A **Ward** is a bounded membership group with shared invariants, shared resources, named oversight, and scoped mutual observability. It is the kernel's grouping primitive — analogous in family to Linux cgroups and namespaces, with the persona/oversight/observability semantics that an agent kernel additionally needs.

A Ward has five properties:

1. **Membership** — bounded set of agents/sessions/capabilities. An entity may belong to one or many wards. Membership is operator-assignable and persona/origin-derivable.
2. **Invariants** — shared constraints every member must satisfy continuously (e.g. `no-pii-egress`, `persona-truthfulness`, `capability-ceiling`). Invariants are typed AINL predicates evaluated by the kernel; they are operator-installed and **cannot be patched by self-learning policies** (see [Phase 4 safety rules](#phase-4--self-learning-under-verification)).
3. **Resources** — shared budgets the ward collectively draws from (token budget, model-cost budget, IO ops/sec, decision-point eval budget).
4. **Oversight** — named accountability: who can install policies for this ward, who reviews proposed patches, who gets notified on violations, what the escalation path is.
5. **Mutual observability** — within-ward, members are legible to each other's AINL policies for pattern-detection (failure clusters, persona drift, contention). Cross-ward observability requires explicit capability and is a tracked tessera lineage.

Conceptual shape (final types defined in Phase 3b):

```rust
pub struct Ward {
    pub id: WardId,
    pub name: String,                        // operator-assigned, human-meaningful
    pub members: WardMembership,             // explicit | persona-derived | origin-derived
    pub invariants: Vec<WardInvariant>,      // typed AINL predicates, kernel-enforced
    pub resources: WardBudget,               // token / cost / IO / eval budgets
    pub oversight: WardOversight,            // installer, reviewer, notify channel, escalation
    pub visibility: WardVisibility,          // who can read this ward's tesserae
}
```

### What this displaces (and doesn't)

| Existing concept | Relationship to Tessera / Ward |
|---|---|
| `Episode` / `Semantic` / `Procedural` / `Persona` nodes (`ainl-memory`) | Per-agent. **Unchanged.** Tesserae are the *kernel-scoped* sibling, not a replacement. |
| `Persona` (`ainl-persona`) | An agent's identity contract. **Unchanged.** A persona may make an agent eligible for membership in particular wards, but the two concepts are orthogonal. |
| Capability checks (`crates/openfang-kernel/src/auth.rs`) | Per-grant decisions. **Unchanged**, but composable: a ward invariant may bound which capabilities its members can ever be granted. |
| Rate limiter (`crates/openfang-api/src/rate_limiter.rs`) | Today: per-IP / per-API-key. Future: also per-ward (resources budget). |
| The vague "tenant" notion that any multi-agent kernel grows into | **Replaced.** "Ward" carries the right semantic load (membership + invariants + resources + oversight + observability); we don't need to invent "tenant" separately. |
| AINL `Policy` (the active rule at a decision point) | Distinct. A policy is what runs and returns an outcome. A ward invariant is the constraint the outcome must respect. Both are AINL programs; they differ in role. |

Where these terms appear in the rest of this doc, they refer strictly to the definitions above.

---

## The four-layer architecture

```
┌─────────────────────────────────────────────────────────────────┐
│  APPLICATION LAYER  (above the kernel)                          │
│  • Smart Suggestions, Eco Mode, Adaptive Compression heuristics │
│  • AINL auto-detect, cost-optimization router                   │
│  • Dashboard learning, project-specific tuning                  │
│  • CLI / TUI / Desktop UX                                       │
│  ─ Reads kernel trajectory graph; never extends it.             │
└─────────────────────────────────────────────────────────────────┘
                              ▲
                              │ ainl-kernel public façade (KernelApi)
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│  AINL POLICY LAYER  (in the kernel, GraphPatch-evolvable)       │
│  • Pattern promotion criteria                                   │
│  • Failure-learning rules                                       │
│  • Persona evolution programs                                   │
│  • Scheduling / dispatch / capability heuristics                │
│  • Retry / escalation / rate-limit policies                     │
│  • Trigger evaluation predicates                                │
│  ─ All hot-patchable, dataflow-verified, fitness-tracked.       │
└─────────────────────────────────────────────────────────────────┘
                              ▲
                              │ embedded ainl-runtime, kernel-scoped PatchRegistry
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│  KERNEL SUBSTRATE  (Rust, fixed, never patched)                 │
│  • Trajectory: typed graph events, causal edges                 │
│  • Pattern memory: ainl-memory at kernel scope                  │
│  • PatchRegistry (kernel-scoped, GraphPatch discipline)         │
│  • Closed-loop validation pipeline                              │
│  • Embedded ainl-runtime for policy evaluation                  │
│  • Boring imperative core: actor supervisor, IO, crypto root    │
│  • Capability ledger, manifest verification                     │
└─────────────────────────────────────────────────────────────────┘
                              ▲
                              │ OS syscalls, drivers
                              ▼
                          (host OS)
```

The kernel substrate is small and fixed. The policy layer is rich and evolves under verification. The application layer is unconstrained and never reaches into the kernel for UX-shaped reasons.

---

## Substrate primitives (live in the kernel)

| Primitive | Status today | Vision in `ainl-kernel` |
|---|---|---|
| **Trajectory** as typed graph | Bolt-on (`OrchestrationTrace`, `audit_log`, `event_bus`) | First-class kernel state; every kernel op auto-emits typed nodes + causal edges into a single graph |
| **Pattern memory** (`ainl-memory`) | Per-agent SQLite (`~/.armaraos/agents/<id>/ainl_memory.db`) | Per-agent shards unchanged; kernel-scoped graph store added for tesserae |
| **Tesserae** | None today | Kernel-scoped, typed, cross-agent knowledge fragments with provenance. Defined in [Kernel vocabulary](#kernel-vocabulary-tessera-and-ward); landed in Phase 3a |
| **Wards** | None today (capability checks + persona enforcement live separately) | Bounded membership groups with invariants + resources + oversight + scoped observability. Defined in [Kernel vocabulary](#kernel-vocabulary-tessera-and-ward); landed in Phase 3b |
| **PatchRegistry** | Scoped to `ainl-runtime` procedural patches (label → adapter dispatch) | Lifted to kernel scope: kernel decision points and ward policy slots are patchable; ward *invariants* are not |
| **AINL evaluator** | `ainl-runtime` lives in agent loop | Embedded inside kernel boundary; evaluates policies at decision points and ward invariants on every relevant op |
| **Closed-loop validation** | `_runtime_validate_patch_dataflow` (Python), `declared_reads` check (Rust adapter) | Same discipline applied to kernel-policy patches; ward invariant validation added pre-adoption |
| **Capability ledger** | `crates/openfang-kernel/src/auth.rs`, manifest signing | Capability grants emit lineage edges into trajectory; AINL policies evaluate grant decisions; ward invariants may bound the granted set |
| **Effect typing** | AINL spec ships it (`pure \| io`) | Used by kernel to reject policy patches whose effects exceed declared bounds |

**What is *not* substrate** (and therefore lives above the kernel): UX strings, suggestion ranking, project-specific tuning, model-routing heuristics. See [Deliberately not in the kernel](#deliberately-not-in-the-kernel).

---

## Kernel state IS the graph

This is the architectural commitment that makes "graph-native kernel" load-bearing rather than decorative. Without it, "graph-native" describes the storage tier of a few subsystems and contradicts itself everywhere else. With it, the whole substrate has a single coherent state model.

It is the kernel-level instantiation of the [Unified Graph Execution Engine](https://www.linkedin.com/pulse/i-built-ai-agent-os-free-heres-my-unique-approach-steven-hooley-guyyc/) thesis: *the graph is a universal representational substrate for everything an AI agent is and does* — applied not just to memory, persona, tools, and adapters within an agent, but to the kernel that hosts the agents.

### The rule

> **The kernel-scoped graph is the canonical record for every kernel entity that has identity, causality, or relationships. Hot-path operational state — counters, indices, in-flight buffers, supervisor handles — is a derived projection or transient working memory above the graph, never a parallel source of truth.**

Three corollaries follow:

1. **Anything with identity, causality, or relationships is a node or edge.** Agents (identity), Sessions (causality chain), Triggers (relationship: source → predicate → action), MCP tool invocations (relationship: server exposes tool, tool invoked by agent), audit events (causality DAG), tesserae, wards, patches, policy evaluations.
2. **Hot-path data structures (HashMap, DashMap, in-memory caches, GCRA buckets) are projections.** They are populated from the canonical graph at boot, kept consistent on writes, and rebuildable from the graph at any point. They are not where state is born; they are where state is *operated on quickly*.
3. **Transient working state is OK to keep outside the graph.** A buffer holding 4 KB of an in-flight HTTP body has no identity, no causality, no relationships. It is mechanical execution, not substrate.

### Why this is the right line, not "everything is a node"

Plan 9, Smalltalk, and Lisp Machines all said "everything is X" and lost. The mistake was not the unification; it was forcing the unification *below* the line where it stopped paying off. The kernel-state-IS-the-graph rule draws the line at **identity, causality, and relationships** — exactly the properties for which a graph repays its overhead. Below that line, the graph is overhead without payoff. Above it, the graph buys you lineage, audit, replay, export, AINL-policy-reasoning, and patch motivation lineage *for free* from a single store.

### What lives in the canonical graph

These are the kernel-committed node and edge types. Each is *typed* (the kernel rejects untyped writes) and each has a stable serde schema (the boundary with AINL policies and external observability projections).

**Node types:**

| Node | Identity | Lineage signal it carries |
|---|---|---|
| `Agent` | `AgentId` | spawn time, parent agent (if delegated), persona ref, ward ref |
| `Session` | `SessionId` | per-agent conversation chain, compaction history |
| `Turn` | `TurnId` | inputs, outputs, tools called, cost, latency |
| `Trigger` | `TriggerId` | source event, predicate, action, last-fired time |
| `CronJob` | `CronJobId` | schedule, last run, last outcome |
| `McpServer` | `McpServerId` | URL, status, last reconnect |
| `McpTool` | `(McpServerId, ToolName)` | schema, last invocation count |
| `ToolCallSpec` | `ToolCallSpecId` | declared intent, input schema, expected output, retry policy, error handling — bullet 3 of the LinkedIn vision |
| `SkillSpec` | `SkillSpecId` | declared skill from clawhub or app store, capabilities required |
| `AuditEvent` | `AuditEventId` | who, what, when, ward attribution |
| `Tessera` | `TesseraId` | kernel-scoped knowledge fragment with provenance (Phase 3a) |
| `Ward` | `WardId` | bounded membership group (Phase 3b) |
| `WardMember` | `(WardId, AgentId)` | membership join with effective time |
| `WardInvariant` | `WardInvariantId` | typed AINL predicate (Phase 3b) |
| `PolicySlot` | `PolicySlot` enum variant | the slot itself as a node so policies and patches edge into it |
| `PolicyEval` | `PolicyEvalId` | one evaluation: frame snapshot, policy version, outcome, applied-at |
| `PatchRecord` | `PatchRecordId` | proposed/adopted/retired patch with full provenance (Phase 3c) |
| `Frame` | `FrameId` | input frame to a `PolicyEval`; one node per evaluation |
| `Outcome` | `OutcomeId` | output of a `PolicyEval`; one node per evaluation |

**Edge types (representative — exact predicate vocabulary nailed down in Phase 3a):**

| Edge | From | To | Meaning |
|---|---|---|---|
| `spawned_by` | `Agent` | `Agent` | parent/child agent relationship |
| `member_of` | `Agent` | `Ward` | ward membership |
| `governed_by` | `Agent` or `Ward` | `WardInvariant` | invariants that bind this entity |
| `exposes` | `McpServer` | `McpTool` | server publishes tool |
| `invoked_by` | `McpTool` | `Agent` (via `Turn`) | tool usage trace |
| `routes_to` | `ToolCallSpec` | `AdapterEdge` | bullet 4: adapter/bridge edges |
| `emits_to` | `Outcome` | `EmitTarget` | bullet 5: structural output routing |
| `causes` | `AuditEvent` | `AuditEvent` | causal DAG |
| `evaluated_by` | `Frame` | `PolicySlot` | which slot was hit |
| `produced` | `PolicyEval` | `Outcome` | this eval produced this outcome |
| `motivated_by` | `PatchRecord` or `PolicyEval` | `Tessera` (one or more) | the tesserae that motivated the decision/patch |
| `parent_patch_of` | `PatchRecord` | `PatchRecord` | patch lineage chain |
| `retired` | `PatchRecord` | `PatchRecord` (replacement) | retirement edge |

### What does NOT live in the canonical graph

These are explicitly carved out. Pulling any of them into the graph is overhead without payoff and should be rejected at review:

- **Hot-path mechanical state.** GCRA bucket counters, `Arc<Mutex<...>>` lock owners, file descriptor numbers, socket buffers, ed25519 verifier internals, JSON parser state. These are derived projections (counter) or pure mechanism (parser).
- **Transient request buffers.** In-flight HTTP request bodies, partial WebSocket frames, streaming SSE chunks before commit. They have no identity until they are committed; once committed, the *event of committing* is a graph node.
- **OS plumbing.** Process IDs of supervised processes (until they crash, at which point the *crash event* is a node), thread pool sizes, log file handles.
- **Pure caches.** Memoization tables, prepared-statement caches, schema caches. Rebuildable from the graph; not the canonical record.
- **Per-eval scratch space.** AINL runtime working memory during a single policy evaluation; only the evaluation's frame and outcome become graph nodes.

### Implementation discipline

These are commitments, not aspirations. Each is a code-review test for any new kernel feature:

1. **Single canonical store.** The kernel-scoped graph (Phase 3a) is one store, not a federation. Per-agent `~/.armaraos/agents/<id>/ainl_memory.db` shards continue to exist for per-agent memory; the kernel-scoped graph is its sibling, not its replacement, and they connect via typed cross-edges (e.g. `Agent` node in the kernel graph carries an edge `has_memory_shard` to the per-agent store's URI).
2. **Projections must be rebuildable.** Every hot-path data structure ships a "rebuild from graph" function exercised by tests. If it cannot be rebuilt, it is not a projection — it is a parallel source of truth, which violates the rule.
3. **Writes go to the graph first, then to projections.** No "write to HashMap, then maybe write to graph later" patterns. The graph is authoritative; projections are derived.
4. **Schemas are additive.** Adding a node type or edge type is a forward-compatible change; removing one requires a migration in `openfang-migrate` and an `ainl-kernel` minor-version bump.
5. **Boundary types are typed.** AINL policies see node IDs as typed handles (`AgentNodeRef`, `WardNodeRef`, `TesseraId`), not as bare strings, so a frame referencing a non-existent node is rejected at the boundary, not at execution.
6. **Hot path is fast.** Adding the graph as canonical state must not regress measured user-visible latency on the [capability-preservation list](#scope-what-this-kernel-is-and-is-not). Per-phase exit criteria measure this.

### What this means for the contracts already shipped

The Phase 2 step 1 contracts (`RateLimitFrame`, `RateLimitOutcome`, `PolicySlot`) are graph-*adjacent* — they reference graph entities by string ID without typed handles. A small Phase 2.5 retrofit will land before any further policy work:

1. Replace `caller.ward_id: Option<String>` with `caller.ward_ref: Option<WardNodeRef>`.
2. Add `motivated_by: Vec<TesseraId>` to `RateLimitOutcome` (empty until Phase 3a, structurally present from now).
3. Model every policy evaluation as a `PolicyEval` node creation: `(Frame) ─[evaluated_by]→ (PolicySlot) ─[produced]→ (Outcome) ─[motivated_by]→ (Tessera*)`. The eval *itself* becomes first-class graph state, queryable by lineage. This is the literal kernel-level instantiation of "memory IS execution."

The retrofit is small (boundary type changes + node-emission on apply) and is captured as Phase 2.5 in the [roadmap](#phase-roadmap).

---

## Policy primitives (AINL programs evaluated by the kernel)

The kernel exposes a finite set of **decision points** where Rust code calls into the embedded AINL runtime to evaluate a policy program. Each decision point has:

1. A **typed input frame** (graph nodes from trajectory + current call context).
2. A **typed output contract** (what the policy can return, e.g. `Allow \| Deny \| Defer \| Patch`).
3. A **default policy** (compiled, non-patchable AINL bundle shipped with the kernel).
4. A **patch slot** (operator- or self-installed AINL program that overrides the default, subject to `GraphPatch` discipline).

Initial decision points (Phase 2 candidates):

| Decision point | Frame inputs | Output contract | Why it benefits from being AINL |
|---|---|---|---|
| **Rate limit** | request metadata, recent trajectory window, agent persona, ward budget | `Allow \| DeferMs(u64) \| DenyReason(string)` | Already heuristic-shaped in `crates/openfang-api/src/rate_limiter.rs`; benefits from per-ward adaptation |
| **Retry policy** | failure trajectory, error class, cost trajectory | `Retry { delay_ms, max_attempts } \| GiveUp { reason }` | Differs sharply by provider/model/tool combo; learning is high-value |
| **Dispatch heuristic** (which model/tool for this turn) | turn intent, agent persona, cost trajectory, trajectory of prior similar turns, ward budget | `model_id, tool_allowlist, eco_mode` | Routing is heuristic-dense and per-agent today; GraphPatch lets it evolve safely |
| **Trigger evaluation** | trigger condition, recent semantic facts, persona | `Fire \| Suppress \| Defer { until }` | Cron / channel / btw triggers benefit from learned suppression of false positives |
| **Escalation rule** | failure pattern, severity, persona, operator presence, ward oversight | `Continue \| Notify(channel) \| Halt(reason)` | Failure-learning surfaces here; today it is hardcoded `loop_guard` |
| **Capability grant** | requesting agent persona, requested capability, trajectory of prior grants, ward invariants | `Grant \| Deny \| RequireApproval { via }` | Persona-aware capability decisions are stronger than static allowlists |

Each decision point is small, well-typed, and individually replaceable. The kernel's Rust code does **not** make these decisions; it asks the embedded AINL runtime to evaluate the current policy program against the current frame, then enforces the typed result.

---

## The kernel-inclusion litmus test

When deciding whether a capability belongs in `ainl-kernel`, ask:

> **Could two different applications, with completely different UX philosophies, both want this exact behavior?**

- **Yes** → it belongs in the substrate or policy layer.
- **No** → it belongs above the kernel.

Concrete worked examples:

| Capability | Two-applications test | Verdict |
|---|---|---|
| Trajectory capture (typed graph events) | Yes — every app benefits from queryable causal trace | ✅ Substrate |
| Closed-loop policy validation | Yes — every app benefits from "propose → verify → fitness-gate → adopt" | ✅ Substrate |
| Pattern promotion (workflow → reusable) | Yes (substrate); criteria (when to promote) differs per app | ✅ Substrate + ⚠️ AINL policy |
| Persona evolution from metadata signals | Yes (substrate); evolution rules differ per app | ✅ Substrate + ⚠️ AINL policy |
| Failure learning | Yes (substrate); response policy differs per app | ✅ Substrate + ⚠️ AINL policy |
| Smart Suggestions to humans | No — kernel never speaks to humans; suggestion ranking is UX | ❌ Application layer |
| Adaptive Compression heuristics | No — context-compilation strategy is per-workspace | ❌ Application layer (`ainl-context-compiler`) |
| Eco Mode (token-saving tactics) | No — chat-layer heuristic | ❌ Application layer |
| AINL auto-detection ("use `.ainl` for this task") | No — orchestration / router concern | ❌ Application layer (`openfang-runtime`) |
| Dashboard / CLI learning | No — UX layer | ❌ Application layer |

**Anti-drift rule:** when in doubt, default to "above the kernel." The cost of pulling something *into* the kernel later is small; the cost of pulling something *out of* the kernel later is high (every dependent crate must be updated, security review must be redone, conformance tests must be rewritten).

---

## Deliberately not in the kernel

Listed explicitly so future contributors do not drift them in. Each entry includes the layer where it *does* belong.

| Capability | Belongs to | Why not in kernel |
|---|---|---|
| Smart Suggestions to users | Dashboard / CLI / TUI | Kernel never speaks to humans. Suggestion ranking depends on UX context (panel position, modal vs toast, mobile vs desktop) that has no business in the kernel. |
| Adaptive Compression / token-saving | `ainl-context-compiler` | Context-compilation strategy is workspace- and corpus-specific; kernel exposes token-cost metering as substrate, not the strategy that consumes it. |
| Eco Mode | `openfang-runtime` + dashboard | Per-agent UX setting persisted in `~/.armaraos/ui-prefs.json`. Kernel has no notion of "eco." |
| AINL auto-detection | `openfang-runtime` (router) | Heuristic about which language/format to use for which task; orchestration concern, not enforcement concern. |
| Cost optimization routing | `openfang-runtime` | Same — routing decisions are above kernel. |
| Project-specific tuning ("learn this workspace's preferences") | Application layer + workspace state | Kernel state is system-wide; per-workspace state belongs to the workspace. |
| Persona *display* / persona-as-UI | Dashboard | Kernel stores `Persona` nodes and enforces persona invariants; rendering them is UX. |
| Smart Detection ("suggest AINL for recurring tasks") | Analyst agent + dashboard | A heuristic over trajectory data exposed through UX; reads kernel substrate, doesn't extend it. |
| Pattern recall *ranking* for chat suggestions | Application layer | Kernel stores patterns; *which* pattern to surface in a UI is UX. |
| Theme, layout, keybindings, telemetry strip | Dashboard | Self-evident. |

Anything in this list that is later proposed for kernel inclusion should require a written justification that defeats the [litmus test](#the-kernel-inclusion-litmus-test).

---

## Non-negotiable mitigations

These are commitments, not aspirations. They map directly to the failure modes that killed prior substrate-rewrite attempts (Singularity, Plan 9, Lisp Machines, semantic-web-as-OS, the 2024–2026 "AI-native everything" startup wave). Each one is a hard rule that gates phase-exit and PR review. If we drop one, we should expect the corresponding failure mode to bite us.

### Observability projections from day one

The kernel-scoped graph is the canonical record. **Standard observability protocols emit projections of the graph from day one of every phase that adds graph writes.**

- Prometheus metrics endpoint exposing per-slot eval counts, p50/p95/p99 latency, outcome distribution, patch adoption/retirement counts, ward budget remaining.
- OpenTelemetry traces emitting spans for every kernel decision, with the `PolicyEval` node ID as the span attribute so an operator can pivot from a trace to the graph lineage.
- Structured JSON logs (already in place via `tracing-subscriber`) carrying the same `PolicyEval` ID for log-aggregator pivots.
- The dashboard's existing log/audit views work unchanged — they read the same canonical graph the projections read.

**No exceptions.** The first time an operator can't ship logs to their Datadog because our kernel speaks graph-native, they leave. The graph is the canonical record; observability is a non-negotiable derived view.

### Latency budget and hot-path discipline

- **No AINL evaluation on a decision that fires more than 100×/sec on a single-tenant deployment.** Per-request rate limit fires far above that ceiling and is therefore not a candidate for AINL eval at the request level (it can be AINL-evaluated at the *cost-table-update* level, which fires hundreds of times less often).
- **Per-decision-point latency budget: < 5 ms p99 for AINL-evaluated slots.** Measured by the conformance harness; regressions block the PR.
- **The graph write on policy-eval emission is async and batched.** It must not extend the user-visible latency of the decision being recorded.
- **Capability-preservation list latency baseline must not regress.** Boot time, idle memory footprint (~40 MB), chat round-trip latency, MCP reconnect latency, and dashboard render time are baselined per release. Phase exit requires no measurable regression.

### Verifier and self-evolution scope discipline

- **Self-installable patches stay disabled by default until Phase 4 ships with formal threat modeling and a bug bounty.** Operator-installable patches (Phase 3c) are the only patch authority before then.
- **Verifier escapes are a one-tenant blast radius, not a global one.** Per-tenant kernel isolation is the default cloud deployment model. A verifier escape on one user's machine is the same threat model as a malicious browser extension or MCP server: bad, recoverable, scoped.
- **Phase 4 hard rules** (already documented in [Phase 4 — Self-learning under verification](#phase-4--self-learning-under-verification)) are enforced by structural absence in the API surface, not by convention. The methods that would let a self-learning policy patch a ward invariant simply don't exist.
- **Kill-switch.** Operators can disable self-installable patches at runtime via a typed kernel verb. The kill-switch is exercised by integration tests on every release.

### Schema evolution discipline

- **Additive schema changes are the default.** New node types, new edge types, new optional fields are forward-compatible. Adopting one is a minor-version bump.
- **Removal requires an explicit migration step in `openfang-migrate` and a major-version bump.** The migration must be reversible (a downgrade restores prior schema or fails cleanly) for at least one minor version.
- **Crash recovery is per-write atomic.** The kernel-scoped graph store inherits the SQLite WAL + atomic-write discipline already used by `ainl-memory`. Mid-write crashes leave the graph in a consistent prior state.
- **`_reinstall_patches` equivalent on boot is non-optional from Phase 3c onward.** Crash mid-adoption rolls back; crash mid-retirement re-applies; the patch lineage chain is the source of truth for boot-time reconciliation.

### Operator skill and tooling discipline

- **Default debug story is the boring one.** Operators reading a 429 see a normal log line with a request ID, a reason, and a trace link. The graph-lineage view is a power-user surface, opt-in, never the only path to understanding a problem.
- **Three-click rule.** From any operator-visible incident (failed request, denied capability, retired patch), an operator should be able to reach the relevant graph subgraph in three clicks from the dashboard. This is the UX bar for "graph-native debug doesn't actually drag operators."
- **Dashboards ship with the kernel.** No "go build your own Grafana board" — the canonical projections are bundled and pre-configured.
- **Operator-readable graph queries.** Common queries ("show me every patch motivated by tesserae from the last 24h") ship as named saved queries; operators don't have to learn a graph DSL to debug.

### Resource budget discipline (single-machine reality check)

Local single-user deployments share a 16 GB / 8-core laptop with the user's editor, browser, and possibly a local LLM. The architecture must respect that envelope.

- **Idle daemon footprint stays at or below today's ~40 MB baseline.** Phase exit measures this.
- **Per-tenant cloud kernel footprint is bounded** (target: < 256 MB per tenant for the kernel substrate; agent-side memory is on top).
- **Graph store growth is monitored.** Kernel-scoped graph compaction rules ship before the store would otherwise hit GB scale on a heavily-used local install.
- **Per-component memory ceilings.** AINL runtime, kernel-scoped graph store, projection caches, and policy-eval working set each get an explicit ceiling enforced by tests.

### Marketing and credibility discipline

This is also a mitigation, not a separate concern. The 2024–2026 "AI-native everything" startup wave produced enough vapor that "AI-native kernel" reads as marketing to experienced operators by default. The mitigation is structural:

- **Lead with capability, not substrate.** Public material leads with "your agents remember what they did last week and you can audit every decision they made." The substrate claim is discovered, not promised.
- **Proof-not-promise discipline.** Every substrate claim in public material has to be backed by a working capability and an open-source repo a reader can verify. Captured in [Marketing discipline](#marketing-discipline).
- **Tessera and Ward defined as bounded technical primitives**, never as mystic vocabulary. Captured in [Marketing discipline](#marketing-discipline).

---

## The GraphPatch lift: from runtime-scoped to kernel-scoped

GraphPatch + `PatchRegistry` already exist (see **[ainl-runtime-graph-patch.md](ainl-runtime-graph-patch.md)** and `crates/ainl-runtime/src/adapters/`). Today they are scoped to procedural patches inside the AINL runtime. The vision is to lift the **same discipline** — not the same code — to kernel scope.

### What stays exactly the same

- The verification machinery: dataflow validation, `declared_reads` check, overwrite guard.
- The provenance schema: `PatchRecord` with `parent_patch_id`, `source_episode_ids`, `patch_version`.
- The fitness model: per-policy EMA, retire-on-degradation.
- The reversibility model: retire + reinstall on boot.
- The effect-typing model: AINL's `pure | io` system.

### What is new at kernel scope

| Concept | At runtime scope (today) | At kernel scope (vision) |
|---|---|---|
| **Patch target** | AINL labels in `ainl-runtime` | AINL policy slots at kernel decision points; ward policies. (Ward *invariants* are not patch targets — see Phase 4.) |
| **Patch authority** | Agent loop / `memory.patch` op | Operator-installed via API + kernel self-installed via verified self-learning policies (themselves AINL programs, themselves patchable, themselves bounded by ward invariants) |
| **Default body** | Compiled IR labels | Kernel-shipped default policy bundle (compiled AINL, non-patchable). Wards ship with default invariants per ward kind. |
| **Verification scope** | Runtime frame + prior writes | Kernel decision-point frame contract (typed) + ward invariants (e.g. "no policy may grant a capability the agent's ward forbids") |
| **Fitness signal** | Per-label outcome | Per-decision-point outcome rolled up by trajectory window; per-ward rollup for ward-scoped policies |
| **Retirement trigger** | Fitness EMA threshold | Same + ward invariant-violation auto-retire (immediate) |
| **Storage** | Per-agent SQLite | Kernel-scoped graph store (tesserae + ward state) + per-agent overlay for agent-specific policies |

### What `ainl-kernel` will expose to make this work

A new sub-trait family added to `KernelApi` (planned, not yet implemented):

| Trait | Verb shape | Used by |
|---|---|---|
| `KernelPolicy` | `eval_policy(slot, frame) -> PolicyOutcome`, `list_policies()`, `default_policy(slot)` | Kernel internals at decision points |
| `KernelPatches` | `propose_patch(slot, ainl_bundle) -> ProposalId`, `validate_patch(id)`, `adopt_patch(id)`, `retire_patch(id, reason)`, `list_patches(slot)` | Operators (via API) + self-learning AINL programs (via privileged op) |
| `KernelTrajectory` | `query(graph_query) -> impl Iterator<Item = TrajectoryNode>`, `subscribe(filter) -> Stream<TrajectoryEvent>` | AINL policies (read-only) + dashboard (read-only) |
| `KernelTesserae` | `emit(tessera) -> TesseraId`, `query(filter) -> impl Iterator<Item = Tessera>`, `lineage(id) -> TesseraLineage` | AINL policies (emit + query, subject to ward visibility) + dashboard (read-only) |
| `KernelWards` | `list_wards()`, `members(ward) -> impl Iterator<Item = AgentId>`, `invariants(ward) -> &[WardInvariant]`, `budget_remaining(ward) -> WardBudget`, `assign(agent, ward) -> Result<()>`, `unassign(agent, ward)`, `propose_invariant(ward, ainl) / adopt_invariant` (operator-only) | Operators (via API) + kernel internals enforcing invariants |

`KernelPatches::propose_patch` is the gate for *policies*. `KernelWards::propose_invariant` is the gate for *ward invariants* and is restricted to operator origin (no self-installed invariants — ever). The verification pipeline (validate → fitness gate → adopt \| reject) is kernel-owned and not bypassable from either gate.

---

## Phase roadmap

### Phase 1 — Façade + conformance harness ✅ landed (Phase 1b)

- New `crates/ainl-kernel/` workspace member.
- `KernelApi` supertrait composed of focused sub-traits (`KernelLifecycle`, `KernelAgents`, more to follow).
- `OpenFangKernelAdapter` newtype that implements the façade over the existing `OpenFangKernel`. Callers can already migrate to `&dyn KernelApi` where it makes sense.
- `conformance::fixture` hermetic test fixture (temp `ARMARAOS_HOME`, sample `AgentManifest` helper).
- `smoke_lifecycle` and `smoke_agents` conformance tests passing against the adapter.

This phase is purely additive. Nothing in the existing kernel changes.

### Phase 2 — AINL policy evaluation at the empirical-wedge decision point

Phase 2 has been reshaped from "three decision points in parallel" to "**one wedge decision point end-to-end first**, then expand." The reasoning:

- We need empirical proof — measurable on a metric an operator cares about — that AINL eval at a kernel decision point clears the latency budget, ships clean observability projections, and produces operator-debuggable lineage. Doing three slots in parallel before any one has been proven end-to-end is exactly the "AINL-flavored kernel" drift the [Honest status footer](#honest-status-footer) warned about.
- The wedge slot has to be a **non-microsecond hot path** so the latency budget is comfortable, and a slot where AINL eval pays off in measurable operator benefit. Per-request rate limit is the *worst* candidate by both criteria — it fires thousands of times per second and the operator benefit of "an AINL policy decided 429" over "a Rust function decided 429" is small.

#### Step 0 — Pick the wedge slot

Three candidates that meet "non-microsecond hot path + measurable operator benefit":

| Candidate | Fires (typical) | Why it's a good wedge | Why it might not be |
|---|---|---|---|
| **Cron schedule admission** (decide whether a cron job should run *this tick* given current ward budget, recent failures, system load) | seconds-to-minutes | Latency budget enormous; AINL adds clear value (learn that this job always fails on Monday mornings, defer); easy to A/B against today's "always run on schedule" behavior. | Lower visibility — operators don't watch cron decisions closely. |
| **MCP reconnect policy** (decide when and how to retry a dropped MCP connection: backoff curve, give-up threshold, fall back to which alternate server) | per-disconnect (rare) | Today's policy is hardcoded exponential backoff; AINL can learn per-server reliability and adapt. Failures are operator-visible (broken MCP tools), so the win is concrete. | The slot itself is small; less general lesson. |
| **Agent spawn admission** (decide whether to allow an `agents.create` request given ward membership, ward budget, recent spawn rate, persona constraints) | per-spawn (low frequency) | Ward integration is natural here (Phase 3b alignment); rejection reasons are operator-debuggable; today's policy is a flat allowlist that operators routinely outgrow. | Requires enough Phase 3b ward scaffolding to be meaningful; could become a dependency tangle. |

**Picked: MCP reconnect policy.** Reasoning: lowest fire frequency (no latency anxiety), highest operator-visible payoff (broken MCP tools are a top incident class), no Phase 3b dependency, today's behavior is a hardcoded backoff curve that's easy to A/B against. **Cron admission** remains a strong second slot for Phase 2.6.

#### Step 1 — Type the frame and outcome for the wedge slot ✅ landed

Same pattern as the rate-limit contracts already in `crates/ainl-kernel/src/policy/`:

- ✅ `policy::mcp_reconnect::{McpReconnectFrame, McpReconnectDecision, McpReconnectOutcome, McpServerStatus, ReconnectCaps}`.
- ✅ `PolicySlot::McpReconnect` variant added; `PolicySlot::all()` helper for exhaustive iteration.
- ✅ Serde round-trip tests cover every variant, partial-input forward-compat, unknown-variant rejection, opaque-string handle round-trip, and absent-`motivated_by` legacy back-compat.

Boundary types use typed graph handles per [Kernel state IS the graph](#kernel-state-is-the-graph) — no bare string IDs.

#### Step 2 — Ship a default AINL policy that reproduces today's hardcoded behavior bit-for-bit

✅ **Built-in fallback landed**: `policy::mcp_reconnect::default_decision` is the bit-for-bit Rust reproduction of `openfang_extensions::health::HealthMonitor::should_reconnect` + `backoff_duration`. Property tests pin it across attempt counts 0..15, custom backoff caps, custom attempt floors, and the exponent-cap plateau (no overflow at `attempt = u32::MAX`). This function serves as both:

- the parity baseline the future AINL strawman policy must match before being allowed to diverge, and
- the production fallback when AINL evaluation is unavailable or disabled per the [kill-switch commitment](#non-negotiable-mitigations).

*(Pending: the AINL strawman itself — i.e. the same logic expressed as an AINL program — lands in step 3.)*

#### Step 3 — Wire the kernel call site to evaluate via the policy seam

Split into two sub-steps so the seam shape is proven before the AINL runtime is added to the path.

**Step 3a — define the kernel-side policy seam ✅ landed.** The trait that the kernel call site consumes:

- `policy::mcp_reconnect::McpReconnectPolicy` — sync `evaluate(&self, frame: &McpReconnectFrame) -> McpReconnectPolicyResult`. `Send + Sync + Debug` because the reconnect loop runs on a multi-threaded runtime and the kernel may swap policies at runtime.
- `policy::mcp_reconnect::DefaultMcpReconnectPolicy` — production-ready built-in: a thin wrapper over `default_decision` so the trait surface has a real impl from day one. Used when no operator policy is loaded *and* whenever AINL eval is disabled (kill switch). Stable name: `ainl_kernel.mcp_reconnect.default`.
- `policy::mcp_reconnect::FallbackMcpReconnectPolicy<P>` — wraps any inner policy and substitutes `default_decision` on `Err`, with structured `tracing::warn!` carrying the inner policy name + server ref + error. This is the **structural** realization of the kill-switch commitment: a misbehaving AINL policy can never stop the reconnect loop.
- `policy::mcp_reconnect::McpReconnectPolicyError` — non-exhaustive enum with `EvaluationFailed` and `FrameRejected` variants so the apply path can drive distinct metric series for kernel-side bugs vs. policy-side bugs.
- `policy::mcp_reconnect::SharedMcpReconnectPolicy = Arc<dyn McpReconnectPolicy>` + `default_shared_policy()` — the expected handle type for storing a policy on the kernel.

Properties pinned by tests: default-policy bit-for-bit parity with `default_decision` across attempts 0..11; fallback pass-through on success; fallback substitution on error with the wrapped inner correctly invoked exactly once; fallback parity property at attempts 0, 1, 5, 9, 10, 100; stable names for both impls; `Arc<dyn ...>` dispatch correctness.

**Step 3b — wire the actual call site (not yet landed).** Modifies `openfang-kernel`'s `auto_reconnect_loop` (and `HealthMonitor`) to build a `McpReconnectFrame`, invoke `kernel.mcp_reconnect_policy.evaluate(&frame)` (typed as `SharedMcpReconnectPolicy`), and apply the outcome (sleep + reconnect for `Retry`; skip-tick for `Defer`; mark-permanently-failed for `GiveUp`). The first `kernel.mcp_reconnect_policy` installed is `FallbackMcpReconnectPolicy::new(DefaultMcpReconnectPolicy)`, which is a pure refactor — same behavior, new code path. Only after the call site is exercising the seam in production does an AINL evaluator get added as a `McpReconnectPolicy` impl that the fallback wraps.

#### Step 4 — Conformance + parity + latency tests

- Conformance: every kernel implementation produces the same outcome on the same frame.
- Parity: outcome distribution over a recorded request trace matches today's hardcoded implementation.
- Latency: p99 < 5 ms per [Non-negotiable mitigations](#non-negotiable-mitigations).

#### Step 5 — Observability projections

Every policy eval emits a `PolicyEval` graph node + Prometheus counter + OTLP span + structured log line, all carrying the same `PolicyEval` ID for cross-projection pivots. This is the day-one observability commitment from [Non-negotiable mitigations](#non-negotiable-mitigations).

#### Step 6 — Empirical-wedge writeup

Short doc: which slot, what we measured, what improved over the hardcoded baseline (incident MTTR, debug time, operator-reported clarity). This is the artifact that turns "AINL-native kernel" from claim into evidence per [Marketing discipline](#marketing-discipline).

Critical: **no patching is enabled yet.** The default policy is compiled and non-patchable. Phase 2 proves AINL evaluation at one decision point; Phase 3c introduces the patch surface.

The contract types intentionally live in `ainl-kernel` (not in `ainl-contracts` or `ainl-runtime`) because they are the kernel's boundary, not generic AINL plumbing. The AINL runtime sees them as opaque JSON; only the kernel pattern-matches on the Rust shape. See `crates/ainl-kernel/src/policy/mod.rs` for the design discipline that governs additions.

### Phase 2.5 — Graph-handle retrofit of existing contracts

Small, mechanical, one PR. Captures the boundary work surfaced by [Kernel state IS the graph](#kernel-state-is-the-graph).

1. ✅ Replace bare-string graph references in `RateLimitFrame` (`caller.ward_id`, `caller.agent_id`) with typed handles (`WardNodeRef`, `AgentNodeRef`) that the kernel can validate at frame construction. Bumped `RATE_LIMIT_SCHEMA_VERSION` to 2; wire format unchanged because handles are `#[serde(transparent)]`.
2. ✅ Restructure `RateLimitOutcome` to wrap `RateLimitDecision` (the existing tagged-action enum) plus `motivated_by: Vec<TesseraId>` (semantically empty until Phase 3a; structurally present so policies and observers can rely on the field existing). Wire format stays flat via `#[serde(flatten)]`.
3. *(Deferred to Phase 3a.)* Define the `PolicyEval` node shape and its outgoing edges (`(Frame) ─[evaluated_by]→ (PolicySlot) ─[produced]→ (Outcome) ─[motivated_by]→ (Tessera*)`) as an emit-once-per-eval pattern in the kernel's apply path. Carved out of 2.5 because it is the *store* concern, not the *schema* concern; the contract types defined in steps 1–2 are already shaped to feed it. The `PolicyEvalId` handle is reserved in `crate::graph::handles` so contract surfaces can mention it today.
4. ✅ Update the `policy/mod.rs` design discipline to require typed graph handles in any new contract.

Landed at: `crate::graph::{handles, AgentNodeRef, WardNodeRef, McpServerNodeRef, TesseraId, PolicyEvalId}`.

Phase 2.5 lands before Phase 2 step 1 for the wedge slot, because doing wedge-slot contracts with the new pattern is cheaper than retrofitting two slots' worth later. The MCP-reconnect wedge contract immediately below is the first contract built using the Phase 2.5 patterns from day one.

### Phase 2.6 — Expand to the remaining decision points

Only after the wedge slot is end-to-end and the empirical writeup exists. Apply Phase 2 steps 1–5 to the next two slots (likely cron admission and rate-limit-cost-table-update — *not* per-request rate limit). Each expansion repeats the latency + parity + observability tests.

### Phase 3a — Tesserae substrate (kernel-scoped graph store)

1. Define final `Tessera` types (`TesseraKind`, `TesseraSource`, `AinlEffect`, `TesseraId`).
2. Stand up the kernel-scoped graph store backing tesserae (alongside, not replacing, per-agent `~/.armaraos/agents/<id>/ainl_memory.db` shards).
3. New `KernelTesserae` sub-trait (`emit`, `query`, `lineage`).
4. Wire Phase 2 decision points to emit a tessera per evaluation (input frame snapshot + outcome + ward attribution + provenance).
5. Operator-readable API: `GET /api/kernel/tesserae?…` with filter + lineage walks.
6. Tesserae are emit-only from agent contexts at this phase; AINL policies cannot yet read across agents (ward visibility lands in 3b).

### Phase 3b — Wards as grouping primitive

1. Define final `Ward` types (`WardId`, `WardMembership`, `WardInvariant`, `WardBudget`, `WardOversight`, `WardVisibility`).
2. New `KernelWards` sub-trait (verb-shaped per discipline above).
3. Kernel-shipped default ward kinds (e.g. `default-user`, `system-services`, `untrusted-experimental`) with sensible default invariants for each.
4. Operator API: `GET/POST /api/kernel/wards`, `POST /api/kernel/wards/{id}/members`, `POST /api/kernel/wards/{id}/invariants/propose`, `POST /api/kernel/wards/{id}/invariants/adopt`.
5. Wire ward invariants into the Phase 2 decision points (every eval consults the calling agent's ward invariants pre-policy and post-policy).
6. Wire ward budgets into rate-limit and dispatch decision points.
7. Enable cross-agent tessera reads gated by ward visibility (within-ward by default; cross-ward requires explicit capability).

### Phase 3c — Kernel-scoped `PatchRegistry`

Only after 3a + 3b are stable. `KernelPatches` lifts the GraphPatch discipline to kernel scope.

1. New `KernelPatches` sub-trait (verb-shaped per discipline above).
2. Lift `PatchAdapter` / `AdapterRegistry` patterns from `ainl-runtime` to `ainl-kernel`, scoped to policy slots.
3. Pre-install dataflow validation, frame-contract validation, ward-invariant validation (every proposed patch must satisfy the invariants of every ward whose members it could affect).
4. `PatchRecord` lineage in the kernel-scoped graph store, linked to the tesserae that motivated the patch.
5. Fitness EMA + auto-retire.
6. `_reinstall_patches` equivalent on kernel boot.
7. API surface: `POST /api/kernel/policies/{slot}/propose`, `POST /api/kernel/policies/{slot}/adopt`, etc. **Operator-driven only at this phase. No self-installed patches yet.**

### Phase 4 — Self-learning under verification

Only after Phase 3c has been operationally stable for some time. Self-learning policies are themselves AINL programs that propose patches via `KernelPatches::propose_patch`. Every self-installed patch goes through the same validation pipeline as operator-installed patches. The kernel never trusts a self-learning policy more than an operator policy; both are gated identically.

**Hard rules for Phase 4 (each one is a structural barrier in the API surface, not a convention):**

- A self-learning policy may never patch the policy that authorizes patches (no recursive trust elevation).
- A self-learning policy may never patch a **ward invariant**. Invariants are operator-only via `KernelWards::propose_invariant`, which has no self-installed code path.
- A self-learning policy may never modify ward **membership**, ward **budgets**, or ward **oversight**. Operator-only.
- A self-learning policy may never grant **cross-ward visibility** to itself or any other policy.
- A self-learning policy may never patch capability-grant decisions (operator-only).
- Every self-installed patch MUST carry `source_episode_ids` AND a list of motivating tesserae IDs (no provenance, no patch).
- Every self-installed patch is fitness-gated for a minimum trajectory window before adoption (no instant adoption).
- Every self-installed patch is bounded by the invariants of every ward whose members it could affect; a patch that fails ward-invariant validation is auto-rejected before it reaches fitness gating.

### Phase 5 — Subsystem migration

Migrate the remaining `OpenFangKernel` subsystems behind the façade (sessions, workflows, triggers, cron, hands, MCP, metering, observability, auth) — one at a time, each with a verb-shaped sub-trait, each with conformance tests. The legacy kernel implementation remains until every subsystem has a native `ainl-kernel` implementation that passes conformance.

Only at the end of Phase 5 does the rename become real (`openfang-kernel` retired, `ainl-kernel` is the kernel). Until then both crates ship.

---

## Marketing discipline

The thing is impressive enough that overclaiming is the only failure mode that can hurt it. Suggested framing, in priority order:

**Use:**
- "AINL-native kernel" — descriptive, accurate, distinct.
- "Graph-native agent OS" — descriptive; differentiates from framework-level graph systems.
- "Self-evolving under verification" — accurate **with the qualifier**; never drop it.
- "Verified runtime policy evolution" — precise, technical.
- "eBPF-class verification for AI behavior" — accurate analogy that technical readers grok.

**Avoid:**
- "Self-aware kernel" — meaningless and inflammatory.
- "Sentient OS" — actively misleading.
- "AGI kernel" — instant credibility loss.
- "Cognitive kernel" — vague, sounds like marketing.
- Any framing that implies the kernel makes free-form decisions. The kernel evaluates verified AINL programs; this is *stronger* and *more interesting* than "the kernel decides," not weaker.

The audience that will recognize the GraphPatch + `PatchRegistry` + AINL combination as substantively novel is the same audience that will dismiss anything dressed in the wrong vocabulary.

**On Tessera and Ward in public material:** define them as bounded technical primitives ("a Tessera is a kernel-scoped knowledge fragment with provenance"; "a Ward is a bounded membership group with shared invariants, resources, oversight, and observability"). Skip the etymology in API docs, trait docs, and marketing copy — at most a one-line glossary note. Never lean on the metaphors as load-bearing argument; they were generative, they are not load-bearing.

---

## Open questions

These are deliberately not answered in this document; they require live design work as we move through Phases 2–4.

1. **Trajectory schema unification.** Today there are at least three event streams (`OrchestrationTrace`, `audit_log`, `event_bus`). Phase 2 needs a single typed graph schema that subsumes all three without breaking any existing reader.
2. **Per-agent vs kernel-scoped graph store.** Per-agent `~/.armaraos/agents/<id>/ainl_memory.db` works well today. Kernel-scoped patterns and cross-agent facts need a kernel-owned graph store. How do we partition without doubling the surface area?
3. **Policy evaluation latency budget.** Calling into `ainl-runtime` per decision point is heavier than an inline Rust check. What's the latency budget per decision point, and which decision points are too hot for AINL eval at all?
4. **Patch invariants beyond effect typing.** AINL's `pure | io` effect system is necessary but not sufficient. What additional invariants do we need (e.g. "no policy may issue more than N IO ops per eval")?
5. **Operator UX for proposing/reviewing/adopting patches.** This is application-layer work, but the kernel API has to make it tractable. What does "review a proposed patch" look like as a typed artifact?
6. **Crash recovery for in-flight patches.** `_reinstall_patches` covers the steady state. What about a crash mid-validation, mid-adoption, mid-retire?
7. **Cross-policy invariants.** "No policy may grant a capability the persona forbids" is a cross-policy constraint. Where does that constraint live, and who enforces it? (Strong candidate: ward invariants. Verify in Phase 3b.)
8. **Ward boundary management.** When and how can wards be split, merged, or have members reassigned? What invariants must be preserved across a split/merge (e.g. budget conservation, in-flight policy continuity, tessera lineage integrity)? What does a "drain" of a ward look like before it's deleted?
9. **Default ward kinds.** What ward kinds does `ainl-kernel` ship out of the box? Strawman: `system-services` (kernel-owned agents, strict invariants), `default-user` (operator-installed agents, moderate invariants), `untrusted-experimental` (sandbox for new agents, tight budgets, broad invariants). Validate in Phase 3b.
10. **Per-agent vs per-ward fitness rollup.** When a self-learning policy fires across many agents in a ward, do we measure fitness per-agent (high variance, slow signal) or rolled up per-ward (low variance, faster signal but masks per-agent regressions)? Likely both, with explicit weighting; design in Phase 4.

---

## Glossary

- **`KernelApi`** — The composed façade trait every kernel implementation must satisfy. Defined in `crates/ainl-kernel/src/api/mod.rs`. Composed of focused sub-traits.
- **Sub-trait** — A focused trait (e.g. `KernelLifecycle`, `KernelAgents`) covering one verb-shaped responsibility. Callers depend on the narrowest sub-trait they actually use.
- **`OpenFangKernelAdapter`** — Newtype wrapper that implements the `ainl-kernel` façade traits over the legacy `OpenFangKernel`. Lives in `crates/ainl-kernel/src/adapter.rs`. Lets the strangler-fig migration proceed without forking callers.
- **Conformance harness** — Generic tests written against the trait surface (`crates/ainl-kernel/src/conformance/`). Both the legacy adapter and the future native `AinlKernel` must pass the same suite.
- **Hermetic fixture** — `KernelFixture` in `crates/ainl-kernel/src/conformance/fixture.rs`. Boots a real kernel against a temporary `ARMARAOS_HOME` so conformance tests never touch the user's actual home.
- **GraphPatch** — Verified runtime modification of AINL programs. Originated in `runtime/engine.py` (Python) and `crates/ainl-runtime/src/adapters/graph_patch.rs` (Rust). Lifted to kernel scope in Phase 3c.
- **`PatchRecord`** — Typed provenance record for a GraphPatch. Includes `parent_patch_id`, `source_episode_ids`, `patch_version`, `patched_at`, `retired_at`, `retired_reason`.
- **Decision point** — A site in kernel Rust code where a typed frame is handed to the embedded AINL runtime, which evaluates the current policy program and returns a typed outcome the kernel then enforces.
- **Policy slot** — A named decision point with a typed input/output contract and a default compiled AINL policy. Patchable subject to `GraphPatch` discipline.
- **AINL** — AI Native Language. Deterministic, effect-typed, agent-generated graph IR. Spec: `AI_Native_Lang/docs/AINL_SPEC.md`. Privileged in `ainl-kernel` the way C is in Linux.
- **Tessera** (plural: **tesserae**) — kernel-scoped, typed, cross-agent knowledge fragment with provenance. Defined precisely in [Kernel vocabulary](#kernel-vocabulary-tessera-and-ward). Distinct from per-agent `Episode` / `Semantic` / `Procedural` / `Persona` nodes by ownership (kernel-owned), visibility (cross-agent subject to ward rules), and composition (designed to be assembled by AINL policies). Lands in Phase 3a. Etymology: a one-line note for the curious — the term names the bounded-knowledge-fragment primitive; technical definition stands on its own.
- **Ward** — kernel-managed bounded membership group with shared invariants, shared resources, named oversight, and scoped mutual observability. Defined precisely in [Kernel vocabulary](#kernel-vocabulary-tessera-and-ward). The kernel's grouping primitive in the cgroup/namespace family. Lands in Phase 3b. Etymology: the traditional sense of a bounded local community/unit; technical definition stands on its own.
- **Ward invariant** — typed AINL predicate continuously enforced on all members of a ward. Operator-installed only; never patchable by self-learning policies. The structural safety boundary of Phase 4.
- **Ward visibility** — rule governing which wards' tesserae a given AINL policy can read. Within-ward by default; cross-ward requires explicit capability and emits a tracked tessera lineage.

---

## Honest status footer

As of this writing:

- **Landed (code), this revision adds:**
  - **Phase 2.5 schema** — `crate::graph` module with typed node-handle newtypes (`AgentNodeRef`, `WardNodeRef`, `McpServerNodeRef`, `TesseraId`, `PolicyEvalId`), all `#[serde(transparent)]` so the wire format stays an opaque string. Rate-limit contracts retrofitted: `CallerIdentity` now carries `agent_ref: Option<AgentNodeRef>` / `ward_ref: Option<WardNodeRef>`; `RateLimitOutcome` is now a wrapper around `RateLimitDecision` (the prior tagged enum) plus `motivated_by: Vec<TesseraId>`. `RATE_LIMIT_SCHEMA_VERSION` bumped to 2.
  - **Phase 2 step 1 for the wedge slot** — `policy::mcp_reconnect` module with `McpReconnectFrame`, `McpReconnectDecision` (`Retry { delay_ms } | Defer { check_again_after_ms, reason } | GiveUp { reason }`), `McpReconnectOutcome` (decision + `motivated_by`), `McpServerStatus` (`Healthy | Reconnecting | Error { reason }`), `ReconnectCaps` (operator-configured `max_attempts` + `max_backoff_secs`), and `MCP_RECONNECT_SCHEMA_VERSION = 1`. `PolicySlot::McpReconnect` variant + `PolicySlot::all()` exhaustive helper. Full serde discipline (round-trip every variant, partial-input forward-compat, opaque-string handle round-trip, absent-`motivated_by` legacy back-compat, unknown-action rejection).
  - **Phase 2 step 2 for the wedge slot** — `mcp_reconnect_default_decision` is the bit-for-bit Rust reproduction of `HealthMonitor::should_reconnect` + `backoff_duration`. Property tests pin parity across attempt counts 0..15, custom backoff caps, custom attempt floors, and the exponent-cap plateau (no overflow at high attempt counts).
  - **Phase 2 step 3a for the wedge slot** — kernel-side policy seam: `McpReconnectPolicy` trait (`Send + Sync + Debug`, sync `evaluate`); `DefaultMcpReconnectPolicy` (parity wrapper, stable name `ainl_kernel.mcp_reconnect.default`); `FallbackMcpReconnectPolicy<P>` (kill-switch wrapper that substitutes `default_decision` on inner-policy `Err` with structured `tracing::warn!`); `McpReconnectPolicyError` (non-exhaustive `EvaluationFailed` / `FrameRejected`); `SharedMcpReconnectPolicy = Arc<dyn McpReconnectPolicy>` + `default_shared_policy()` constructor. Tests pin parity, fallback substitution, fallback pass-through, name stability, and `Arc<dyn ...>` dispatch.
  - **Test status:** 49 tests pass; `cargo clippy -p ainl-kernel --all-targets -- -D warnings` is clean.
- **Landed (code), already shipped before this revision:** Phase 1b — `ainl-kernel` crate exists, `KernelApi` + `KernelLifecycle` + `KernelAgents` traits, `OpenFangKernelAdapter`, hermetic conformance fixture, conformance smoke tests passing against `OpenFangKernel`.
- **Landed (vision), this revision:** Phase 2.5 status updated to "schema landed, store deferred to Phase 3a"; Phase 2 wedge slot promoted from strawman to picked; Phase 2 step 1 + step 2 for MCP reconnect marked complete with code references; Phase 2 step 3 split into 3a (eval seam — landed) and 3b (call-site rewire — pending).
- **Not landed:** Phase 2 step 3b for the wedge slot (`openfang-kernel`'s `auto_reconnect_loop` still calls the inline `HealthMonitor` logic; the seam exists but no caller uses it yet). Phase 2 steps 4–6 for the wedge slot (no AINL evaluator on the path; conformance/parity/latency tests live at the contract level only, not the call site; no observability projections; no empirical writeup). Phase 2.6 (other slots). All of Phases 3a/3b/3c, 4, 5. The kernel today still uses the inline `HealthMonitor` logic at the actual call site — the new contract types, parity function, and policy seam exist but are not yet plugged into the production reconnect loop.
- **Risk if we drift:** the failure modes in [Non-negotiable mitigations](#non-negotiable-mitigations) are exactly the ones that killed the historical precedents (Plan 9, Singularity, Lisp Machines, semantic-web-as-OS, the recent "AI-native everything" startup wave). Each one is a hard rule. Dropping any of them — observability projections, latency budget, schema-evolution discipline, default-debug story, resource ceilings, marketing discipline — should be expected to bite us in the predictable shape.

This document will be updated at the end of each phase with what actually shipped versus what was planned, including honest deltas.
