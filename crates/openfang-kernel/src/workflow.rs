//! Workflow engine — multi-step agent pipeline execution.
//!
//! A workflow defines a sequence of steps where each step routes
//! a task to a specific agent. Steps can:
//! - Pass their output as input to the next step
//! - Run in sequence (pipeline) or in parallel (fan-out)
//! - Conditionally skip based on previous output
//! - Loop until a condition is met
//! - Store outputs in named variables for later reference
//!
//! Workflows are defined as Rust structs or loaded from JSON.

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use openfang_types::agent::AgentId;
use openfang_types::runtime_limits::WorkflowRetentionLimits;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Unique identifier for a workflow definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkflowId(pub Uuid);

impl WorkflowId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for WorkflowId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for WorkflowId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Unique identifier for a running workflow instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkflowRunId(pub Uuid);

impl WorkflowRunId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for WorkflowRunId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for WorkflowRunId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A workflow definition — a named sequence of steps.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    /// Unique identifier.
    pub id: WorkflowId,
    /// Human-readable name.
    pub name: String,
    /// Description of what this workflow does.
    pub description: String,
    /// The steps in execution order.
    pub steps: Vec<WorkflowStep>,
    /// Created at.
    pub created_at: DateTime<Utc>,
}

/// A single step in a workflow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStep {
    /// Step name for logging/display.
    pub name: String,
    /// Which agent to route this step to.
    pub agent: StepAgent,
    /// The prompt template. Use `{{input}}` for previous output, `{{var_name}}` for variables.
    pub prompt_template: String,
    /// Execution mode for this step.
    pub mode: StepMode,
    /// Maximum time for this step in seconds (default: 120).
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    /// Error handling mode for this step (default: Fail).
    #[serde(default)]
    pub error_mode: ErrorMode,
    /// Optional variable name to store this step's output in.
    #[serde(default)]
    pub output_var: Option<String>,
    /// When `mode` is [`StepMode::Collect`], how to merge preceding fan-out outputs.
    #[serde(default)]
    pub collect_aggregation: Option<AggregationStrategy>,
}

fn default_timeout() -> u64 {
    120
}

fn default_allow_subagents() -> bool {
    true
}

/// How to aggregate fan-out outputs in a `Collect` step (design: `docs/agent-orchestration-design.md` §5).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AggregationStrategy {
    /// Join with separator (default workflow collect behavior).
    Concatenate { separator: String },
    /// JSON array of fan-out strings.
    JsonArray,
    /// Majority / plurality vote on normalized responses (`threshold` 0.0–1.0 = minimum fraction).
    Consensus { threshold: f32 },
    /// Evaluator agent picks the best fan-out output (1-based index in reply).
    BestOf {
        evaluator_agent: String,
        criteria: String,
    },
    /// Summarizer agent merges all fan-out outputs into one text.
    Summarize {
        summarizer_agent: String,
        max_length: Option<usize>,
    },
    Custom {
        aggregator_agent: String,
        aggregation_prompt: String,
    },
}

impl Default for AggregationStrategy {
    fn default() -> Self {
        Self::Concatenate {
            separator: "\n\n---\n\n".to_string(),
        }
    }
}

/// Overrides for one workflow step running in `Adaptive` mode (full agent loop limits).
#[derive(Debug, Clone, Default)]
pub struct AdaptiveWorkflowOverrides {
    pub max_iterations: u32,
    pub tool_allowlist: Option<Vec<String>>,
    pub allow_subagents: bool,
    pub max_tokens: Option<u64>,
}

/// How to identify the agent for a step.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StepAgent {
    /// Reference an agent by UUID.
    ById { id: String },
    /// Reference an agent by name (first match).
    ByName { name: String },
}

/// Execution mode for a workflow step.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepMode {
    /// Execute sequentially — this step runs after the previous completes.
    #[default]
    Sequential,
    /// Fan-out — this step runs in parallel with subsequent FanOut steps until Collect.
    FanOut,
    /// Collect results from all preceding fan-out steps.
    Collect,
    /// Conditional — skip this step if previous output doesn't contain `condition` (case-insensitive).
    Conditional { condition: String },
    /// Loop — repeat this step until output contains `until` or `max_iterations` reached.
    Loop { max_iterations: u32, until: String },
    /// Adaptive — one user message with overridden loop limits / tools (full agent turn).
    Adaptive {
        max_iterations: u32,
        #[serde(default)]
        tool_allowlist: Option<Vec<String>>,
        #[serde(default = "default_allow_subagents")]
        allow_subagents: bool,
        #[serde(default)]
        max_tokens: Option<u64>,
    },
}

/// Error handling mode for a workflow step.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorMode {
    /// Abort the workflow on error (default).
    #[default]
    Fail,
    /// Skip this step on error and continue.
    Skip,
    /// Retry the step up to N times before failing.
    Retry { max_retries: u32 },
}

/// The current state of a workflow run.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowRunState {
    Pending,
    Running,
    Completed,
    Failed,
}

/// A running workflow instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowRun {
    /// Run instance ID.
    pub id: WorkflowRunId,
    /// The workflow being run.
    pub workflow_id: WorkflowId,
    /// Workflow name (copied for quick access).
    pub workflow_name: String,
    /// Initial input to the workflow.
    pub input: String,
    /// Current state.
    pub state: WorkflowRunState,
    /// Results from each completed step.
    pub step_results: Vec<StepResult>,
    /// Final output (set when workflow completes).
    pub output: Option<String>,
    /// Error message if failed.
    pub error: Option<String>,
    /// Started at.
    pub started_at: DateTime<Utc>,
    /// Completed at.
    pub completed_at: Option<DateTime<Utc>>,
}

/// Result from a single workflow step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepResult {
    /// Step name.
    pub step_name: String,
    /// Agent that executed this step.
    pub agent_id: String,
    /// Agent name.
    pub agent_name: String,
    /// Output from this step.
    pub output: String,
    /// Token usage.
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Duration in milliseconds.
    pub duration_ms: u64,
}

/// The workflow engine — manages definitions and executes pipeline runs.
pub struct WorkflowEngine {
    /// Registered workflow definitions.
    workflows: Arc<RwLock<HashMap<WorkflowId, Workflow>>>,
    /// Active and completed workflow runs.
    runs: Arc<RwLock<HashMap<WorkflowRunId, WorkflowRun>>>,
}

