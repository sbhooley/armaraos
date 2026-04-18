//! Execute a [`DeterministicPlan`] from the inference server (sequential ready-queue, v1).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use ainl_agent_snapshot::{
    DeterministicPlan, OnErrorPolicy, PlanStep, PlanStepError, PolicyCaps, RepairContext,
    STRUCTURED_KIND_PLANNER_INVALID_PLAN,
};
use armara_provider_api::{ChatMessage, InferRequest, ModelHint, SessionRef};
use async_trait::async_trait;
use openfang_types::agent::AgentId;
use openfang_types::orchestration_trace::{OrchestrationTraceEvent, TraceEventType};
use serde_json::Value;
use tracing::warn;
use uuid::Uuid;

use crate::drivers::native_infer::NativeInferDriver;
use crate::kernel_handle::KernelHandle;
use crate::llm_driver::{CompletionRequest, LlmDriver, LlmError};

/// Emits [`OrchestrationTraceEvent`]s for planner-mode execution (dashboard `#orchestration-traces`).
pub struct PlanExecutionTrace<'a> {
    pub kernel: &'a Arc<dyn KernelHandle>,
    pub agent_id: AgentId,
    pub trace_id: String,
    pub orchestrator_id: AgentId,
    pub parent_agent_id: Option<AgentId>,
}

impl PlanExecutionTrace<'_> {
    pub fn emit(&self, event_type: TraceEventType) {
        self.kernel.record_orchestration_trace(OrchestrationTraceEvent {
            trace_id: self.trace_id.clone(),
            orchestrator_id: self.orchestrator_id,
            agent_id: self.agent_id,
            parent_agent_id: self.parent_agent_id,
            event_type,
            timestamp: chrono::Utc::now(),
            metadata: std::collections::HashMap::new(),
        });
    }

    /// Fire a lightweight `planner_step_progress` event on the SSE kernel event bus so
    /// dashboard consumers and any active WS sessions can show per-step progress without
    /// waiting for the full plan to complete. Non-blocking: spawned on the Tokio runtime.
    pub fn emit_step_progress(&self, step_id: &str, tool: &str, status: &str) {
        use serde_json::json;
        let agent_id = self.agent_id;
        let kernel = Arc::clone(self.kernel);
        let payload = json!({
            "agent_id": agent_id,
            "step_id": step_id,
            "tool": tool,
            "status": status,
        });
        tokio::spawn(async move {
            let _ = kernel
                .publish_event("planner_step_progress", payload)
                .await;
        });
    }
}

/// Host dispatches a single plan step to OpenFang tools.
#[async_trait]
pub trait PlanStepDispatch: Send + Sync {
    async fn dispatch(
        &self,
        step_id: &str,
        tool: &str,
        args: &Value,
    ) -> Result<Value, PlanStepError>;
}

/// Optional hook to persist per-step episodic records.
#[async_trait]
pub trait PlanEpisodeRecorder: Send + Sync {
    async fn record_step_completed(
        &self,
        step_id: &str,
        tool: &str,
        args: &Value,
        output: &Value,
    ) -> Result<(), String>;
}

