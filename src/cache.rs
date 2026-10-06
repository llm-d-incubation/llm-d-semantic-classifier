//! Exact-result cache for classification.
//!
//! AC-006 requires a cache hit to bypass the tokenizer and the model forward
//! entirely: an identical previously-classified input must be served from the
//! cache without re-tokenizing or running the Candle model forward.
//!
//! Per `specs/0.1-mvp/design.md`, the cache key is a versioned fingerprint of
//! every semantic input to the classification result: classifier/model/
//! tokenizer/taxonomy revision plus the normalized supplied context. A raw
//! prompt string must never be the sole cache identity.
//!
//! The fingerprint is computed with **blake3** (not a `DefaultHasher`): a
//! `DefaultHasher` is not guaranteed stable across Rust versions, and its 64-bit
//! output is collision-prone enough to serve a wrong classification under
//! revision changes. The key is a 32-byte blake3 fingerprint over classifier_id
//! plus every revision field and the normalized text; each field is length
//! prefixed so concatenation cannot alias across field boundaries.
//!
//! The cache stores the typed [`crate::classify::ClassificationResult`], not a
//! `String`.

// The Redis semantic (L2) backend and its supporting modules are compiled only
// when the `redis-semantic` feature is enabled. The `SemanticCache` trait,
// `NoopSemanticCache`, and `identity_tag` below stay always-compiled — they are
// the seam `ServiceCore` uses, defaulting to the Noop (off) cache.
pub mod text;

#[cfg(feature = "redis-semantic")]
pub mod breaker;
#[cfg(feature = "redis-semantic")]
pub mod redis;
#[cfg(feature = "redis-semantic")]
pub mod redis_codec;
// Semantic cache for full LLM responses (playground/gateway use). Reuses the
// RediSearch vector-KNN machinery in a separate index/namespace.
#[cfg(feature = "redis-semantic")]
pub mod response;

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::num::NonZeroUsize;
use std::str::FromStr;
use std::sync::{Arc, Condvar, Mutex};

use crate::classify::{ClassificationResult, ClassifyError, Embedding};

/// The bounded exact-result cache's eviction policy.
///
/// FIFO remains the default so existing constructors and configurations keep
/// their current behavior and hit-path cost. LRU is an explicit opt-in for
/// workloads whose recently accessed entries are likely to be reused.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CacheEvictionPolicy {
    /// Evict the oldest inserted entry, without updating order on a hit.
    #[default]
    Fifo,
    /// Evict the entry that has gone unaccessed for the longest time.
    Lru,
}

impl FromStr for CacheEvictionPolicy {
    type Err = ParseCacheEvictionPolicyError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "fifo" => Ok(Self::Fifo),
            "lru" => Ok(Self::Lru),
            _ => Err(ParseCacheEvictionPolicyError(value.to_string())),
        }
    }
}

impl<'de> serde::Deserialize<'de> for CacheEvictionPolicy {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = <String as serde::Deserialize>::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

/// An explicit cache eviction policy was not one of the supported values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseCacheEvictionPolicyError(String);

impl fmt::Display for ParseCacheEvictionPolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "unsupported cache eviction policy '{}'; expected 'fifo' or 'lru'",
            self.0
        )
    }
}

impl std::error::Error for ParseCacheEvictionPolicyError {}

/// The path a request took through the cache pipeline.
///
/// A cache interaction has three possible outcomes, each with distinct
/// latency characteristics (AC-006/AC-007):
///   - `Hit`:       result was already cached (~microseconds)
///   - `Miss`:      ran the forward closure (tokenize + embed + rank)
///   - `Coalesced`: waited for another thread's in-flight forward
///
/// The distinction matters for metrics accuracy: a coalesced wait
/// experiences miss-class latency but was previously counted as a hit
/// because no forward closure ran on that thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CachePath {
    /// Served from the exact-result cache (no forward, no wait).
    Hit,
    /// Ran the forward closure (designated single-flight forwarder).
    Miss,
    /// Waited for another thread's in-flight forward result.
    Coalesced,
}

/// A versioned fingerprint cache key (design.md).
///
/// The key is a 32-byte blake3 fingerprint over classifier/model/tokenizer/
/// taxonomy revisions plus the normalized input text (length-prefixed so field
/// boundaries cannot collide). Two keys are equal only if the fingerprints
/// match; a revision change with identical text yields a different fingerprint,
/// so a stale cached classification is never served under a new revision.
#[derive(Debug, Clone)]
pub struct CacheKey {
    fingerprint: [u8; 32],
}

