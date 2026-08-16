# AC-001 GREEN evidence

## Acceptance criterion
AC-001 clean Rust build/server lifecycle.

## Proving test(s)
- U-001 minimal valid configuration parses
- U-002 missing classifier config rejected
- U-003 unknown runtime backend rejected
- U-004 duplicate classifier ID rejected
- U-005 invalid model path rejected

## Command
```
cargo test --locked config::
```
plus the clean build that now succeeds:
```
cargo build --locked
```

## Worktree state
- Branch: main
- HEAD: `6286ff70abecc707a9bdce23b8debe79c1afb20a` (unchanged — implementation is uncommitted)
- `git status` (short):
  - ` M .agent/state/current.md`
  - `?? Cargo.lock`
  - `?? Cargo.toml`
  - `?? src/` (src/lib.rs, src/config.rs)
  - `?? specs/0.1-mvp/evidence/AC-001/`
- Phase: first Rust crate landed (library crate `llm-d-sc` with config parser/validator).

## Green result
```
$ cargo test --locked config::
running 5 tests
test config::tests::u002_missing_classifier_config_rejected ... ok
test config::tests::u005_invalid_model_path_rejected ... ok
test config::tests::u003_unknown_runtime_backend_rejected ... ok
test config::tests::u001_minimal_valid_configuration_parses ... ok
test config::tests::u004_duplicate_classifier_id_rejected ... ok

test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

```
$ cargo build --locked
Finished `dev` profile [unoptimized + debuginfo] target(s) in 5.27s
```

## Why this is GREEN
The minimal implementation adds a Rust library crate with a TOML config parser
and validator (`src/config.rs`):
- U-001: a config with one classifier using a known backend and a non-empty
  model path parses; listen/server defaults applied.
- U-002: a config with zero classifiers is rejected with `MissingClassifiers`.
- U-003: an unknown runtime backend string (e.g. `vllm`) is rejected with
  `UnknownBackend`.
- U-004: two classifiers sharing an ID are rejected with `DuplicateClassifierId`.
- U-005: an empty model path is rejected with `InvalidModelPath`.

`cargo build --locked` succeeds against the newly generated `Cargo.lock`, so
AC-001's "clean Rust build" requirement is demonstrated, and U-001..U-005 are
green. This is the smallest change: no networking, gRPC, cache, or model code
yet — those belong to later acceptance criteria.