#[derive(Debug, Clone)]
pub struct PlanExecutionResult {
    pub completed: Vec<String>,
    pub skipped: Vec<String>,
    pub outputs: HashMap<String, Value>,
    pub fell_back_to_legacy: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum PlanExecutionError {
    #[error("wall clock exceeded")]
    WallClockExceeded,
    #[error("max plan steps exceeded")]
    MaxStepsExceeded,
    #[error("step {0}: {1:?}")]
    StepFailed(String, PlanStepError),
    #[error("infer repair failed: {0}")]
    RepairInfer(String),
    #[error("replan budget exhausted")]
    ReplanBudgetExceeded,
}

fn navigate_json_path(root: &Value, path: &[String]) -> Option<Value> {
    let mut cur = root;
    for p in path {
        match cur {
            Value::Object(map) => {
                cur = map.get(p)?;
            }
            _ => return None,
        }
    }
    Some(cur.clone())
}

/// Resolve `${outputs.<step_id>.a.b}` templates in string leaves (recursive).
pub fn resolve_output_templates(
    value: &Value,
    outputs: &HashMap<String, Value>,
) -> Result<Value, String> {
    match value {
        Value::String(s) => {
            if let Some(inner) = strip_template(s) {
                let parts: Vec<&str> = inner.split('.').collect();
                if parts.len() < 2 {
                    return Err("invalid outputs template".into());
                }
                let step_id = parts[0];
                let path: Vec<String> = parts[1..].iter().map(|x| (*x).to_string()).collect();
                let step_out = outputs
                    .get(step_id)
                    .ok_or_else(|| format!("unknown step id in template: {step_id}"))?;
                navigate_json_path(step_out, &path)
                    .ok_or_else(|| format!("path not found in step output: {inner}"))
            } else {
                Ok(Value::String(s.clone()))
            }
        }
        Value::Array(a) => {
            let mut out = Vec::with_capacity(a.len());
            for x in a {
                out.push(resolve_output_templates(x, outputs)?);
            }
            Ok(Value::Array(out))
        }
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                out.insert(k.clone(), resolve_output_templates(v, outputs)?);
            }
            Ok(Value::Object(out))
        }
        x => Ok(x.clone()),
    }
}

fn strip_template(s: &str) -> Option<&str> {
    let t = s.trim();
    let prefix = "${outputs.";
    if !t.starts_with(prefix) || !t.ends_with('}') {
        return None;
    }
    let end = t.rfind('}')?;
    Some(&t[prefix.len()..end])
}

fn parse_repair_step(text: &str) -> Result<PlanStep, String> {
    let v: Value = serde_json::from_str(text.trim()).map_err(|e| e.to_string())?;
    if v.get("kind").and_then(|k| k.as_str()) != Some("plan_step_repair") {
        return Err("expected kind plan_step_repair".into());
    }
    serde_json::from_value(
        v.get("step")
            .cloned()
            .ok_or_else(|| "missing step".to_string())?,
    )
    .map_err(|e| e.to_string())
}

/// Validate `value` against a JSON Schema.
///
/// Checks `required` fields explicitly for stable error messages, then delegates to `jsonschema`
/// for type and constraint validation.
fn validate_against_schema(value: &Value, schema: &Value) -> Result<(), String> {
    // Explicit required-field check for stable contract messaging.
    if let Some(req) = schema.get("required").and_then(|r| r.as_array()) {
        if let Some(obj) = value.as_object() {
            for key in req.iter().filter_map(|k| k.as_str()) {
                if !obj.contains_key(key) {
                    return Err(format!("missing required field `{key}`"));
                }
            }
        } else {
            return Err("expected JSON object".into());
        }
    }
    let compiled =
        jsonschema::validator_for(schema).map_err(|e| format!("invalid schema: {e}"))?;
    if let Err(err) = compiled.validate(value) {
        return Err(err.to_string());
    }
    Ok(())
}

pub struct PlanExecutor;