impl CacheKey {
    /// Build a versioned fingerprint key.
    ///
    /// `normalized_text` is the preprocessed/normalized input; only a stable
    /// hash of it is retained, never the raw prompt string.
    pub fn new(
        classifier_id: impl Into<String>,
        model_revision: impl Into<String>,
        tokenizer_revision: impl Into<String>,
        taxonomy_revision: impl Into<String>,
        normalized_text: &str,
    ) -> Self {
        Self::new_with_artifact_digest(
            classifier_id,
            model_revision,
            tokenizer_revision,
            taxonomy_revision,
            normalized_text,
            None,
        )
    }

    /// Build a key with both the declared revision and the loaded artifact's
    /// content digest. The optional digest is a separate length-prefixed field;
    /// it never replaces the model revision. `None` preserves `new`'s identity.
    pub fn new_with_artifact_digest(
        classifier_id: impl Into<String>,
        model_revision: impl Into<String>,
        tokenizer_revision: impl Into<String>,
        taxonomy_revision: impl Into<String>,
        normalized_text: &str,
        artifact_digest: Option<&str>,
    ) -> Self {
        let mut hasher = blake3::Hasher::new();
        update_field(&mut hasher, &classifier_id.into());
        update_field(&mut hasher, &model_revision.into());
        update_field(&mut hasher, &tokenizer_revision.into());
        update_field(&mut hasher, &taxonomy_revision.into());
        update_field(&mut hasher, normalized_text);
        if let Some(digest) = artifact_digest {
            update_field(&mut hasher, digest);
        }
        CacheKey {
            fingerprint: hasher.finalize().into(),
        }
    }

    /// The 32-byte blake3 fingerprint used as the cache key.
    pub fn fingerprint(&self) -> [u8; 32] {
        self.fingerprint
    }
}

/// Update `hasher` with a length-prefixed field so concatenation of adjacent
/// fields cannot alias (e.g. ("ab","c") vs ("a","bc")).
fn update_field(hasher: &mut blake3::Hasher, field: &str) {
    hasher.update(&(field.len() as u64).to_le_bytes());
    hasher.update(field.as_bytes());
}

impl PartialEq for CacheKey {
    fn eq(&self, other: &Self) -> bool {
        self.fingerprint == other.fingerprint
    }
}
impl Eq for CacheKey {}

impl Hash for CacheKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.fingerprint.hash(state);
    }
}

/// An exact-result cache mapping an input key to its classification output.
///
/// AC-006 contract (U-040): a cache HIT must bypass the tokenizer and model
/// forward. The forward closure is the tokenize + model-forward stage; on a hit
/// it must not be invoked at all. The forward returns a `Result`; only
/// successful results are stored, failures are returned without caching.
/// Default entry ceiling.
///
/// A classification result is small (a handful of labels and revision strings),
/// so tens of thousands of entries is a modest footprint. The point is that the
/// number is FINITE: this cache sits in a long-lived service on a network
/// request path, and an unbounded map there is a memory leak with a delay fuse.
pub const DEFAULT_CAPACITY: usize = 50_000;

enum CacheEntries {
    /// The existing low-overhead default: a map plus insertion-order queue.
    Fifo {
        entries: HashMap<CacheKey, ClassificationResult>,
        order: VecDeque<CacheKey>,
    },
    /// Opt-in recency-aware storage. `LruCache` keeps both entries and its
    /// linked recency metadata bounded by the configured capacity.
    Lru(lru::LruCache<CacheKey, ClassificationResult>),
}

pub struct ExactCache {
    entries: CacheEntries,
    policy: CacheEvictionPolicy,
    capacity: usize,
    forward_count: u64,
    hit_count: u64,
    evicted_count: u64,
}

impl ExactCache {
    /// An empty cache with no entries.
    pub fn new() -> Self {
        Self::with_capacity_and_policy(DEFAULT_CAPACITY, CacheEvictionPolicy::Fifo)
    }

    /// An empty cache using `policy` and the default capacity.
    pub fn with_policy(policy: CacheEvictionPolicy) -> Self {
        Self::with_capacity_and_policy(DEFAULT_CAPACITY, policy)
    }

    /// An empty FIFO cache holding at most `capacity` entries.
    pub fn with_capacity(capacity: usize) -> Self {
        Self::with_capacity_and_policy(capacity, CacheEvictionPolicy::Fifo)
    }

    /// An empty cache holding at most `capacity` entries under `policy`.
    pub fn with_capacity_and_policy(capacity: usize, policy: CacheEvictionPolicy) -> Self {
        let capacity = capacity.max(1);
        let entries = match policy {
            CacheEvictionPolicy::Fifo => CacheEntries::Fifo {
                entries: HashMap::new(),
                order: VecDeque::new(),
            },
            CacheEvictionPolicy::Lru => CacheEntries::Lru(lru::LruCache::new(
                NonZeroUsize::new(capacity).expect("cache capacity was clamped to at least one"),
            )),
        };
        ExactCache {
            entries,
            policy,
            capacity,
            forward_count: 0,
            hit_count: 0,
            evicted_count: 0,
        }
    }

