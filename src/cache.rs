use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::types::{ExecuteResponse, Language};

pub const DEFAULT_CACHE_TTL_SECONDS: u64 = 300;
pub const DEFAULT_CACHE_CAPACITY: usize = 1000;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CacheKey {
    pub language: Language,
    pub code_hash: String,
    pub timeout_ms: Option<u64>,
}

impl CacheKey {
    pub fn new(language: Language, code: &str, timeout_ms: Option<u64>) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(code.as_bytes());
        let code_hash = format!("{:x}", hasher.finalize());

        Self {
            language,
            code_hash,
            timeout_ms,
        }
    }

    pub fn to_hash_string(&self) -> String {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.hash(&mut hasher);
        format!("{:x}", hasher.finish())
    }
}

#[derive(Debug, Clone)]
struct CacheEntry {
    response: ExecuteResponse,
    created_at: Instant,
    ttl: Duration,
}

impl CacheEntry {
    fn is_expired(&self) -> bool {
        self.created_at.elapsed() > self.ttl
    }
}

#[derive(Debug, Clone)]
pub struct ExecutionCache {
    inner: Arc<Mutex<CacheInner>>,
}

#[derive(Debug)]
struct CacheInner {
    entries: HashMap<String, CacheEntry>,
    capacity: usize,
    default_ttl: Duration,
    hits: u64,
    misses: u64,
}

impl ExecutionCache {
    pub fn new() -> Self {
        Self::with_config(DEFAULT_CACHE_CAPACITY, Duration::from_secs(DEFAULT_CACHE_TTL_SECONDS))
    }

    pub fn with_config(capacity: usize, default_ttl: Duration) -> Self {
        let inner = CacheInner {
            entries: HashMap::with_capacity(capacity),
            capacity,
            default_ttl,
            hits: 0,
            misses: 0,
        };

        Self {
            inner: Arc::new(Mutex::new(inner)),
        }
    }

    pub fn get(&self, key: &CacheKey) -> Option<ExecuteResponse> {
        let key_str = key.to_hash_string();
        let mut inner = self.inner.lock();

        if let Some(entry) = inner.entries.get(&key_str) {
            if entry.is_expired() {
                inner.entries.remove(&key_str);
                inner.misses += 1;
                return None;
            }
            inner.hits += 1;
            Some(entry.response.clone())
        } else {
            inner.misses += 1;
            None
        }
    }

    pub fn insert(&self, key: &CacheKey, response: ExecuteResponse) {
        self.insert_with_ttl(key, response, None)
    }

    pub fn insert_with_ttl(
        &self,
        key: &CacheKey,
        response: ExecuteResponse,
        ttl: Option<Duration>,
    ) {
        let key_str = key.to_hash_string();
        let mut inner = self.inner.lock();

        if inner.entries.len() >= inner.capacity {
            self.evict_expired(&mut inner);
        }

        if inner.entries.len() >= inner.capacity {
            self.evict_lru(&mut inner);
        }

        let entry = CacheEntry {
            response,
            created_at: Instant::now(),
            ttl: ttl.unwrap_or(inner.default_ttl),
        };

        inner.entries.insert(key_str, entry);
    }

    fn evict_expired(&self, inner: &mut CacheInner) {
        inner.entries.retain(|_, entry| !entry.is_expired());
    }

    fn evict_lru(&self, inner: &mut CacheInner) {
        let mut oldest_key: Option<String> = None;
        let mut oldest_time = Instant::now();

        for (key, entry) in inner.entries.iter() {
            if entry.created_at < oldest_time {
                oldest_time = entry.created_at;
                oldest_key = Some(key.clone());
            }
        }

        if let Some(key) = oldest_key {
            inner.entries.remove(&key);
        }
    }

    pub fn invalidate(&self, key: &CacheKey) {
        let key_str = key.to_hash_string();
        let mut inner = self.inner.lock();
        inner.entries.remove(&key_str);
    }

    pub fn clear(&self) -> usize {
        let mut inner = self.inner.lock();
        let count = inner.entries.len();
        inner.entries.clear();
        inner.hits = 0;
        inner.misses = 0;
        count
    }

    pub fn len(&self) -> usize {
        let inner = self.inner.lock();
        inner.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn stats(&self) -> CacheStats {
        let inner = self.inner.lock();
        CacheStats {
            size: inner.entries.len(),
            capacity: inner.capacity,
            hits: inner.hits,
            misses: inner.misses,
            hit_rate: if inner.hits + inner.misses > 0 {
                inner.hits as f64 / (inner.hits + inner.misses) as f64
            } else {
                0.0
            },
            default_ttl_seconds: inner.default_ttl.as_secs(),
        }
    }

    pub fn cleanup_expired(&self) -> usize {
        let mut inner = self.inner.lock();
        let before = inner.entries.len();
        self.evict_expired(&mut inner);
        before - inner.entries.len()
    }
}

impl Default for ExecutionCache {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheStats {
    pub size: usize,
    pub capacity: usize,
    pub hits: u64,
    pub misses: u64,
    pub hit_rate: f64,
    pub default_ttl_seconds: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn create_test_response(success: bool) -> ExecuteResponse {
        ExecuteResponse {
            execution_id: Uuid::new_v4(),
            success,
            stdout: "test output".to_string(),
            stderr: String::new(),
            execution_time_ms: 100,
            error: None,
            cached: false,
        }
    }

