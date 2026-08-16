# AC-007 RED evidence — U-041 (slice)

## Proving test
- U-041 `u041_identical_concurrent_misses_coalesce` (`src/cache.rs`, plain
  `#[test]`, offline — no model fetch required).
- Creates a `SharedCache` and fires `CONCURRENCY = 8` simultaneous identical
  misses on an empty cache (same key, all cache cold). A `Barrier` makes every
  thread arrive at the forward stage at the same time, so they genuinely overlap
  as concurrent misses. Each caller asserts the tokenizer/model forward closure
  is run exactly ONCE total (`forward_count() == 1`), i.e. the 8 identical
  concurrent misses are coalesced into a SINGLE forward (bounded), and that every
  caller receives the same classification result.

## Why this is the proving test for AC-007
`specs/0.1-mvp/test-plan.md` maps AC-007 to U-041 ("identical concurrent misses
coalesce/bound duplicate forwards"). AC-007 requires that identical concurrent
misses do not create unbounded forwards. The forward closure stands in for the
tokenize + model-forward stage. A correct cache must run that forward once per
distinct key even under concurrency; without coalescing, N simultaneous identical
misses produce N redundant forwards.

## RED state (no coalescing)
For the RED slice I added a `SharedCache` whose `classify_concurrent` serves an
already-cached result on the fast path but, on a miss, runs the forward closure
for EVERY caller and stores each result (no single-flight / in-flight
deduplication). This is the exact AC-007 violation: N simultaneous misses yield N
forwards. The coalescing fix (deduplicate in-flight identical misses so only one
runs the forward; the rest wait for its result) is NOT yet implemented this turn.

## Command
```
cargo test --locked u041
```

## Result
FAILED. Expected RED reason: without coalescing, all 8 concurrent identical
misses each run their own forward, so `forward_count() == 8`, not `1` — identical
concurrent misses created 8 (unbounded) forwards.

Failure excerpt:
```
running 1 test
test cache::tests::u041_identical_concurrent_misses_coalesce ... FAILED

---- cache::tests::u041_identical_concurrent_misses_coalesce stdout ----
thread 'cache::tests::u041_identical_concurrent_misses_coalesce' (18900299) panicked at src/cache.rs:361:9:
assertion `left == right` failed: identical concurrent misses must coalesce into ONE forward, not 8
  left: 8
 right: 1

failures:
    cache::tests::u041_identical_concurrent_misses_coalesce

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 21 filtered out; finished in 0.00s
```

## Why this is the expected failure
The proving test asserts the AC-007 bounded-forward contract: N identical
concurrent misses must produce exactly ONE forward. The RED `SharedCache` runs
the forward per caller (no coalescing), so the `Barrier` guarantees all 8 threads
are simultaneously in the forward stage and each records its own forward, giving
`forward_count() == 8` instead of `1`. The failure is deterministic (the Barrier
forces overlap), confirming U-041 is non-vacuous: a cache that forwards per
concurrent miss fails, while a coalescing cache that runs the forward once per
distinct key and lets the other waiters read the shared result would keep the
count at 1.

## Worktree / SHA
- HEAD SHA: `c5ccc0e4ae16fc0a0f86bd8ff23db5b9f9310259` (uncommitted changes).
- `git status`: `M src/cache.rs` (adds RED `SharedCache` + U-041 test). No
  commits/pushes.
