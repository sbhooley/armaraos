//! Event-driven agent triggers — agents auto-activate when events match patterns.
//!
//! Agents register triggers that describe which events should wake them.
//! When a matching event arrives on the EventBus, the trigger system
//! sends the event content as a message to the subscribing agent.

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use openfang_types::agent::AgentId;
use openfang_types::event::{Event, EventPayload, LifecycleEvent, SystemEvent};
use openfang_types::orchestration::{OrchestrationContext, OrchestrationPattern};
use openfang_types::orchestration_trace::OrchestrationTraceEvent;
use serde::{Deserialize, Serialize};
use tracing::{debug, info};
use uuid::Uuid;

/// Unique identifier for a trigger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TriggerId(pub Uuid);

impl TriggerId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for TriggerId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for TriggerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// What kind of events a trigger matches on.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerPattern {
    /// Match any lifecycle event (agent spawned, started, terminated, etc.).
    Lifecycle,
    /// Match when a specific agent is spawned.
    AgentSpawned { name_pattern: String },
    /// Match when any agent is terminated.
    AgentTerminated,
    /// Match any system event.
    System,
    /// Match a specific system event by keyword.
    SystemKeyword { keyword: String },
    /// Match any memory update event.
    MemoryUpdate,
    /// Match memory updates for a specific key pattern.
    MemoryKeyPattern { key_pattern: String },
    /// Match all events (wildcard).
    All,
    /// Match custom events by content substring.
    ContentMatch { substring: String },
    /// Match orchestration trace events (multi-agent observability).
    ///
    /// `event_types` uses [`openfang_types::orchestration_trace::TraceEventType::discriminant_name`]
    /// values (e.g. `agent_failed`). Empty = all types.
    OrchestrationTrace {
        /// Empty = all trace event types.
        event_types: Vec<String>,
        #[serde(default)]
        trace_id_substring: Option<String>,
        #[serde(default)]
        orchestrator_id: Option<AgentId>,
    },
}

/// Result of evaluating triggers — includes orchestration context for every wake (trace events
/// carry the distributed trace; other patterns get a minimal `AdHoc` context with trigger metadata).
#[derive(Debug, Clone)]
pub struct TriggerDispatch {
    pub agent_id: AgentId,
    pub message: String,
    pub orchestration_ctx: Option<OrchestrationContext>,
}

fn trigger_pattern_kind(pattern: &TriggerPattern) -> &'static str {
    match pattern {
        TriggerPattern::All => "all",
        TriggerPattern::Lifecycle => "lifecycle",
        TriggerPattern::AgentSpawned { .. } => "agent_spawned",
        TriggerPattern::AgentTerminated => "agent_terminated",
        TriggerPattern::System => "system",
        TriggerPattern::SystemKeyword { .. } => "system_keyword",
        TriggerPattern::MemoryUpdate => "memory_update",
        TriggerPattern::MemoryKeyPattern { .. } => "memory_key_pattern",
        TriggerPattern::ContentMatch { .. } => "content_match",
        TriggerPattern::OrchestrationTrace { .. } => "orchestration_trace",
    }
}

fn orchestration_ctx_for_generic_trigger(
    trigger_id: TriggerId,
    pattern: &TriggerPattern,
    subscriber: AgentId,
    event_description: &str,
    default_budget_ms: Option<u64>,
) -> OrchestrationContext {
    let mut ctx = OrchestrationContext::new_root(
        subscriber,
        OrchestrationPattern::AdHoc,
        None, // Trigger events use subscriber's own settings
    );
    // Stable id per trigger (not a fresh UUID every wake) so trace lists and logs are not
    // flooded with one-off traces for chatty triggers.
    ctx.trace_id = format!("trigger-wake-{}", trigger_id.0);
    ctx.shared_vars.insert(
        "trigger_id".to_string(),
        serde_json::json!(trigger_id.to_string()),
    );
    ctx.shared_vars.insert(
        "trigger_pattern".to_string(),
        serde_json::json!(trigger_pattern_kind(pattern)),
    );
    ctx.shared_vars.insert(
        "trigger_event_preview".to_string(),
        serde_json::json!(openfang_types::truncate_str(event_description, 500)),
    );
    if ctx.remaining_budget_ms.is_none() {
        ctx.remaining_budget_ms = default_budget_ms;
    }
    ctx
}