    /// Store a result, evicting the oldest entry if the cache is at capacity.
    fn store(&mut self, key: CacheKey, result: ClassificationResult) {
        match &mut self.entries {
            CacheEntries::Fifo { entries, order } => {
                // Re-storing an existing key must not consume a second slot,
                // and FIFO must not touch insertion order.
                if let Some(existing) = entries.get_mut(&key) {
                    *existing = result;
                    return;
                }
                while entries.len() >= self.capacity {
                    match order.pop_front() {
                        Some(oldest) => {
                            if entries.remove(&oldest).is_some() {
                                self.evicted_count += 1;
                            }
                        }
                        None => break,
                    }
                }
                order.push_back(key.clone());
                entries.insert(key, result);
            }
            CacheEntries::Lru(entries) => {
                // `put` refreshes an existing key without consuming a slot.
                // For a new key, `push` returns the capacity eviction, if any.
                if entries.peek(&key).is_some() {
                    entries.put(key, result);
                } else if entries.push(key, result).is_some() {
                    self.evicted_count += 1;
                }
            }
        }
    }

    /// Number of entries evicted to stay within capacity.
    pub fn evicted_count(&self) -> u64 {
        self.evicted_count
    }

    /// Current number of cached entries.
    pub fn len(&self) -> usize {
        match &self.entries {
            CacheEntries::Fifo { entries, .. } => entries.len(),
            CacheEntries::Lru(entries) => entries.len(),
        }
    }

    /// True when the cache holds no entries.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The eviction policy selected when this cache was constructed.
    pub fn policy(&self) -> CacheEvictionPolicy {
        self.policy
    }

    /// Classify `key`.
    ///
    /// On a cache hit the cached result is returned WITHOUT invoking the
    /// forward closure (tokenizer + model forward bypassed). On a miss the
    /// forward closure is invoked exactly once; a successful result is stored
    /// and returned, a failure is returned without caching (never fabricated).
    pub fn classify(
        &mut self,
        key: CacheKey,
        forward: impl FnOnce() -> Result<ClassificationResult, ClassifyError>,
    ) -> Result<ClassificationResult, ClassifyError> {
        // AC-006: a cache HIT must bypass the tokenizer and model forward.
        if let Some(cached) = self.cached_value(&key) {
            self.hit_count += 1;
            return Ok(cached);
        }
        // Miss: run the forward exactly once, store, and return.
        let result = forward();
        match &result {
            Ok(result) => {
                self.forward_count += 1;
                self.store(key, result.clone());
            }
            Err(_) => {
                // A failed forward is not cached; the error is explicit.
                self.forward_count += 1;
            }
        }
        result
    }

    /// Number of times the tokenizer/model forward was invoked. AC-006: a
    /// cache hit must not increment this.
    pub fn forward_count(&self) -> u64 {
        self.forward_count
    }

    /// Number of cache hits served. AC-006 observability: increments on hits.
    pub fn hit_count(&self) -> u64 {
        self.hit_count
    }

    /// Read a cached result without recording a hit or running a forward.
    /// Used by the concurrent `SharedCache` fast path (AC-007).
    pub(crate) fn cached_value(&mut self, key: &CacheKey) -> Option<ClassificationResult> {
        match &mut self.entries {
            CacheEntries::Fifo { entries, .. } => entries.get(key).cloned(),
            CacheEntries::Lru(entries) => entries.get(key).cloned(),
        }
    }

    /// Record a forward and store its freshly-computed successful result.
    /// Used by the concurrent `SharedCache` after it runs the forward (AC-007).
    pub(crate) fn store_after_forward(
        &mut self,
        key: CacheKey,
        result: ClassificationResult,
    ) -> ClassificationResult {
        self.forward_count += 1;
        self.store(key, result.clone());
        result
    }
}

/// A shared exact cache that can be called concurrently from multiple threads.
///
/// AC-007 requires that identical concurrent misses do not create unbounded
/// forwards: when N identical requests miss simultaneously, they must be
/// coalesced into ONE forward rather than N redundant tokenizer/model forwards.
/// A per-key single-flight slot (single-flight) is used so the first miss for a
/// key runs the forward once while the other identical misses wait for its
/// result.
///
/// [`Clone`] is derived (both fields are [`Arc`]s) so the cache can be shared by
/// a [`crate::classify::ClassifyService`] that must itself be `Clone` to back a
/// tonic server.
#[derive(Clone)]
pub struct SharedCache {
    inner: Arc<Mutex<ExactCache>>,
    in_flight: Arc<Mutex<HashMap<CacheKey, Arc<InFlight>>>>,
}

