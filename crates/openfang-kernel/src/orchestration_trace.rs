//! Bounded in-memory orchestration trace ring buffer.

use chrono::{DateTime, Utc};
use openfang_types::agent::AgentId;
use openfang_types::orchestration_trace::{
    OrchestrationTraceCostLine, OrchestrationTraceCostSummary, OrchestrationTraceEvent,
    OrchestrationTraceSummary, OrchestrationTraceTreeNode, TraceEventType,
};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Mutex;

const DEFAULT_CAP: usize = 4096;

/// Ring buffer of trace events (process-local, best-effort observability).
///
/// **Retention:** capped (default **4096** events). When full, the oldest entries are dropped.
/// Traces are not persisted; use the API or SSE `OrchestrationTrace` payloads for export.
pub struct OrchestrationTraceBuffer {
    inner: Mutex<VecDeque<OrchestrationTraceEvent>>,
    cap: usize,
}

impl OrchestrationTraceBuffer {
    pub fn new(cap: usize) -> Self {
        Self {
            inner: Mutex::new(VecDeque::with_capacity(cap.min(DEFAULT_CAP))),
            cap: cap.max(256),
        }
    }

    pub fn push(&self, event: OrchestrationTraceEvent) {
        let mut q = self.inner.lock().unwrap();
        if q.len() >= self.cap {
            q.pop_front();
        }
        q.push_back(event);
    }

    fn events_snapshot(&self) -> Vec<OrchestrationTraceEvent> {
        self.inner.lock().unwrap().iter().cloned().collect()
    }

    /// Recent distinct traces, newest activity first.
    pub fn list_summaries(&self, limit: usize) -> Vec<OrchestrationTraceSummary> {
        let events = self.events_snapshot();
        let mut by_trace: HashMap<String, (AgentId, DateTime<Utc>, usize)> = HashMap::new();
        for e in &events {
            let entry =
                by_trace
                    .entry(e.trace_id.clone())
                    .or_insert((e.orchestrator_id, e.timestamp, 0));
            entry.2 += 1;
            if e.timestamp > entry.1 {
                entry.1 = e.timestamp;
            }
        }
        let mut v: Vec<_> = by_trace
            .into_iter()
            .map(
                |(trace_id, (orchestrator_id, last_event_at, event_count))| {
                    OrchestrationTraceSummary {
                        trace_id,
                        orchestrator_id,
                        last_event_at,
                        event_count,
                    }
                },
            )
            .collect();
        v.sort_by(|a, b| b.last_event_at.cmp(&a.last_event_at));
        v.truncate(limit.max(1));
        v
    }

    pub fn events_for_trace(&self, trace_id: &str) -> Vec<OrchestrationTraceEvent> {
        self.events_snapshot()
            .into_iter()
            .filter(|e| e.trace_id == trace_id)
            .collect()
    }

    /// Build a shallow tree from delegation edges; root is the orchestrator.
    pub fn trace_tree(&self, trace_id: &str) -> Option<OrchestrationTraceTreeNode> {
        let events: Vec<_> = self.events_for_trace(trace_id);
        if events.is_empty() {
            return None;
        }
        let orchestrator = events[0].orchestrator_id;
        let mut children_map: HashMap<AgentId, Vec<AgentId>> = HashMap::new();
        let mut seen: HashSet<AgentId> = HashSet::new();
        seen.insert(orchestrator);

        for e in &events {
            seen.insert(e.agent_id);
            if let TraceEventType::AgentDelegated { target_agent, .. } = &e.event_type {
                seen.insert(*target_agent);
                children_map
                    .entry(e.agent_id)
                    .or_default()
                    .push(*target_agent);
            }
        }

        fn build(
            id: AgentId,
            parent: Option<AgentId>,
            map: &HashMap<AgentId, Vec<AgentId>>,
        ) -> OrchestrationTraceTreeNode {
            let ch = map.get(&id).cloned().unwrap_or_default();
            let children: Vec<_> = ch.into_iter().map(|c| build(c, Some(id), map)).collect();
            OrchestrationTraceTreeNode {
                agent_id: id,
                parent_agent_id: parent,
                children,
            }
        }

        Some(build(orchestrator, None, &children_map))
    }

