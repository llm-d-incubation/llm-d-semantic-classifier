# AC-006 hardening — blake3 cache-key fingerprint + typed cache value

## Slice scope (CORRECTIVE SLICE 2)
Typed core + hardened cache key for `src/classify.rs` and `src/cache.rs`:

1. Typed classification contract: `ClassificationInput`, `RankedSignal`,
   `ClassificationResult`, `ClassifyStatus` (Ok/Abstain/Error), `ClassifyError`,
   and the `ClassifierRuntime` trait. The Candle embedder+ranker path
   (`CandleClassifier`) implements the trait. No route/endpoint field anywhere in
   the response (AC-010).
2. Cache key hardening: replaced the `u64` `DefaultHasher` fingerprint with
   **blake3**. The key is a 32-byte fingerprint over classifier_id + all revision
   fields + normalized text (length-prefixed to disambiguate field boundaries).
3. The cache now stores the typed `ClassificationResult`, not a `String`.

## Why the DefaultHasher was replaced
`DefaultHasher` is **not guaranteed stable across Rust versions** — its algorithm
may change between toolchains, so a cache key computed under one Rust version may
differ under another for identical inputs. It also produces only a **64-bit**
fingerprint; a 64-bit collision could serve a wrong classification under a
revision change (AC-006 requires a stale cached classification never be served).
blake3 is a stable, collision-resistant 32-byte hash.

## Contract changes to tests (TDD — adjusted first, semantics preserved)
- `U-040` / `U-041`: forward closures and assertions now operate on the typed
  `Result<ClassificationResult, ClassifyError>` and stored `ClassificationResult`
  (the cache value type changed from `String`). Assertion semantics unchanged:
  a hit must bypass the forward (forward_count stays 1), concurrent misses must
  coalesce to ONE forward.
- `U-042` / `U-043` / `U-044`: only the forward-closure value type changed
  (`String` -> `ClassificationResult`); the key TYPE changed from a `u64` hash to
  a `[u8; 32]` fingerprint. The assertions (`assert_ne!` on revision change,
  `forward_count() == 2`) are **unchanged** — a revision change still yields a
  different key and a miss.
- New: `U-070` (typed result carries ranked signals + revision fields, no route),
  `U-071` (identical inputs -> identical results), `U-072` (ignored: the Candle
  embedder+ranker path implements `ClassifierRuntime`).

## Files changed
- `Cargo.toml` / `Cargo.lock`: added `blake3 v1.8.6`.
- `src/cache.rs`: `CacheKey` now holds a blake3 `[u8; 32]` fingerprint; cache
  stores `ClassificationResult`; `classify`/`classify_concurrent` return
  `Result<ClassificationResult, ClassifyError>`; failed forwards are propagated
  but never cached.
- `src/classify.rs`: typed core + `ClassifierRuntime` + `CandleClassifier` +
  updated `ClassifyService` pipeline.
- `src/grpc/classify.rs`: handler builds a `ClassificationInput` and maps typed
  errors to an explicit tonic `unavailable` status (never a fabricated label).
- `src/tokenizer.rs` / `src/embedding.rs`: no behavioral change (the Clone
  derives were reverted; `ClassifyError` carries message strings).

## Gates
- `./hack/verify` -> GREEN (fmt, clippy `-D warnings`, build, full test:
  22 unit + 2 grpc passed, 5 ignored).
- `./hack/test-impact <changed-files> --run` -> GREEN (full suite).
- `./hack/spec-check 0.1-mvp` -> OK (14 ACs mapped; 48 test IDs present).
- `./hack/test-parity` -> GREEN (model already present; 5 ignored model-dependent
  tests passed including the new `u072_candle_classifier_implements_classifier_runtime`).

## Worktree / SHA
- HEAD SHA `99e529f5261a94d845246f16edee808c5c07af35` (uncommitted).
- `git status`: `M Cargo.lock Cargo.toml src/cache.rs src/lib.rs`; `?? src/classify.rs
  src/grpc/ build.rs proto/ tests/grpc.rs specs/0.1-mvp/evidence/AC-009/`. No commits/pushes.
