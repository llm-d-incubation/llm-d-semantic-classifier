//! Exact-result cache for classification.
//!
//! AC-006 requires a cache hit to bypass the tokenizer and the model forward
//! entirely: an identical previously-classified input must be served from the
//! cache without re-tokenizing or running the Candle model forward.
//!
//! Per `specs/0.1-mvp/design.md`, the cache key is a versioned fingerprint of
//! every semantic input to the classification result: classifier/model/
//! tokenizer/taxonomy revision plus a stable hash of the normalized supplied
//! context. A raw prompt string must never be the sole cache identity.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

/// A versioned fingerprint cache key (design.md).
///
/// The key identifies the classification result by the classifier/model/
/// tokenizer/taxonomy revisions plus a stable hash of the normalized input
/// text. Two keys are equal only if every revision and the normalized-text hash
/// match; a revision change with identical text must yield a different key, so a
/// stale cached classification is never served under a new revision.
#[derive(Debug, Clone)]
pub struct CacheKey {
    classifier_id: String,
    model_revision: String,
    tokenizer_revision: String,
    taxonomy_revision: String,
    normalized_text_hash: u64,
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
        let mut hasher = DefaultHasher::new();
        normalized_text.hash(&mut hasher);
        CacheKey {
            classifier_id: classifier_id.into(),
            model_revision: model_revision.into(),
            tokenizer_revision: tokenizer_revision.into(),
            taxonomy_revision: taxonomy_revision.into(),
            normalized_text_hash: hasher.finish(),
        }
    }
}

impl PartialEq for CacheKey {
    fn eq(&self, other: &Self) -> bool {
        self.classifier_id == other.classifier_id
            && self.model_revision == other.model_revision
            && self.tokenizer_revision == other.tokenizer_revision
            && self.taxonomy_revision == other.taxonomy_revision
            && self.normalized_text_hash == other.normalized_text_hash
    }
}
impl Eq for CacheKey {}

impl Hash for CacheKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.classifier_id.hash(state);
        self.model_revision.hash(state);
        self.tokenizer_revision.hash(state);
        self.taxonomy_revision.hash(state);
        self.normalized_text_hash.hash(state);
    }
}

/// An exact-result cache mapping an input key to its classification output.
///
/// AC-006 contract (U-040): a cache HIT must bypass the tokenizer and model
/// forward. The forward closure is the tokenize + model-forward stage; on a hit
/// it must not be invoked at all.
pub struct ExactCache {
    entries: HashMap<CacheKey, String>,
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
    /// forward closure is invoked exactly once, its result is stored, and
    /// returned.
    pub fn classify(&mut self, key: CacheKey, forward: impl FnOnce() -> String) -> String {
        // AC-006: a cache HIT must bypass the tokenizer and model forward.
        if let Some(cached) = self.entries.get(&key) {
            self.hit_count += 1;
            return cached.clone();
        }
        // Miss: run the forward exactly once, store, and return.
        let result = forward();
        self.forward_count += 1;
        self.entries.insert(key, result.clone());
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
}

impl Default for ExactCache {
    fn default() -> Self {
        ExactCache::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{CacheKey, ExactCache};

    #[test]
    fn u040_exact_cache_hit_bypasses_tokenizer_and_runtime() {
        // U-040 (AC-006): an exact cache hit must bypass the tokenizer and the
        // model forward. The forward closure stands in for the tokenize +
        // model-forward stage; it must run exactly ONCE (on the miss) and must
        // NOT run again when the identical input is served from the cache.
        let mut cache = ExactCache::new();
        let key = CacheKey::new(
            "clf",
            "model-rev",
            "tok-rev",
            "tax-rev",
            "golden sensitivity input",
        );

        // First call: a miss, so tokenizer + model forward must run once.
        let first = cache.classify(key.clone(), || "result".to_string());
        assert_eq!(
            cache.forward_count(),
            1,
            "a cache miss must run the tokenizer/model forward once"
        );

        // Second identical call: a HIT. The tokenizer/model forward must be
        // bypassed entirely and the cached result returned unchanged.
        let second = cache.classify(key.clone(), || "result".to_string());
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

        cache.classify(key_a.clone(), || "result".to_string());
        cache.classify(key_b.clone(), || "result".to_string());
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

        cache.classify(key_a.clone(), || "result".to_string());
        cache.classify(key_b.clone(), || "result".to_string());
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

        cache.classify(key_a.clone(), || "result".to_string());
        cache.classify(key_b.clone(), || "result".to_string());
        assert_eq!(
            cache.forward_count(),
            2,
            "a taxonomy/prototype revision change must not serve the stale cached result (miss)"
        );
    }
}
