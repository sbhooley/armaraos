# Agent Orchestration Enhancement - Phased Implementation Plan

**Version:** 1.0  
**Date:** 2026-04-11  
**Parent Document:** `docs/agent-orchestration-design.md`

## Overview

This document breaks down the agent orchestration enhancement design into **non-breaking, independently deployable phases**. Each phase delivers value on its own and can be merged to main without disrupting existing functionality.

**Guiding Principles:**
- Every phase is 100% backward compatible
- Each phase passes all existing tests
- Features are opt-in (default behavior unchanged)
- Phases can be deployed independently to production

**Repository status (2026-04-11):** Phases **1–8** are implemented on `main` (see **`docs/orchestration-implementation-audit.md`** and **`docs/agent-orchestration-design.md`**). This file remains the **historical decomposition** and checklist; Phase **9** is partially complete (guides and API docs exist under `docs/orchestration-*.md`, `docs/workflows.md`, `docs/api-reference.md` — not every original filename below). **Checkbox hygiene:** Phases **1–2** and **9** were refreshed in this doc pass; Phases **3–8** checkboxes may still show `[ ]` even though the corresponding code is shipped—trust the audit doc for ground truth.

---

## Phase 1: Orchestration Context Foundation (Week 1-2)

**Goal:** Establish the type system and threading infrastructure for orchestration context, without changing any existing behavior.

### Deliverables

#### 1.1 New Types (No Breaking Changes)

**File:** `crates/openfang-types/src/orchestration.rs` (new file)

```rust
// Complete OrchestrationContext, OrchestrationPattern, MapReducePhase types
// See design doc section 1 for full implementation
```

**File:** `crates/openfang-types/src/lib.rs`

```rust
pub mod orchestration;  // Add new module export
```

#### 1.2 Thread Context Through Agent Loop

**File:** `crates/openfang-runtime/src/agent_loop.rs`

Changes:
- Add `orchestration_ctx: Option<OrchestrationContext>` parameter to `run_agent_loop()`
- Add `orchestration_ctx: Option<OrchestrationContext>` parameter to `run_agent_loop_streaming()`
- Store context in function scope, don't use it yet (preparation for Phase 2)
- Pass `None` from all existing callers

**Backward Compatibility:**
- All existing code passes `None`
- No behavior changes
- All existing tests pass unchanged

#### 1.3 Add Context Field to Message Type

**File:** `crates/openfang-types/src/message.rs`

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    // ...existing fields...
    
    /// Orchestration context (optional, for multi-agent coordination)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orchestration_ctx: Option<OrchestrationContext>,
}
```

**Backward Compatibility:**
- `#[serde(skip_serializing_if = "Option::is_none")]` ensures existing messages serialize identically
- Deserialization handles missing field gracefully (defaults to `None`)
- No database migration needed (SQLite JSONB is flexible)

### Testing

```bash
# Must pass all existing tests
cargo test --workspace

# Must compile cleanly
cargo clippy --workspace --all-targets -- -D warnings

# Must build
cargo build --workspace --release
```

### Success Criteria

- [x] New types compile and pass doc tests
- [x] Agent loop accepts context parameter (callers may pass `None` or workflow-built `Some`)
- [x] Message type extended without breaking serialization
- [x] All 1744+ existing tests pass (CI expectation; run locally before release)
- [x] Zero clippy warnings (`cargo clippy --workspace --all-targets -- -D warnings`)
- [x] Production-safe: optional context preserves legacy behavior where not wired

---

## Phase 2: Context Propagation (Week 3)

**Goal:** Propagate context through `agent_send` and `agent_spawn`, enabling distributed tracing and call chain tracking.

### Deliverables

#### 2.1 Tool Runner Context Propagation

**File:** `crates/openfang-runtime/src/tool_runner.rs`

Changes:
1. Add `orchestration_ctx: Option<&OrchestrationContext>` parameter to `execute_tool()`
2. In `tool_agent_send`: create child context and pass through kernel
3. In `tool_agent_spawn`: create child context and pass through kernel

```rust
async fn tool_agent_send(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    orchestration_ctx: Option<&OrchestrationContext>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agent_id = input["agent_id"].as_str().ok_or("Missing 'agent_id'")?;
    let message = input["message"].as_str().ok_or("Missing 'message'")?;
    
    // Create child context if parent context exists
    let child_ctx = orchestration_ctx.map(|ctx| {
        // Parse agent_id to AgentId (or use current agent's ID as fallback)
        let child_id = AgentId::from_str(agent_id).unwrap_or_else(|_| AgentId::new());
        ctx.child(child_id)
    });
    
    kh.send_to_agent_with_context(agent_id, message, child_ctx).await
}
```