/// A single-flight slot for one cache key: the shared result (initially empty)
/// and a condvar that the waiting callers observe. The shared result is the
/// full `Result` (success or the explicit error) so every waiting caller
/// receives the same outcome.
struct InFlight {
    result: Mutex<Option<Result<ClassificationResult, ClassifyError>>>,
    condvar: Condvar,
}

impl SharedCache {
    /// An empty shared FIFO cache.
    pub fn new() -> Self {
        Self::with_policy(CacheEvictionPolicy::Fifo)
    }

    /// An empty shared cache using `policy` and the default capacity.
    pub fn with_policy(policy: CacheEvictionPolicy) -> Self {
        Self::with_capacity_and_policy(DEFAULT_CAPACITY, policy)
    }

    /// An empty shared cache using `policy` and holding at most `capacity`
    /// stored results. In-flight work is tracked separately and remains under
    /// the existing single-flight contract.
    pub fn with_capacity_and_policy(capacity: usize, policy: CacheEvictionPolicy) -> Self {
        SharedCache {
            inner: Arc::new(Mutex::new(ExactCache::with_capacity_and_policy(
                capacity, policy,
            ))),
            in_flight: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// The stored-entry eviction policy. Single-flight behavior is independent
    /// of this setting.
    pub fn policy(&self) -> CacheEvictionPolicy {
        self.inner.lock().unwrap().policy()
    }

    /// Classify `key` concurrently, returning the classification result and
    /// the [`CachePath`] that produced it.
    ///
    /// Serves an already-cached result on the fast path (`CachePath::Hit`).
    /// On a miss, the FIRST caller for `key` runs the forward closure once
    /// (single-flight, `CachePath::Miss`); every other identical concurrent
    /// miss waits for and reads that shared result instead of running its own
    /// forward (`CachePath::Coalesced`). This bounds identical concurrent
    /// misses to ONE forward per distinct key (AC-007). A failed forward is
    /// propagated to every caller and is NOT cached.
    pub fn classify_concurrent(
        &self,
        key: CacheKey,
        forward: impl FnOnce() -> Result<ClassificationResult, ClassifyError>,
    ) -> (Result<ClassificationResult, ClassifyError>, CachePath) {
        // Fast path: serve an already-cached result.
        {
            let mut inner = self.inner.lock().unwrap();
            if let Some(cached) = inner.cached_value(&key) {
                return (Ok(cached), CachePath::Hit);
            }
        }
        // Single-flight: if another thread is already forwarding this key, wait
        // for and read its shared result instead of forwarding again.
        let wait_slot = {
            let mut in_flight = self.in_flight.lock().unwrap();
            match in_flight.get(&key) {
                Some(slot) => Some(Arc::clone(slot)),
                None => {
                    let slot = Arc::new(InFlight {
                        result: Mutex::new(None),
                        condvar: Condvar::new(),
                    });
                    in_flight.insert(key.clone(), Arc::clone(&slot));
                    None
                }
            }
        };
        if let Some(slot) = wait_slot {
            let mut result = slot.result.lock().unwrap();
            while result.is_none() {
                result = slot.condvar.wait(result).unwrap();
            }
            return (result.clone().unwrap(), CachePath::Coalesced);
        }
        // We are the designated forwarder: run the forward exactly once.
        let result = forward();
        if let Ok(result) = &result {
            let mut inner = self.inner.lock().unwrap();
            inner.store_after_forward(key.clone(), result.clone());
        }
        // Publish the result to the waiting callers and remove the in-flight slot.
        let slot = {
            let mut in_flight = self.in_flight.lock().unwrap();
            in_flight.remove(&key).unwrap()
        };
        let mut result_guard = slot.result.lock().unwrap();
        *result_guard = Some(result.clone());
        drop(result_guard);
        slot.condvar.notify_all();
        (result, CachePath::Miss)
    }

    /// Number of times the tokenizer/model forward was invoked (all threads).
    pub fn forward_count(&self) -> u64 {
        self.inner.lock().unwrap().forward_count()
    }

    /// Number of stored results. In-flight work is not part of this bound.
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().len()
    }

    /// True when no completed classification result is cached.
    pub fn is_empty(&self) -> bool {
        self.inner.lock().unwrap().is_empty()
    }

    /// Number of stored results evicted to preserve the configured capacity.
    pub fn evicted_count(&self) -> u64 {
        self.inner.lock().unwrap().evicted_count()
    }
}

impl Default for SharedCache {
    fn default() -> Self {
        SharedCache::new()
    }
}

impl Default for ExactCache {
    fn default() -> Self {
        ExactCache::new()
    }
}

/// The pluggable L2 (semantic / approximate) cache seam.
///
/// Interposes on the embedding between `embed` and `rank`. It is BEST-EFFORT:
/// `lookup` returns `None` on any error (fail-open to compute) and `insert`
/// is fire-and-forget. `identity` isolates entries by classifier/model/
/// tokenizer/taxonomy so a revision change can never serve a stale label.
pub trait SemanticCache: Send + Sync {
    /// Return a stored result whose embedding is within the configured
    /// similarity threshold of `embedding` and shares `identity`, else `None`.
    fn lookup(&self, embedding: &Embedding, identity: &str) -> Option<ClassificationResult>;