impl WorkflowEngine {
    /// Create a new workflow engine.
    pub fn new() -> Self {
        Self {
            workflows: Arc::new(RwLock::new(HashMap::new())),
            runs: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a new workflow definition.
    pub async fn register(&self, workflow: Workflow) -> WorkflowId {
        let id = workflow.id;
        self.workflows.write().await.insert(id, workflow);
        info!(workflow_id = %id, "Workflow registered");
        id
    }

    /// List all registered workflows.
    pub async fn list_workflows(&self) -> Vec<Workflow> {
        self.workflows.read().await.values().cloned().collect()
    }

    /// Get a specific workflow by ID.
    pub async fn get_workflow(&self, id: WorkflowId) -> Option<Workflow> {
        self.workflows.read().await.get(&id).cloned()
    }

    /// Remove a workflow definition.
    pub async fn remove_workflow(&self, id: WorkflowId) -> bool {
        self.workflows.write().await.remove(&id).is_some()
    }

    /// Update an existing workflow definition.
    ///
    /// Preserves the original `id` and `created_at`. Replaces `name`,
    /// `description`, and `steps`. Returns `true` if the workflow was
    /// found and updated.
    pub async fn update_workflow(&self, id: WorkflowId, updated: Workflow) -> bool {
        let mut workflows = self.workflows.write().await;
        if let Some(existing) = workflows.get_mut(&id) {
            existing.name = updated.name;
            existing.description = updated.description;
            existing.steps = updated.steps;
            info!(workflow_id = %id, "Workflow updated");
            true
        } else {
            false
        }
    }

    /// Start a workflow run. Returns the run ID and a handle to check progress.
    ///
    /// The actual execution is driven externally by calling `execute_run()`
    /// with the kernel handle, since the workflow engine doesn't own the kernel.
    pub async fn create_run(
        &self,
        workflow_id: WorkflowId,
        input: String,
        retention: WorkflowRetentionLimits,
    ) -> Option<WorkflowRunId> {
        let workflow = self.workflows.read().await.get(&workflow_id)?.clone();
        let run_id = WorkflowRunId::new();

        let run = WorkflowRun {
            id: run_id,
            workflow_id,
            workflow_name: workflow.name,
            input,
            state: WorkflowRunState::Pending,
            step_results: Vec::new(),
            output: None,
            error: None,
            started_at: Utc::now(),
            completed_at: None,
        };

        let mut runs = self.runs.write().await;
        runs.insert(run_id, run);

        let now = Utc::now();
        if let Some(ttl_secs) = retention.run_ttl_secs {
            runs.retain(|rid, r| {
                if !matches!(
                    r.state,
                    WorkflowRunState::Completed | WorkflowRunState::Failed
                ) {
                    return true;
                }
                let anchor = r.completed_at.unwrap_or(r.started_at);
                let age = now.signed_duration_since(anchor);
                if age > ChronoDuration::seconds(ttl_secs as i64) {
                    debug!(run_id = %rid, ttl_secs, "TTL eviction workflow run");
                    false
                } else {
                    true
                }
            });
        }

        // Evict oldest completed/failed runs when we exceed the cap
        if runs.len() > retention.max_retained_runs {
            let mut evictable: Vec<(WorkflowRunId, DateTime<Utc>)> = runs
                .iter()
                .filter(|(_, r)| {
                    matches!(
                        r.state,
                        WorkflowRunState::Completed | WorkflowRunState::Failed
                    )
                })
                .map(|(id, r)| (*id, r.started_at))
                .collect();

            // Sort oldest first
            evictable.sort_by_key(|(_, t)| *t);

            let to_remove = runs.len() - retention.max_retained_runs;
            for (id, _) in evictable.into_iter().take(to_remove) {
                runs.remove(&id);
                debug!(run_id = %id, "Evicted old workflow run (count cap)");
            }
        }

        Some(run_id)
    }

    /// Get the current state of a workflow run.
    pub async fn get_run(&self, run_id: WorkflowRunId) -> Option<WorkflowRun> {
        self.runs.read().await.get(&run_id).cloned()
    }

    /// List all workflow runs (optionally filtered by state).
    pub async fn list_runs(&self, state_filter: Option<&str>) -> Vec<WorkflowRun> {
        self.runs
            .read()
            .await
            .values()
            .filter(|r| {
                state_filter
                    .map(|f| match f {
                        "pending" => matches!(r.state, WorkflowRunState::Pending),
                        "running" => matches!(r.state, WorkflowRunState::Running),
                        "completed" => matches!(r.state, WorkflowRunState::Completed),
                        "failed" => matches!(r.state, WorkflowRunState::Failed),
                        _ => true,
                    })
                    .unwrap_or(true)
            })
            .cloned()
            .collect()
    }

    /// Replace `{{var_name}}` references in a template with stored variable values.
    fn expand_variables(template: &str, input: &str, vars: &HashMap<String, String>) -> String {
        let mut result = template.replace("{{input}}", input);
        for (key, value) in vars {
            result = result.replace(&format!("{{{{{key}}}}}"), value);
        }
        result
    }

    /// Numbered list of candidate outputs (1-based) for evaluator prompts.
    fn format_numbered_candidates(outputs: &[String]) -> String {
        outputs
            .iter()
            .enumerate()
            .map(|(i, s)| format!("{}. {}", i + 1, s))
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    fn best_of_prompt(criteria: &str, outputs: &[String]) -> String {
        let body = Self::format_numbered_candidates(outputs);
        format!(
            "You are evaluating multiple candidate responses.\n\n\
             Criteria:\n{criteria}\n\n\
             Candidates:\n\n{body}\n\n\
             Reply with ONLY the number (1 to {}) of the single best candidate. No other text.",
            outputs.len()
        )
    }

    fn summarize_prompt(max_length: Option<usize>, outputs: &[String]) -> String {
        let numbered = Self::format_numbered_candidates(outputs);
        let limit = match max_length {
            Some(n) => format!("Keep the summary under approximately {n} characters."),
            None => String::new(),
        };
        format!("Summarize the following texts into one coherent response. {limit}\n\n{numbered}")
    }

    /// Expands `{{outputs}}`, `{{outputs_json}}`, `{{input}}`, and `{{var}}` placeholders.
    fn expand_custom_aggregation_prompt(
        template: &str,
        outputs: &[String],
        workflow_initial: &str,
        vars: &HashMap<String, String>,
    ) -> String {
        let outputs_json = serde_json::to_string(outputs).unwrap_or_else(|_| "[]".to_string());
        let outputs_block = Self::format_numbered_candidates(outputs);
        let mut result = template
            .replace("{{outputs_json}}", &outputs_json)
            .replace("{{outputs}}", &outputs_block)
            .replace("{{input}}", workflow_initial);
        for (key, value) in vars {
            result = result.replace(&format!("{{{{{key}}}}}"), value);
        }
        result
    }

    /// Parse a 1-based candidate index from the evaluator reply.
    fn parse_best_of_choice(reply: &str, n: usize) -> Result<usize, String> {
        if n == 0 {
            return Err("no candidates for BestOf".to_string());
        }
        let trimmed = reply.trim();
        for token in trimmed.split_whitespace() {
            let t = token.trim_matches(|c: char| ".,;:!?()[]{}\"'".contains(c));
            if let Ok(v) = t.parse::<usize>() {
                if (1..=n).contains(&v) {
                    return Ok(v - 1);
                }
            }
        }
        for word in trimmed.split(|c: char| !c.is_ascii_digit()) {
            if word.is_empty() {
                continue;
            }
            if let Ok(v) = word.parse::<usize>() {
                if (1..=n).contains(&v) {
                    return Ok(v - 1);
                }
            }
        }
        Err(format!(
            "could not parse candidate index 1..{n} from evaluator reply: {reply:?}"
        ))
    }

    fn synth_aggregate_step(
        collect_step: &WorkflowStep,
        agent: StepAgent,
        prompt: String,
    ) -> WorkflowStep {
        WorkflowStep {
            name: format!("{}→aggregate", collect_step.name),
            agent,
            prompt_template: prompt,
            mode: StepMode::Sequential,
            timeout_secs: collect_step.timeout_secs,
            error_mode: collect_step.error_mode.clone(),
            output_var: None,
            collect_aggregation: None,
        }
    }

    /// Merge fan-out step outputs for a `Collect` step (pure strategies only).
    ///
    /// For [`AggregationStrategy::BestOf`], [`AggregationStrategy::Summarize`], and
    /// [`AggregationStrategy::Custom`], use the async collect path in [`Self::execute_run`].
    pub fn apply_aggregation(
        strategy: &AggregationStrategy,
        outputs: &[String],
    ) -> Result<String, String> {
        if outputs.is_empty() {
            return Ok(String::new());
        }
        match strategy {
            AggregationStrategy::Concatenate { separator } => Ok(outputs.join(separator)),
            AggregationStrategy::JsonArray => {
                serde_json::to_string(outputs).map_err(|e| e.to_string())
            }
            AggregationStrategy::Consensus { threshold } => {
                let t = threshold.clamp(0.0, 1.0);
                let n = outputs.len();
                let mut counts: HashMap<String, usize> = HashMap::new();
                for o in outputs {
                    let k = o.trim().to_string();
                    *counts.entry(k).or_insert(0) += 1;
                }
                let need = ((n as f32) * t).ceil().max(1.0) as usize;
                let mut best: Option<(String, usize)> = None;
                for (s, c) in counts {
                    let better = best.as_ref().map(|(_, bc)| c > *bc).unwrap_or(true);
                    if better {
                        best = Some((s, c));
                    }
                }
                if let Some((s, c)) = best {
                    if c >= need {
                        return Ok(s);
                    }
                }
                Ok(outputs.join("\n"))
            }
            AggregationStrategy::BestOf { .. }
            | AggregationStrategy::Summarize { .. }
            | AggregationStrategy::Custom { .. } => Err(
                "apply_aggregation() supports only concatenate/json_array/consensus; use agent collect path"
                    .to_string(),
            ),
        }
    }

    /// Execute a single step with error mode handling. Returns (output, input_tokens, output_tokens).
    async fn execute_step_with_error_mode<F, Fut>(
        step: &WorkflowStep,
        step_index: usize,
        agent_id: AgentId,
        prompt: String,
        send_message: &mut F,
    ) -> Result<Option<(String, u64, u64)>, String>
    where
        F: FnMut(AgentId, String, &WorkflowStep, usize) -> Fut,
        Fut: std::future::Future<Output = Result<(String, u64, u64), String>>,
    {
        let timeout_dur = std::time::Duration::from_secs(step.timeout_secs);

        match &step.error_mode {
            ErrorMode::Fail => {
                let result = tokio::time::timeout(
                    timeout_dur,
                    send_message(agent_id, prompt, step, step_index),
                )
                .await
                .map_err(|_| {
                    format!(
                        "Step '{}' timed out after {}s",
                        step.name, step.timeout_secs
                    )
                })?
                .map_err(|e| format!("Step '{}' failed: {}", step.name, e))?;
                Ok(Some(result))
            }
            ErrorMode::Skip => {
                match tokio::time::timeout(
                    timeout_dur,
                    send_message(agent_id, prompt, step, step_index),
                )
                .await
                {
                    Ok(Ok(result)) => Ok(Some(result)),
                    Ok(Err(e)) => {
                        warn!("Step '{}' failed (skipping): {e}", step.name);
                        Ok(None)
                    }
                    Err(_) => {
                        warn!(
                            "Step '{}' timed out (skipping) after {}s",
                            step.name, step.timeout_secs
                        );
                        Ok(None)
                    }
                }
            }
            ErrorMode::Retry { max_retries } => {
                let mut last_err = String::new();
                for attempt in 0..=*max_retries {
                    match tokio::time::timeout(
                        timeout_dur,
                        send_message(agent_id, prompt.clone(), step, step_index),
                    )
                    .await
                    {
                        Ok(Ok(result)) => return Ok(Some(result)),
                        Ok(Err(e)) => {
                            last_err = e.to_string();
                            if attempt < *max_retries {
                                warn!(
                                    "Step '{}' attempt {} failed: {e}, retrying",
                                    step.name,
                                    attempt + 1
                                );
                            }
                        }
                        Err(_) => {
                            last_err = format!("timed out after {}s", step.timeout_secs);
                            if attempt < *max_retries {
                                warn!(
                                    "Step '{}' attempt {} timed out, retrying",
                                    step.name,
                                    attempt + 1
                                );
                            }
                        }
                    }
                }
                Err(format!(
                    "Step '{}' failed after {} retries: {last_err}",
                    step.name, max_retries
                ))
            }
        }
    }

    /// Execute a workflow run step-by-step.
    ///
    /// This method takes a closure that sends messages to agents,
    /// so the workflow engine remains decoupled from the kernel.
    ///
    /// The closure receives `step_index`: the index of `step` in the workflow definition
    /// (`0..steps.len()`), including fan-out branches (each fan-out step uses its own index).
    pub async fn execute_run<F, Fut>(
        &self,
        run_id: WorkflowRunId,
        agent_resolver: impl Fn(&StepAgent) -> Option<(AgentId, String)>,
        mut send_message: F,
    ) -> Result<String, String>
    where
        F: FnMut(AgentId, String, &WorkflowStep, usize) -> Fut,
        Fut: std::future::Future<Output = Result<(String, u64, u64), String>>,
    {
        // Get the run and workflow
        let (workflow, input) = {
            let mut runs = self.runs.write().await;
            let run = runs.get_mut(&run_id).ok_or("Workflow run not found")?;
            run.state = WorkflowRunState::Running;

            let workflow = self
                .workflows
                .read()
                .await
                .get(&run.workflow_id)
                .ok_or("Workflow definition not found")?
                .clone();

            (workflow, run.input.clone())
        };

        info!(
            run_id = %run_id,
            workflow = %workflow.name,
            steps = workflow.steps.len(),
            "Starting workflow execution"
        );

        let run_initial_input = input.clone();
        let mut current_input = input;
        let mut all_outputs: Vec<String> = Vec::new();
        let mut variables: HashMap<String, String> = HashMap::new();
        let mut i = 0;

        while i < workflow.steps.len() {
            let step = &workflow.steps[i];

            debug!(
                step = i + 1,
                name = %step.name,
                "Executing workflow step"
            );

            match &step.mode {
                StepMode::Sequential | StepMode::Adaptive { .. } => {
                    let (agent_id, agent_name) = agent_resolver(&step.agent)
                        .ok_or_else(|| format!("Agent not found for step '{}'", step.name))?;

                    let prompt =
                        Self::expand_variables(&step.prompt_template, &current_input, &variables);

                    let start = std::time::Instant::now();
                    let result = Self::execute_step_with_error_mode(
                        step,
                        i,
                        agent_id,
                        prompt,
                        &mut send_message,
                    )
                    .await;
                    let duration_ms = start.elapsed().as_millis() as u64;

                    match result {
                        Ok(Some((output, input_tokens, output_tokens))) => {
                            let step_result = StepResult {
                                step_name: step.name.clone(),
                                agent_id: agent_id.to_string(),
                                agent_name,
                                output: output.clone(),
                                input_tokens,
                                output_tokens,
                                duration_ms,
                            };
                            if let Some(r) = self.runs.write().await.get_mut(&run_id) {
                                r.step_results.push(step_result);
                            }

                            if let Some(ref var) = step.output_var {
                                variables.insert(var.clone(), output.clone());
                            }

                            all_outputs.push(output.clone());
                            current_input = output;
                            info!(step = i + 1, name = %step.name, duration_ms, "Step completed");
                        }
                        Ok(None) => {
                            // Step was skipped (ErrorMode::Skip)
                            info!(step = i + 1, name = %step.name, "Step skipped");
                        }
                        Err(e) => {
                            if let Some(r) = self.runs.write().await.get_mut(&run_id) {
                                r.state = WorkflowRunState::Failed;
                                r.error = Some(e.clone());
                                r.completed_at = Some(Utc::now());
                            }
                            return Err(e);
                        }
                    }
                }

                StepMode::FanOut => {
                    // Collect consecutive FanOut steps and run them in parallel
                    let mut fan_out_steps = vec![(i, step)];
                    let mut j = i + 1;
                    while j < workflow.steps.len() {
                        if matches!(workflow.steps[j].mode, StepMode::FanOut) {
                            fan_out_steps.push((j, &workflow.steps[j]));
                            j += 1;
                        } else {
                            break;
                        }
                    }

                    // Build all futures
                    let mut futures = Vec::new();
                    let mut step_infos = Vec::new();

                    for (idx, fan_step) in &fan_out_steps {
                        let (agent_id, agent_name) =
                            agent_resolver(&fan_step.agent).ok_or_else(|| {
                                format!("Agent not found for step '{}'", fan_step.name)
                            })?;
                        let prompt = Self::expand_variables(
                            &fan_step.prompt_template,
                            &current_input,
                            &variables,
                        );
                        let timeout_dur = std::time::Duration::from_secs(fan_step.timeout_secs);

                        step_infos.push((*idx, fan_step.name.clone(), agent_id, agent_name));
                        futures.push(tokio::time::timeout(
                            timeout_dur,
                            send_message(agent_id, prompt, fan_step, *idx),
                        ));
                    }

                    let start = std::time::Instant::now();
                    let results = futures::future::join_all(futures).await;
                    let duration_ms = start.elapsed().as_millis() as u64;

                    for (k, result) in results.into_iter().enumerate() {
                        let (_, ref step_name, agent_id, ref agent_name) = step_infos[k];
                        let fan_step = fan_out_steps[k].1;

                        match result {
                            Ok(Ok((output, input_tokens, output_tokens))) => {
                                let step_result = StepResult {
                                    step_name: step_name.clone(),
                                    agent_id: agent_id.to_string(),
                                    agent_name: agent_name.clone(),
                                    output: output.clone(),
                                    input_tokens,
                                    output_tokens,
                                    duration_ms,
                                };
                                if let Some(r) = self.runs.write().await.get_mut(&run_id) {
                                    r.step_results.push(step_result);
                                }
                                if let Some(ref var) = fan_step.output_var {
                                    variables.insert(var.clone(), output.clone());
                                }
                                all_outputs.push(output.clone());
                                current_input = output;
                            }
                            Ok(Err(e)) => {
                                let error_msg =
                                    format!("FanOut step '{}' failed: {}", step_name, e);
                                warn!(%error_msg);
                                if let Some(r) = self.runs.write().await.get_mut(&run_id) {
                                    r.state = WorkflowRunState::Failed;
                                    r.error = Some(error_msg.clone());
                                    r.completed_at = Some(Utc::now());
                                }
                                return Err(error_msg);
                            }
                            Err(_) => {
                                let error_msg = format!(
                                    "FanOut step '{}' timed out after {}s",
                                    step_name, fan_step.timeout_secs
                                );
                                warn!(%error_msg);
                                if let Some(r) = self.runs.write().await.get_mut(&run_id) {
                                    r.state = WorkflowRunState::Failed;
                                    r.error = Some(error_msg.clone());
                                    r.completed_at = Some(Utc::now());
                                }
                                return Err(error_msg);
                            }
                        }
                    }

                    info!(
                        count = fan_out_steps.len(),
                        duration_ms, "FanOut steps completed"
                    );

                    // Skip past the fan-out steps we just processed
                    i = j;
                    continue;
                }

                StepMode::Collect => {
                    let strategy = step
                        .collect_aggregation
                        .as_ref()
                        .cloned()
                        .unwrap_or_default();
                    current_input = match &strategy {
                        AggregationStrategy::Concatenate { .. }
                        | AggregationStrategy::JsonArray
                        | AggregationStrategy::Consensus { .. } => {
                            Self::apply_aggregation(&strategy, &all_outputs)?
                        }
                        AggregationStrategy::BestOf {
                            evaluator_agent,
                            criteria,
                        } => {
                            if all_outputs.is_empty() {
                                String::new()
                            } else if all_outputs.len() == 1 {
                                all_outputs[0].clone()
                            } else {
                                let prompt = Self::best_of_prompt(criteria, &all_outputs);
                                let agg_agent = StepAgent::ByName {
                                    name: evaluator_agent.clone(),
                                };
                                let (agent_id, agent_name) = agent_resolver(&agg_agent)
                                    .ok_or_else(|| {
                                        format!(
                                            "Evaluator agent '{evaluator_agent}' not found for collect BestOf"
                                        )
                                    })?;
                                let synth =
                                    Self::synth_aggregate_step(step, agg_agent, prompt.clone());
                                let start = std::time::Instant::now();
                                let result = Self::execute_step_with_error_mode(
                                    &synth,
                                    i,
                                    agent_id,
                                    prompt,
                                    &mut send_message,
                                )
                                .await;
                                let duration_ms = start.elapsed().as_millis() as u64;
                                match result {
                                    Ok(Some((reply, input_tokens, output_tokens))) => {
                                        let idx =
                                            Self::parse_best_of_choice(&reply, all_outputs.len())?;
                                        let picked = all_outputs[idx].clone();
                                        let step_result = StepResult {
                                            step_name: format!("{}:best_of", step.name),
                                            agent_id: agent_id.to_string(),
                                            agent_name,
                                            output: reply,
                                            input_tokens,
                                            output_tokens,
                                            duration_ms,
                                        };
                                        if let Some(r) = self.runs.write().await.get_mut(&run_id) {
                                            r.step_results.push(step_result);
                                        }
                                        picked
                                    }
                                    Ok(None) => {
                                        return Err(format!(
                                            "Collect BestOf step '{}' was skipped",
                                            step.name
                                        ));
                                    }
                                    Err(e) => {
                                        if let Some(r) = self.runs.write().await.get_mut(&run_id) {
                                            r.state = WorkflowRunState::Failed;
                                            r.error = Some(e.clone());
                                            r.completed_at = Some(Utc::now());
                                        }
                                        return Err(e);
                                    }
                                }
                            }
                        }
                        AggregationStrategy::Summarize {
                            summarizer_agent,
                            max_length,
                        } => {
                            if all_outputs.is_empty() {
                                String::new()
                            } else if all_outputs.len() == 1 {
                                all_outputs[0].clone()
                            } else {
                                let prompt = Self::summarize_prompt(*max_length, &all_outputs);
                                let agg_agent = StepAgent::ByName {
                                    name: summarizer_agent.clone(),
                                };
                                let (agent_id, agent_name) = agent_resolver(&agg_agent)
                                    .ok_or_else(|| {
                                        format!(
                                            "Summarizer agent '{summarizer_agent}' not found for collect"
                                        )
                                    })?;
                                let synth =
                                    Self::synth_aggregate_step(step, agg_agent, prompt.clone());
                                let start = std::time::Instant::now();
                                let result = Self::execute_step_with_error_mode(
                                    &synth,
                                    i,
                                    agent_id,
                                    prompt,
                                    &mut send_message,
                                )
                                .await;
                                let duration_ms = start.elapsed().as_millis() as u64;
                                match result {
                                    Ok(Some((summary, input_tokens, output_tokens))) => {
                                        let step_result = StepResult {
                                            step_name: format!("{}:summarize", step.name),
                                            agent_id: agent_id.to_string(),
                                            agent_name,
                                            output: summary.clone(),
                                            input_tokens,
                                            output_tokens,
                                            duration_ms,
                                        };
                                        if let Some(r) = self.runs.write().await.get_mut(&run_id) {
                                            r.step_results.push(step_result);
                                        }
                                        summary
                                    }
                                    Ok(None) => {
                                        return Err(format!(
                                            "Collect Summarize step '{}' was skipped",
                                            step.name
                                        ));
                                    }
                                    Err(e) => {
                                        if let Some(r) = self.runs.write().await.get_mut(&run_id) {
                                            r.state = WorkflowRunState::Failed;
                                            r.error = Some(e.clone());
                                            r.completed_at = Some(Utc::now());
                                        }
                                        return Err(e);
                                    }
                                }
                            }
                        }
                        AggregationStrategy::Custom {
                            aggregator_agent,
                            aggregation_prompt,
                        } => {
                            if all_outputs.is_empty() {
                                String::new()
                            } else if all_outputs.len() == 1 {
                                all_outputs[0].clone()
                            } else {
                                let prompt = Self::expand_custom_aggregation_prompt(
                                    aggregation_prompt,
                                    &all_outputs,
                                    &run_initial_input,
                                    &variables,
                                );
                                let agg_agent = StepAgent::ByName {
                                    name: aggregator_agent.clone(),
                                };
                                let (agent_id, agent_name) = agent_resolver(&agg_agent)
                                    .ok_or_else(|| {
                                        format!(
                                            "Aggregator agent '{aggregator_agent}' not found for collect"
                                        )
                                    })?;
                                let synth =
                                    Self::synth_aggregate_step(step, agg_agent, prompt.clone());
                                let start = std::time::Instant::now();
                                let result = Self::execute_step_with_error_mode(
                                    &synth,
                                    i,
                                    agent_id,
                                    prompt,
                                    &mut send_message,
                                )
                                .await;
                                let duration_ms = start.elapsed().as_millis() as u64;
                                match result {
                                    Ok(Some((out, input_tokens, output_tokens))) => {
                                        let step_result = StepResult {
                                            step_name: format!("{}:custom", step.name),
                                            agent_id: agent_id.to_string(),
                                            agent_name,
                                            output: out.clone(),
                                            input_tokens,
                                            output_tokens,
                                            duration_ms,
                                        };
                                        if let Some(r) = self.runs.write().await.get_mut(&run_id) {
                                            r.step_results.push(step_result);
                                        }
                                        out
                                    }
                                    Ok(None) => {
                                        return Err(format!(
                                            "Collect Custom step '{}' was skipped",
                                            step.name
                                        ));
                                    }
                                    Err(e) => {
                                        if let Some(r) = self.runs.write().await.get_mut(&run_id) {
                                            r.state = WorkflowRunState::Failed;
                                            r.error = Some(e.clone());
                                            r.completed_at = Some(Utc::now());
                                        }
                                        return Err(e);
                                    }
                                }
                            }
                        }
                    };
                    all_outputs.clear();
                    all_outputs.push(current_input.clone());
                    if let Some(ref var) = step.output_var {
                        variables.insert(var.clone(), current_input.clone());
                    }
                }

                StepMode::Conditional { condition } => {
                    let prev_lower = current_input.to_lowercase();
                    let cond_lower = condition.to_lowercase();

                    if !prev_lower.contains(&cond_lower) {
                        info!(
                            step = i + 1,
                            name = %step.name,
                            condition,
                            "Conditional step skipped (condition not met)"
                        );
                        i += 1;
                        continue;
                    }

                    // Condition met — execute like sequential
                    let (agent_id, agent_name) = agent_resolver(&step.agent)
                        .ok_or_else(|| format!("Agent not found for step '{}'", step.name))?;

                    let prompt =
                        Self::expand_variables(&step.prompt_template, &current_input, &variables);

                    let start = std::time::Instant::now();
                    let result = Self::execute_step_with_error_mode(
                        step,
                        i,
                        agent_id,
                        prompt,
                        &mut send_message,
                    )
                    .await;
                    let duration_ms = start.elapsed().as_millis() as u64;

                    match result {
                        Ok(Some((output, input_tokens, output_tokens))) => {
                            let step_result = StepResult {
                                step_name: step.name.clone(),
                                agent_id: agent_id.to_string(),
                                agent_name,
                                output: output.clone(),
                                input_tokens,
                                output_tokens,
                                duration_ms,
                            };
                            if let Some(r) = self.runs.write().await.get_mut(&run_id) {
                                r.step_results.push(step_result);
                            }
                            if let Some(ref var) = step.output_var {
                                variables.insert(var.clone(), output.clone());
                            }
                            all_outputs.push(output.clone());
                            current_input = output;
                        }
                        Ok(None) => {}
                        Err(e) => {
                            if let Some(r) = self.runs.write().await.get_mut(&run_id) {
                                r.state = WorkflowRunState::Failed;
                                r.error = Some(e.clone());
                                r.completed_at = Some(Utc::now());
                            }
                            return Err(e);
                        }
                    }
                }

                StepMode::Loop {
                    max_iterations,
                    until,
                } => {
                    let (agent_id, agent_name) = agent_resolver(&step.agent)
                        .ok_or_else(|| format!("Agent not found for step '{}'", step.name))?;

                    let until_lower = until.to_lowercase();

                    for loop_iter in 0..*max_iterations {
                        let prompt = Self::expand_variables(
                            &step.prompt_template,
                            &current_input,
                            &variables,
                        );

                        let start = std::time::Instant::now();
                        let result = Self::execute_step_with_error_mode(
                            step,
                            i,
                            agent_id,
                            prompt,
                            &mut send_message,
                        )
                        .await;
                        let duration_ms = start.elapsed().as_millis() as u64;

                        match result {
                            Ok(Some((output, input_tokens, output_tokens))) => {
                                let step_result = StepResult {
                                    step_name: format!("{} (iter {})", step.name, loop_iter + 1),
                                    agent_id: agent_id.to_string(),
                                    agent_name: agent_name.clone(),
                                    output: output.clone(),
                                    input_tokens,
                                    output_tokens,
                                    duration_ms,
                                };
                                if let Some(r) = self.runs.write().await.get_mut(&run_id) {
                                    r.step_results.push(step_result);
                                }

                                current_input = output.clone();

                                if output.to_lowercase().contains(&until_lower) {
                                    info!(
                                        step = i + 1,
                                        name = %step.name,
                                        iterations = loop_iter + 1,
                                        "Loop terminated (until condition met)"
                                    );
                                    break;
                                }

                                if loop_iter + 1 == *max_iterations {
                                    info!(
                                        step = i + 1,
                                        name = %step.name,
                                        "Loop terminated (max iterations reached)"
                                    );
                                }
                            }
                            Ok(None) => break,
                            Err(e) => {
                                if let Some(r) = self.runs.write().await.get_mut(&run_id) {
                                    r.state = WorkflowRunState::Failed;
                                    r.error = Some(e.clone());
                                    r.completed_at = Some(Utc::now());
                                }
                                return Err(e);
                            }
                        }
                    }

                    if let Some(ref var) = step.output_var {
                        variables.insert(var.clone(), current_input.clone());
                    }
                    all_outputs.push(current_input.clone());
                }
            }

            i += 1;
        }

        // Mark workflow as completed
        let final_output = current_input.clone();
        if let Some(r) = self.runs.write().await.get_mut(&run_id) {
            r.state = WorkflowRunState::Completed;
            r.output = Some(final_output.clone());
            r.completed_at = Some(Utc::now());
        }

        info!(run_id = %run_id, "Workflow completed successfully");
        Ok(final_output)
    }
}

impl Default for WorkflowEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_workflow() -> Workflow {
        Workflow {
            id: WorkflowId::new(),
            name: "test-pipeline".to_string(),
            description: "A test pipeline".to_string(),
            steps: vec![
                WorkflowStep {
                    name: "analyze".to_string(),
                    agent: StepAgent::ByName {
                        name: "analyst".to_string(),
                    },
                    prompt_template: "Analyze this: {{input}}".to_string(),
                    mode: StepMode::Sequential,
                    timeout_secs: 30,
                    error_mode: ErrorMode::Fail,
                    output_var: None,
                    collect_aggregation: None,
                },
                WorkflowStep {
                    name: "summarize".to_string(),
                    agent: StepAgent::ByName {
                        name: "writer".to_string(),
                    },
                    prompt_template: "Summarize this analysis: {{input}}".to_string(),
                    mode: StepMode::Sequential,
                    timeout_secs: 30,
                    error_mode: ErrorMode::Fail,
                    output_var: None,
                    collect_aggregation: None,
                },
            ],
            created_at: Utc::now(),
        }
    }

    fn mock_resolver(agent: &StepAgent) -> Option<(AgentId, String)> {
        let _ = agent;
        Some((AgentId::new(), "mock-agent".to_string()))
    }

    #[tokio::test]
    async fn test_register_workflow() {
        let engine = WorkflowEngine::new();
        let wf = test_workflow();
        let id = engine.register(wf.clone()).await;
        assert_eq!(id, wf.id);

        let retrieved = engine.get_workflow(id).await;
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().name, "test-pipeline");
    }

    #[tokio::test]
    async fn test_create_run() {
        let engine = WorkflowEngine::new();
        let wf = test_workflow();
        let wf_id = engine.register(wf).await;

        let run_id = engine
            .create_run(
                wf_id,
                "test input".to_string(),
                WorkflowRetentionLimits::legacy_default(),
            )
            .await;
        assert!(run_id.is_some());

        let run = engine.get_run(run_id.unwrap()).await.unwrap();
        assert_eq!(run.input, "test input");
        assert!(matches!(run.state, WorkflowRunState::Pending));
    }

    #[tokio::test]
    async fn test_list_workflows() {
        let engine = WorkflowEngine::new();
        let wf = test_workflow();
        engine.register(wf).await;

        let list = engine.list_workflows().await;
        assert_eq!(list.len(), 1);
    }

    #[tokio::test]
    async fn test_remove_workflow() {
        let engine = WorkflowEngine::new();
        let wf = test_workflow();
        let id = engine.register(wf).await;

        assert!(engine.remove_workflow(id).await);
        assert!(engine.get_workflow(id).await.is_none());
    }

    #[tokio::test]
    async fn test_execute_pipeline() {
        let engine = WorkflowEngine::new();
        let wf = test_workflow();
        let wf_id = engine.register(wf).await;
        let run_id = engine
            .create_run(
                wf_id,
                "raw data".to_string(),
                WorkflowRetentionLimits::legacy_default(),
            )
            .await
            .unwrap();

        let sender = |_id: AgentId, msg: String, _step: &WorkflowStep, _idx: usize| async move {
            Ok((format!("Processed: {msg}"), 100u64, 50u64))
        };

        let result = engine.execute_run(run_id, mock_resolver, sender).await;
        assert!(result.is_ok());

        let output = result.unwrap();
        assert!(output.contains("Processed:"));

        let run = engine.get_run(run_id).await.unwrap();
        assert!(matches!(run.state, WorkflowRunState::Completed));
        assert_eq!(run.step_results.len(), 2);
        assert!(run.output.is_some());
    }

    #[tokio::test]
    async fn test_execute_run_adaptive_step() {
        let engine = WorkflowEngine::new();
        let wf = Workflow {
            id: WorkflowId::new(),
            name: "adaptive-exec".to_string(),
            description: "".to_string(),
            steps: vec![WorkflowStep {
                name: "think".to_string(),
                agent: StepAgent::ByName {
                    name: "agent-a".to_string(),
                },
                prompt_template: "Task: {{input}}".to_string(),
                mode: StepMode::Adaptive {
                    max_iterations: 7,
                    tool_allowlist: Some(vec!["web_search".to_string()]),
                    allow_subagents: false,
                    max_tokens: Some(4096),
                },
                timeout_secs: 10,
                error_mode: ErrorMode::Fail,
                output_var: None,
                collect_aggregation: None,
            }],
            created_at: Utc::now(),
        };
        let wf_id = engine.register(wf).await;
        let run_id = engine
            .create_run(
                wf_id,
                "hello".to_string(),
                WorkflowRetentionLimits::legacy_default(),
            )
            .await
            .unwrap();

        let sender = |_id: AgentId, _msg: String, step: &WorkflowStep, idx: usize| {
            assert_eq!(idx, 0);
            match &step.mode {
                StepMode::Adaptive {
                    max_iterations,
                    tool_allowlist,
                    allow_subagents,
                    max_tokens,
                } => {
                    assert_eq!(*max_iterations, 7);
                    assert_eq!(tool_allowlist, &Some(vec!["web_search".to_string()]));
                    assert!(!*allow_subagents);
                    assert_eq!(*max_tokens, Some(4096));
                }
                _ => panic!("expected Adaptive mode"),
            }
            async { Ok(("adaptive-result".to_string(), 3u64, 4u64)) }
        };

        let result = engine.execute_run(run_id, mock_resolver, sender).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "adaptive-result");

        let run = engine.get_run(run_id).await.unwrap();
        assert!(matches!(run.state, WorkflowRunState::Completed));
        assert_eq!(run.step_results.len(), 1);
        assert_eq!(run.step_results[0].step_name, "think");
        assert_eq!(run.step_results[0].output, "adaptive-result");
    }

