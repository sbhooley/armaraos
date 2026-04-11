//! LRU-backed [`LlmDriver`] factory: stable `reqwest::Client` reuse, timeouts, and metrics.

use super::instrumenting_llm::{InstrumentingLlmDriver, LlmCallMetrics};
use super::{create_driver, driver_cache_key};
use crate::llm_driver::{DriverConfig, LlmDriver, LlmError};
use lru::LruCache;
use openfang_types::config::LlmConfig;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Build an HTTP client for LLM traffic using `[llm]` timeouts.
pub fn build_llm_http_client(cfg: &LlmConfig) -> Result<reqwest::Client, LlmError> {
    reqwest::Client::builder()
        .user_agent(crate::USER_AGENT)
        .timeout(Duration::from_millis(cfg.client_timeout_ms))
        .connect_timeout(Duration::from_millis(cfg.connect_timeout_ms))
        .build()
        .map_err(|e| LlmError::Http(format!("LLM HTTP client build failed: {e}")))
}

struct FactoryInner {
    llm: LlmConfig,
    cache: LruCache<super::DriverCacheKey, Arc<dyn LlmDriver>>,
}

/// Factory for cached, instrumented LLM drivers (kernel lifetime).
pub struct LlmDriverFactory {
    metrics: Arc<LlmCallMetrics>,
    inner: Mutex<FactoryInner>,
}

impl LlmDriverFactory {
    pub fn new(mut initial_llm: LlmConfig, metrics: Arc<LlmCallMetrics>) -> Self {
        initial_llm.clamp_bounds();
        let cap = NonZeroUsize::new(initial_llm.max_cached_drivers.max(1)).unwrap();
        Self {
            metrics,
            inner: Mutex::new(FactoryInner {
                llm: initial_llm,
                cache: LruCache::new(cap),
            }),
        }
    }

    pub fn metrics(&self) -> Arc<LlmCallMetrics> {
        Arc::clone(&self.metrics)
    }

    /// Hot-reload `[llm]`: updates timeouts / isolation / cache size and clears the LRU.
    pub fn apply_llm_config(&self, mut llm: LlmConfig) {
        llm.clamp_bounds();
        let cap = NonZeroUsize::new(llm.max_cached_drivers.max(1)).unwrap();
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.llm = llm;
        g.cache = LruCache::new(cap);
    }

    /// Current effective `[llm]` (timeouts, isolation, cache cap) for code paths that build drivers outside the LRU.
    pub fn live_llm_config(&self) -> LlmConfig {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .llm
            .clone()
    }

    pub fn prometheus_snippet(&self) -> String {
        self.metrics.render_prometheus()
    }

    /// Resolve or create a driver for `config` (instrumented; LRU in `shared` mode).
    pub fn get_driver(&self, config: &DriverConfig) -> Result<Arc<dyn LlmDriver>, LlmError> {
        let key = driver_cache_key(config);
        let label = super::sanitize_provider_prometheus(&key.provider);

        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let isolated = inner.llm.is_isolated_mode();

        if !isolated {
            if let Some(hit) = inner.cache.get(&key) {
                return Ok(Arc::clone(hit));
            }
        }

        let client = build_llm_http_client(&inner.llm)?;
        let http_arc = Arc::new(client);
        let mut dc = config.clone();
        dc.http_client = Some(http_arc);
        let raw = create_driver(&dc)?;
        let wrapped: Arc<dyn LlmDriver> = Arc::new(InstrumentingLlmDriver::new(
            raw,
            label,
            Arc::clone(&self.metrics),
        ));

        if !isolated {
            let _old = inner.cache.put(key, Arc::clone(&wrapped));
        }

        Ok(wrapped)
    }

    #[cfg(test)]
    pub(crate) fn cache_len(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .cache
            .len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm_driver::DriverConfig;

    fn custom_local_cfg(suffix: &str) -> DriverConfig {
        DriverConfig {
            provider: format!("my-factory-{suffix}"),
            api_key: Some("test-key".to_string()),
            base_url: Some(format!("http://127.0.0.1:1{suffix}/v1")),
            skip_permissions: true,
            ..Default::default()
        }
    }

    #[test]
    fn shared_mode_reuses_cached_driver() {
        let f = LlmDriverFactory::new(LlmConfig::default(), Arc::new(LlmCallMetrics::new()));
        let cfg = custom_local_cfg("01");
        let a = f.get_driver(&cfg).expect("driver");
        assert_eq!(f.cache_len(), 1);
        let b = f.get_driver(&cfg).expect("driver 2");
        assert_eq!(f.cache_len(), 1);
        assert!(Arc::ptr_eq(&a, &b));
    }

    #[test]
    fn isolated_mode_skips_lru() {
        let llm = LlmConfig {
            driver_isolation: "isolated".to_string(),
            ..Default::default()
        };
        let f = LlmDriverFactory::new(llm, Arc::new(LlmCallMetrics::new()));
        let cfg = custom_local_cfg("02");
        let a = f.get_driver(&cfg).expect("driver");
        let b = f.get_driver(&cfg).expect("driver 2");
        assert_eq!(f.cache_len(), 0);
        assert!(!Arc::ptr_eq(&a, &b));
    }

    #[tokio::test]
    async fn concurrent_shared_uses_one_cache_slot() {
        let f = Arc::new(LlmDriverFactory::new(
            LlmConfig::default(),
            Arc::new(LlmCallMetrics::new()),
        ));
        let cfg = custom_local_cfg("03");
        let mut handles = Vec::new();
        for _ in 0..24 {
            let fc = Arc::clone(&f);
            let c = cfg.clone();
            handles.push(tokio::spawn(async move { fc.get_driver(&c) }));
        }
        for h in handles {
            h.await.expect("join").expect("get_driver");
        }
        assert_eq!(f.cache_len(), 1);
    }

    #[test]
    fn live_llm_config_tracks_hot_reload() {
        let f = LlmDriverFactory::new(LlmConfig::default(), Arc::new(LlmCallMetrics::new()));
        assert_eq!(
            f.live_llm_config().client_timeout_ms,
            LlmConfig::default().client_timeout_ms
        );

        let updated = LlmConfig {
            client_timeout_ms: 200_000,
            ..Default::default()
        };
        f.apply_llm_config(updated);
        assert_eq!(f.live_llm_config().client_timeout_ms, 200_000);
    }
}
