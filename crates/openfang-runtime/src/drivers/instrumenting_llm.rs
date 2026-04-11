//! Per-provider LLM call metrics (Prometheus text + lightweight atomics).

use crate::llm_driver::{CompletionRequest, CompletionResponse, LlmDriver, LlmError, StreamEvent};
use async_trait::async_trait;
use dashmap::DashMap;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;

/// Aggregated counters for one provider label.
#[derive(Debug)]
pub struct ProviderLlmAgg {
    pub requests: AtomicU64,
    pub errors: AtomicU64,
    pub in_flight: AtomicI64,
    pub latency_sum_ns: AtomicU64,
    pub latency_count: AtomicU64,
}

impl ProviderLlmAgg {
    fn new() -> Self {
        Self {
            requests: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            in_flight: AtomicI64::new(0),
            latency_sum_ns: AtomicU64::new(0),
            latency_count: AtomicU64::new(0),
        }
    }
}

/// Thread-safe per-provider metrics for LLM HTTP traffic.
#[derive(Debug)]
pub struct LlmCallMetrics {
    by_provider: DashMap<String, Arc<ProviderLlmAgg>>,
}

impl Default for LlmCallMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl LlmCallMetrics {
    pub fn new() -> Self {
        Self {
            by_provider: DashMap::new(),
        }
    }

    fn agg(&self, provider: &str) -> Arc<ProviderLlmAgg> {
        self.by_provider
            .entry(provider.to_string())
            .or_insert_with(|| Arc::new(ProviderLlmAgg::new()))
            .clone()
    }

    pub fn on_request_start(&self, provider: &str) {
        let a = self.agg(provider);
        a.requests.fetch_add(1, Ordering::Relaxed);
        a.in_flight.fetch_add(1, Ordering::Relaxed);
    }

    pub fn on_request_end(&self, provider: &str, start: std::time::Instant, is_error: bool) {
        let a = self.agg(provider);
        if is_error {
            a.errors.fetch_add(1, Ordering::Relaxed);
        }
        let ns = start.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
        a.latency_sum_ns.fetch_add(ns, Ordering::Relaxed);
        a.latency_count.fetch_add(1, Ordering::Relaxed);
        a.in_flight.fetch_sub(1, Ordering::Relaxed);
    }

    /// Append Prometheus lines (histogram-style sum + count + gauges).
    pub fn render_prometheus(&self) -> String {
        let mut out = String::with_capacity(512);
        out.push_str("# HELP llm_requests_total Total LLM HTTP requests started.\n");
        out.push_str("# TYPE llm_requests_total counter\n");
        out.push_str("# HELP llm_errors_total Total LLM HTTP requests that ended in error.\n");
        out.push_str("# TYPE llm_errors_total counter\n");
        out.push_str("# HELP llm_in_flight In-flight LLM HTTP requests.\n");
        out.push_str("# TYPE llm_in_flight gauge\n");
        out.push_str("# HELP llm_latency_seconds_sum Sum of LLM request wall times (seconds).\n");
        out.push_str("# TYPE llm_latency_seconds_sum counter\n");
        out.push_str("# HELP llm_latency_seconds_count LLM requests with recorded latency.\n");
        out.push_str("# TYPE llm_latency_seconds_count counter\n");

        for e in self.by_provider.iter() {
            let prov = escape_label(e.key());
            let a = e.value();
            let req = a.requests.load(Ordering::Relaxed);
            let err = a.errors.load(Ordering::Relaxed);
            let inflight = a.in_flight.load(Ordering::Relaxed);
            let sum_ns = a.latency_sum_ns.load(Ordering::Relaxed);
            let cnt = a.latency_count.load(Ordering::Relaxed);
            let sum_sec = (sum_ns as f64) / 1e9;
            out.push_str(&format!(
                "llm_requests_total{{provider=\"{prov}\"}} {req}\n\
                 llm_errors_total{{provider=\"{prov}\"}} {err}\n\
                 llm_in_flight{{provider=\"{prov}\"}} {inflight}\n\
                 llm_latency_seconds_sum{{provider=\"{prov}\"}} {sum_sec:.9}\n\
                 llm_latency_seconds_count{{provider=\"{prov}\"}} {cnt}\n",
            ));
        }
        out.push('\n');
        out
    }
}

fn escape_label(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\\' => "\\\\".to_string(),
            '"' => "\\\"".to_string(),
            '\n' | '\r' => '_'.to_string(),
            c if c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-' => c.to_string(),
            _ => "_".to_string(),
        })
        .collect()
}

/// Wraps an [`LlmDriver`] to record request counts, latency, errors, and in-flight gauge.
pub struct InstrumentingLlmDriver {
    inner: Arc<dyn LlmDriver>,
    provider_label: String,
    metrics: Arc<LlmCallMetrics>,
}

impl InstrumentingLlmDriver {
    pub fn new(
        inner: Arc<dyn LlmDriver>,
        provider_label: String,
        metrics: Arc<LlmCallMetrics>,
    ) -> Self {
        Self {
            inner,
            provider_label,
            metrics,
        }
    }
}

#[async_trait]
impl LlmDriver for InstrumentingLlmDriver {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        self.metrics.on_request_start(&self.provider_label);
        let start = std::time::Instant::now();
        let res = self.inner.complete(request).await;
        self.metrics
            .on_request_end(&self.provider_label, start, res.is_err());
        res
    }

    async fn stream(
        &self,
        request: CompletionRequest,
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<CompletionResponse, LlmError> {
        self.metrics.on_request_start(&self.provider_label);
        let start = std::time::Instant::now();
        let res = self.inner.stream(request, tx).await;
        self.metrics
            .on_request_end(&self.provider_label, start, res.is_err());
        res
    }
}
