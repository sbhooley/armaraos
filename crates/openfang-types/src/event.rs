//! Event types for the OpenFang internal event bus.
//!
//! All inter-agent and system communication flows through events.

use crate::agent::AgentId;
use crate::orchestration_trace::OrchestrationTraceEvent;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;
use uuid::Uuid;

/// Serde helper for `Option<Duration>` as milliseconds.
mod duration_ms {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::Duration;

    /// Serialize `Duration` as `u64` milliseconds.
    pub fn serialize<S: Serializer>(dur: &Option<Duration>, s: S) -> Result<S::Ok, S::Error> {
        match dur {
            Some(d) => d.as_millis().serialize(s),
            None => s.serialize_none(),
        }
    }

    /// Deserialize `u64` milliseconds into `Duration`.
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Duration>, D::Error> {
        let opt: Option<u64> = Option::deserialize(d)?;
        Ok(opt.map(Duration::from_millis))
    }
}

/// Unique identifier for an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EventId(pub Uuid);

impl EventId {
    /// Create a new random EventId.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for EventId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for EventId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Where an event is directed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum EventTarget {
    /// Send to a specific agent.
    Agent(AgentId),
    /// Broadcast to all agents.
    Broadcast,
    /// Send to agents matching a pattern (e.g., tag-based).
    Pattern(String),
    /// Send to the kernel/system.
    System,
}

/// The payload of an event.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum EventPayload {
    /// Direct agent-to-agent message.
    Message(AgentMessage),
    /// Tool execution result.
    ToolResult(ToolOutput),
    /// Memory changed notification.
    MemoryUpdate(MemoryDelta),
    /// Agent lifecycle event.
    Lifecycle(LifecycleEvent),
    /// Network event (remote agent activity).
    Network(NetworkEvent),
    /// System event (health, resources).
    System(SystemEvent),
    /// Multi-agent orchestration trace step (also in ring buffer; mirrored here for SSE).
    OrchestrationTrace(OrchestrationTraceEvent),
    /// User-defined payload.
    Custom(Vec<u8>),
}

/// A message between agents or from user to agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMessage {
    /// The text content of the message.
    pub content: String,
    /// Optional structured metadata.
    pub metadata: HashMap<String, serde_json::Value>,
    /// The role of the message sender.
    pub role: MessageRole,
}

/// Role of a message sender.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    /// A human user.
    User,
    /// An AI agent.
    Agent,
    /// The system.
    System,
    /// A tool.
    Tool,
}

/// Output from a tool execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOutput {
    /// Which tool produced this output.
    pub tool_id: String,
    /// The tool_use ID this result corresponds to.
    pub tool_use_id: String,
    /// The output content.
    pub content: String,
    /// Whether the tool execution succeeded.
    pub success: bool,
    /// How long the tool took to execute.
    pub execution_time_ms: u64,
}

/// A change in the memory substrate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryDelta {
    /// What kind of memory operation.
    pub operation: MemoryOperation,
    /// The key that changed.
    pub key: String,
    /// Which agent's memory changed.
    pub agent_id: AgentId,
}

/// The type of memory operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryOperation {
    /// A new value was created.
    Created,
    /// An existing value was updated.
    Updated,
    /// A value was deleted.
    Deleted,
}

/// Agent lifecycle event.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event")]
pub enum LifecycleEvent {
    /// An agent was spawned.
    Spawned {
        /// The new agent's ID.
        agent_id: AgentId,
        /// The new agent's name.
        name: String,
    },
    /// An agent started running.
    Started {
        /// The agent's ID.
        agent_id: AgentId,
    },
    /// An agent was suspended.
    Suspended {
        /// The agent's ID.
        agent_id: AgentId,
    },
    /// An agent was resumed.
    Resumed {
        /// The agent's ID.
        agent_id: AgentId,
    },
    /// An agent was terminated.
    Terminated {
        /// The agent's ID.
        agent_id: AgentId,
        /// The reason for termination.
        reason: String,
    },
    /// An agent crashed.
    Crashed {
        /// The agent's ID.
        agent_id: AgentId,
        /// The error that caused the crash.
        error: String,
    },
}