    #[tokio::test]
    async fn test_conditional_skip() {
        let engine = WorkflowEngine::new();
        let wf = Workflow {
            id: WorkflowId::new(),
            name: "conditional-test".to_string(),
            description: "".to_string(),
            steps: vec![
                WorkflowStep {
                    name: "first".to_string(),
                    agent: StepAgent::ByName {
                        name: "a".to_string(),
                    },
                    prompt_template: "{{input}}".to_string(),
                    mode: StepMode::Sequential,
                    timeout_secs: 10,
                    error_mode: ErrorMode::Fail,
                    output_var: None,
                    collect_aggregation: None,
                },
                WorkflowStep {
                    name: "only-if-error".to_string(),
                    agent: StepAgent::ByName {
                        name: "a".to_string(),
                    },
                    prompt_template: "Fix: {{input}}".to_string(),
                    mode: StepMode::Conditional {
                        condition: "ERROR".to_string(),
                    },
                    timeout_secs: 10,
                    error_mode: ErrorMode::Fail,
                    output_var: None,
                    collect_aggregation: None,
                },
            ],
            created_at: Utc::now(),
        };
        let wf_id = engine.register(wf).await;
        let run_id = engine
            .create_run(
                wf_id,
                "all good".to_string(),
                WorkflowRetentionLimits::legacy_default(),
            )
            .await
            .unwrap();

        let sender = |_id: AgentId, msg: String, _step: &WorkflowStep, _idx: usize| async move {
            Ok((format!("OK: {msg}"), 10u64, 5u64))
        };

        let result = engine.execute_run(run_id, mock_resolver, sender).await;
        assert!(result.is_ok());

        let run = engine.get_run(run_id).await.unwrap();
        // Only 1 step executed (conditional was skipped)
        assert_eq!(run.step_results.len(), 1);
    }