impl PlanExecutor {
    /// Sequential ready-queue execution (single-threaded steps).
    #[allow(clippy::too_many_arguments)]
    pub async fn execute(
        plan: &DeterministicPlan,
        limits: &PolicyCaps,
        dispatch: &dyn PlanStepDispatch,
        episode_recorder: Option<&dyn PlanEpisodeRecorder>,
        reasoning_driver: Option<&(dyn LlmDriver + Send + Sync)>,
        native_infer: Option<&NativeInferDriver>,
        infer_model: &str,
        agent_snapshot: &ainl_agent_snapshot::AgentSnapshot,
        messages_for_infer: Vec<ChatMessage>,
        trace: Option<&PlanExecutionTrace<'_>>,
    ) -> Result<PlanExecutionResult, PlanExecutionError> {
        let started = Instant::now();
        let mut completed: HashSet<String> = HashSet::new();
        let mut skipped: Vec<String> = Vec::new();
        let mut outputs: HashMap<String, Value> = HashMap::new();
        let mut replans_used: u32 = 0;

        let reasoning: HashSet<&str> = plan
            .reasoning_required_at
            .iter()
            .map(|s| s.as_str())
            .collect();

        let max_steps = limits.max_steps.max(1) as usize;

        if let Some(t) = trace {
            t.emit(TraceEventType::PlanStarted {
                step_count: plan.steps.len(),
                confidence: plan.confidence,
                reasoning_step_ids: plan.reasoning_required_at.clone(),
            });
        }

        loop {
            if started.elapsed().as_millis() as u64 > limits.max_wall_ms {
                if let Some(t) = trace {
                    t.emit(TraceEventType::PlanFallback {
                        reason: "plan_wall_clock_exceeded".into(),
                    });
                }
                return Err(PlanExecutionError::WallClockExceeded);
            }
            if completed.len() >= max_steps {
                if let Some(t) = trace {
                    t.emit(TraceEventType::PlanFallback {
                        reason: "plan_max_steps_exceeded".into(),
                    });
                }
                return Err(PlanExecutionError::MaxStepsExceeded);
            }

            let next = plan.steps.iter().find(|s| {
                !completed.contains(&s.id)
                    && !skipped.iter().any(|x| x == &s.id)
                    && s.depends_on.iter().all(|d| completed.contains(d))
            });
            let Some(step) = next else {
                break;
            };

            if let Some(t) = trace {
                t.emit(TraceEventType::PlanStepStarted {
                    step_id: step.id.clone(),
                    tool: step.tool.clone(),
                });
            }

            let resolved_args = resolve_output_templates(&step.args, &outputs).map_err(|e| {
                if let Some(t) = trace {
                    t.emit(TraceEventType::PlanStepFailed {
                        step_id: step.id.clone(),
                        tool: step.tool.clone(),
                        error: e.clone(),
                    });
                }
                PlanExecutionError::StepFailed(step.id.clone(), PlanStepError::Deterministic(e))
            })?;

            crate::planner_metrics::record_plan_step_dispatched(&step.tool);
            let mut out = if reasoning.contains(step.id.as_str()) {
                Self::run_reasoning_step(
                    reasoning_driver,
                    infer_model,
                    step,
                    &resolved_args,
                    &outputs,
                    trace,
                )
                .await
            } else {
                dispatch
                    .dispatch(&step.id, &step.tool, &resolved_args)
                    .await
            };
            if out.is_err() && step.on_error == OnErrorPolicy::RetryOnce {
                out = if reasoning.contains(step.id.as_str()) {
                    Self::run_reasoning_step(
                        reasoning_driver,
                        infer_model,
                        step,
                        &resolved_args,
                        &outputs,
                        trace,
                    )
                    .await
                } else {
                    dispatch
                        .dispatch(&step.id, &step.tool, &resolved_args)
                        .await
                };
            }

            let last_err = match out {
                Ok(v) => {
                    crate::planner_metrics::record_plan_step_success(&step.tool);
                    if let Some(rec) = episode_recorder {
                        if let Err(err) = rec
                            .record_step_completed(&step.id, &step.tool, &resolved_args, &v)
                            .await
                        {
                            warn!(step_id = %step.id, tool = %step.tool, error = %err, "planner episode write failed");
                        }
                    }
                    outputs.insert(step.id.clone(), v);
                    completed.insert(step.id.clone());
                    if let Some(t) = trace {
                        t.emit(TraceEventType::PlanStepCompleted {
                            step_id: step.id.clone(),
                            tool: step.tool.clone(),
                        });
                        t.emit_step_progress(&step.id, &step.tool, "completed");
                    }
                    None
                }
                Err(e) => {
                    if step.optional {
                        crate::planner_metrics::record_plan_step_optional_skipped(&step.tool);
                        if let Some(t) = trace {
                            t.emit(TraceEventType::PlanStepFailed {
                                step_id: step.id.clone(),
                                tool: step.tool.clone(),
                                error: format!("optional: {}", e.to_message()),
                            });
                            t.emit_step_progress(&step.id, &step.tool, "optional_skipped");
                        }
                        skipped.push(step.id.clone());
                        None
                    } else {
                        crate::planner_metrics::record_plan_step_error(&step.tool);
                        if let Some(t) = trace {
                            t.emit(TraceEventType::PlanStepFailed {
                                step_id: step.id.clone(),
                                tool: step.tool.clone(),
                                error: e.to_message(),
                            });
                            t.emit_step_progress(&step.id, &step.tool, "failed");
                        }
                        Some(e)
                    }
                }
            };

            if let Some(e) = last_err {
                match step.on_error {
                    OnErrorPolicy::LocalPatch => {
                        if replans_used >= limits.max_replan_calls {
                            if let Some(t) = trace {
                                t.emit(TraceEventType::PlanFallback {
                                    reason: "replan_budget_exhausted".into(),
                                });
                            }
                            return Err(PlanExecutionError::ReplanBudgetExceeded);
                        }
                        let Some(ni) = native_infer else {
                            return Err(PlanExecutionError::StepFailed(step.id.clone(), e));
                        };
                        let prior = serde_json::to_value(&outputs).unwrap_or(Value::Null);
                        let repair = RepairContext {
                            failed_step_id: step.id.clone(),
                            failed_step_tool: step.tool.clone(),
                            error_msg: e.to_message(),
                            prior_outputs: prior,
                        };
                        if let Some(t) = trace {
                            t.emit(TraceEventType::PlanLocalPatch {
                                step_id: step.id.clone(),
                                replan_attempt: replans_used + 1,
                            });
                        }
                        let req = InferRequest {
                            request_id: Uuid::new_v4(),
                            tenant_id: None,
                            session: Some(SessionRef {
                                agent_id: Some(agent_snapshot.agent_id.clone()),
                                turn_id: None,
                            }),
                            model: ModelHint {
                                policy: None,
                                hint: Some(infer_model.to_string()),
                            },
                            messages: messages_for_infer.clone(),
                            graph_context: None,
                            constraints: Default::default(),
                            policy: Default::default(),
                            backend_preference: vec![],
                            agent_snapshot: Some(agent_snapshot.clone()),
                            repair_context: Some(repair),
                        };
                        let resp = match ni.infer(req).await {
                            Ok(r) => r,
                            Err(err) => {
                                if let Some(t) = trace {
                                    t.emit(TraceEventType::PlanFallback {
                                        reason: format!("local_patch_infer_failed: {err}"),
                                    });
                                }
                                return Err(PlanExecutionError::RepairInfer(err.to_string()));
                            }
                        };
                        replans_used += 1;
                        crate::planner_metrics::record_local_patch_replan_call();

                        if let Some(st) = resp.output.structured {
                            if st.get("kind").and_then(|k| k.as_str())
                                == Some(STRUCTURED_KIND_PLANNER_INVALID_PLAN)
                            {
                                if let Some(t) = trace {
                                    t.emit(TraceEventType::PlanFallback {
                                        reason: st
                                            .get("reason")
                                            .and_then(|r| r.as_str())
                                            .unwrap_or("planner_invalid_plan")
                                            .to_string(),
                                    });
                                }
                                return Ok(PlanExecutionResult {
                                    completed: completed.iter().cloned().collect(),
                                    skipped,
                                    outputs,
                                    fell_back_to_legacy: true,
                                });
                            }
                        }

                        let replacement =
                            parse_repair_step(&resp.output.text).map_err(|e| {
                                if let Some(t) = trace {
                                    t.emit(TraceEventType::PlanFallback {
                                        reason: format!("parse_repair_step: {e}"),
                                    });
                                }
                                PlanExecutionError::RepairInfer(e)
                            })?;
                        let merged_args = resolve_output_templates(&replacement.args, &outputs)
                            .map_err(|msg| {
                                PlanExecutionError::StepFailed(
                                    step.id.clone(),
                                    PlanStepError::Deterministic(msg),
                                )
                            })?;
                        let r2 = if reasoning.contains(step.id.as_str()) {
                            Self::run_reasoning_step(
                                reasoning_driver,
                                infer_model,
                                &replacement,
                                &merged_args,
                                &outputs,
                                trace,
                            )
                            .await
                        } else {
                            dispatch
                                .dispatch(&step.id, &replacement.tool, &merged_args)
                                .await
                        };
                        match r2 {
                            Ok(v) => {
                                crate::planner_metrics::record_plan_step_success(&replacement.tool);
                                if let Some(rec) = episode_recorder {
                                    if let Err(err) = rec
                                        .record_step_completed(
                                            &step.id,
                                            &replacement.tool,
                                            &merged_args,
                                            &v,
                                        )
                                        .await
                                    {
                                        warn!(step_id = %step.id, tool = %replacement.tool, error = %err, "planner episode write failed");
                                    }
                                }
                                outputs.insert(step.id.clone(), v);
                                completed.insert(step.id.clone());
                                if let Some(t) = trace {
                                    t.emit(TraceEventType::PlanStepCompleted {
                                        step_id: step.id.clone(),
                                        tool: replacement.tool.clone(),
                                    });
                                }
                            }
                            Err(e2) => {
                                crate::planner_metrics::record_plan_step_error(&replacement.tool);
                                return Err(PlanExecutionError::StepFailed(step.id.clone(), e2));
                            }
                        }
                    }
                    OnErrorPolicy::Abort | OnErrorPolicy::RetryOnce => {
                        return Err(PlanExecutionError::StepFailed(step.id.clone(), e));
                    }
                }
            }
        }

        Ok(PlanExecutionResult {
            completed: completed.into_iter().collect(),
            skipped,
            outputs,
            fell_back_to_legacy: false,
        })
    }