    /// Record `result` under `embedding` and `identity`. Best-effort; never blocks.
    fn insert(&self, embedding: &Embedding, result: &ClassificationResult, identity: &str);
}

/// The default L2 cache: always misses, never stores. Zero cost when the
/// semantic tier is disabled.
pub struct NoopSemanticCache;

impl SemanticCache for NoopSemanticCache {
    fn lookup(&self, _embedding: &Embedding, _identity: &str) -> Option<ClassificationResult> {
        None
    }
    fn insert(&self, _embedding: &Embedding, _result: &ClassificationResult, _identity: &str) {}
}

/// Build the L2 isolation tag from a cache-identity tuple (same fields as the
/// blake3 L1 key), pipe-separated so field boundaries cannot alias.
///
/// The artifact digest is a fifth, OPTIONAL field and is appended only when
/// present, mirroring how the L1 key hashes it. That is what keeps the two
/// tiers in step: L1 now isolates by loaded-artifact digest, so an L2 tag that
/// ignored it would let a digest change with an UNCHANGED revision serve a
/// stale semantic label from L2 while L1 correctly missed -- reintroducing, one
/// tier down, exactly the aliasing the digest was added to prevent.
///
/// `None` emits the original four-field tag, so identities without a digest
/// keep their previous value and no existing L2 entry is orphaned.
pub fn identity_tag(id: (&str, &str, &str, &str, Option<&str>)) -> String {
    match id.4 {
        Some(digest) => format!("{}|{}|{}|{}|{}", id.0, id.1, id.2, id.3, digest),
        None => format!("{}|{}|{}|{}", id.0, id.1, id.2, id.3),
    }
}

#[cfg(test)]
mod bounded_tests {
    use super::*;
    use std::sync::{Arc, Barrier};
    use std::thread;

    fn result(id: &str) -> ClassificationResult {
        ClassificationResult {
            classifier_id: "t".into(),
            model_revision: "t".into(),
            tokenizer_revision: "t".into(),
            taxonomy_revision: "t".into(),
            status: crate::classify::ClassifyStatus::Ok,
            ranked: vec![crate::classify::RankedSignal {
                id: id.into(),
                score: 1.0,
            }],
        }
    }

    /// U-046: neither policy may grow beyond its configured capacity.
    ///
    /// This is the defect that does not show up in any functional test: an
    /// unbounded map serves correct results forever and simply consumes the
    /// process. Asserting on the entry count is the only way to see it.
    #[test]
    fn u046_cache_respects_its_capacity_under_both_policies() {
        for policy in [CacheEvictionPolicy::Fifo, CacheEvictionPolicy::Lru] {
            let mut cache = ExactCache::with_capacity_and_policy(16, policy);
            for i in 0..500 {
                let key = CacheKey::new("c", "m", "t", "x", &format!("distinct input {i}"));
                cache.classify(key, || Ok(result("a"))).unwrap();
            }
            assert!(
                cache.len() <= 16,
                "{policy:?} cache holds {} entries with a capacity of 16",
                cache.len()
            );
            assert!(
                cache.evicted_count() > 0,
                "{policy:?} eviction must have occurred"
            );
        }
    }

    /// U-046: FIFO removes the oldest inserted entry, and a re-stored key does
    /// not consume a second slot.
    #[test]
    fn u046_fifo_eviction_is_oldest_first_and_does_not_double_count() {
        let mut cache = ExactCache::with_capacity(2);
        let k = |n: &str| CacheKey::new("c", "m", "t", "x", n);

        cache.classify(k("first"), || Ok(result("a"))).unwrap();
        cache.classify(k("second"), || Ok(result("b"))).unwrap();
        // Re-classifying an existing key is a HIT and must not grow the cache.
        cache
            .classify(k("first"), || panic!("must be a hit"))
            .unwrap();
        assert_eq!(cache.len(), 2);

        // The third distinct key evicts the oldest, which is "first".
        cache.classify(k("third"), || Ok(result("c"))).unwrap();
        assert_eq!(cache.len(), 2);

        let mut forwarded = false;
        cache
            .classify(k("first"), || {
                forwarded = true;
                Ok(result("a"))
            })
            .unwrap();
        assert!(forwarded, "the oldest entry must have been evicted");
    }