#### 2.2 Extend KernelHandle Trait

**File:** `crates/openfang-runtime/src/kernel_handle.rs`

```rust
#[async_trait]
pub trait KernelHandle: Send + Sync {
    // ...existing methods...
    
    /// Send message with orchestration context (backward compatible with send_to_agent)
    async fn send_to_agent_with_context(
        &self,
        agent_id: &str,
        message: &str,
        orchestration_ctx: Option<OrchestrationContext>,
    ) -> Result<String, String> {
        // Default implementation delegates to existing method
        let _ = orchestration_ctx;
        self.send_to_agent(agent_id, message).await
    }
    
    /// Spawn agent with context (backward compatible with spawn_agent)
    async fn spawn_agent_with_context(
        &self,
        manifest_toml: &str,
        parent_id: Option<&str>,
        orchestration_ctx: Option<OrchestrationContext>,
    ) -> Result<(String, String), String> {
        // Default implementation delegates to existing method
        let _ = orchestration_ctx;
        self.spawn_agent(manifest_toml, parent_id).await
    }
}
```

#### 2.3 Kernel Implementation

**File:** `crates/openfang-kernel/src/kernel.rs`

Implement the new methods in `KernelHandle` impl for `OpenFangKernel`:

```rust
async fn send_to_agent_with_context(
    &self,
    agent_id: &str,
    message: &str,
    orchestration_ctx: Option<OrchestrationContext>,
) -> Result<String, String> {
    // Store context in Message and pass through to agent_loop
    // Implementation details in kernel.rs
}
```

**Backward Compatibility:**
- Existing `send_to_agent` remains unchanged
- New methods have default implementations
- Context is optional everywhere

#### 2.4 System Prompt Injection

**File:** `crates/openfang-runtime/src/agent_loop.rs`

When building system prompt, check for `orchestration_ctx` and inject:

```rust
fn build_system_prompt(
    manifest: &AgentManifest,
    orchestration_ctx: Option<&OrchestrationContext>,
) -> String {
    let mut prompt = manifest.model.system_prompt.clone();
    
    if let Some(ctx) = orchestration_ctx {
        prompt.push_str("\n\n## Orchestration Context\n");
        prompt.push_str(&format!("You are operating as part of a larger orchestration:\n"));
        prompt.push_str(&format!("- Trace ID: {}\n", ctx.trace_id));
        prompt.push_str(&format!("- Pattern: {:?}\n", ctx.pattern));
        prompt.push_str(&format!("- Call depth: {}\n", ctx.depth));
        
        if ctx.depth > 0 {
            prompt.push_str(&format!("- Parent agent: {}\n", ctx.call_chain[ctx.call_chain.len() - 2]));
        }
        
        if !ctx.shared_vars.is_empty() {
            prompt.push_str(&format!("- Shared variables: {} available\n", ctx.shared_vars.len()));
        }
    }
    
    prompt
}
```

**Backward Compatibility:**
- Only adds content when context is present
- Doesn't affect agents without context

### Testing

```bash
# Unit tests for context propagation
cargo test -p openfang-runtime context

# Integration test: verify context flows through agent_send
cargo test -p openfang-kernel test_context_propagation

# All existing tests still pass
cargo test --workspace
```

### Success Criteria

- [x] Context creates child contexts correctly (lineage tracking works)
- [x] `agent_send` propagates context to target agent
- [x] `agent_spawn` propagates context to spawned agent
- [x] System prompt includes context info when present
- [x] Trace IDs are consistent across multi-agent calls
- [x] All existing tests pass (no regressions)
- [x] Production: existing agents unaffected; orchestration opt-in per turn/tool

---

## Phase 3: Capability-Based Agent Discovery (Week 4)

**Goal:** Enable intelligent agent selection based on capabilities and tags, without requiring manual agent ID lookup.

### Deliverables

#### 3.1 Selection Strategy Types

**File:** `crates/openfang-types/src/orchestration.rs`

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SelectionStrategy {
    RoundRobin,
    LeastBusy,
    CostEfficient,
    BestMatch,
    Random,
}
```

#### 3.2 Extend KernelHandle Trait

**File:** `crates/openfang-runtime/src/kernel_handle.rs`

```rust
#[async_trait]
pub trait KernelHandle: Send + Sync {
    // ...existing methods...
    
