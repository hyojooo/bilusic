//! A tiny in-memory TTL cache used to shield upstream metadata providers
//! (QQ Music, …) from redundant fetches.
//!
//! Home / chart data only changes on the provider's schedule (hourly / daily),
//! so re-requesting it on every tab switch or app relaunch is pure waste —
//! and, worse, it's what used to make the home page "flicker" while covers
//! streamed in one-by-one. The frontend already caches in localStorage with a
//! TTL; this backend layer is a second, process-lifetime safety net:
//!
//!   • cold start within the TTL window serves instantly (no QQ round-trip);
//!   • if the frontend and backend TTLs drift slightly, the backend still has
//!     fresh data and absorbs the extra call;
//!   • it's bounded — `put` evicts expired entries when the map gets large.
//!
//! `Mutex<HashMap>` (not `RwLock`) because reads clone the value anyway and
//! the cache is tiny (a handful of entries). The whole struct is `Send + Sync`
//! for any `T: Clone + Send + Sync`, which `HomeFeed` / `Vec<FeedSong>` are.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

struct Entry<T> {
    value: T,
    at: Instant,
}

pub struct TtlCache<T> {
    map: Mutex<HashMap<String, Entry<T>>>,
    ttl: Duration,
    /// Soft cap; once exceeded, `put` sweeps expired entries first.
    capacity: usize,
}

impl<T: Clone> TtlCache<T> {
    pub fn new(ttl: Duration, capacity: usize) -> Self {
        Self {
            map: Mutex::new(HashMap::new()),
            ttl,
            capacity,
        }
    }

    /// Return a clone of the cached value if `key` exists and is still fresh.
    pub fn get(&self, key: &str) -> Option<T> {
        let guard = self.map.lock().unwrap();
        let entry = guard.get(key)?;
        if entry.at.elapsed() > self.ttl {
            return None;
        }
        Some(entry.value.clone())
    }

    /// Insert / refresh `key`. Lazily evicts expired entries once `capacity`
    /// is exceeded so the map can't grow without bound.
    pub fn put(&self, key: String, value: T) {
        let mut guard = self.map.lock().unwrap();
        if guard.len() >= self.capacity {
            self.evict_expired(&mut guard);
        }
        guard.insert(key, Entry { value, at: Instant::now() });
    }

    fn evict_expired(&self, guard: &mut MutexGuard<'_, HashMap<String, Entry<T>>>) {
        let ttl = self.ttl;
        guard.retain(|_, e| e.at.elapsed() <= ttl);
    }

    /// Drop every entry. Used when the active metadata source changes: home /
    /// toplist data is source-specific, so a stale entry for the previous
    /// source must not be served (or written under the new source's key) once
    /// the selection moves on. See [`crate::coordinator::Coordinator::set_active_metadata`].
    pub fn clear(&self) {
        self.map.lock().unwrap().clear();
    }
}