    pub fn trace_cost(&self, trace_id: &str) -> Option<OrchestrationTraceCostSummary> {
        let events: Vec<_> = self.events_for_trace(trace_id);
        if events.is_empty() {
            return None;
        }
        let mut by_agent: HashMap<AgentId, OrchestrationTraceCostLine> = HashMap::new();
        let mut total_tokens = 0u64;
        let mut total_cost = 0f64;
        let mut total_dur = 0u64;

        for e in &events {
            if let TraceEventType::AgentCompleted {
                tokens_in,
                tokens_out,
                duration_ms,
                cost_usd,
                ..
            } = e.event_type
            {
                let line = by_agent
                    .entry(e.agent_id)
                    .or_insert(OrchestrationTraceCostLine {
                        agent_id: e.agent_id,
                        tokens_in: 0,
                        tokens_out: 0,
                        cost_usd: 0.0,
                        duration_ms: 0,
                    });
                line.tokens_in += tokens_in;
                line.tokens_out += tokens_out;
                line.cost_usd += cost_usd;
                line.duration_ms += duration_ms;
                total_tokens += tokens_in + tokens_out;
                total_cost += cost_usd;
                total_dur += duration_ms;
            }
        }

        let mut by_agent: Vec<_> = by_agent.into_values().collect();
        by_agent.sort_by(|a, b| a.agent_id.to_string().cmp(&b.agent_id.to_string()));

        Some(OrchestrationTraceCostSummary {
            trace_id: trace_id.to_string(),
            total_tokens,
            total_cost_usd: total_cost,
            total_duration_ms: total_dur,
            by_agent,
        })
    }
}

impl Default for OrchestrationTraceBuffer {
    fn default() -> Self {
        Self::new(DEFAULT_CAP)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openfang_types::orchestration_trace::TraceEventType;

    #[test]
    fn ring_buffer_groups_traces_and_cost() {
        let buf = OrchestrationTraceBuffer::new(100);
        let a = AgentId::new();
        let b = AgentId::new();
        let ev = OrchestrationTraceEvent {
            trace_id: "t1".to_string(),
            orchestrator_id: a,
            agent_id: b,
            parent_agent_id: Some(a),
            event_type: TraceEventType::AgentCompleted {
                result_size: 3,
                tokens_in: 10,
                tokens_out: 20,
                duration_ms: 5,
                cost_usd: 0.01,
            },
            timestamp: chrono::Utc::now(),
            metadata: HashMap::new(),
        };
        buf.push(ev);
        let s = buf.list_summaries(10);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].trace_id, "t1");
        let c = buf.trace_cost("t1").expect("cost");
        assert_eq!(c.total_tokens, 30);
        assert!((c.total_cost_usd - 0.01).abs() < f64::EPSILON);
    }

    #[test]
    fn trace_tree_follows_delegation_edges() {
        let buf = OrchestrationTraceBuffer::new(100);
        let root = AgentId::new();
        let child = AgentId::new();
        buf.push(OrchestrationTraceEvent {
            trace_id: "t-deleg".to_string(),
            orchestrator_id: root,
            agent_id: root,
            parent_agent_id: None,
            event_type: TraceEventType::OrchestrationStart {
                pattern: "test".to_string(),
                initial_input: "hi".to_string(),
            },
            timestamp: chrono::Utc::now(),
            metadata: HashMap::new(),
        });
        buf.push(OrchestrationTraceEvent {
            trace_id: "t-deleg".to_string(),
            orchestrator_id: root,
            agent_id: root,
            parent_agent_id: None,
            event_type: TraceEventType::AgentDelegated {
                target_agent: child,
                task: "subtask".to_string(),
            },
            timestamp: chrono::Utc::now(),
            metadata: HashMap::new(),
        });
        let tree = buf.trace_tree("t-deleg").expect("tree");
        assert_eq!(tree.agent_id, root);
        assert_eq!(tree.children.len(), 1);
        assert_eq!(tree.children[0].agent_id, child);
    }

    /// Stress: many events on one trace; run with `cargo test -p openfang-kernel orchestration_trace_stress -- --ignored`.
    #[test]
    #[ignore = "stress / timing (150 events)"]
    fn orchestration_trace_stress_push_many() {
        let buf = OrchestrationTraceBuffer::new(8192);
        let root = AgentId::new();
        let worker = AgentId::new();
        for i in 0..150 {
            buf.push(OrchestrationTraceEvent {
                trace_id: "stress-trace".to_string(),
                orchestrator_id: root,
                agent_id: worker,
                parent_agent_id: Some(root),
                event_type: TraceEventType::AgentCompleted {
                    result_size: i,
                    tokens_in: 1,
                    tokens_out: 1,
                    duration_ms: 1,
                    cost_usd: 0.0,
                },
                timestamp: chrono::Utc::now(),
                metadata: HashMap::new(),
            });
        }
        let sums = buf.list_summaries(50);
        assert!(
            sums.iter().any(|s| s.trace_id == "stress-trace"),
            "summary row"
        );
        let cost = buf.trace_cost("stress-trace").expect("cost");
        assert_eq!(cost.total_tokens, 300);
        assert!(cost.by_agent.iter().any(|l| l.agent_id == worker));
    }
}