fn orchestration_ctx_for_trace_trigger(
    ev: &OrchestrationTraceEvent,
    subscriber: AgentId,
    default_budget_ms: Option<u64>,
) -> OrchestrationContext {
    let mut ctx = OrchestrationContext::new_root(
        subscriber,
        OrchestrationPattern::AdHoc,
        None, // Trigger events use subscriber's own settings
    );
    ctx.trace_id = ev.trace_id.clone();
    ctx.orchestrator_id = ev.orchestrator_id;
    ctx.call_chain = vec![ev.orchestrator_id, subscriber];
    ctx.depth = 1;
    ctx.shared_vars = ev.metadata.clone();
    ctx.shared_vars.insert(
        "trigger_source_agent".to_string(),
        serde_json::json!(ev.agent_id.to_string()),
    );
    ctx.shared_vars.insert(
        "trigger_event_type".to_string(),
        serde_json::json!(ev.event_type.discriminant_name()),
    );
    if ctx.remaining_budget_ms.is_none() {
        ctx.remaining_budget_ms = default_budget_ms;
    }
    ctx
}

/// A registered trigger definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trigger {
    /// Unique trigger ID.
    pub id: TriggerId,
    /// Which agent owns this trigger.
    pub agent_id: AgentId,
    /// The event pattern to match.
    pub pattern: TriggerPattern,
    /// Prompt template to send when triggered. Use `{{event}}` for event description.
    pub prompt_template: String,
    /// Whether this trigger is currently active.
    pub enabled: bool,
    /// When this trigger was created.
    pub created_at: DateTime<Utc>,
    /// How many times this trigger has fired.
    pub fire_count: u64,
    /// Maximum number of times this trigger can fire (0 = unlimited).
    pub max_fires: u64,
}

/// The trigger engine manages event-to-agent routing.
pub struct TriggerEngine {
    /// All registered triggers.
    triggers: DashMap<TriggerId, Trigger>,
    /// Index: agent_id → list of trigger IDs belonging to that agent.
    agent_triggers: DashMap<AgentId, Vec<TriggerId>>,
}

impl TriggerEngine {
    /// Create a new trigger engine.
    pub fn new() -> Self {
        Self {
            triggers: DashMap::new(),
            agent_triggers: DashMap::new(),
        }
    }

    /// Register a new trigger.
    pub fn register(
        &self,
        agent_id: AgentId,
        pattern: TriggerPattern,
        prompt_template: String,
        max_fires: u64,
    ) -> TriggerId {
        let trigger = Trigger {
            id: TriggerId::new(),
            agent_id,
            pattern,
            prompt_template,
            enabled: true,
            created_at: Utc::now(),
            fire_count: 0,
            max_fires,
        };
        let id = trigger.id;
        self.triggers.insert(id, trigger);
        self.agent_triggers.entry(agent_id).or_default().push(id);

        info!(trigger_id = %id, agent_id = %agent_id, "Trigger registered");
        id
    }

    /// Remove a trigger.
    pub fn remove(&self, trigger_id: TriggerId) -> bool {
        if let Some((_, trigger)) = self.triggers.remove(&trigger_id) {
            if let Some(mut list) = self.agent_triggers.get_mut(&trigger.agent_id) {
                list.retain(|id| *id != trigger_id);
            }
            true
        } else {
            false
        }
    }

    /// Remove all triggers for an agent.
    pub fn remove_agent_triggers(&self, agent_id: AgentId) {
        if let Some((_, trigger_ids)) = self.agent_triggers.remove(&agent_id) {
            for id in trigger_ids {
                self.triggers.remove(&id);
            }
        }
    }

    /// Take all triggers for an agent, removing them from the engine.
    ///
    /// Returns the extracted triggers so they can be restored under a
    /// different agent ID via [`restore_triggers`]. This is used during
    /// hand reactivation: triggers must be saved before `kill_agent`
    /// destroys them, then restored with the new agent ID after spawn.
    pub fn take_agent_triggers(&self, agent_id: AgentId) -> Vec<Trigger> {
        let trigger_ids = self
            .agent_triggers
            .remove(&agent_id)
            .map(|(_, ids)| ids)
            .unwrap_or_default();
        let mut taken = Vec::with_capacity(trigger_ids.len());
        for id in trigger_ids {
            if let Some((_, t)) = self.triggers.remove(&id) {
                taken.push(t);
            }
        }
        if !taken.is_empty() {
            info!(
                agent = %agent_id,
                count = taken.len(),
                "Took triggers for agent (pending reassignment)"
            );
        }
        taken
    }