    async fn run_reasoning_step(
        driver: Option<&(dyn LlmDriver + Send + Sync)>,
        model: &str,
        step: &PlanStep,
        args: &Value,
        prior: &HashMap<String, Value>,
        trace: Option<&PlanExecutionTrace<'_>>,
    ) -> Result<Value, PlanStepError> {
        if let Some(t) = trace {
            t.emit(TraceEventType::PlanReasoningReentry {
                step_id: step.id.clone(),
            });
        }
        let Some(drv) = driver else {
            return Err(PlanStepError::Deterministic(
                "reasoning step but no LLM driver".into(),
            ));
        };
        let schema = step
            .expected_output_schema
            .clone()
            .unwrap_or_else(|| serde_json::json!({"type":"object","properties":{"output":{}}}));

        let mut ctx = serde_json::Map::new();
        ctx.insert("step_id".into(), Value::String(step.id.clone()));
        ctx.insert("args".into(), args.clone());
        ctx.insert(
            "prior_outputs".into(),
            serde_json::to_value(prior).unwrap_or(Value::Null),
        );

        let sys = format!(
            "Reasoning sub-task (scoped). Respond with JSON only matching schema:\n{}",
            schema
        );
        let req = CompletionRequest {
            model: model.to_string(),
            messages: vec![openfang_types::message::Message::user(
                serde_json::to_string(&Value::Object(ctx)).unwrap_or_else(|_| "{}".into()),
            )],
            tools: vec![],
            max_tokens: 2048,
            temperature: 0.2,
            system: Some(sys),
            thinking: None,
        };
        let resp = drv.complete(req).await.map_err(|e| match e {
            LlmError::RateLimited { .. } | LlmError::Overloaded { .. } => {
                PlanStepError::Transient(e.to_string())
            }
            _ => PlanStepError::Deterministic(e.to_string()),
        })?;
        let txt = resp.text();
        let parsed: serde_json::Value =
            serde_json::from_str(&txt).map_err(|_| {
                PlanStepError::Deterministic(format!("reasoning JSON parse failed: {txt}"))
            })?;

        // Validate the parsed value against expected_output_schema if one was declared.
        // Only `required` fields and type checks are enforced (jsonschema); additional
        // properties are allowed by default so small models are not over-constrained.
        if let Some(ref declared_schema) = step.expected_output_schema {
            if let Err(e) = validate_against_schema(&parsed, declared_schema) {
                return Err(PlanStepError::Deterministic(format!(
                    "reasoning output schema violation (step {}): {e}",
                    step.id
                )));
            }
        }

        Ok(parsed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Debug)]
    struct DispatchCall {
        step_id: String,
        tool: String,
        args: Value,
    }

