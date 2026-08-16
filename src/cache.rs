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

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Condvar, Mutex};

use crate::classify::{ClassificationResult, ClassifyError};

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
        let mut hasher = blake3::Hasher::new();
        update_field(&mut hasher, &classifier_id.into());
        update_field(&mut hasher, &model_revision.into());
        update_field(&mut hasher, &tokenizer_revision.into());
        update_field(&mut hasher, &taxonomy_revision.into());
        update_field(&mut hasher, normalized_text);
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
pub struct ExactCache {
    entries: HashMap<CacheKey, ClassificationResult>,
    forward_count: u64,
    hit_count: u64,
}

impl ExactCache {
    /// An empty cache with no entries.
    pub fn new() -> Self {
        ExactCache {
            entries: HashMap::new(),
            forward_count: 0,
            hit_count: 0,
        }
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
        if let Some(cached) = self.entries.get(&key) {
            self.hit_count += 1;
            return Ok(cached.clone());
        }
        // Miss: run the forward exactly once, store, and return.
        let result = forward();
        match &result {
            Ok(result) => {
                self.forward_count += 1;
                self.entries.insert(key, result.clone());
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
    pub(crate) fn cached_value(&self, key: &CacheKey) -> Option<ClassificationResult> {
        self.entries.get(key).cloned()
    }

    /// Record a forward and store its freshly-computed successful result.
    /// Used by the concurrent `SharedCache` after it runs the forward (AC-007).
    pub(crate) fn store_after_forward(
        &mut self,
        key: CacheKey,
        result: ClassificationResult,
    ) -> ClassificationResult {
        self.forward_count += 1;
        self.entries.insert(key, result.clone());
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
    /// An empty shared cache.
    pub fn new() -> Self {
        SharedCache {
            inner: Arc::new(Mutex::new(ExactCache::new())),
            in_flight: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Classify `key` concurrently, returning the classification result.
    ///
    /// Serves an already-cached result on the fast path. On a miss, the FIRST
    /// caller for `key` runs the forward closure once (single-flight); every
    /// other identical concurrent miss waits for and reads that shared result
    /// instead of running its own forward. This bounds identical concurrent
    /// misses to ONE forward per distinct key (AC-007). A failed forward is
    /// propagated to every caller and is NOT cached.
    pub fn classify_concurrent(
        &self,
        key: CacheKey,
        forward: impl FnOnce() -> Result<ClassificationResult, ClassifyError>,
    ) -> Result<ClassificationResult, ClassifyError> {
        // Fast path: serve an already-cached result.
        {
            let inner = self.inner.lock().unwrap();
            if let Some(cached) = inner.cached_value(&key) {
                return Ok(cached);
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
            return result.clone().unwrap();
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
        result
    }

    /// Number of times the tokenizer/model forward was invoked (all threads).
    pub fn forward_count(&self) -> u64 {
        self.inner.lock().unwrap().forward_count()
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

#[cfg(test)]
mod tests {
    use super::{CacheKey, ExactCache, SharedCache};
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
        let cache = Arc::new(SharedCache::new());
        let key = CacheKey::new(
            "clf",
            "model-rev",
            "tok-rev",
            "tax-rev",
            "same sensitivity input",
        );

        // A barrier synchronizes all N threads so they reach `classify_concurrent`
        // together as concurrent misses. The barrier is OUTSIDE the forward
        // closure: a correct single-flight cache runs the forward on exactly one
        // thread, so an N-way barrier INSIDE the forward would deadlock (only one
        // thread ever arrives). To still guarantee the misses genuinely overlap, the
        // forward closure holds the forward stage open for a generous duration, so
        // every concurrent miss enters the forward stage before the first one
        // completes and stores (forcing N redundant forwards on the buggy cache).
        let barrier = Arc::new(Barrier::new(CONCURRENCY));
        let mut handles = Vec::new();
        for _ in 0..CONCURRENCY {
            let cache = Arc::clone(&cache);
            let key = key.clone();
            let barrier = Arc::clone(&barrier);
            handles.push(thread::spawn(move || {
                barrier.wait();
                cache.classify_concurrent(key, || {
                    // Hold the forward stage open so concurrent misses overlap.
                    std::thread::sleep(std::time::Duration::from_millis(250));
                    Ok(result("sensitivity"))
                })
            }));
        }

        let results: Vec<Result<ClassificationResult, ClassifyError>> =
            handles.into_iter().map(|h| h.join().unwrap()).collect();

        assert_eq!(
            cache.forward_count(),
            1,
            "identical concurrent misses must coalesce into ONE forward, not {}",
            CONCURRENCY
        );
        assert!(
            results
                .iter()
                .all(|r| matches!(r, Ok(c) if c.ranked.first().map(|s| s.id.as_str()) == Some("sensitivity"))),
            "every concurrent caller must receive the same classification result"
        );
    }
}