    #[test]
    fn u046_lru_evicts_the_least_recently_used_entry() {
        let mut cache = ExactCache::with_capacity_and_policy(2, CacheEvictionPolicy::Lru);
        let k = |n: &str| CacheKey::new("c", "m", "t", "x", n);

        cache.classify(k("first"), || Ok(result("a"))).unwrap();
        cache.classify(k("second"), || Ok(result("b"))).unwrap();
        cache
            .classify(k("first"), || panic!("must be a hit"))
            .unwrap();
        cache.classify(k("third"), || Ok(result("c"))).unwrap();

        let forwards = cache.forward_count();
        cache
            .classify(k("first"), || panic!("recently used entry must remain"))
            .unwrap();
        assert_eq!(cache.forward_count(), forwards);

        let mut second_forwarded = false;
        cache
            .classify(k("second"), || {
                second_forwarded = true;
                Ok(result("b"))
            })
            .unwrap();
        assert!(
            second_forwarded,
            "least recently used entry must be evicted"
        );
    }

    #[test]
    fn u007_existing_constructors_default_to_fifo() {
        assert_eq!(ExactCache::new().policy(), CacheEvictionPolicy::Fifo);
        assert_eq!(
            ExactCache::with_capacity(2).policy(),
            CacheEvictionPolicy::Fifo
        );
        assert_eq!(SharedCache::new().policy(), CacheEvictionPolicy::Fifo);
        assert_eq!(CacheEvictionPolicy::default(), CacheEvictionPolicy::Fifo);
    }