    struct FakeDispatch {
        calls: Arc<Mutex<Vec<DispatchCall>>>,
        fail_counts: Arc<Mutex<HashMap<String, usize>>>,
    }

    impl FakeDispatch {
        fn new() -> Self {
            Self {
                calls: Arc::new(Mutex::new(Vec::new())),
                fail_counts: Arc::new(Mutex::new(HashMap::new())),
            }
        }

        fn with_fail_counts(self, fail_counts: HashMap<String, usize>) -> Self {
            *self.fail_counts.lock().expect("fail counts lock") = fail_counts;
            self
        }
    }

    struct FakeEpisodeRecorder {
        recorded_step_ids: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl PlanEpisodeRecorder for FakeEpisodeRecorder {
        async fn record_step_completed(
            &self,
            step_id: &str,
            _tool: &str,
            _args: &Value,
            _output: &Value,
        ) -> Result<(), String> {
            self.recorded_step_ids
                .lock()
                .expect("recorded lock")
                .push(step_id.to_string());
            Ok(())
        }
    }

    #[async_trait]
    impl PlanStepDispatch for FakeDispatch {
        async fn dispatch(
            &self,
            step_id: &str,
            tool: &str,
            args: &Value,
        ) -> Result<Value, PlanStepError> {
            self.calls.lock().expect("calls lock").push(DispatchCall {
                step_id: step_id.to_string(),
                tool: tool.to_string(),
                args: args.clone(),
            });

            let mut counts = self.fail_counts.lock().expect("fail counts lock");
            let remaining = counts.get(step_id).copied().unwrap_or(0);
            if remaining > 0 {
                counts.insert(step_id.to_string(), remaining - 1);
                return Err(PlanStepError::Transient(format!("forced failure for {step_id}")));
            }
            Ok(serde_json::json!({
                "step_id": step_id,
                "ok": true,
                "echo_args": args
            }))
        }
    }

