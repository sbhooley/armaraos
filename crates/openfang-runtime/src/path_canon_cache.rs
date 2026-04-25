//! Bounded cache for [`std::fs::canonicalize`] used by `shell_path_guard` to avoid
//! repeated `stat` work within the process lifetime.

use lru::LruCache;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

fn cache() -> &'static Mutex<LruCache<String, Option<PathBuf>>> {
    static C: OnceLock<Mutex<LruCache<String, Option<PathBuf>>>> = OnceLock::new();
    C.get_or_init(|| {
        Mutex::new(LruCache::new(
            NonZeroUsize::new(512).expect("path canon cache cap"),
        ))
    })
}

fn key_for(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// Canonicalize `p` with an in-process LRU cache (up to 512 keys).
pub fn cached_canonicalize(p: &Path) -> Option<PathBuf> {
    let k = key_for(p);
    {
        let mut c = cache().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(v) = c.get(&k) {
            return v.clone();
        }
    }
    let got = p.canonicalize().ok();
    {
        let mut c = cache().lock().unwrap_or_else(|e| e.into_inner());
        c.put(k, got.clone());
    }
    got
}