    /// Find agents matching capability requirements
    fn find_by_capabilities(
        &self,
        required_caps: &[Capability],
        preferred_tags: &[String],
        exclude_agents: &[AgentId],
    ) -> Vec<AgentInfo> {
        // Default: empty (not all implementations need this)
        let _ = (required_caps, preferred_tags, exclude_agents);
        vec![]
    }
    
    /// Select best agent for a task using strategy
    async fn select_agent_for_task(
        &self,
        _task_description: &str,
        required_caps: &[Capability],
        selection_strategy: SelectionStrategy,
    ) -> Result<AgentId, String> {
        // Default: error (not implemented)
        let _ = (required_caps, selection_strategy);
        Err("Agent selection not available".to_string())
    }
}
```

#### 3.3 Kernel Implementation

**File:** `crates/openfang-kernel/src/kernel.rs`

Implement capability-based discovery:

```rust
fn find_by_capabilities(
    &self,
    required_caps: &[Capability],
    preferred_tags: &[String],
    exclude_agents: &[AgentId],
) -> Vec<AgentInfo> {
    self.registry
        .list()
        .into_iter()
        .filter(|agent| {
            // Must not be in exclude list
            !exclude_agents.contains(&agent.id)
            
            // Must have ALL required capabilities
            && required_caps.iter().all(|cap| {
                agent.manifest.capabilities.grants(cap)
            })
        })
        .map(|agent| {
            // Calculate match score based on preferred tags
            let tag_matches = preferred_tags.iter()
                .filter(|tag| agent.manifest.tags.contains(*tag))
                .count();
            
            (agent, tag_matches)
        })
        .collect()
}

async fn select_agent_for_task(
    &self,
    _task_description: &str,
    required_caps: &[Capability],
    selection_strategy: SelectionStrategy,
) -> Result<AgentId, String> {
    let candidates = self.find_by_capabilities(required_caps, &[], &[]);
    
    if candidates.is_empty() {
        return Err("No agents found matching required capabilities".to_string());
    }
    
    let selected = match selection_strategy {
        SelectionStrategy::RoundRobin => {
            // Round-robin state in kernel (Arc<AtomicUsize>)
            // ... implementation
        }
        SelectionStrategy::LeastBusy => {
            // Sort by last_active timestamp (oldest = least busy)
            // ... implementation
        }
        SelectionStrategy::CostEfficient => {
            // Sort by model cost (cheapest first)
            // ... implementation
        }
        SelectionStrategy::BestMatch => {
            // First candidate (already sorted by tag matches)
            candidates[0]
        }
        SelectionStrategy::Random => {
            use rand::seq::SliceRandom;
            candidates.choose(&mut rand::thread_rng()).unwrap()
        }
    };
    
    Ok(selected.id)
}
```

#### 3.4 New Tool: agent_delegate

**File:** `crates/openfang-runtime/src/tool_runner.rs`

```rust
// In builtin_tool_definitions()
ToolDefinition {
    name: "agent_delegate".to_string(),
    description: "Intelligently delegate a task to the most appropriate agent based on capabilities and load.".to_string(),
    input_schema: serde_json::json!({
        "type": "object",
        "properties": {
            "task": { "type": "string", "description": "Task to delegate" },
            "required_capabilities": { 
                "type": "array", 
                "items": { "type": "string" },
                "description": "Required capabilities"
            },
            "preferred_tags": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Preferred agent tags"
            },
            "strategy": {
                "type": "string",
                "enum": ["round_robin", "least_busy", "cost_efficient", "best_match", "random"],
                "description": "Selection strategy (default: best_match)"
            }
        },
        "required": ["task"]
    }),
}

// In execute_tool()
"agent_delegate" => tool_agent_delegate(input, kernel, orchestration_ctx, caller_agent_id).await,