    fn test_limits() -> PolicyCaps {
        PolicyCaps {
            max_steps: 16,
            max_depth: 8,
            max_wall_ms: 30_000,
            max_replan_calls: 1,
            deny_tools: vec![],
        }
    }

    fn test_snapshot() -> ainl_agent_snapshot::AgentSnapshot {
        ainl_agent_snapshot::AgentSnapshot {
            agent_id: "agent-test".to_string(),
            snapshot_version: ainl_agent_snapshot::SNAPSHOT_SCHEMA_VERSION,
            persona: vec![],
            episodic: vec![],
            semantic: vec![],
            procedural: vec![],
            tool_allowlist: vec!["file_read".to_string(), "shell_exec".to_string()],
            policy_caps: test_limits(),
        }
    }

    #[tokio::test]
    async fn execute_respects_depends_on_and_resolves_output_templates() {
        let dispatch = FakeDispatch::new();
        let recorder_steps = Arc::new(Mutex::new(Vec::new()));
        let recorder = FakeEpisodeRecorder {
            recorded_step_ids: Arc::clone(&recorder_steps),
        };
        let plan = DeterministicPlan {
            steps: vec![
                PlanStep {
                    id: "step1".to_string(),
                    tool: "file_read".to_string(),
                    args: serde_json::json!({"path": "input.txt"}),
                    depends_on: vec![],
                    on_error: OnErrorPolicy::Abort,
                    idempotency_key: None,
                    optional: false,
                    expected_output_schema: None,
                },
                PlanStep {
                    id: "step2".to_string(),
                    tool: "shell_exec".to_string(),
                    args: serde_json::json!({
                        "command": "echo",
                        "from_prev": "${outputs.step1.echo_args.path}"
                    }),
                    depends_on: vec!["step1".to_string()],
                    on_error: OnErrorPolicy::Abort,
                    idempotency_key: None,
                    optional: false,
                    expected_output_schema: None,
                },
            ],
            graph_writes: vec![],
            confidence: 0.9,
            reasoning_required_at: vec![],
        };

        let res = PlanExecutor::execute(
            &plan,
            &test_limits(),
            &dispatch,
            Some(&recorder),
            None,
            None,
            "test-model",
            &test_snapshot(),
            vec![],
            None,
        )
        .await
        .expect("plan execute");

        assert!(!res.fell_back_to_legacy);
        assert!(res.completed.contains(&"step1".to_string()));
        assert!(res.completed.contains(&"step2".to_string()));
        assert!(res.skipped.is_empty());

        let calls = dispatch.calls.lock().expect("calls lock");
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].step_id, "step1");
        assert_eq!(calls[1].step_id, "step2");
        assert_eq!(calls[1].tool, "shell_exec");
        assert_eq!(calls[1].args["from_prev"], "input.txt");
        assert_eq!(
            *recorder_steps.lock().expect("recorded lock"),
            vec!["step1".to_string(), "step2".to_string()]
        );
    }

    #[tokio::test]
    async fn execute_retries_retry_once_and_then_completes() {
        let mut fail_counts = HashMap::new();
        fail_counts.insert("step1".to_string(), 1);
        let dispatch = FakeDispatch::new().with_fail_counts(fail_counts);
        let plan = DeterministicPlan {
            steps: vec![PlanStep {
                id: "step1".to_string(),
                tool: "file_read".to_string(),
                args: serde_json::json!({"path": "retry.txt"}),
                depends_on: vec![],
                on_error: OnErrorPolicy::RetryOnce,
                idempotency_key: None,
                optional: false,
                expected_output_schema: None,
            }],
            graph_writes: vec![],
            confidence: 0.95,
            reasoning_required_at: vec![],
        };

        let res = PlanExecutor::execute(
            &plan,
            &test_limits(),
            &dispatch,
            None,
            None,
            None,
            "test-model",
            &test_snapshot(),
            vec![],
            None,
        )
        .await
        .expect("plan execute");

        assert_eq!(res.completed, vec!["step1".to_string()]);
        assert!(res.skipped.is_empty());
        assert!(!res.fell_back_to_legacy);
        assert_eq!(dispatch.calls.lock().expect("calls lock").len(), 2);
    }

    /// parse_repair_step: valid envelope → returns the embedded PlanStep.
    #[test]
    fn parse_repair_step_valid() {
        let json = serde_json::json!({
            "kind": "plan_step_repair",
            "step": {
                "id": "s1",
                "tool": "file_write",
                "args": {"path": "out.txt", "content": "hello"},
                "on_error": "abort"
            }
        });
        let step = parse_repair_step(&json.to_string()).expect("parse");
        assert_eq!(step.id, "s1");
        assert_eq!(step.tool, "file_write");
    }

    /// parse_repair_step: wrong kind → Err.
    #[test]
    fn parse_repair_step_wrong_kind() {
        let json = serde_json::json!({ "kind": "deterministic_plan", "step": {} });
        assert!(parse_repair_step(&json.to_string()).is_err());
    }

    /// parse_repair_step: missing step field → Err.
    #[test]
    fn parse_repair_step_missing_step() {
        let json = serde_json::json!({ "kind": "plan_step_repair" });
        assert!(parse_repair_step(&json.to_string()).is_err());
    }

    /// LocalPatch with native_infer=None: step with on_error=LocalPatch + no NI driver
    /// returns StepFailed immediately (no replan budget wasted, no panic).
    #[tokio::test]
    async fn local_patch_without_native_infer_returns_step_failed() {
        let mut fail_counts = HashMap::new();
        fail_counts.insert("step1".to_string(), 1);
        let dispatch = FakeDispatch::new().with_fail_counts(fail_counts);
        let plan = DeterministicPlan {
            steps: vec![PlanStep {
                id: "step1".to_string(),
                tool: "file_read".to_string(),
                args: serde_json::json!({"path": "missing.txt"}),
                depends_on: vec![],
                on_error: OnErrorPolicy::LocalPatch,
                idempotency_key: None,
                optional: false,
                expected_output_schema: None,
            }],
            graph_writes: vec![],
            confidence: 0.8,
            reasoning_required_at: vec![],
        };

        let err = PlanExecutor::execute(
            &plan,
            &test_limits(),
            &dispatch,
            None,
            None,
            None, // native_infer absent → LocalPatch falls through to StepFailed
            "test-model",
            &test_snapshot(),
            vec![],
            None,
        )
        .await
        .expect_err("should fail when NI driver absent");

        assert!(
            matches!(err, PlanExecutionError::StepFailed(ref id, _) if id == "step1"),
            "expected StepFailed for step1, got: {err:?}"
        );
    }

    /// LocalPatch budget exhausted → ReplanBudgetExceeded (max_replan_calls=0).
    #[tokio::test]
    async fn local_patch_budget_exhausted_returns_error() {
        let mut fail_counts = HashMap::new();
        fail_counts.insert("step1".to_string(), 99);
        let dispatch = FakeDispatch::new().with_fail_counts(fail_counts);
        let plan = DeterministicPlan {
            steps: vec![PlanStep {
                id: "step1".to_string(),
                tool: "file_read".to_string(),
                args: serde_json::json!({"path": "missing.txt"}),
                depends_on: vec![],
                on_error: OnErrorPolicy::LocalPatch,
                idempotency_key: None,
                optional: false,
                expected_output_schema: None,
            }],
            graph_writes: vec![],
            confidence: 0.8,
            reasoning_required_at: vec![],
        };
        let mut zero_replan_limits = test_limits();
        zero_replan_limits.max_replan_calls = 0;

        let err = PlanExecutor::execute(
            &plan,
            &zero_replan_limits,
            &dispatch,
            None,
            None,
            None,
            "test-model",
            &test_snapshot(),
            vec![],
            None,
        )
        .await
        .expect_err("should fail");

        assert!(
            matches!(err, PlanExecutionError::ReplanBudgetExceeded),
            "expected ReplanBudgetExceeded, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn execute_skips_optional_step_on_failure_and_continues() {
        let mut fail_counts = HashMap::new();
        fail_counts.insert("optional_step".to_string(), 1);
        let dispatch = FakeDispatch::new().with_fail_counts(fail_counts);
        let plan = DeterministicPlan {
            steps: vec![
                PlanStep {
                    id: "optional_step".to_string(),
                    tool: "file_read".to_string(),
                    args: serde_json::json!({"path": "missing.txt"}),
                    depends_on: vec![],
                    on_error: OnErrorPolicy::Abort,
                    idempotency_key: None,
                    optional: true,
                    expected_output_schema: None,
                },
                PlanStep {
                    id: "step2".to_string(),
                    tool: "shell_exec".to_string(),
                    args: serde_json::json!({"command": "echo ok"}),
                    depends_on: vec![],
                    on_error: OnErrorPolicy::Abort,
                    idempotency_key: None,
                    optional: false,
                    expected_output_schema: None,
                },
            ],
            graph_writes: vec![],
            confidence: 0.8,
            reasoning_required_at: vec![],
        };

        let res = PlanExecutor::execute(
            &plan,
            &test_limits(),
            &dispatch,
            None,
            None,
            None,
            "test-model",
            &test_snapshot(),
            vec![],
            None,
        )
        .await
        .expect("plan execute");

        assert!(res.skipped.contains(&"optional_step".to_string()));
        assert!(res.completed.contains(&"step2".to_string()));
    }
}
