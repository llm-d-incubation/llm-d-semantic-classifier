# AC-005 RED evidence — U-021

## Proving test
- U-021 `u021_model_tokenizer_load_once_per_active_revision` (`src/runtime.rs`,
  plain `#[test]`, offline — the tokenizer is the committed ModelCar fixture
  `tests/fixtures/modelcar/tokenizer.json`, so no `./hack/fetch-model` is needed).
- Loads a resident tokenizer for the active revision
  `43f21d21ac48134464f8510a9ac9c95bdac7ba86` (from the classifier manifest) ten
  times in a row (simulating ten classification calls for the SAME active
  revision) and asserts the tokenizer was loaded exactly ONCE
  (`tokenizer_load_count() == 1`), not once per call.

## Why this is the proving test for AC-005
`specs/0.1-mvp/test-plan.md` maps AC-005 to U-021 ("model/tokenizer load once
per active revision"). The `Runtime` in `src/runtime.rs` is the resident
model/tokenizer holder. AC-005 requires the resident model/tokenizer to be
loaded at most once per active revision and reused across calls. To run the
test I added a minimal resident-tokenizer stub on `Runtime`
(`load_tokenizer_once`) whose behavior is the pre-fix wrong behavior: it
RELOADS the tokenizer on every call regardless of the active revision (the
load counter increments per call). The fix (cache by active revision and reuse
the resident instance) is NOT yet implemented this turn.

## Command
```
cargo test --locked u021
```

## Result
FAILED. Expected RED reason: the current runtime reloads the tokenizer on every
call (10 loads for 10 calls) instead of loading once per active revision.

Failure excerpt:
```
running 1 test
test runtime::tests::u021_model_tokenizer_load_once_per_active_revision ... FAILED

---- runtime::tests::u021_model_tokenizer_load_once_per_active_revision stdout ----
thread 'runtime::tests::u021_model_tokenizer_load_once_per_active_revision' (18729750) panicked at src/runtime.rs:204:9:
assertion `left == right` failed: model/tokenizer must be loaded once per active revision, not on every call
  left: 10
 right: 1

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 16 filtered out
```

## Why this is the expected failure
The proving test asserts the AC-005 load-once-per-active-revision contract. The
stub's pre-fix behavior reloads the tokenizer on every call, so the load count
is 10 (one per call) rather than 1 (one per revision). The failure is at the
primary load-count assertion (`left: 10, right: 1`), confirming the test
guards the residency contract and is non-vacuous: a runtime that reloads per
call fails, while a correct resident holder would keep the count at 1.

## Worktree / SHA
- SHA: `13cac01` (uncommitted)
- `git status`: `M src/runtime.rs` (resident-tokenizer stub + U-021 test). No
  commits/pushes.
