//! Integration tests for [`llm_d_sc::cache::redis::RedisSemanticCache`].
//!
//! The fail-open test runs WITHOUT any Redis server (points at a dead
//! address) and must always pass under plain `cargo test`. The two
//! live-Redis tests are `#[ignore]`d (repo convention for external
//! resources) and require a running Redis Stack at `REDIS_URL`
//! (see hack/redis-stack.sh).

use llm_d_sc::cache::redis::RedisSemanticCache;
use llm_d_sc::cache::SemanticCache;
use llm_d_sc::classify::{ClassificationResult, ClassifyStatus, Embedding, RankedSignal};
use llm_d_sc::config::CacheConfig;
use llm_d_sc::metrics::Metrics;

fn cfg(url: &str) -> CacheConfig {
    CacheConfig {
        strategy: "redis-semantic".into(),
        redis_url: Some(url.into()),
        threshold: 0.90,
        ttl_secs: 3600,
        timeout_ms: 50,
    }
}

fn result(id: &str) -> ClassificationResult {
    ClassificationResult {
        classifier_id: "complexity".into(),
        model_revision: "m".into(),
        tokenizer_revision: "t".into(),
        taxonomy_revision: "x".into(),
        status: ClassifyStatus::Ok,
        ranked: vec![RankedSignal {
            id: id.into(),
            score: 0.9,
        }],
    }
}

#[test]
fn lookup_is_fail_open_when_redis_is_unreachable() {
    // Port 1 is never a Redis; connect may succeed lazily, but lookup must
    // never panic or error — it returns None (fail-open to compute).
    let cache = RedisSemanticCache::connect(&cfg("redis://127.0.0.1:1"), Metrics::new());
    if let Ok(cache) = cache {
        let e = Embedding::new(vec![0.1, 0.2, 0.3]);
        assert!(cache.lookup(&e, "complexity|m|t|x").is_none());
        // insert must not panic either.
        cache.insert(&e, &result("SIMPLE"), "complexity|m|t|x");
    }
}

#[test]
fn breaker_open_short_circuits_without_calling_redis() {
    // Every lookup that reaches (and fails against) Redis records a degraded
    // outcome, and — this is the fix under test — so does every lookup that
    // is short-circuited by an open breaker: an open breaker is a degraded
    // L2 op, not a free miss. Deterministic (metric-based, not
    // timing-based): the degraded counter must advance by exactly one per
    // call in both phases.
    let metrics = Metrics::new();
    let cache = RedisSemanticCache::connect(&cfg("redis://127.0.0.1:1"), metrics.clone())
        .expect("connect (lazy) must succeed even though the address is dead");
    let e = Embedding::new(vec![0.1, 0.2, 0.3]);

    // Phase 1: drive the breaker open (failure_threshold is 5, see
    // RedisSemanticCache::connect). Each of these calls actually reaches
    // (and fails against) the dead address, recording one degraded outcome
    // apiece.
    for _ in 0..5 {
        assert!(cache.lookup(&e, "complexity|m|t|x").is_none());
    }
    let degraded_at_open = metrics.snapshot().l2_degraded;
    assert_eq!(
        degraded_at_open, 5,
        "each of the 5 failed Redis attempts before the breaker opens must record a degraded outcome"
    );

    // Phase 2: once open, further lookups short-circuit BEFORE touching
    // Redis, but must still count as degraded — exactly one per call.
    for i in 1..=5u64 {
        assert!(cache.lookup(&e, "complexity|m|t|x").is_none());
        assert_eq!(
            metrics.snapshot().l2_degraded,
            degraded_at_open + i,
            "an open breaker must record exactly one degraded outcome per short-circuited lookup"
        );
    }

    // Phase 3: `insert` must apply the same breaker-open accounting.
    let before_insert = metrics.snapshot().l2_degraded;
    cache.insert(&e, &result("SIMPLE"), "complexity|m|t|x");
    assert_eq!(
        metrics.snapshot().l2_degraded,
        before_insert + 1,
        "insert must also record a degraded outcome when short-circuited by an open breaker"
    );
}

#[test]
#[ignore] // requires a running Redis Stack at REDIS_URL (see hack/redis-stack.sh)
fn paraphrase_hit_after_insert() {
    let url = std::env::var("REDIS_URL").expect("REDIS_URL for the live test");
    let cache = RedisSemanticCache::connect(&cfg(&url), Metrics::new()).expect("connect");
    let a = Embedding::new(vec![1.0, 0.0, 0.0]);
    let close = Embedding::new(vec![0.98, 0.02, 0.0]); // near-identical direction
    cache.insert(&a, &result("SIMPLE"), "complexity|m|t|x");
    std::thread::sleep(std::time::Duration::from_millis(100)); // async write-back settles
    let hit = cache.lookup(&close, "complexity|m|t|x");
    assert_eq!(hit.map(|r| r.ranked[0].id.clone()), Some("SIMPLE".into()));
}

#[test]
#[ignore] // requires a running Redis Stack at REDIS_URL
fn identity_isolates_across_revisions() {
    let url = std::env::var("REDIS_URL").expect("REDIS_URL for the live test");
    let cache = RedisSemanticCache::connect(&cfg(&url), Metrics::new()).expect("connect");
    let e = Embedding::new(vec![0.0, 1.0, 0.0]);
    cache.insert(&e, &result("SIMPLE"), "complexity|m1|t|x");
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(
        cache.lookup(&e, "complexity|m2|t|x").is_none(),
        "different revision must not hit"
    );
}