    #[tokio::test]
    async fn test_conditional_executes() {
        let engine = WorkflowEngine::new();
        let wf = Workflow {
            id: WorkflowId::new(),
            name: "conditional-test".to_string(),
            description: "".to_string(),
            steps: vec![
                WorkflowStep {
                    name: "first".to_string(),
                    agent: StepAgent::ByName {
                        name: "a".to_string(),
                    },
                    prompt_template: "{{input}}".to_string(),
                    mode: StepMode::Sequential,
                    timeout_secs: 10,
                    error_mode: ErrorMode::Fail,
                    output_var: None,
                    collect_aggregation: None,
                },
                WorkflowStep {
                    name: "only-if-error".to_string(),
                    agent: StepAgent::ByName {
                        name: "a".to_string(),
                    },
                    prompt_template: "Fix: {{input}}".to_string(),
                    mode: StepMode::Conditional {
                        condition: "ERROR".to_string(),
                    },
                    timeout_secs: 10,
                    error_mode: ErrorMode::Fail,
                    output_var: None,
                    collect_aggregation: None,
                },
            ],
            created_at: Utc::now(),
        };
        let wf_id = engine.register(wf).await;
        let run_id = engine
            .create_run(
                wf_id,
                "data".to_string(),
                WorkflowRetentionLimits::legacy_default(),
            )
            .await
            .unwrap();

        // This sender returns output containing "ERROR"
        let sender = |_id: AgentId, _msg: String, _step: &WorkflowStep, _idx: usize| async move {
            Ok(("Found an ERROR in the data".to_string(), 10u64, 5u64))
        };

        let result = engine.execute_run(run_id, mock_resolver, sender).await;
        assert!(result.is_ok());

        let run = engine.get_run(run_id).await.unwrap();
        // Both steps executed
        assert_eq!(run.step_results.len(), 2);
    }