// Implementation
async fn tool_agent_delegate(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    orchestration_ctx: Option<&OrchestrationContext>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    
    let task = input["task"].as_str().ok_or("Missing 'task'")?;
    
    // Parse capabilities
    let required_caps: Vec<Capability> = input["required_capabilities"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .filter_map(|s| Capability::from_str(s).ok())
                .collect()
        })
        .unwrap_or_default();
    
    // Parse strategy
    let strategy = input["strategy"]
        .as_str()
        .and_then(|s| match s {
            "round_robin" => Some(SelectionStrategy::RoundRobin),
            "least_busy" => Some(SelectionStrategy::LeastBusy),
            "cost_efficient" => Some(SelectionStrategy::CostEfficient),
            "best_match" => Some(SelectionStrategy::BestMatch),
            "random" => Some(SelectionStrategy::Random),
            _ => None,
        })
        .unwrap_or(SelectionStrategy::BestMatch);
    
    // Select agent (see `DelegateSelectionOptions` in openfang-types::orchestration)
    let selected_agent = kh
        .select_agent_for_task(
            task,
            &required_caps,
            &[],
            strategy,
            DelegateSelectionOptions::default(),
        )
        .await?;
    
    // Create delegation context
    let child_ctx = orchestration_ctx.map(|ctx| {
        let mut child = ctx.child(selected_agent);
        child.pattern = OrchestrationPattern::Delegation {
            delegator_id: caller_agent_id.and_then(|id| AgentId::from_str(id).ok()).unwrap_or_else(AgentId::new),
            capability_required: required_caps.iter().map(|c| format!("{:?}", c)).collect::<Vec<_>>().join(", "),
        };
        child
    });
    
    // Send task to selected agent
    kh.send_to_agent_with_context(&selected_agent.to_string(), task, child_ctx).await
}
```

**Backward Compatibility:**
- New tool, doesn't affect existing tools
- All parameters except `task` are optional
- Graceful fallback if no agents match

### Testing

```bash
# Unit tests for selection strategies
cargo test -p openfang-kernel test_selection_strategy

# Integration test: delegate to agent with specific capability
cargo test -p openfang-kernel test_agent_delegate

# All existing tests still pass
cargo test --workspace
```

### Success Criteria

- [ ] `find_by_capabilities` returns correct agents
- [ ] All selection strategies work (RoundRobin, LeastBusy, etc.)
- [ ] `agent_delegate` tool successfully routes to capable agents
- [ ] Graceful errors when no agents match capabilities
- [ ] All existing tests pass
- [ ] Can deploy to production (new tool available, no disruption)

---

## Phase 4: Map-Reduce Pattern Tool (Week 5)

**Goal:** Enable parallel map-reduce orchestration within agent loops, reducing boilerplate for common parallel patterns.

### Deliverables

#### 4.1 New Tool: agent_map_reduce

**File:** `crates/openfang-runtime/src/tool_runner.rs`

```rust
// In builtin_tool_definitions()
ToolDefinition {
    name: "agent_map_reduce".to_string(),
    description: "Parallel map-reduce across multiple items.".to_string(),
    input_schema: serde_json::json!({
        "type": "object",
        "properties": {
            "items": { "type": "array", "items": { "type": "string" } },
            "map_prompt_template": { "type": "string" },
            "map_agent": { "type": "string" },
            "reduce_prompt_template": { "type": "string" },
            "reduce_agent": { "type": "string" },
            "max_parallelism": { "type": "integer", "description": "Default: 5, max: 20" }
        },
        "required": ["items", "map_prompt_template", "map_agent"]
    }),
}