/// Network-related event.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event")]
pub enum NetworkEvent {
    /// A peer connected.
    PeerConnected {
        /// The peer's ID.
        peer_id: String,
    },
    /// A peer disconnected.
    PeerDisconnected {
        /// The peer's ID.
        peer_id: String,
    },
    /// A message was received from a remote agent.
    MessageReceived {
        /// The peer that sent the message.
        from_peer: String,
        /// The agent that sent the message.
        from_agent: String,
    },
    /// A discovery query returned results.
    DiscoveryResult {
        /// The service that was searched for.
        service: String,
        /// The peers that provide the service.
        providers: Vec<String>,
    },
}

/// Optional provenance for [`SystemEvent::GraphMemoryWrite`] (dashboard timelines, graph UI).
///
/// Older payloads omit this field; clients should treat `None` as “kind only”.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GraphMemoryWriteProvenance {
    /// Node ids (UUID strings) tied to this write notification.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub node_ids: Vec<String>,
    /// `episode` | `semantic` | `procedural` | `persona` | `runtime_state` when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_kind: Option<String>,
    /// Machine-readable reason, e.g. `turn_complete`, `pattern_persisted`, `graph_extractor_pass`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// One-line summary for live timelines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Orchestration / trace correlation when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
}

/// System-level event.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event")]
pub enum SystemEvent {
    /// The kernel has started.
    KernelStarted,
    /// The kernel is stopping.
    KernelStopping,
    /// An agent is approaching a resource quota.
    QuotaWarning {
        /// The agent's ID.
        agent_id: AgentId,
        /// Which resource is running low.
        resource: String,
        /// How much of the quota has been used (0-100).
        usage_percent: f32,
    },
    /// A health check was performed.
    HealthCheck {
        /// The health status.
        status: String,
    },
    /// A quota enforcement event.
    QuotaEnforced {
        /// The agent whose quota was enforced.
        agent_id: AgentId,
        /// Amount spent in the current window.
        spent: f64,
        /// The quota limit.
        limit: f64,
    },
    /// A model was auto-routed based on complexity.
    ModelRouted {
        /// The agent using the routed model.
        agent_id: AgentId,
        /// The detected complexity level.
        complexity: String,
        /// The model selected.
        model: String,
    },
    /// A user action was performed.
    UserAction {
        /// The user who performed the action.
        user_id: String,
        /// The action performed.
        action: String,
        /// The result of the action.
        result: String,
    },
    /// A heartbeat health check failed for an agent.
    HealthCheckFailed {
        /// The agent that failed the health check.
        agent_id: AgentId,
        /// How long the agent has been unresponsive.
        unresponsive_secs: u64,
    },
    /// A scheduled cron job completed successfully.
    CronJobCompleted {
        /// Cron job id.
        job_id: String,
        /// Cron job name.
        job_name: String,
        /// Agent associated with this cron job.
        agent_id: AgentId,
        /// Short preview of the output (truncated).
        output_preview: String,
        /// High-level action classifier for dashboards (`ainl_run`, `agent_turn`, …).
        #[serde(default)]
        action_kind: Option<String>,
    },
    /// A scheduled cron job failed.
    CronJobFailed {
        /// Cron job id.
        job_id: String,
        /// Cron job name.
        job_name: String,
        /// Agent associated with this cron job.
        agent_id: AgentId,
        /// Error message (truncated).
        error: String,
        /// High-level action classifier for dashboards (`ainl_run`, `agent_turn`, …).
        #[serde(default)]
        action_kind: Option<String>,
    },
    /// High-level agent loop progress (thinking, tool use, etc.) for dashboards.
    AgentActivity {
        /// e.g. `thinking`, `tool_use`, `streaming`
        phase: String,
        /// Tool name when `phase` is `tool_use`, optional otherwise.
        detail: Option<String>,
    },
    /// A dangerous tool call is waiting for human approval (dashboard / desktop notify).
    ApprovalPending {
        /// Approval request id (matches API).
        request_id: Uuid,
        /// Agent that requested approval.
        agent_id: AgentId,
        /// Tool name (e.g. `shell_exec`).
        tool_name: String,
        /// Short preview of the action (truncated).
        action_summary: String,
    },
    /// AINL graph memory (`ainl_memory.db`) was written for an agent (dashboard refresh).
    GraphMemoryWrite {
        /// Agent whose graph DB changed.
        agent_id: AgentId,
        /// High-level write kind: `episode`, `fact`, `delegation`, `procedural`, `persona`, etc.
        kind: String,
        /// Optional details for dashboards (node ids, summary, reason).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provenance: Option<GraphMemoryWriteProvenance>,
    },
    /// Multi-step workflow run finished (API or scheduler).
    WorkflowRunFinished {
        /// Workflow definition id (UUID string).
        workflow_id: String,
        /// Registered workflow name.
        workflow_name: String,
        /// Workflow run id (UUID string).
        run_id: String,
        /// Whether the run completed successfully.
        ok: bool,
        /// Output preview on success, or error / timeout text on failure.
        summary: String,
    },
    /// Agent produced an assistant reply visible to the user (chat, channels, etc.).
    ///
    /// Emitted once per completed turn for non-workflow-step turns so workflow runs
    /// surface as [`SystemEvent::WorkflowRunFinished`] instead of per-step noise.
    AgentAssistantReply {
        /// Agent that authored the reply.
        agent_id: AgentId,
        /// Human-readable agent name.
        agent_name: String,
        /// Truncated assistant text.
        message_preview: String,
    },
}