    #[tokio::test]
    async fn test_loop_until_condition() {
        let engine = WorkflowEngine::new();
        let wf = Workflow {
            id: WorkflowId::new(),
            name: "loop-test".to_string(),
            description: "".to_string(),
            steps: vec![WorkflowStep {
                name: "refine".to_string(),
                agent: StepAgent::ByName {
                    name: "a".to_string(),
                },
                prompt_template: "Refine: {{input}}".to_string(),
                mode: StepMode::Loop {
                    max_iterations: 5,
                    until: "DONE".to_string(),
                },
                timeout_secs: 10,
                error_mode: ErrorMode::Fail,
                output_var: None,
                collect_aggregation: None,
            }],
            created_at: Utc::now(),
        };
        let wf_id = engine.register(wf).await;
        let run_id = engine
            .create_run(
                wf_id,
                "draft".to_string(),
                WorkflowRetentionLimits::legacy_default(),
            )
            .await
            .unwrap();

        let call_count = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let cc = call_count.clone();
        let sender = move |_id: AgentId, _msg: String, _step: &WorkflowStep, _idx: usize| {
            let cc = cc.clone();
            async move {
                let n = cc.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if n >= 2 {
                    Ok(("Result: DONE".to_string(), 10u64, 5u64))
                } else {
                    Ok(("Still working...".to_string(), 10u64, 5u64))
                }
            }
        };

        let result = engine.execute_run(run_id, mock_resolver, sender).await;
        assert!(result.is_ok());
        assert!(result.unwrap().contains("DONE"));
        assert_eq!(call_count.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_loop_max_iterations() {
        let engine = WorkflowEngine::new();
        let wf = Workflow {
            id: WorkflowId::new(),
            name: "loop-max-test".to_string(),
            description: "".to_string(),
            steps: vec![WorkflowStep {
                name: "refine".to_string(),
                agent: StepAgent::ByName {
                    name: "a".to_string(),
                },
                prompt_template: "{{input}}".to_string(),
                mode: StepMode::Loop {
                    max_iterations: 3,
                    until: "NEVER_MATCH".to_string(),
                },
                timeout_secs: 10,
                error_mode: ErrorMode::Fail,
                output_var: None,
                collect_aggregation: None,
            }],
            created_at: Utc::now(),
        };
        let wf_id = engine.register(wf).await;
        let run_id = engine
            .create_run(
                wf_id,
                "data".to_string(),
                WorkflowRetentionLimits::legacy_default(),
            )
            .await
            .unwrap();

        let sender = |_id: AgentId, _msg: String, _step: &WorkflowStep, _idx: usize| async move {
            Ok(("iteration output".to_string(), 10u64, 5u64))
        };

        let result = engine.execute_run(run_id, mock_resolver, sender).await;
        assert!(result.is_ok());

        let run = engine.get_run(run_id).await.unwrap();
        assert_eq!(run.step_results.len(), 3); // max_iterations
    }

    #[tokio::test]
    async fn test_error_mode_skip() {
        let engine = WorkflowEngine::new();
        let wf = Workflow {
            id: WorkflowId::new(),
            name: "skip-test".to_string(),
            description: "".to_string(),
            steps: vec![
                WorkflowStep {
                    name: "will-fail".to_string(),
                    agent: StepAgent::ByName {
                        name: "a".to_string(),
                    },
                    prompt_template: "{{input}}".to_string(),
                    mode: StepMode::Sequential,
                    timeout_secs: 10,
                    error_mode: ErrorMode::Skip,
                    output_var: None,
                    collect_aggregation: None,
                },
                WorkflowStep {
                    name: "succeeds".to_string(),
                    agent: StepAgent::ByName {
                        name: "a".to_string(),
                    },
                    prompt_template: "{{input}}".to_string(),
                    mode: StepMode::Sequential,
                    timeout_secs: 10,
                    error_mode: ErrorMode::Fail,
                    output_var: None,
                    collect_aggregation: None,
                },
            ],
            created_at: Utc::now(),
        };
        let wf_id = engine.register(wf).await;
        let run_id = engine
            .create_run(
                wf_id,
                "data".to_string(),
                WorkflowRetentionLimits::legacy_default(),
            )
            .await
            .unwrap();

        let call_count = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let cc = call_count.clone();
        let sender = move |_id: AgentId, _msg: String, _step: &WorkflowStep, _idx: usize| {
            let cc = cc.clone();
            async move {
                let n = cc.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if n == 0 {
                    Err("simulated error".to_string())
                } else {
                    Ok(("success".to_string(), 10u64, 5u64))
                }
            }
        };

        let result = engine.execute_run(run_id, mock_resolver, sender).await;
        assert!(result.is_ok());

        let run = engine.get_run(run_id).await.unwrap();
        // Only 1 step result (the first was skipped due to error)
        assert_eq!(run.step_results.len(), 1);
        assert!(matches!(run.state, WorkflowRunState::Completed));
    }

    #[tokio::test]
    async fn test_error_mode_retry() {
        let engine = WorkflowEngine::new();
        let wf = Workflow {
            id: WorkflowId::new(),
            name: "retry-test".to_string(),
            description: "".to_string(),
            steps: vec![WorkflowStep {
                name: "flaky".to_string(),
                agent: StepAgent::ByName {
                    name: "a".to_string(),
                },
                prompt_template: "{{input}}".to_string(),
                mode: StepMode::Sequential,
                timeout_secs: 10,
                error_mode: ErrorMode::Retry { max_retries: 2 },
                output_var: None,
                collect_aggregation: None,
            }],
            created_at: Utc::now(),
        };
        let wf_id = engine.register(wf).await;
        let run_id = engine
            .create_run(
                wf_id,
                "data".to_string(),
                WorkflowRetentionLimits::legacy_default(),
            )
            .await
            .unwrap();

        let call_count = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let cc = call_count.clone();
        let sender = move |_id: AgentId, _msg: String, _step: &WorkflowStep, _idx: usize| {
            let cc = cc.clone();
            async move {
                let n = cc.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if n < 2 {
                    Err("transient error".to_string())
                } else {
                    Ok(("finally worked".to_string(), 10u64, 5u64))
                }
            }
        };

        let result = engine.execute_run(run_id, mock_resolver, sender).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "finally worked");
        assert_eq!(call_count.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_output_variables() {
        let engine = WorkflowEngine::new();
        let wf = Workflow {
            id: WorkflowId::new(),
            name: "vars-test".to_string(),
            description: "".to_string(),
            steps: vec![
                WorkflowStep {
                    name: "produce".to_string(),
                    agent: StepAgent::ByName {
                        name: "a".to_string(),
                    },
                    prompt_template: "{{input}}".to_string(),
                    mode: StepMode::Sequential,
                    timeout_secs: 10,
                    error_mode: ErrorMode::Fail,
                    output_var: Some("first_result".to_string()),
                    collect_aggregation: None,
                },
                WorkflowStep {
                    name: "transform".to_string(),
                    agent: StepAgent::ByName {
                        name: "a".to_string(),
                    },
                    prompt_template: "{{input}}".to_string(),
                    mode: StepMode::Sequential,
                    timeout_secs: 10,
                    error_mode: ErrorMode::Fail,
                    output_var: Some("second_result".to_string()),
                    collect_aggregation: None,
                },
                WorkflowStep {
                    name: "combine".to_string(),
                    agent: StepAgent::ByName {
                        name: "a".to_string(),
                    },
                    prompt_template: "First: {{first_result}} | Second: {{second_result}}"
                        .to_string(),
                    mode: StepMode::Sequential,
                    timeout_secs: 10,
                    error_mode: ErrorMode::Fail,
                    output_var: None,
                    collect_aggregation: None,
                },
            ],
            created_at: Utc::now(),
        };
        let wf_id = engine.register(wf).await;
        let run_id = engine
            .create_run(
                wf_id,
                "start".to_string(),
                WorkflowRetentionLimits::legacy_default(),
            )
            .await
            .unwrap();

        let call_count = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let cc = call_count.clone();
        let sender = move |_id: AgentId, msg: String, _step: &WorkflowStep, _idx: usize| {
            let cc = cc.clone();
            async move {
                let n = cc.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                match n {
                    0 => Ok(("alpha".to_string(), 10u64, 5u64)),
                    1 => Ok(("beta".to_string(), 10u64, 5u64)),
                    _ => Ok((format!("Combined: {msg}"), 10u64, 5u64)),
                }
            }
        };

        let result = engine.execute_run(run_id, mock_resolver, sender).await;
        assert!(result.is_ok());
        let output = result.unwrap();
        // The third step receives "First: alpha | Second: beta" as its prompt
        assert!(output.contains("First: alpha"));
        assert!(output.contains("Second: beta"));
    }

    #[tokio::test]
    async fn test_fan_out_parallel() {
        let engine = WorkflowEngine::new();
        let wf = Workflow {
            id: WorkflowId::new(),
            name: "fanout-test".to_string(),
            description: "".to_string(),
            steps: vec![
                WorkflowStep {
                    name: "task-a".to_string(),
                    agent: StepAgent::ByName {
                        name: "a".to_string(),
                    },
                    prompt_template: "Task A: {{input}}".to_string(),
                    mode: StepMode::FanOut,
                    timeout_secs: 10,
                    error_mode: ErrorMode::Fail,
                    output_var: None,
                    collect_aggregation: None,
                },
                WorkflowStep {
                    name: "task-b".to_string(),
                    agent: StepAgent::ByName {
                        name: "b".to_string(),
                    },
                    prompt_template: "Task B: {{input}}".to_string(),
                    mode: StepMode::FanOut,
                    timeout_secs: 10,
                    error_mode: ErrorMode::Fail,
                    output_var: None,
                    collect_aggregation: None,
                },
                WorkflowStep {
                    name: "collect".to_string(),
                    agent: StepAgent::ByName {
                        name: "c".to_string(),
                    },
                    prompt_template: "unused".to_string(),
                    mode: StepMode::Collect,
                    timeout_secs: 10,
                    error_mode: ErrorMode::Fail,
                    output_var: None,
                    collect_aggregation: None,
                },
            ],
            created_at: Utc::now(),
        };
        let wf_id = engine.register(wf).await;
        let run_id = engine
            .create_run(
                wf_id,
                "data".to_string(),
                WorkflowRetentionLimits::legacy_default(),
            )
            .await
            .unwrap();

        let sender = |_id: AgentId, msg: String, _step: &WorkflowStep, _idx: usize| async move {
            Ok((format!("Done: {msg}"), 10u64, 5u64))
        };

        let result = engine.execute_run(run_id, mock_resolver, sender).await;
        assert!(result.is_ok());

        let output = result.unwrap();
        // Collect step joins all outputs
        assert!(output.contains("Done: Task A"));
        assert!(output.contains("Done: Task B"));
        assert!(output.contains("---"));
    }

    #[tokio::test]
    async fn test_expand_variables() {
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "Alice".to_string());
        vars.insert("task".to_string(), "code review".to_string());

        let result = WorkflowEngine::expand_variables(
            "Hello {{name}}, please do {{task}} on {{input}}",
            "main.rs",
            &vars,
        );
        assert_eq!(result, "Hello Alice, please do code review on main.rs");
    }

    #[tokio::test]
    async fn test_error_mode_serialization() {
        let fail_json = serde_json::to_string(&ErrorMode::Fail).unwrap();
        assert_eq!(fail_json, "\"fail\"");

        let skip_json = serde_json::to_string(&ErrorMode::Skip).unwrap();
        assert_eq!(skip_json, "\"skip\"");

        let retry_json = serde_json::to_string(&ErrorMode::Retry { max_retries: 3 }).unwrap();
        let retry: ErrorMode = serde_json::from_str(&retry_json).unwrap();
        assert!(matches!(retry, ErrorMode::Retry { max_retries: 3 }));
    }

    #[tokio::test]
    async fn test_step_mode_conditional_serialization() {
        let mode = StepMode::Conditional {
            condition: "error".to_string(),
        };
        let json = serde_json::to_string(&mode).unwrap();
        let parsed: StepMode = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, StepMode::Conditional { condition } if condition == "error"));
    }