    #[test]
    fn test_cache_key_hashing() {
        let key1 = CacheKey::new(Language::Python, "print('hello')", Some(5000));
        let key2 = CacheKey::new(Language::Python, "print('hello')", Some(5000));
        let key3 = CacheKey::new(Language::Python, "print('world')", Some(5000));
        let key4 = CacheKey::new(Language::JavaScript, "print('hello')", Some(5000));

        assert_eq!(key1.to_hash_string(), key2.to_hash_string());
        assert_ne!(key1.to_hash_string(), key3.to_hash_string());
        assert_ne!(key1.to_hash_string(), key4.to_hash_string());
    }

    #[test]
    fn test_cache_key_excludes_sandbox_id() {
        let code = "print('test')";
        let key1 = CacheKey::new(Language::Python, code, Some(5000));
        let key2 = CacheKey::new(Language::Python, code, Some(5000));

        assert_eq!(key1.code_hash, key2.code_hash);
        assert_eq!(key1.to_hash_string(), key2.to_hash_string());
    }

    #[test]
    fn test_cache_insert_and_get() {
        let cache = ExecutionCache::with_config(100, Duration::from_secs(60));
        let key = CacheKey::new(Language::Python, "print('test')", Some(5000));
        let response = create_test_response(true);

        assert!(cache.get(&key).is_none());
        cache.insert(&key, response.clone());
        assert_eq!(cache.len(), 1);

        let cached = cache.get(&key).unwrap();
        assert_eq!(cached.success, response.success);
        assert_eq!(cached.stdout, response.stdout);
    }

    #[test]
    fn test_cache_ttl_expiration() {
        let cache = ExecutionCache::with_config(100, Duration::from_millis(100));
        let key = CacheKey::new(Language::Python, "print('test')", Some(5000));
        let response = create_test_response(true);

        cache.insert(&key, response);
        assert!(cache.get(&key).is_some());

        std::thread::sleep(Duration::from_millis(150));
        assert!(cache.get(&key).is_none());
    }

    #[test]
    fn test_cache_capacity_eviction() {
        let cache = ExecutionCache::with_config(3, Duration::from_secs(60));

        for i in 0..5 {
            let key = CacheKey::new(
                Language::Python,
                &format!("print({})", i),
                Some(5000),
            );
            let response = create_test_response(true);
            cache.insert(&key, response);
        }

        assert_eq!(cache.len(), 3);
    }

    #[test]
    fn test_cache_stats() {
        let cache = ExecutionCache::new();
        let key1 = CacheKey::new(Language::Python, "print(1)", Some(5000));
        let key2 = CacheKey::new(Language::Python, "print(2)", Some(5000));
        let response = create_test_response(true);

        cache.insert(&key1, response.clone());

        let _ = cache.get(&key1);
        let _ = cache.get(&key1);
        let _ = cache.get(&key2);

        let stats = cache.stats();
        assert_eq!(stats.hits, 2);
        assert_eq!(stats.misses, 1);
        assert!((stats.hit_rate - 2.0 / 3.0).abs() < 0.001);
    }

    #[test]
    fn test_cache_invalidate_and_clear() {
        let cache = ExecutionCache::new();
        let key = CacheKey::new(Language::Python, "print('test')", Some(5000));
        let response = create_test_response(true);

        cache.insert(&key, response);
        assert_eq!(cache.len(), 1);

        cache.invalidate(&key);
        assert_eq!(cache.len(), 0);

        cache.insert(&key, create_test_response(true));
        let cleared = cache.clear();
        assert_eq!(cleared, 1);
        assert!(cache.is_empty());

        let stats = cache.stats();
        assert_eq!(stats.hits, 0);
        assert_eq!(stats.misses, 0);
    }

    #[test]
    fn test_different_timeout_different_keys() {
        let code = "print('test')";
        let key1 = CacheKey::new(Language::Python, code, Some(1000));
        let key2 = CacheKey::new(Language::Python, code, Some(5000));

        assert_ne!(key1.to_hash_string(), key2.to_hash_string());
    }

    #[test]
    fn test_code_hash_consistency() {
        let code = r#"
def hello():
    print("Hello, World!")
    return 42
"#;
        let key1 = CacheKey::new(Language::Python, code, None);
        let key2 = CacheKey::new(Language::Python, code, None);

        assert_eq!(key1.code_hash, key2.code_hash);

        let key3 = CacheKey::new(Language::Python, &code.replace("42", "43"), None);
        assert_ne!(key1.code_hash, key3.code_hash);
    }

    #[test]
    fn test_cleanup_expired() {
        let cache = ExecutionCache::with_config(10, Duration::from_millis(50));

        for i in 0..5 {
            let key = CacheKey::new(
                Language::Python,
                &format!("print({})", i),
                Some(5000),
            );
            cache.insert(&key, create_test_response(true));
        }

        std::thread::sleep(Duration::from_millis(100));
        let cleaned = cache.cleanup_expired();
        assert_eq!(cleaned, 5);
        assert!(cache.is_empty());
    }
}
