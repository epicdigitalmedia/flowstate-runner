use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Simple in-memory TTL cache. Entries expire after `ttl` has elapsed
/// since they were inserted.
pub struct TtlCache<V> {
    entries: HashMap<String, CacheEntry<V>>,
    ttl: Duration,
}

struct CacheEntry<V> {
    value: V,
    inserted_at: Instant,
}

impl<V: Clone> TtlCache<V> {
    /// Create a cache with the given time-to-live.
    pub fn new(ttl: Duration) -> Self {
        Self {
            entries: HashMap::new(),
            ttl,
        }
    }

    /// Get a cached value if present and not expired.
    pub fn get(&self, key: &str) -> Option<&V> {
        let entry = self.entries.get(key)?;
        if entry.inserted_at.elapsed() < self.ttl {
            Some(&entry.value)
        } else {
            None
        }
    }

    /// Insert or replace a cached value.
    pub fn insert(&mut self, key: String, value: V) {
        self.entries.insert(
            key,
            CacheEntry {
                value,
                inserted_at: Instant::now(),
            },
        );
    }

    /// Remove expired entries. Call periodically in long-running processes
    /// to prevent unbounded memory growth.
    pub fn evict_expired(&mut self) {
        self.entries
            .retain(|_, entry| entry.inserted_at.elapsed() < self.ttl);
    }

    /// Number of entries (including expired ones not yet evicted).
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_insert_and_get() {
        let mut cache = TtlCache::new(Duration::from_secs(60));
        cache.insert("key1".to_string(), "value1".to_string());
        assert_eq!(cache.get("key1"), Some(&"value1".to_string()));
    }

    #[test]
    fn test_cache_miss() {
        let cache: TtlCache<String> = TtlCache::new(Duration::from_secs(60));
        assert_eq!(cache.get("nonexistent"), None);
    }

    #[test]
    fn test_cache_expired() {
        let mut cache = TtlCache::new(Duration::from_millis(1));
        cache.insert("key1".to_string(), "value1".to_string());
        std::thread::sleep(Duration::from_millis(10));
        assert_eq!(cache.get("key1"), None);
    }

    #[test]
    fn test_cache_evict_expired() {
        let mut cache = TtlCache::new(Duration::from_millis(1));
        cache.insert("key1".to_string(), "value1".to_string());
        std::thread::sleep(Duration::from_millis(10));
        assert_eq!(cache.len(), 1); // still in map
        cache.evict_expired();
        assert_eq!(cache.len(), 0); // now evicted
    }

    #[test]
    fn test_cache_overwrite() {
        let mut cache = TtlCache::new(Duration::from_secs(60));
        cache.insert("key1".to_string(), "v1".to_string());
        cache.insert("key1".to_string(), "v2".to_string());
        assert_eq!(cache.get("key1"), Some(&"v2".to_string()));
    }
}