    #[test]
    fn u046_shared_cache_stays_bounded_during_concurrent_eviction() {
        const CAPACITY: usize = 4;
        const CALLERS: usize = 64;

        for policy in [CacheEvictionPolicy::Fifo, CacheEvictionPolicy::Lru] {
            let cache = Arc::new(SharedCache::with_capacity_and_policy(CAPACITY, policy));
            let barrier = Arc::new(Barrier::new(CALLERS));
            let handles: Vec<_> = (0..CALLERS)
                .map(|index| {
                    let cache = Arc::clone(&cache);
                    let barrier = Arc::clone(&barrier);
                    thread::spawn(move || {
                        barrier.wait();
                        let key = CacheKey::new(
                            "classifier",
                            "model",
                            "tokenizer",
                            "taxonomy",
                            &format!("distinct-{index}"),
                        );
                        cache.classify_concurrent(key, || Ok(result("value")))
                    })
                })
                .collect();

            for handle in handles {
                handle.join().unwrap().0.unwrap();
            }
            assert!(cache.len() <= CAPACITY, "{policy:?} exceeded capacity");
            assert!(cache.evicted_count() > 0, "{policy:?} did not evict");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CacheEvictionPolicy, CacheKey, CachePath, ExactCache, SharedCache};
    use crate::classify::{ClassificationResult, ClassifyError, ClassifyStatus, RankedSignal};
    use std::sync::{Arc, Barrier};
    use std::thread;

    /// A typed successful classification result for cache tests.
    fn result(id: &str) -> ClassificationResult {
        ClassificationResult {
            classifier_id: "clf".to_string(),
            model_revision: "model-rev".to_string(),
            tokenizer_revision: "tok-rev".to_string(),
            taxonomy_revision: "tax-rev".to_string(),
            status: ClassifyStatus::Ok,
            ranked: vec![RankedSignal {
                id: id.to_string(),
                score: 1.0,
            }],
        }
    }

    #[test]
    fn u040_exact_cache_hit_bypasses_tokenizer_and_runtime() {
        // U-040 (AC-006): an exact cache hit must bypass the tokenizer and the
        // model forward. The forward closure stands in for the tokenize +
        // model-forward stage; it must run exactly ONCE (on the miss) and must
        // NOT run again when the identical input is served from the cache. The
        // cache stores the typed ClassificationResult.
        let mut cache = ExactCache::new();
        let key = CacheKey::new(
            "clf",
            "model-rev",
            "tok-rev",
            "tax-rev",
            "golden sensitivity input",
        );

        // First call: a miss, so tokenizer + model forward must run once.
        let first = cache
            .classify(key.clone(), || Ok(result("sensitivity")))
            .expect("first classify must succeed");
        assert_eq!(
            cache.forward_count(),
            1,
            "a cache miss must run the tokenizer/model forward once"
        );

        // Second identical call: a HIT. The tokenizer/model forward must be
        // bypassed entirely and the cached result returned unchanged.
        let second = cache
            .classify(key.clone(), || Ok(result("sensitivity")))
            .expect("second classify must succeed");
        assert_eq!(
            cache.forward_count(),
            1,
            "cache hit must bypass the tokenizer and model forward"
        );

        // The served result is the original cached classification.
        assert_eq!(
            first, second,
            "cache hit must return the exact cached result"
        );
        assert_eq!(
            cache.hit_count(),
            1,
            "the second identical call must be counted as a cache hit"
        );
    }

    #[test]
    fn u046_failed_forwards_are_not_cached_under_either_policy() {
        for policy in [CacheEvictionPolicy::Fifo, CacheEvictionPolicy::Lru] {
            let mut cache = ExactCache::with_capacity_and_policy(2, policy);
            let key = CacheKey::new("clf", "model", "tokenizer", "taxonomy", "input");

            let first = cache.classify(key.clone(), || {
                Err(ClassifyError::Unavailable("failed".into()))
            });
            assert!(first.is_err());

            let mut retried = false;
            cache
                .classify(key, || {
                    retried = true;
                    Ok(result("recovered"))
                })
                .unwrap();
            assert!(retried, "{policy:?} must not cache a failed forward");
        }
    }

    #[test]
    fn u042_cache_key_changes_with_model_classifier_revision() {
        // U-042 (AC-006): the cache key must change when the model/classifier
        // revision changes. Identical normalized text under a different
        // model/classifier revision must produce a different key -> a cache MISS
        // (never a stale cached classification from the old revision).
        let mut cache = ExactCache::new();
        let text = "same normalized input";
        let key_a = CacheKey::new("clf", "model-rev-1", "tok-rev", "tax-rev", text);
        let key_b = CacheKey::new("clf", "model-rev-2", "tok-rev", "tax-rev", text);

        assert_ne!(
            key_a, key_b,
            "a model/classifier revision change must change the cache key"
        );

        cache
            .classify(key_a.clone(), || Ok(result("result")))
            .unwrap();
        cache
            .classify(key_b.clone(), || Ok(result("result")))
            .unwrap();
        assert_eq!(
            cache.forward_count(),
            2,
            "a model/classifier revision change must not serve the stale cached result (miss)"
        );
    }

    #[test]
    fn u043_cache_key_changes_with_tokenizer_revision() {
        // U-043 (AC-006): the cache key must change when the tokenizer revision
        // changes. Identical normalized text under a different tokenizer
        // revision must produce a different key -> a cache MISS.
        let mut cache = ExactCache::new();
        let text = "same normalized input";
        let key_a = CacheKey::new("clf", "model-rev", "tok-rev-1", "tax-rev", text);
        let key_b = CacheKey::new("clf", "model-rev", "tok-rev-2", "tax-rev", text);

        assert_ne!(
            key_a, key_b,
            "a tokenizer revision change must change the cache key"
        );

        cache
            .classify(key_a.clone(), || Ok(result("result")))
            .unwrap();
        cache
            .classify(key_b.clone(), || Ok(result("result")))
            .unwrap();
        assert_eq!(
            cache.forward_count(),
            2,
            "a tokenizer revision change must not serve the stale cached result (miss)"
        );
    }

    #[test]
    fn u044_cache_key_changes_with_taxonomy_revision() {
        // U-044 (AC-006): the cache key must change when the taxonomy/prototype
        // revision changes. Identical normalized text under a different
        // taxonomy/prototype revision must produce a different key -> a MISS.
        let mut cache = ExactCache::new();
        let text = "same normalized input";
        let key_a = CacheKey::new("clf", "model-rev", "tok-rev", "tax-rev-1", text);
        let key_b = CacheKey::new("clf", "model-rev", "tok-rev", "tax-rev-2", text);

        assert_ne!(
            key_a, key_b,
            "a taxonomy/prototype revision change must change the cache key"
        );

        cache
            .classify(key_a.clone(), || Ok(result("result")))
            .unwrap();
        cache
            .classify(key_b.clone(), || Ok(result("result")))
            .unwrap();
        assert_eq!(
            cache.forward_count(),
            2,
            "a taxonomy/prototype revision change must not serve the stale cached result (miss)"
        );
    }

    #[test]
    fn u041_identical_concurrent_misses_coalesce() {
        // U-041 (AC-007): identical concurrent MISSES on an empty cache must be
        // coalesced into a SINGLE forward, not one forward per request. N
        // simultaneous misses on the same key must produce exactly ONE
        // tokenizer/model forward (bounded), and every caller must receive the
        // same result.
        const CONCURRENCY: usize = 8;
        for policy in [CacheEvictionPolicy::Fifo, CacheEvictionPolicy::Lru] {
            let cache = Arc::new(SharedCache::with_policy(policy));
            let key = CacheKey::new(
                "clf",
                "model-rev",
                "tok-rev",
                "tax-rev",
                "same sensitivity input",
            );

            // A barrier synchronizes all N threads so they reach
            // `classify_concurrent` together as concurrent misses.
            let barrier = Arc::new(Barrier::new(CONCURRENCY));
            let mut handles = Vec::new();
            for _ in 0..CONCURRENCY {
                let cache = Arc::clone(&cache);
                let key = key.clone();
                let barrier = Arc::clone(&barrier);
                handles.push(thread::spawn(move || {
                    barrier.wait();
                    cache.classify_concurrent(key, || {
                        std::thread::sleep(std::time::Duration::from_millis(250));
                        Ok(result("sensitivity"))
                    })
                }));
            }

            let results: Vec<(Result<ClassificationResult, ClassifyError>, CachePath)> =
                handles.into_iter().map(|h| h.join().unwrap()).collect();

            assert_eq!(
                cache.forward_count(),
                1,
                "{policy:?} identical concurrent misses must coalesce into one forward"
            );
            assert!(
                results.iter().all(|r| matches!(
                    &r.0,
                    Ok(c) if c.ranked.first().map(|s| s.id.as_str()) == Some("sensitivity")
                )),
                "every concurrent caller must receive the same classification result"
            );
            let misses = results.iter().filter(|r| r.1 == CachePath::Miss).count();
            let coalesced = results
                .iter()
                .filter(|r| r.1 == CachePath::Coalesced)
                .count();
            let hits = results.iter().filter(|r| r.1 == CachePath::Hit).count();
            assert_eq!(misses, 1, "exactly one designated forwarder");
            assert_eq!(coalesced, CONCURRENCY - 1);
            assert_eq!(hits, 0, "no true cache hits on a cold cache");
        }
    }
}

#[cfg(test)]
mod semantic_tests {
    use super::*;
    use crate::classify::{ClassifyStatus, RankedSignal};