/// A complete event in the OpenFang event system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    /// Unique event ID.
    pub id: EventId,
    /// Which agent (or system) produced this event.
    pub source: AgentId,
    /// Where this event is directed.
    pub target: EventTarget,
    /// The event payload.
    pub payload: EventPayload,
    /// When the event was created.
    pub timestamp: DateTime<Utc>,
    /// For request-response patterns: links response to request.
    pub correlation_id: Option<EventId>,
    /// Time-to-live: event expires after this duration.
    #[serde(with = "duration_ms")]
    pub ttl: Option<Duration>,
}

impl Event {
    /// Create a new event with the given source, target, and payload.
    pub fn new(source: AgentId, target: EventTarget, payload: EventPayload) -> Self {
        Self {
            id: EventId::new(),
            source,
            target,
            payload,
            timestamp: Utc::now(),
            correlation_id: None,
            ttl: None,
        }
    }

    /// Set the correlation ID for request-response linking.
    pub fn with_correlation(mut self, correlation_id: EventId) -> Self {
        self.correlation_id = Some(correlation_id);
        self
    }

    /// Set the TTL for this event.
    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = Some(ttl);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_creation() {
        let agent_id = AgentId::new();
        let event = Event::new(
            agent_id,
            EventTarget::Broadcast,
            EventPayload::System(SystemEvent::KernelStarted),
        );
        assert_eq!(event.source, agent_id);
        assert!(event.correlation_id.is_none());
        assert!(event.ttl.is_none());
    }

    #[test]
    fn test_event_with_correlation() {
        let agent_id = AgentId::new();
        let corr_id = EventId::new();
        let event = Event::new(
            agent_id,
            EventTarget::System,
            EventPayload::System(SystemEvent::HealthCheck {
                status: "ok".to_string(),
            }),
        )
        .with_correlation(corr_id);
        assert_eq!(event.correlation_id, Some(corr_id));
    }

