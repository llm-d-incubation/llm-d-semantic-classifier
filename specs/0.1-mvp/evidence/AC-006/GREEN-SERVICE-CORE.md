# GREEN — SERVICE-CORE (P0): production Candle cache hit performs ZERO tokenizer calls and ZERO model forwards

## Criterion
`specs/0.1-mvp/spec.md` AC-006 (exact-result cache + single-flight) applied to the
PRODUCTION Candle backend: a cache hit through the generic `ServiceCore` must NOT
reach the raw Candle runtime's tokenizer or model forward.

## Proving test (parity tier, `#[ignore]`)
`src/classify.rs` `classify::tests::service_core_production_candle_cache_hit_zero_tokenizer_zero_forward`

## Command
```
cargo test --locked --lib -- --ignored service_core_production_candle_cache_hit_zero_tokenizer_zero_forward
```

## Result
```
test classify::tests::service_core_production_candle_cache_hit_zero_tokenizer_zero_forward ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 39 filtered out; finished in 0.29s
```

## What the test proves
1. A `CandleClassifier` (real weights from `artifacts/models/sensitivity`) is moved
   into `ServiceCore::with_metrics`.
2. The runtime's tokenizer-call and forward-call counters are observed AFTER the
   move (production path).
3. Miss (distinct input): tokenizer calls +1, forwards +1.
4. Identical re-classification (cache hit): tokenizer and forward counters
   UNCHANGED (ZERO additional calls), result byte-identical.
5. Second distinct input: counters advance to 2/2, proving the counters were
   genuinely live (not stubbed).
6. `miss == hit` result equality asserted.

## Worktree state
- Branch: `agent/0.1-mvp`
- Commit: `1bd6596`
- Files changed this slice: `src/classify.rs`, `src/grpc/classify.rs`

## Existing AC-006/AC-007 tests still GREEN (routed through the core)
- `cargo test --locked --lib` -> 34 passed, 0 failed (includes u070, u071, cache
  u040..u044, metrics u080/u081).
- `cargo test --locked --test bench_rtt --test metrics --test grpc` -> 22 passed,
  0 failed (bench_rtt 13, grpc 6, metrics 3).
