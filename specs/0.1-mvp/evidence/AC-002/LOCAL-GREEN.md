# AC-002 GREEN evidence

## Acceptance criterion
AC-002 not-ready before model load/warmup.

## Proving test(s)
- U-020 readiness false before successful warmup
- U-022 warmup failure keeps not-ready
- I-010/I-011, S-006 (later integration/system phases)

## Command
Focused tests:
```
cargo test --locked u020
cargo test --locked u022
```
plus the full unit suite and clean build:
```
cargo test --locked
cargo build --locked
```

## Worktree state
- Branch: agent/0.1-mvp
- HEAD: `da15ab9b68af324e63ea58be404625f83265adca` (unchanged — implementation is uncommitted)
- `git status` (short):
  - ` M .agent/state/current.md`
  - ` M src/lib.rs`
  - `?? src/runtime.rs`
  - `?? specs/0.1-mvp/evidence/AC-002/`
- Implementation changes: `src/lib.rs` registers `pub mod runtime;`;
  `src/runtime.rs` adds the minimal `Runtime`/`Readiness` abstraction, a
  `warmup` that validates the model path, and the U-020 + U-022 proving tests.

## Green result
```
$ cargo test --locked u020
test runtime::tests::u020_readiness_false_before_successful_warmup ... ok
test result: ok. 1 passed; 0 failed; ...

$ cargo test --locked u022
test runtime::tests::u022_warmup_failure_keeps_not_ready ... ok
test result: ok. 1 passed; 0 failed; ...
```

```
$ cargo test --locked
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

```
$ cargo build --locked
Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.01s
```

## Why this is GREEN
The smallest change adds a readiness/lifecycle abstraction in `src/runtime.rs`:
- `Readiness` is an enum that starts `NotReady` and is `Ready` only after a
  successful warmup; `Readiness::ready()` is true only in the ready state.
- `Runtime::new()` constructs a runtime that has loaded/warmed no model, so its
  readiness is NOT ready.
- `Runtime::warmup(path)` validates that the model `path` exists and is readable
  (`exists()` then `std::fs::metadata`), returning `Err` and leaving readiness
  NOT ready if the path is missing or unreadable; only on a valid path does it
  flip to ready and return `Ok`.

U-020 drives the success path: before any warmup `readiness().ready()` is false;
after `warmup` on a real readable directory it is true. U-022 drives the
failure contract: `warmup("/nonexistent/model/path")` returns `Err` and
readiness stays not-ready.

Both tests were RED for the right reason: U-020 at exit 101 (`error[E0432]:
unresolved import super::Runtime` — no readiness abstraction existed) and U-022
at a runtime panic ("warmup must reject a missing model path: ()") because
`warmup` ignored its path and returned `Ok`. Both are now GREEN. The full unit
suite (5 AC-001 config tests + U-020 + U-022) passes and the crate builds
cleanly, so AC-001's build is not regressed.
