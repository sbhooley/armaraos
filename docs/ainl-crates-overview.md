# AINL crates overview

The `ainl-*` crates form a cooperative, dependency-light family that any AINL host
(`armaraos`, `ainl-inference-server`, the AINL MCP, `ainativelang` web) can pull from
without committing to any specific runtime. They group into three families:

```mermaid
flowchart LR
    subgraph policy [Policy]
        ainl_contracts[ainl-contracts<br/>shared types + telemetry vocab]
        ainl_context_freshness[ainl-context-freshness<br/>tool-execution gate]
        ainl_impact_policy[ainl-impact-policy<br/>blast-radius decisions]
        ainl_repo_intel[ainl-repo-intel<br/>capability normalization]
    end

    subgraph orchestration [Orchestration]
        ainl_compression[ainl-compression<br/>per-segment heuristic compression]
        ainl_context_compiler[ainl-context-compiler<br/>LLM context-window assembly]
        ainl_runtime[ainl-runtime<br/>agent-loop primitives]
        ainl_persona[ainl-persona<br/>persona signal extraction]
        ainl_semantic_tagger[ainl-semantic-tagger<br/>topic + tone tagging]
        ainl_agent_snapshot[ainl-agent-snapshot<br/>turn snapshots]
    end

    subgraph data [Data]
        ainl_memory[ainl-memory<br/>episodic graph store]
        ainl_graph_extractor[ainl-graph-extractor<br/>node/edge extraction]
        ainl_trajectory[ainl-trajectory<br/>trajectory log + replay]
    end

    ainl_context_compiler -. consumes .-> ainl_context_freshness
    ainl_context_compiler -. consumes .-> ainl_compression
    ainl_context_compiler -. consumes .-> ainl_semantic_tagger
    ainl_context_compiler -. consumes .-> ainl_memory
    ainl_context_freshness --> ainl_contracts
    ainl_impact_policy --> ainl_contracts
    ainl_compression --> ainl_contracts
```

## The two `ainl-context-*` crates (boundary table)

These two crates **share a name prefix but solve different problems**. They can be used
independently or together; the compiler crate optionally consumes the freshness crate
as a per-segment rank-down signal.

| Aspect | `ainl-context-freshness` | `ainl-context-compiler` |
|---|---|---|
| Lifecycle phase | **Pre-tool execution** policy gate | **Prompt assembly** / window management |
| "Context" means | the agent's *knowledge of the world* (repo / index state vs HEAD) | the *LLM's input context window* (prompt bytes about to be sent) |
| Key types | `FreshnessInputs`, `impact_decision_*` | `Segment`, `BudgetPolicy`, `ContextCompiler`, `ComposedPrompt` |
| Returns | `ImpactDecision::{AllowExecute, RequireImpactFirst, BlockUntilFresh}` | `ComposedPrompt { segments, anchored_summary, telemetry }` |
| Stateful | No (pure functions) | Builder + per-call orchestrator |
| Optional ML | Never | Tier-gated: heuristic → LLM summarization → embedding rerank |
| Roadmap status | Stable since 0.1 | Phase 6 of [SELF_LEARNING_INTEGRATION_MAP](./SELF_LEARNING_INTEGRATION_MAP.md) |

## When to reach for which crate

- Adding **a new cross-host telemetry field**? → `ainl-contracts`.
- Compressing **a single user prompt** (the original eco-mode use case)? → `ainl-compression`.
- Deciding **whether a tool call is safe** given index staleness? → `ainl-context-freshness`.
- Assembling **the entire LLM input** (system + history + tool outputs + user msg) within a
  token budget, with question-aware ranking and optional anchored summarization?
  → `ainl-context-compiler`.

## Host integration (OpenFang, M1)

ArmaraOS wires the compiler in **measurement mode** first: `openfang-runtime` runs
`ainl_context_compiler::ContextCompiler::compose` on the assembled system prompt + history
+ user message, then stashes the resulting whole-prompt token estimates in
`openfang_runtime::compose_telemetry` (same side-channel idea as `eco_telemetry::record_turn`).
`openfang-kernel` calls `take_compose_turn` when persisting `eco_compression_events` so
dashboard *input tokens not billed* / *USD not spent* reflect the full context window, not
only the compressed user string. M2 replaces the wire-format messages with
`ComposedPrompt.segments` and can surface the compiler tier in API payloads.

## Cross-references

- [`SELF_LEARNING_INTEGRATION_MAP.md`](./SELF_LEARNING_INTEGRATION_MAP.md) §15.6 lists the
  full integration-phase dependency graph.
- [`prompt-compression-efficient-mode.md`](./prompt-compression-efficient-mode.md)
  documents the per-segment compression algorithm reused by the compiler crate.
