use std::collections::HashMap;
use parking_lot::Mutex;
use crate::errors::FrameError;

pub const TIME_WINDOW_MS: u64 = 30_000;
pub const NONCE_CACHE_TTL_MS: u64 = 60_000;

struct Inner {
    seen: HashMap<(u8, u64, [u8; 12]), u64>,
}

/// Replay-protection cache. Uses interior mutability so callers hold `&NonceCache`.
pub struct NonceCache {
    inner: Mutex<Inner>,
}

impl NonceCache {
    pub fn new() -> Self {
        Self { inner: Mutex::new(Inner { seen: HashMap::new() }) }
    }

    /// Rejects if the timestamp is outside the ±30s window, or if the
    /// (src, ts, nonce) tuple was already seen. Evicts stale entries on each call.
    pub fn check_and_insert(
        &self,
        src: u8,
        ts_ms: u64,
        nonce: &[u8; 12],
        local_time_ms: u64,
    ) -> Result<(), FrameError> {
        let diff = local_time_ms.abs_diff(ts_ms);
        if diff > TIME_WINDOW_MS {
            return Err(FrameError::TimeWindowExpired);
        }
        let key = (src, ts_ms, *nonce);
        let mut lock = self.inner.lock();
        if lock.seen.contains_key(&key) {
            return Err(FrameError::ReplayedNonce);
        }
        // Evict before inserting to keep memory bounded.
        lock.seen.retain(|_, inserted_at| {
            local_time_ms.saturating_sub(*inserted_at) < NONCE_CACHE_TTL_MS
        });
        lock.seen.insert(key, local_time_ms);
        Ok(())
    }
}

impl Default for NonceCache {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_replay() {
        let cache = NonceCache::new();
        let nonce = [0u8; 12];
        cache.check_and_insert(1, 1000, &nonce, 1000).unwrap();
        assert_eq!(
            cache.check_and_insert(1, 1000, &nonce, 1000),
            Err(FrameError::ReplayedNonce)
        );
    }

    #[test]
    fn rejects_stale_timestamp() {
        let cache = NonceCache::new();
        let nonce = [0u8; 12];
        let now = 100_000u64;
        let old = now - TIME_WINDOW_MS - 1;
        assert_eq!(
            cache.check_and_insert(1, old, &nonce, now),
            Err(FrameError::TimeWindowExpired)
        );
    }

    #[test]
    fn accepts_fresh_unique_nonce() {
        let cache = NonceCache::new();
        let nonce1 = [0u8; 12];
        let mut nonce2 = [0u8; 12];
        nonce2[0] = 1;
        let now = 50_000u64;
        cache.check_and_insert(1, now, &nonce1, now).unwrap();
        cache.check_and_insert(1, now, &nonce2, now).unwrap();
    }

    #[test]
    fn evicts_old_entries() {
        let cache = NonceCache::new();
        let nonce = [0u8; 12];
        cache.check_and_insert(1, 0, &nonce, 0).unwrap();
        // After TTL passes the entry is gone; same nonce is accepted again.
        let later = NONCE_CACHE_TTL_MS + 1;
        cache.check_and_insert(1, later, &nonce, later).unwrap();
    }
}