    /// Restore previously taken triggers under a new agent ID.
    ///
    /// Each trigger keeps its original pattern, prompt template, fire count,
    /// and max_fires, but is re-keyed to `new_agent_id`. New trigger IDs are
    /// generated so there are no stale references.
    ///
    /// Returns the number of triggers restored.
    pub fn restore_triggers(&self, new_agent_id: AgentId, triggers: Vec<Trigger>) -> usize {
        let count = triggers.len();
        for old in triggers {
            let new_id = TriggerId::new();
            let trigger = Trigger {
                id: new_id,
                agent_id: new_agent_id,
                pattern: old.pattern,
                prompt_template: old.prompt_template,
                enabled: old.enabled,
                created_at: old.created_at,
                fire_count: old.fire_count,
                max_fires: old.max_fires,
            };
            self.triggers.insert(new_id, trigger);
            self.agent_triggers
                .entry(new_agent_id)
                .or_default()
                .push(new_id);
        }
        if count > 0 {
            info!(
                agent = %new_agent_id,
                count,
                "Restored triggers under new agent"
            );
        }
        count
    }

    /// Reassign all triggers from one agent to another in place.
    ///
    /// Used during cold boot when the old agent ID (from persisted state) no
    /// longer exists and a new agent was spawned. Updates the `agent_id` field
    /// on each trigger and moves the index entry.
    ///
    /// Returns the number of triggers reassigned.
    pub fn reassign_agent_triggers(&self, old_agent_id: AgentId, new_agent_id: AgentId) -> usize {
        let trigger_ids = self
            .agent_triggers
            .remove(&old_agent_id)
            .map(|(_, ids)| ids)
            .unwrap_or_default();
        let count = trigger_ids.len();
        for id in &trigger_ids {
            if let Some(mut t) = self.triggers.get_mut(id) {
                t.agent_id = new_agent_id;
            }
        }
        if !trigger_ids.is_empty() {
            self.agent_triggers
                .entry(new_agent_id)
                .or_default()
                .extend(trigger_ids);
            info!(
                old_agent = %old_agent_id,
                new_agent = %new_agent_id,
                count,
                "Reassigned triggers to new agent"
            );
        }
        count
    }

    /// Enable or disable a trigger. Returns true if the trigger was found.
    pub fn set_enabled(&self, trigger_id: TriggerId, enabled: bool) -> bool {
        if let Some(mut t) = self.triggers.get_mut(&trigger_id) {
            t.enabled = enabled;
            true
        } else {
            false
        }
    }