    #[test]
    fn test_event_serialization() {
        let agent_id = AgentId::new();
        let event = Event::new(
            agent_id,
            EventTarget::Agent(AgentId::new()),
            EventPayload::Message(AgentMessage {
                content: "Hello".to_string(),
                metadata: HashMap::new(),
                role: MessageRole::User,
            }),
        );
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: Event = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.id, event.id);
    }

    #[test]
    fn test_event_with_ttl_serialization() {
        let agent_id = AgentId::new();
        let event = Event::new(
            agent_id,
            EventTarget::Broadcast,
            EventPayload::System(SystemEvent::KernelStarted),
        )
        .with_ttl(Duration::from_secs(60));
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: Event = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.ttl, Some(Duration::from_millis(60_000)));
    }

    #[test]
    fn test_system_event_notification_payloads_roundtrip() {
        let agent_id = AgentId::new();
        let evt = Event::new(
            AgentId::new(),
            EventTarget::Broadcast,
            EventPayload::System(SystemEvent::CronJobCompleted {
                job_id: "j1".into(),
                job_name: "nightly".into(),
                agent_id,
                output_preview: "ok".into(),
                action_kind: Some("ainl_run".into()),
            }),
        );
        let json = serde_json::to_string(&evt).unwrap();
        let back: Event = serde_json::from_str(&json).unwrap();
        match back.payload {
            EventPayload::System(SystemEvent::CronJobCompleted {
                action_kind,
                job_name,
                ..
            }) => {
                assert_eq!(job_name, "nightly");
                assert_eq!(action_kind.as_deref(), Some("ainl_run"));
            }
            _ => panic!("expected CronJobCompleted"),
        }

        let wf = Event::new(
            AgentId::new(),
            EventTarget::Broadcast,
            EventPayload::System(SystemEvent::WorkflowRunFinished {
                workflow_id: "wf-uuid".into(),
                workflow_name: "Demo WF".into(),
                run_id: "run-uuid".into(),
                ok: true,
                summary: "done".into(),
            }),
        );
        let j2 = serde_json::to_string(&wf).unwrap();
        let wf2: Event = serde_json::from_str(&j2).unwrap();
        match wf2.payload {
            EventPayload::System(SystemEvent::WorkflowRunFinished { ok, summary, .. }) => {
                assert!(ok);
                assert_eq!(summary, "done");
            }
            _ => panic!("expected WorkflowRunFinished"),
        }
    }

    #[test]
    fn graph_memory_write_provenance_roundtrips() {
        let aid = AgentId::new();
        let ev = SystemEvent::GraphMemoryWrite {
            agent_id: aid,
            kind: "procedural".into(),
            provenance: Some(GraphMemoryWriteProvenance {
                node_ids: vec!["aaaaaaaa-bbbb-4ccc-dddd-eeeeeeeeeeee".into()],
                node_kind: Some("procedural".into()),
                reason: Some("pattern_persisted".into()),
                summary: Some("Pattern test".into()),
                trace_id: Some("trace-1".into()),
            }),
        };
        let json = serde_json::to_string(&ev).unwrap();
        let back: SystemEvent = serde_json::from_str(&json).unwrap();
        match back {
            SystemEvent::GraphMemoryWrite {
                kind, provenance, ..
            } => {
                assert_eq!(kind, "procedural");
                let p = provenance.expect("provenance");
                assert_eq!(p.node_ids.len(), 1);
                assert_eq!(p.reason.as_deref(), Some("pattern_persisted"));
            }
            _ => panic!("expected GraphMemoryWrite"),
        }
    }

    #[test]
    fn graph_memory_write_deserializes_without_provenance_field() {
        let aid = AgentId::new();
        let raw = format!(
            r#"{{"event":"GraphMemoryWrite","agent_id":"{}","kind":"episode"}}"#,
            aid
        );
        let parsed: SystemEvent = serde_json::from_str(&raw).expect("legacy json");
        match parsed {
            SystemEvent::GraphMemoryWrite {
                kind, provenance, ..
            } => {
                assert_eq!(kind, "episode");
                assert!(provenance.is_none());
            }
            _ => panic!("expected GraphMemoryWrite"),
        }
    }
}