// Implementation
async fn tool_agent_map_reduce(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    orchestration_ctx: Option<&OrchestrationContext>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    use futures::future::join_all;
    
    let kh = require_kernel(kernel)?;
    
    let items: Vec<String> = input["items"]
        .as_array()
        .ok_or("Missing 'items'")?
        .iter()
        .filter_map(|v| v.as_str().map(String::from))
        .collect();
    
    let map_prompt_template = input["map_prompt_template"]
        .as_str()
        .ok_or("Missing 'map_prompt_template'")?;
    
    let map_agent = input["map_agent"]
        .as_str()
        .ok_or("Missing 'map_agent'")?;
    
    let max_parallelism = input["max_parallelism"]
        .as_u64()
        .unwrap_or(5)
        .min(20) as usize;
    
    // Generate unique job ID
    let job_id = uuid::Uuid::new_v4().to_string();
    
    // Map phase: process items in parallel (batched by max_parallelism)
    let mut map_results = Vec::new();
    
    for chunk in items.chunks(max_parallelism) {
        let futures: Vec<_> = chunk.iter().enumerate().map(|(idx, item)| {
            let prompt = map_prompt_template.replace("{{item}}", item);
            let child_ctx = orchestration_ctx.map(|ctx| {
                let mut child = ctx.child(AgentId::new());  // Resolve map_agent to AgentId
                child.pattern = OrchestrationPattern::MapReduce {
                    job_id: job_id.clone(),
                    phase: MapReducePhase::Map,
                    item_index: Some(idx),
                };
                child
            });
            
            async move {
                kh.send_to_agent_with_context(map_agent, &prompt, child_ctx).await
            }
        }).collect();
        
        let results = join_all(futures).await;
        map_results.extend(results);
    }
    
    // Collect map results
    let map_outputs: Vec<String> = map_results
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;
    
    // Reduce phase (optional)
    if let Some(reduce_prompt_template) = input["reduce_prompt_template"].as_str() {
        let reduce_agent = input["reduce_agent"]
            .as_str()
            .unwrap_or("self");
        
        let combined_results = map_outputs.join("\n\n---\n\n");
        let reduce_prompt = reduce_prompt_template.replace("{{results}}", &combined_results);
        
        let reduce_ctx = orchestration_ctx.map(|ctx| {
            let mut child = ctx.child(AgentId::new());  // Resolve reduce_agent to AgentId
            child.pattern = OrchestrationPattern::MapReduce {
                job_id: job_id.clone(),
                phase: MapReducePhase::Reduce,
                item_index: None,
            };
            child
        });
        
        let reduce_result = if reduce_agent == "self" {
            // Return to caller agent (simulate by just returning the reduce prompt)
            // In real implementation, would route back through caller's agent loop
            format!("REDUCE_RESULT: {}", reduce_prompt)
        } else {
            kh.send_to_agent_with_context(reduce_agent, &reduce_prompt, reduce_ctx).await?
        };
        
        Ok(reduce_result)
    } else {
        // No reduce phase, return map results as JSON
        Ok(serde_json::to_string_pretty(&map_outputs).unwrap())
    }
}
```

**Backward Compatibility:**
- New tool, doesn't affect existing functionality
- All existing tests pass unchanged

### Testing

```bash
# Unit test: map-reduce with 5 items
cargo test -p openfang-runtime test_map_reduce

# Integration test: real agents with map-reduce
cargo test -p openfang-kernel test_map_reduce_integration

# All existing tests
cargo test --workspace
```

### Success Criteria

- [ ] Map phase executes items in parallel (respects max_parallelism)
- [ ] Reduce phase aggregates results correctly
- [ ] Orchestration context flows through map and reduce phases
- [ ] Graceful error handling when map tasks fail
- [ ] All existing tests pass
- [ ] Can deploy to production (new capability, zero disruption)

---

## Phase 5: Supervisor & Coordinator Patterns (Week 6)

**Goal:** Add supervision and coordination tools for advanced orchestration patterns.

### Deliverables

#### 5.1 New Tool: agent_supervise

**File:** `crates/openfang-runtime/src/tool_runner.rs`

```rust
ToolDefinition {
    name: "agent_supervise".to_string(),
    description: "Spawn and supervise a sub-agent. Monitor progress and enforce timeout.".to_string(),
    input_schema: serde_json::json!({
        "type": "object",
        "properties": {
            "agent_id": { "type": "string" },
            "task": { "type": "string" },
            "check_interval_secs": { "type": "integer", "description": "Default: 30" },
            "max_duration_secs": { "type": "integer", "description": "Default: 600" },
            "success_criteria": { "type": "string" },
            "intervention_allowed": { "type": "boolean", "description": "Default: false" }
        },
        "required": ["agent_id", "task"]
    }),
}

// Implementation with timeout, progress checking, optional intervention
async fn tool_agent_supervise(...) -> Result<String, String> {
    // Implementation details
}
```

#### 5.2 New Tool: agent_coordinate

**File:** `crates/openfang-runtime/src/tool_runner.rs`

```rust
ToolDefinition {
    name: "agent_coordinate".to_string(),
    description: "Coordinate multiple agents with dependencies (dynamic workflow).".to_string(),
    input_schema: serde_json::json!({
        "type": "object",
        "properties": {
            "tasks": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string" },
                        "agent": { "type": "string" },
                        "prompt": { "type": "string" },
                        "depends_on": { "type": "array", "items": { "type": "string" } }
                    }
                }
            },
            "strategy": {
                "type": "string",
                "enum": ["sequential", "parallel_where_possible"],
                "description": "Default: parallel_where_possible"
            }
        },
        "required": ["tasks"]
    }),
}