    /// List all triggers for an agent.
    pub fn list_agent_triggers(&self, agent_id: AgentId) -> Vec<Trigger> {
        self.agent_triggers
            .get(&agent_id)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| self.triggers.get(id).map(|t| t.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// List all registered triggers.
    pub fn list_all(&self) -> Vec<Trigger> {
        self.triggers.iter().map(|e| e.value().clone()).collect()
    }

    /// Evaluate an event against all triggers. Returns dispatches for matching triggers.
    ///
    /// `EventPayload::OrchestrationTrace` payloads use the trace's
    /// `trace_id` and metadata. All other matches include a minimal [`OrchestrationContext`]
    /// (`trigger_id`, `trigger_pattern`, `trigger_event_preview` in `shared_vars`) for observability,
    /// with `trace_id` of the form `trigger-wake-<trigger_uuid>` (stable per trigger).
    pub fn evaluate(
        &self,
        event: &Event,
        default_orch_budget_ms: Option<u64>,
    ) -> Vec<TriggerDispatch> {
        let event_description = describe_event(event);
        let mut matches = Vec::new();

        for mut entry in self.triggers.iter_mut() {
            let trigger = entry.value_mut();

            if !trigger.enabled {
                continue;
            }

            // Check max fires
            if trigger.max_fires > 0 && trigger.fire_count >= trigger.max_fires {
                trigger.enabled = false;
                continue;
            }

            if matches_pattern(&trigger.pattern, event, &event_description) {
                let message = trigger
                    .prompt_template
                    .replace("{{event}}", &event_description);
                let orchestration_ctx =
                    if let EventPayload::OrchestrationTrace(ref ev) = event.payload {
                        orchestration_ctx_for_trace_trigger(
                            ev,
                            trigger.agent_id,
                            default_orch_budget_ms,
                        )
                    } else {
                        orchestration_ctx_for_generic_trigger(
                            trigger.id,
                            &trigger.pattern,
                            trigger.agent_id,
                            &event_description,
                            default_orch_budget_ms,
                        )
                    };
                matches.push(TriggerDispatch {
                    agent_id: trigger.agent_id,
                    message,
                    orchestration_ctx: Some(orchestration_ctx),
                });
                trigger.fire_count += 1;

                debug!(
                    trigger_id = %trigger.id,
                    agent_id = %trigger.agent_id,
                    fire_count = trigger.fire_count,
                    "Trigger fired"
                );
            }
        }

        matches
    }

    /// Get a trigger by ID.
    pub fn get(&self, trigger_id: TriggerId) -> Option<Trigger> {
        self.triggers.get(&trigger_id).map(|t| t.clone())
    }
}

impl Default for TriggerEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Check if an event matches a trigger pattern.
fn matches_pattern(pattern: &TriggerPattern, event: &Event, description: &str) -> bool {
    match pattern {
        TriggerPattern::All => true,
        TriggerPattern::Lifecycle => {
            matches!(event.payload, EventPayload::Lifecycle(_))
        }
        TriggerPattern::AgentSpawned { name_pattern } => {
            if let EventPayload::Lifecycle(LifecycleEvent::Spawned { name, .. }) = &event.payload {
                name.contains(name_pattern.as_str()) || name_pattern == "*"
            } else {
                false
            }
        }
        TriggerPattern::AgentTerminated => matches!(
            event.payload,
            EventPayload::Lifecycle(LifecycleEvent::Terminated { .. })
                | EventPayload::Lifecycle(LifecycleEvent::Crashed { .. })
        ),
        TriggerPattern::System => {
            matches!(event.payload, EventPayload::System(_))
        }
        TriggerPattern::SystemKeyword { keyword } => {
            if let EventPayload::System(se) = &event.payload {
                let se_str = format!("{:?}", se).to_lowercase();
                se_str.contains(&keyword.to_lowercase())
            } else {
                false
            }
        }
        TriggerPattern::MemoryUpdate => {
            matches!(event.payload, EventPayload::MemoryUpdate(_))
        }
        TriggerPattern::MemoryKeyPattern { key_pattern } => {
            if let EventPayload::MemoryUpdate(delta) = &event.payload {
                delta.key.contains(key_pattern.as_str()) || key_pattern == "*"
            } else {
                false
            }
        }
        TriggerPattern::ContentMatch { substring } => description
            .to_lowercase()
            .contains(&substring.to_lowercase()),
        TriggerPattern::OrchestrationTrace {
            event_types,
            trace_id_substring,
            orchestrator_id,
        } => {
            if let EventPayload::OrchestrationTrace(ev) = &event.payload {
                if let Some(oid) = orchestrator_id {
                    if oid != &ev.orchestrator_id {
                        return false;
                    }
                }
                if let Some(sub) = trace_id_substring {
                    if !ev.trace_id.contains(sub.as_str()) {
                        return false;
                    }
                }
                if !event_types.is_empty()
                    && !event_types
                        .iter()
                        .any(|t| t == ev.event_type.discriminant_name())
                {
                    return false;
                }
                true
            } else {
                false
            }
        }
    }
}

/// Create a human-readable description of an event for use in prompts.
fn describe_event(event: &Event) -> String {
    match &event.payload {
        EventPayload::Message(msg) => {
            format!("Message from {:?}: {}", msg.role, msg.content)
        }
        EventPayload::ToolResult(tr) => {
            format!(
                "Tool '{}' {} ({}ms): {}",
                tr.tool_id,
                if tr.success { "succeeded" } else { "failed" },
                tr.execution_time_ms,
                openfang_types::truncate_str(&tr.content, 200)
            )
        }
        EventPayload::MemoryUpdate(delta) => {
            format!(
                "Memory {:?} on key '{}' for agent {}",
                delta.operation, delta.key, delta.agent_id
            )
        }
        EventPayload::Lifecycle(le) => match le {
            LifecycleEvent::Spawned { agent_id, name } => {
                format!("Agent '{name}' (id: {agent_id}) was spawned")
            }
            LifecycleEvent::Started { agent_id } => {
                format!("Agent {agent_id} started")
            }
            LifecycleEvent::Suspended { agent_id } => {
                format!("Agent {agent_id} suspended")
            }
            LifecycleEvent::Resumed { agent_id } => {
                format!("Agent {agent_id} resumed")
            }
            LifecycleEvent::Terminated { agent_id, reason } => {
                format!("Agent {agent_id} terminated: {reason}")
            }
            LifecycleEvent::Crashed { agent_id, error } => {
                format!("Agent {agent_id} crashed: {error}")
            }
        },
        EventPayload::Network(ne) => {
            format!("Network event: {:?}", ne)
        }
        EventPayload::System(se) => match se {
            SystemEvent::KernelStarted => "Kernel started".to_string(),
            SystemEvent::KernelStopping => "Kernel stopping".to_string(),
            SystemEvent::QuotaWarning {
                agent_id,
                resource,
                usage_percent,
            } => format!("Quota warning: agent {agent_id}, {resource} at {usage_percent:.1}%"),
            SystemEvent::HealthCheck { status } => {
                format!("Health check: {status}")
            }
            SystemEvent::QuotaEnforced {
                agent_id,
                spent,
                limit,
            } => {
                format!("Quota enforced: agent {agent_id}, spent ${spent:.4} / ${limit:.4}")
            }
            SystemEvent::ModelRouted {
                agent_id,
                complexity,
                model,
            } => {
                format!("Model routed: agent {agent_id}, complexity={complexity}, model={model}")
            }
            SystemEvent::UserAction {
                user_id,
                action,
                result,
            } => {
                format!("User action: {user_id} {action} -> {result}")
            }
            SystemEvent::HealthCheckFailed {
                agent_id,
                unresponsive_secs,
            } => {
                format!(
                    "Health check failed: agent {agent_id}, unresponsive for {unresponsive_secs}s"
                )
            }
            SystemEvent::CronJobCompleted {
                job_id,
                job_name,
                agent_id,
                output_preview,
                action_kind,
            } => {
                let kind = action_kind
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .map(|k| format!(" ({k})"))
                    .unwrap_or_default();
                format!(
                    "Cron job completed{kind}: {job_name} ({job_id}) for agent {agent_id}: {output_preview}"
                )
            },
            SystemEvent::CronJobFailed {
                job_id,
                job_name,
                agent_id,
                error,
                action_kind,
            } => {
                let kind = action_kind
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .map(|k| format!(" ({k})"))
                    .unwrap_or_default();
                format!(
                    "Cron job failed{kind}: {job_name} ({job_id}) for agent {agent_id}: {error}"
                )
            },
            SystemEvent::AgentActivity { phase, detail } => {
                if let Some(d) = detail {
                    format!("Agent activity: {phase} ({d})")
                } else {
                    format!("Agent activity: {phase}")
                }
            }
            SystemEvent::ApprovalPending {
                request_id,
                agent_id,
                tool_name,
                action_summary,
            } => format!(
                "Approval pending: {tool_name} for agent {agent_id} ({request_id}): {action_summary}"
            ),
            SystemEvent::GraphMemoryWrite {
                agent_id,
                kind,
                provenance,
            } => {
                let mut s = format!("Graph memory write: agent {agent_id}, kind={kind}");
                if let Some(p) = provenance {
                    if let Some(sum) = p.summary.as_ref() {
                        use std::fmt::Write as _;
                        let _ = write!(&mut s, " — {}", sum);
                    }
                }
                s
            }
            SystemEvent::TrajectoryRecorded {
                agent_id,
                trajectory_node_id,
                episode_node_id,
                summary,
            } => {
                let ep = episode_node_id
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .unwrap_or("—");
                let sum = summary.as_deref().unwrap_or("");
                format!(
                    "Trajectory recorded: agent {agent_id}, traj={trajectory_node_id}, episode={ep}: {sum}"
                )
            }
            SystemEvent::FailureLearned {
                agent_id,
                failure_node_id,
                tool_name,
                source,
                message_preview,
            } => {
                let tool = tool_name.as_deref().unwrap_or("—");
                let src = source.as_deref().unwrap_or("—");
                let msg = message_preview.as_deref().unwrap_or("");
                format!(
                    "Failure learned: agent {agent_id}, id={failure_node_id}, tool={tool}, source={src}: {msg}"
                )
            }
            SystemEvent::WorkflowRunFinished {
                workflow_name,
                run_id,
                ok,
                summary,
                ..
            } => {
                let status = if *ok { "completed" } else { "failed" };
                format!("Workflow {workflow_name} run {run_id} {status}: {summary}")
            }
            SystemEvent::AgentAssistantReply {
                agent_id,
                agent_name,
                message_preview,
            } => {
                format!("Agent '{agent_name}' ({agent_id}) replied: {message_preview}")
            }
            SystemEvent::ImprovementProposalAdopted {
                agent_id,
                proposal_id,
                graph_node_id,
                kind,
            } => {
                let k = if kind.is_empty() { "—" } else { kind.as_str() };
                format!(
                    "Improvement proposal adopted: agent {agent_id}, proposal={proposal_id}, graph={graph_node_id}, kind={k}"
                )
            }
        },
        EventPayload::OrchestrationTrace(ev) => {
            format!(
                "Orchestration trace {} — {:?}",
                ev.trace_id, ev.event_type
            )
        }
        EventPayload::Custom(data) => {
            format!("Custom event ({} bytes)", data.len())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openfang_types::event::*;

    #[test]
    fn test_register_trigger() {
        let engine = TriggerEngine::new();
        let agent_id = AgentId::new();
        let id = engine.register(
            agent_id,
            TriggerPattern::All,
            "Event occurred: {{event}}".to_string(),
            0,
        );
        assert!(engine.get(id).is_some());
    }

    #[test]
    fn test_evaluate_lifecycle() {
        let engine = TriggerEngine::new();
        let watcher = AgentId::new();
        engine.register(
            watcher,
            TriggerPattern::Lifecycle,
            "Lifecycle: {{event}}".to_string(),
            0,
        );

        let event = Event::new(
            AgentId::new(),
            EventTarget::Broadcast,
            EventPayload::Lifecycle(LifecycleEvent::Spawned {
                agent_id: AgentId::new(),
                name: "new-agent".to_string(),
            }),
        );

        let matches = engine.evaluate(&event, None);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].agent_id, watcher);
        assert!(matches[0].message.contains("new-agent"));
        let ctx = matches[0].orchestration_ctx.as_ref().expect("ctx");
        assert_eq!(ctx.orchestrator_id, watcher);
        assert!(
            ctx.trace_id.starts_with("trigger-wake-"),
            "expected stable generic trace id, got {}",
            ctx.trace_id
        );
        assert_eq!(
            ctx.shared_vars
                .get("trigger_pattern")
                .and_then(|v| v.as_str()),
            Some("lifecycle")
        );
    }

    #[test]
    fn test_evaluate_agent_spawned_pattern() {
        let engine = TriggerEngine::new();
        let watcher = AgentId::new();
        engine.register(
            watcher,
            TriggerPattern::AgentSpawned {
                name_pattern: "coder".to_string(),
            },
            "Coder spawned: {{event}}".to_string(),
            0,
        );

        // This should match
        let event = Event::new(
            AgentId::new(),
            EventTarget::Broadcast,
            EventPayload::Lifecycle(LifecycleEvent::Spawned {
                agent_id: AgentId::new(),
                name: "coder".to_string(),
            }),
        );
        assert_eq!(engine.evaluate(&event, None).len(), 1);

        // This should NOT match
        let event2 = Event::new(
            AgentId::new(),
            EventTarget::Broadcast,
            EventPayload::Lifecycle(LifecycleEvent::Spawned {
                agent_id: AgentId::new(),
                name: "researcher".to_string(),
            }),
        );
        assert_eq!(engine.evaluate(&event2, None).len(), 0);
    }

    #[test]
    fn test_max_fires() {
        let engine = TriggerEngine::new();
        let agent_id = AgentId::new();
        engine.register(
            agent_id,
            TriggerPattern::All,
            "Event: {{event}}".to_string(),
            2, // max 2 fires
        );

        let event = Event::new(
            AgentId::new(),
            EventTarget::Broadcast,
            EventPayload::System(SystemEvent::HealthCheck {
                status: "ok".to_string(),
            }),
        );

        // First two should match
        assert_eq!(engine.evaluate(&event, None).len(), 1);
        assert_eq!(engine.evaluate(&event, None).len(), 1);
        // Third should not
        assert_eq!(engine.evaluate(&event, None).len(), 0);
    }

    #[test]
    fn test_remove_trigger() {
        let engine = TriggerEngine::new();
        let agent_id = AgentId::new();
        let id = engine.register(agent_id, TriggerPattern::All, "msg".to_string(), 0);
        assert!(engine.remove(id));
        assert!(engine.get(id).is_none());
    }

    #[test]
    fn test_remove_agent_triggers() {
        let engine = TriggerEngine::new();
        let agent_id = AgentId::new();
        engine.register(agent_id, TriggerPattern::All, "a".to_string(), 0);
        engine.register(agent_id, TriggerPattern::System, "b".to_string(), 0);
        assert_eq!(engine.list_agent_triggers(agent_id).len(), 2);

        engine.remove_agent_triggers(agent_id);
        assert_eq!(engine.list_agent_triggers(agent_id).len(), 0);
    }

    #[test]
    fn test_content_match() {
        let engine = TriggerEngine::new();
        let agent_id = AgentId::new();
        engine.register(
            agent_id,
            TriggerPattern::ContentMatch {
                substring: "quota".to_string(),
            },
            "Alert: {{event}}".to_string(),
            0,
        );

        let event = Event::new(
            AgentId::new(),
            EventTarget::System,
            EventPayload::System(SystemEvent::QuotaWarning {
                agent_id: AgentId::new(),
                resource: "tokens".to_string(),
                usage_percent: 85.0,
            }),
        );
        assert_eq!(engine.evaluate(&event, None).len(), 1);
    }

    // -- reassign_agent_triggers (#519) ------------------------------------

    #[test]
    fn test_reassign_agent_triggers_basic() {
        let engine = TriggerEngine::new();
        let old_agent = AgentId::new();
        let new_agent = AgentId::new();
        engine.register(old_agent, TriggerPattern::All, "a".to_string(), 0);
        engine.register(old_agent, TriggerPattern::System, "b".to_string(), 0);

        let count = engine.reassign_agent_triggers(old_agent, new_agent);
        assert_eq!(count, 2);
        assert_eq!(engine.list_agent_triggers(old_agent).len(), 0);
        assert_eq!(engine.list_agent_triggers(new_agent).len(), 2);

        // Verify triggers actually fire for the new agent
        let event = Event::new(
            AgentId::new(),
            EventTarget::Broadcast,
            EventPayload::System(SystemEvent::HealthCheck {
                status: "ok".to_string(),
            }),
        );
        let matches = engine.evaluate(&event, None);
        assert_eq!(matches.len(), 2);
        assert!(matches.iter().all(|d| d.agent_id == new_agent));
    }

    #[test]
    fn test_reassign_agent_triggers_no_match_returns_zero() {
        let engine = TriggerEngine::new();
        let agent_a = AgentId::new();
        engine.register(agent_a, TriggerPattern::All, "a".to_string(), 0);

        let count = engine.reassign_agent_triggers(AgentId::new(), AgentId::new());
        assert_eq!(count, 0);
        // Original triggers untouched
        assert_eq!(engine.list_agent_triggers(agent_a).len(), 1);
    }

    #[test]
    fn test_reassign_does_not_touch_other_agents() {
        let engine = TriggerEngine::new();
        let agent_a = AgentId::new();
        let agent_b = AgentId::new();
        let agent_c = AgentId::new();
        engine.register(agent_a, TriggerPattern::All, "a".to_string(), 0);
        engine.register(agent_b, TriggerPattern::System, "b".to_string(), 0);

        let count = engine.reassign_agent_triggers(agent_a, agent_c);
        assert_eq!(count, 1);
        // agent_b untouched
        assert_eq!(engine.list_agent_triggers(agent_b).len(), 1);
        assert_eq!(engine.list_agent_triggers(agent_c).len(), 1);
    }

    // -- take / restore triggers (#519) ------------------------------------

    #[test]
    fn test_take_and_restore_triggers() {
        let engine = TriggerEngine::new();
        let old_agent = AgentId::new();
        let new_agent = AgentId::new();
        engine.register(
            old_agent,
            TriggerPattern::ContentMatch {
                substring: "deploy".to_string(),
            },
            "Deploy alert: {{event}}".to_string(),
            5,
        );
        engine.register(old_agent, TriggerPattern::Lifecycle, "lc".to_string(), 0);

        // Take triggers — engine should be empty for old agent
        let taken = engine.take_agent_triggers(old_agent);
        assert_eq!(taken.len(), 2);
        assert_eq!(engine.list_agent_triggers(old_agent).len(), 0);
        assert_eq!(engine.list_all().len(), 0);

        // Restore under new agent
        let restored = engine.restore_triggers(new_agent, taken);
        assert_eq!(restored, 2);
        assert_eq!(engine.list_agent_triggers(new_agent).len(), 2);

        // Verify patterns and max_fires are preserved
        let triggers = engine.list_agent_triggers(new_agent);
        let has_content_match = triggers.iter().any(|t| {
            matches!(&t.pattern, TriggerPattern::ContentMatch { substring } if substring == "deploy")
                && t.max_fires == 5
        });
        assert!(
            has_content_match,
            "ContentMatch trigger with max_fires=5 should be preserved"
        );
    }

    #[test]
    fn test_take_empty_returns_empty() {
        let engine = TriggerEngine::new();
        let taken = engine.take_agent_triggers(AgentId::new());
        assert!(taken.is_empty());
    }

    #[test]
    fn test_restore_preserves_enabled_state() {
        let engine = TriggerEngine::new();
        let old_agent = AgentId::new();
        let new_agent = AgentId::new();
        let tid = engine.register(old_agent, TriggerPattern::All, "a".to_string(), 0);
        engine.set_enabled(tid, false);

        let taken = engine.take_agent_triggers(old_agent);
        assert_eq!(taken.len(), 1);
        assert!(!taken[0].enabled);

        engine.restore_triggers(new_agent, taken);
        let restored = engine.list_agent_triggers(new_agent);
        assert_eq!(restored.len(), 1);
        assert!(
            !restored[0].enabled,
            "Disabled state should survive take/restore"
        );
    }

    #[test]
    fn test_orchestration_trace_trigger_carries_context() {
        use openfang_types::orchestration_trace::{OrchestrationTraceEvent, TraceEventType};

        let engine = TriggerEngine::new();
        let watcher = AgentId::new();
        let orch = AgentId::new();
        engine.register(
            watcher,
            TriggerPattern::OrchestrationTrace {
                event_types: vec!["agent_failed".to_string()],
                trace_id_substring: None,
                orchestrator_id: Some(orch),
            },
            "Trace: {{event}}".to_string(),
            0,
        );

        let ev = OrchestrationTraceEvent {
            trace_id: "tid-1".to_string(),
            orchestrator_id: orch,
            agent_id: AgentId::new(),
            parent_agent_id: None,
            event_type: TraceEventType::AgentFailed {
                error: "boom".to_string(),
            },
            timestamp: chrono::Utc::now(),
            metadata: std::collections::HashMap::new(),
        };
        let event = Event::new(
            AgentId::new(),
            EventTarget::Broadcast,
            EventPayload::OrchestrationTrace(ev.clone()),
        );
        let matches = engine.evaluate(&event, None);
        assert_eq!(matches.len(), 1);
        let ctx = matches[0].orchestration_ctx.as_ref().expect("ctx");
        assert_eq!(ctx.trace_id, "tid-1");
        assert_eq!(ctx.orchestrator_id, orch);
        assert_eq!(ctx.call_chain, vec![orch, watcher]);
    }
}
