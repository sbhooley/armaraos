//! Cache-aware **TTL coordination** hints for compression-related caches.
//!
//! Hosts attach real storage; this module only models **hysteresis** on repeated hits so
//! hot keys do not thrash TTL without unbounded growth.

/// Result of [`effective_ttl_with_hysteresis`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheTtlResult {
    pub base_ttl_secs: u64,
    pub consecutive_hits_same_key: u32,
    pub effective_ttl_secs: u64,
    /// `true` when the global safety cap applied.
    pub hit_global_cap: bool,
}

const MAX_EFFECTIVE_TTL_SECS: u64 = 86_400;

/// Stretch effective TTL modestly on repeated access to the same logical key, capped at **2× base**
/// and at [`MAX_EFFECTIVE_TTL_SECS`].
///
/// `consecutive_hits_same_key`: monotonic counter reset by the host when the key changes.
#[must_use]
pub fn effective_ttl_with_hysteresis(
    base_ttl_secs: u64,
    consecutive_hits_same_key: u32,
) -> CacheTtlResult {
    let base = base_ttl_secs.max(1);
    let hits = consecutive_hits_same_key.min(32);
    // +10% per hit for up to 10 stacked hits → up to 2× base before cap.
    let bump_pct: u64 = (hits as u64).min(10) * 10;
    let numer = 100 + bump_pct;
    let stretched = (base.saturating_mul(numer)) / 100;
    let doubled_cap = stretched.min(base.saturating_mul(2));
    let eff = doubled_cap.min(MAX_EFFECTIVE_TTL_SECS);
    let hit_global_cap = eff == MAX_EFFECTIVE_TTL_SECS && doubled_cap > MAX_EFFECTIVE_TTL_SECS;
    CacheTtlResult {
        base_ttl_secs: base,
        consecutive_hits_same_key: hits,
        effective_ttl_secs: eff,
        hit_global_cap,
    }
}

/// One-line description for operator CLIs / docs.
#[must_use]
pub fn cache_policy_summary() -> &'static str {
    "Cache coordinator v0: same-key hit streak increases effective TTL by +10%/hit (max +100%), then min(2× base, 86400s)."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_hits_is_base() {
        let r = effective_ttl_with_hysteresis(300, 0);
        assert_eq!(r.effective_ttl_secs, 300);
        assert!(!r.hit_global_cap);
    }

    #[test]
    fn many_hits_double_then_cap() {
        let r = effective_ttl_with_hysteresis(50_000, 100);
        assert_eq!(r.effective_ttl_secs, 86_400);
        assert!(r.hit_global_cap);
    }
}