// Implementation with topological sort for dependency resolution
async fn tool_agent_coordinate(...) -> Result<String, String> {
    // Build dependency graph
    // Topological sort
    // Execute in parallel where possible
    // Implementation details
}
```

**Backward Compatibility:**
- New tools, additive only
- No changes to existing functionality

### Testing

```bash
# Unit tests
cargo test -p openfang-runtime test_supervise
cargo test -p openfang-runtime test_coordinate

# Integration tests
cargo test -p openfang-kernel test_supervisor_intervention
cargo test -p openfang-kernel test_coordination_parallel

# All tests
cargo test --workspace
```

### Success Criteria

- [ ] Supervisor monitors progress and enforces timeout
- [ ] Coordinator resolves dependencies correctly
- [ ] Parallel execution where dependencies allow
- [ ] All existing tests pass
- [ ] Production ready

---

## Phase 6: Enhanced Workflow Integration (Week 7-8)

**Goal:** Add Adaptive step mode and advanced aggregation strategies to workflows.

### Deliverables

#### 6.0 Workflow orchestration context (shipped with §1 / §4)

**Files:** `crates/openfang-kernel/src/kernel.rs` (`orchestration_context_for_workflow_step`, `run_workflow`), `crates/openfang-kernel/src/workflow.rs` (`execute_run` sender arity includes **`step_index`**), `crates/openfang-api/src/channel_bridge.rs` (`run_workflow_text`).

- Each LLM-driven workflow step runs with **`OrchestrationPattern::Workflow`** and a per-run **`trace_id`** (`wf:{workflow_uuid}:run:{run_uuid}`).
- Optional **`[runtime_limits] orchestration_default_budget_ms`** fills **`remaining_budget_ms`** when building the step context.

#### 6.1 Adaptive Step Mode

**File:** `crates/openfang-kernel/src/workflow.rs`

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum StepMode {
    Sequential,
    FanOut,
    Collect,
    Conditional { condition: String },
    Loop { max_iterations: u32, until: String },
    
    Adaptive {
        max_iterations: u32,
        tool_allowlist: Option<Vec<String>>,
        allow_subagents: bool,
        max_tokens: Option<u64>,
    },
}

// Execution logic in WorkflowEngine::execute_step
```

#### 6.2 Aggregation Strategies

**File:** `crates/openfang-kernel/src/workflow.rs`

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AggregationStrategy {
    Concatenate { separator: String },
    JsonArray,
    Consensus { threshold: f32 },
    BestOf { evaluator_agent: String, criteria: String },
    Summarize { summarizer_agent: String, max_length: Option<usize> },
    Custom { aggregator_agent: String, aggregation_prompt: String },
}

// Update Collect mode
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StepMode {
    // ...
    Collect {
        #[serde(default = "default_aggregation")]
        strategy: AggregationStrategy,
    },
}
```

**Backward Compatibility:**
- `#[serde(default)]` ensures old workflows deserialize correctly
- Existing `Collect` steps work unchanged (default to concatenation)

### Testing

```bash
# Workflow with Adaptive mode
cargo test -p openfang-kernel test_adaptive_workflow_step

# Workflow with Consensus aggregation
cargo test -p openfang-kernel test_consensus_aggregation

# Backward compatibility: old workflows still work
cargo test -p openfang-kernel test_workflow_backward_compat

# All tests
cargo test --workspace
```

### Success Criteria

- [ ] Adaptive mode executes multi-iteration agent loop within workflow
- [ ] All aggregation strategies work correctly
- [ ] Existing workflows load and execute unchanged
- [ ] All tests pass
- [ ] Production ready

---

## Phase 7: Resource Quotas (Week 9)

**Goal:** Add hierarchical resource management for sub-agents.

### Deliverables

#### 7.1 Resource Quota Types

