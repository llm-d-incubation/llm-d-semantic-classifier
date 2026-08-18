# AC-009 GREEN evidence — I-001 (real tonic round trip)

## Criterion
AC-009 dummy gateway consumes response over persistent gRPC.

## Proving test (this slice)
- I-001 `i001_real_tonic_round_trip` (`tests/grpc.rs`, integration, async `#[tokio::test]`).

## Command
```
cargo test --locked --test grpc
```

## Result
PASSED. Both proving tests in `tests/grpc.rs` are green:
```
running 2 tests
test i001_real_tonic_round_trip ... ok
test i002_persistent_http2_channel_reused ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```
Exit code: 0.

## What I-001 asserts
`i001_real_tonic_round_trip` starts a REAL tonic classify server on an ephemeral
localhost port (`127.0.0.1:0`), connects a REAL tonic client channel over the
wire, sends one `Classify` request for a fixture input, and asserts a
ranked-signals response arrives over the wire. It further asserts
`final_route.is_none()` (AC-010: llm-d-sc must never dictate a final route).

## Implementation (smallest change)
- `src/classify.rs` (new): deterministic classification pipeline wiring
  tokenizer -> versioned cache -> single-flight -> ranker over the committed
  synthetic prototypes. `ClassifyService::classify` returns ranked signal names,
  never a final route. No Candle model forward (hard rule: no unrestricted model
  forward from Tokio request workers; I-001 pins the RPC contract, not the model).
- `src/grpc/classify.rs`: the tonic `ClassifyServiceImpl` now runs the pipeline
  and returns its ranked signals instead of a hardcoded signal. `ClassifyServer`
  builds the pipeline-backed service. Fixed the tokio 1.x blocking-socket issue
  (registering a blocking std listener with tokio is unsupported — issue 7172) by
  binding the tokio listener inside the runtime.
- `src/cache.rs`: derived `Clone` for `SharedCache` (both fields are `Arc`s) so
  the pipeline service can back a tonic server (`Clone + Send + Sync + 'static`).
- `src/lib.rs`: `pub mod classify;`.
- `Cargo.toml`: added `macros` to the tokio features for `#[tokio::test]`.
- Clippy: fixed the dead `runtime` field (renamed `_runtime`) and replaced three
  `io::Error::new(io::ErrorKind::Other, e)` calls with `io::Error::other(..)`.

## Local gate (./hack/verify)
Fully green (exit 0): `cargo fmt --check`, `cargo clippy --all-targets
--all-features -- -D warnings`, `cargo build --locked`,
`cargo test --workspace --all-features --locked`.

## Worktree / SHA
- HEAD SHA: `a49dc474725486c24dc6764ec1d75f76e689e1e7` (uncommitted changes).
- `git status` (relevant): `?? tests/grpc.rs`, `?? src/grpc/`, `?? src/classify.rs`,
  `M src/cache.rs`, `M src/lib.rs`, `M Cargo.toml`. No commits/pushes.