    #[tokio::test]
    async fn test_step_mode_loop_serialization() {
        let mode = StepMode::Loop {
            max_iterations: 5,
            until: "done".to_string(),
        };
        let json = serde_json::to_string(&mode).unwrap();
        let parsed: StepMode = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, StepMode::Loop { max_iterations: 5, until } if until == "done"));
    }

    #[tokio::test]
    async fn test_step_mode_adaptive_serialization() {
        let mode = StepMode::Adaptive {
            max_iterations: 10,
            tool_allowlist: Some(vec!["a".to_string(), "b".to_string()]),
            allow_subagents: true,
            max_tokens: Some(8192),
        };
        let json = serde_json::to_string(&mode).unwrap();
        let parsed: StepMode = serde_json::from_str(&json).unwrap();
        assert!(matches!(
            parsed,
            StepMode::Adaptive {
                max_iterations: 10,
                ref tool_allowlist,
                allow_subagents: true,
                max_tokens: Some(8192),
            } if tool_allowlist.as_ref().map(|v| v.len()) == Some(2)
        ));
    }

    #[tokio::test]
    async fn test_workflow_json_roundtrip_with_adaptive_step() {
        let wf = Workflow {
            id: WorkflowId::new(),
            name: "json-adaptive".to_string(),
            description: "rt".to_string(),
            steps: vec![WorkflowStep {
                name: "ad".to_string(),
                agent: StepAgent::ByName {
                    name: "x".to_string(),
                },
                prompt_template: "p".to_string(),
                mode: StepMode::Adaptive {
                    max_iterations: 3,
                    tool_allowlist: None,
                    allow_subagents: false,
                    max_tokens: None,
                },
                timeout_secs: 1,
                error_mode: ErrorMode::Fail,
                output_var: Some("out".to_string()),
                collect_aggregation: None,
            }],
            created_at: Utc::now(),
        };
        let json = serde_json::to_string(&wf).unwrap();
        let back: Workflow = serde_json::from_str(&json).unwrap();
        assert_eq!(back.name, "json-adaptive");
        assert!(matches!(
            back.steps[0].mode,
            StepMode::Adaptive {
                max_iterations: 3,
                tool_allowlist: None,
                allow_subagents: false,
                max_tokens: None,
            }
        ));
    }

    #[test]
    fn test_apply_aggregation_rejects_agent_strategies() {
        let s = AggregationStrategy::BestOf {
            evaluator_agent: "e".to_string(),
            criteria: "c".to_string(),
        };
        assert!(
            WorkflowEngine::apply_aggregation(&s, &["a".to_string(), "b".to_string()]).is_err()
        );
    }

    #[test]
    fn test_aggregation_strategy_best_of_json() {
        let j = r#"{"type":"best_of","evaluator_agent":"j1","criteria":"x"}"#;
        let s: AggregationStrategy = serde_json::from_str(j).unwrap();
        assert!(matches!(
            s,
            AggregationStrategy::BestOf {
                ref evaluator_agent,
                ..
            } if evaluator_agent == "j1"
        ));
    }

    #[tokio::test]
    async fn test_collect_best_of_execute_run() {
        let engine = WorkflowEngine::new();
        let wf = Workflow {
            id: WorkflowId::new(),
            name: "bestof-collect".to_string(),
            description: String::new(),
            steps: vec![
                WorkflowStep {
                    name: "fan1".to_string(),
                    agent: StepAgent::ByName {
                        name: "worker".to_string(),
                    },
                    prompt_template: "{{input}}".to_string(),
                    mode: StepMode::FanOut,
                    timeout_secs: 30,
                    error_mode: ErrorMode::Fail,
                    output_var: None,
                    collect_aggregation: None,
                },
                WorkflowStep {
                    name: "fan2".to_string(),
                    agent: StepAgent::ByName {
                        name: "worker".to_string(),
                    },
                    prompt_template: "{{input}}".to_string(),
                    mode: StepMode::FanOut,
                    timeout_secs: 30,
                    error_mode: ErrorMode::Fail,
                    output_var: None,
                    collect_aggregation: None,
                },
                WorkflowStep {
                    name: "merge".to_string(),
                    agent: StepAgent::ByName {
                        name: "judge".to_string(),
                    },
                    prompt_template: String::new(),
                    mode: StepMode::Collect,
                    timeout_secs: 30,
                    error_mode: ErrorMode::Fail,
                    output_var: None,
                    collect_aggregation: Some(AggregationStrategy::BestOf {
                        evaluator_agent: "judge".to_string(),
                        criteria: "accuracy".to_string(),
                    }),
                },
            ],
            created_at: Utc::now(),
        };
        let wf_id = engine.register(wf).await;
        let run_id = engine
            .create_run(
                wf_id,
                "seed".to_string(),
                WorkflowRetentionLimits::legacy_default(),
            )
            .await
            .unwrap();

        let sender = |_id: AgentId, _msg: String, _step: &WorkflowStep, idx: usize| async move {
            if idx < 2 {
                Ok((format!("C{}", idx + 1), 1u64, 1u64))
            } else {
                Ok(("2".to_string(), 1u64, 1u64))
            }
        };

        let result = engine.execute_run(run_id, mock_resolver, sender).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "C2");

        let run = engine.get_run(run_id).await.unwrap();
        assert!(run
            .step_results
            .iter()
            .any(|s| s.step_name == "merge:best_of"));
    }
}