    fn result(id: &str) -> ClassificationResult {
        ClassificationResult {
            classifier_id: "c".into(),
            model_revision: "m".into(),
            tokenizer_revision: "t".into(),
            taxonomy_revision: "x".into(),
            status: ClassifyStatus::Ok,
            ranked: vec![RankedSignal {
                id: id.into(),
                score: 1.0,
            }],
        }
    }

    #[test]
    fn noop_semantic_cache_never_hits() {
        let cache = NoopSemanticCache;
        let e = Embedding::new(vec![1.0, 0.0]);
        cache.insert(&e, &result("simple"), "c|m|t|x");
        assert!(
            cache.lookup(&e, "c|m|t|x").is_none(),
            "noop must always miss"
        );
    }

    #[test]
    fn identity_tag_is_stable_and_field_separated() {
        assert_eq!(identity_tag(("c", "m", "t", "x", None)), "c|m|t|x");
        assert_ne!(
            identity_tag(("a", "bc", "d", "e", None)),
            identity_tag(("ab", "c", "d", "e", None))
        );
    }

    /// The L2 tag must isolate on the loaded artifact digest exactly as the L1
    /// blake3 key does. If it did not, a rebuilt artifact under an UNCHANGED
    /// revision would miss in L1 and then HIT in L2, serving a semantic label
    /// computed by the previous model -- the aliasing the digest exists to
    /// prevent, reintroduced one tier down.
    #[test]
    fn identity_tag_isolates_on_artifact_digest() {
        let a = identity_tag(("c", "m", "t", "x", Some("sha256:aaa")));
        let b = identity_tag(("c", "m", "t", "x", Some("sha256:bbb")));
        assert_ne!(
            a, b,
            "same revision, different artifact must not share an L2 tag"
        );

        // None keeps the historical four-field tag, so identities without a
        // recorded digest are not orphaned from their existing L2 entries.
        assert_eq!(identity_tag(("c", "m", "t", "x", None)), "c|m|t|x");
        assert_ne!(identity_tag(("c", "m", "t", "x", Some("d"))), "c|m|t|x");

        // NOT asserted here: that a pipe INSIDE a field cannot alias. It can,
        // and it could before the digest was added --
        //     ("a","b","c|d","e") and ("a","b","c","d|e") both render "a|b|c|d|e".
        // L1 does not have this problem because update_field length-prefixes
        // every field; the L2 tag only separates them. Fixing it changes the
        // tag format and orphans existing L2 entries, so it is left as a
        // separate change rather than folded into this one. Left undisturbed,
        // not endorsed.
    }
}