**File:** `crates/openfang-types/src/agent.rs`

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentManifest {
    // ...existing fields...
    
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_quota: Option<ResourceQuota>,
    
    #[serde(default)]
    pub inherit_parent_quota: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceQuota {
    pub max_tokens_per_hour: Option<u64>,
    pub max_subagents: Option<u32>,
    pub max_cost_per_hour: Option<f64>,
    pub max_depth: Option<u32>,
    pub max_iterations: Option<u32>,
}
```

#### 7.2 Quota Enforcement

**File:** `crates/openfang-runtime/src/tool_runner.rs`

```rust
// In tool_agent_send and tool_agent_spawn:
// Check quota before executing
if let Some(ctx) = orchestration_ctx {
    // Check depth limit
    // Check token budget
    // Check subagent count
    // Return QuotaExceeded error if over limit
}
```

#### 7.3 Quota Tracking API

**File:** `crates/openfang-api/src/routes.rs`

```rust
// GET /api/orchestration/quota-tree/:agent_id
async fn get_quota_tree(
    Path(agent_id): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<QuotaTree>, StatusCode> {
    // Return hierarchical quota consumption
}
```

**Backward Compatibility:**
- Quotas are optional (None = no limits, current behavior)
- Only enforced when explicitly configured
- `#[serde(skip_serializing_if)]` ensures old manifests compatible

### Testing

```bash
# Quota enforcement tests
cargo test -p openfang-runtime test_quota_exceeded

# Quota inheritance tests
cargo test -p openfang-kernel test_quota_inheritance

# All tests
cargo test --workspace
```

### Success Criteria

- [ ] Quota checked before expensive operations
- [ ] QuotaExceeded errors returned when limits hit
- [ ] Inheritance works correctly
- [ ] Dashboard shows quota tree
- [ ] All tests pass
- [ ] Production ready

---

## Phase 8: Observability & Tracing (Week 10-11)

**Goal:** Add distributed tracing and orchestration debugging tools.

### Deliverables

#### 8.1 Trace Event Types

**File:** `crates/openfang-types/src/event.rs`

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EventPayload {
    // ...existing variants...
    OrchestrationTrace(OrchestrationTraceEvent),
}

// Full OrchestrationTraceEvent and TraceEventType definitions
```

#### 8.2 Trace Collection in Agent Loop

**File:** `crates/openfang-runtime/src/agent_loop.rs`

```rust
// Emit trace events at key points:
// - OrchestrationStart (when context present)
// - AgentDelegated (before agent_send)
// - AgentCompleted (after successful return)
// - AgentFailed (on error)
// - OrchestrationComplete (at end of root agent)
```

#### 8.3 Trace API Endpoints

**File:** `crates/openfang-api/src/routes.rs`

```rust
// GET /api/orchestration/traces
// GET /api/orchestration/traces/:trace_id
// GET /api/orchestration/traces/:trace_id/tree
// GET /api/orchestration/traces/:trace_id/cost
```

#### 8.4 Dashboard Page

**File:** `crates/openfang-api/static/js/pages/orchestration-traces.js` (new file)

- Tree view component
- Timeline view component
- Cost breakdown chart
- Filters (trace_id, date, pattern)

**File:** `crates/openfang-api/static/index_body.html`

- Add `#orchestration-traces` navigation link
- Add page container

#### 8.5 CLI Commands

**File:** `crates/openfang-cli/src/main.rs`

```rust
// Add orchestration subcommand
#[derive(Subcommand)]
enum Commands {
    // ...existing...
    
    Orchestration {
        #[command(subcommand)]
        action: OrchestrationAction,
    },
}

#[derive(Subcommand)]
enum OrchestrationAction {
    List,
    Trace { trace_id: String },
    Cost { trace_id: String },
    Export { trace_id: String, output: String },
    Watch,
}
```

**Backward Compatibility:**
- Trace events are additive (don't affect existing events)
- Dashboard page is new (doesn't change existing pages)
- CLI subcommand is new (doesn't affect existing commands)

### Testing

```bash
# Trace collection tests
cargo test -p openfang-runtime test_trace_collection

# API endpoint tests
cargo test -p openfang-api test_orchestration_traces_api

# Dashboard smoke test
./scripts/verify-dashboard-smoke.sh

# All tests
cargo test --workspace
```

### Success Criteria

- [ ] Trace events emitted for all orchestrations
- [ ] API endpoints return correct trace data
- [ ] Dashboard displays traces correctly
- [ ] CLI commands work
- [ ] All tests pass
- [ ] Production ready

---

## Phase 9: Documentation & Examples (Week 12)

**Goal:** Comprehensive documentation and example workflows.

### Deliverables

#### 9.1 Updated Documentation

**Files (as shipped; names differ from original sketch):**
- **`docs/orchestration-guide.md`** — CLI, dashboard, API index
- **`docs/orchestration-walkthrough.md`** — hands-on traces + workflows + optional queue
- **`docs/agent-orchestration-design.md`** — design + implementation status
- **`docs/orchestration-implementation-audit.md`** — audit checklist
- **`docs/task-queue-orchestration.md`** — sticky `trace_id` + claim rehydration
- **`docs/workflows.md`**, **`docs/workflow-examples.md`**, **`docs/api-reference.md`** — workflows REST + orchestration endpoints

#### 9.2 Example Workflows

**Directory:** `docs/examples/workflows/`

```
orchestration/
├── map-reduce-research.json
├── consensus-code-review.json
├── adaptive-deep-dive.json
├── coordinated-pipeline.json
└── supervised-long-task.json
```

#### 9.3 Tutorial

**File:** `docs/tutorials/building-orchestrations.md`

Step-by-step guide:
1. Simple delegation
2. Map-reduce pattern
3. Multi-agent coordination
4. Adaptive workflows
5. Debugging with traces

### Success Criteria

- [x] Core orchestration + workflow features documented (guides above)
- [x] **`docs/workflow-examples.md`** recipes align with REST shape
- [ ] Optional: dedicated **`docs/tutorials/building-orchestrations.md`** (not created; walkthrough covers much of this)
- [x] API reference includes orchestration + workflow routes
- [x] Ready for operator adoption (polish ongoing)

---

## Deployment Strategy

### Per-Phase Deployment

Each phase can be deployed independently:

1. **Merge to main** after phase completes and passes tests
2. **Deploy to staging** for integration testing
3. **Deploy to production** after 48h staging soak
4. **Monitor** for regressions (existing agent behavior unchanged)
5. **Enable for beta users** (opt-in features)
6. **General availability** after feedback

### Rollback Plan

Each phase is independently rollback-able:

- Phase N fails → revert Phase N commit
- Earlier phases remain deployed
- No cascading failures (backward compatibility)

### Feature Flags (Optional)

For extra safety, wrap new tools in feature flags:

```rust
// In config.toml
[experimental_features]
agent_delegate = true
agent_map_reduce = true
agent_supervise = false  # Not ready yet
```

Check flag before registering tool:

```rust
if config.experimental_features.agent_delegate {
    tools.push(agent_delegate_definition());
}
```

---

## Summary Timeline

| Phase | Weeks | Deliverable | Deploy Risk |
|-------|-------|-------------|-------------|
| 1 | 1-2 | Orchestration context types | None (no-op) |
| 2 | 3 | Context propagation | Very Low |
| 3 | 4 | Capability discovery + agent_delegate | Low |
| 4 | 5 | agent_map_reduce tool | Low |
| 5 | 6 | agent_supervise + agent_coordinate | Medium |
| 6 | 7-8 | Workflow Adaptive mode + aggregation | Medium |
| 7 | 9 | Resource quotas | Low |
| 8 | 10-11 | Observability + tracing | Low |
| 9 | 12 | Documentation + examples | None |

**Total:** 12 weeks for complete implementation

**Minimum Viable Product (MVP):** Phases 1-4 (5 weeks)
- Provides context, delegation, and map-reduce
- Immediately useful for most orchestration needs
- Can ship without later phases

---

## Risk Mitigation

### Technical Risks

1. **Performance overhead from context**
   - Mitigation: Benchmark each phase, keep context <1KB
   - Rollback: Context is optional, can disable

2. **Complexity in coordination tool**
   - Mitigation: Start with simple dependency resolution
   - Rollback: Tool is optional, doesn't affect existing code

3. **Trace storage memory pressure**
   - Mitigation: Ring buffer with 1000 event limit
   - Rollback: Traces are in-memory, restart clears

### Adoption Risks

1. **Users don't understand new tools**
   - Mitigation: Comprehensive docs and examples (Phase 9)
   - Training sessions for beta users

2. **Quotas too restrictive**
   - Mitigation: Quotas are opt-in, generous defaults
   - Easy to adjust per-agent

---

## Success Metrics (Cumulative)

Track across all phases:

| Metric | Target | Measurement |
|--------|--------|-------------|
| Backward compatibility | 100% existing tests pass | CI |
| Performance overhead | <5% latency increase | Benchmarks |
| Adoption rate | 30% agents use new tools in 3 months | Telemetry |
| Bug reports | <5 critical bugs | GitHub issues |
| Documentation coverage | 100% features documented | Review |

---

**Next Steps:**
1. Use this document as a **checklist map** when changing orchestration or workflows (which phase does a PR touch?).
2. Keep **`docs/agent-orchestration-design.md`** and **`docs/orchestration-implementation-audit.md`** in sync with kernel/runtime changes.
3. Optional backlog: dedicated long-form tutorial file, SVG trace graph, stress/E2E CI jobs.
