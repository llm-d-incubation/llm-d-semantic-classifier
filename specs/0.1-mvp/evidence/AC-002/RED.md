# AC-002 RED evidence

## Acceptance criterion
AC-002 not-ready before model load/warmup.

## Proving test(s)
- U-020 readiness false before successful warmup (selected this iteration)
- U-022 warmup failure keeps not-ready (future)
- I-010/I-011, S-006 (later integration/system phases)

## Command
```
cargo test --locked u020
```

## Worktree state
- Branch: agent/0.1-mvp
- HEAD: `da15ab9b68af324e63ea58be404625f83265adca`
- `git status`:
  ```
   M src/lib.rs
  ?? src/runtime.rs
  ```
- Working tree has uncommitted test-only changes: `src/lib.rs` registers a
  new `runtime` module; `src/runtime.rs` holds the U-020 proving test.

## Failure excerpt
```
$ cargo test --locked u020
   Compiling llm-d-sc v0.1.0 (/Users/cnuland/llm-d-sc-genesis)
error[E0432]: unresolved import `super::Runtime`
 --> src/runtime.rs:8:9
  |
8 |     use super::Runtime;
  |         ^^^^^^^^^^^^^^ no `Runtime` in `runtime`

For more information about this error, try `rustc --explain E0432`.
error: could not compile `llm-d-sc` (lib test) due to 1 previous error
--- exit 101 ---
```

## Why this is the expected failure
AC-002 demands that the service report NOT ready until the resident
model/tokenizer is loaded and warmed. The proving test U-020 exercises a
readiness abstraction (`Runtime::readiness()` returns not-ready before
`Runtime::warmup()` succeeds and ready afterward). No such readiness/lifecycle
abstraction exists yet: the crate only ships `src/config.rs` (configuration
parsing) from AC-001. Because `Runtime` is undefined, the U-020 test cannot be
selected or compiled — `cargo test --locked u020` fails at exit 101 with
"no `Runtime` in `runtime`". This is precisely the expected RED: the feature
(not-ready-before-warmup readiness gating) does not exist, so the proving test
cannot run, let alone pass.

Note: `./hack/verify` is NOT run this iteration — per AGENTS.md steps 1-4 only
the RED proof and evidence are required before implementation. The
GREEN/implementation step will add the minimal `Runtime`/`Readiness`
abstraction so U-020 compiles and passes, then U-022 and the remaining AC-002
tests are exercised.
