# AC-005 GREEN evidence — U-021 (slice)

## Proving test
- U-021 `u021_model_tokenizer_load_once_per_active_revision` (`src/runtime.rs`).
- Loads the resident tokenizer for the active revision
  `43f21d21ac48134464f8510a9ac9c95bdac7ba86` ten times in a row (simulating ten
  classification calls for the SAME active revision) and asserts the tokenizer
  was loaded exactly ONCE (`tokenizer_load_count() == 1`), not once per call.

## Why this proves AC-005
`specs/0.1-mvp/test-plan.md` maps AC-005 to U-021 ("model/tokenizer load once
per active revision"). AC-005 requires the resident model/tokenizer to be loaded
at most once per active revision and reused across calls.

## Change
`src/runtime.rs`: replaced the RED stub (which reloaded the tokenizer on every
call) with load-once-per-active-revision caching:
- `Runtime` now holds a `resident_tokenizer: Option<Tokenizer>` and an
  `active_revision: Option<String>`.
- `load_tokenizer_once` reuses the resident tokenizer when `active_revision`
  matches the requested revision (no reload, no count increment); only when the
  active revision actually changes does it reload the tokenizer and increment
  `tokenizer_load_count` by one.

## Command
```
cargo test --locked u021
```

## Result
GREEN.

```
running 1 test
test runtime::tests::u021_model_tokenizer_load_once_per_active_revision ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 16 filtered out; finished in 0.08s
```

## Suites / worktree
- `./hack/test-impact src/runtime.rs` -> `src/runtime.rs` maps to `src/*` which
  is an unknown surface -> FULL SUITE required.
- `./hack/spec-check 0.1-mvp` -> OK (deterministic checks passed; reviewer judges
  scope).
- `./hack/verify` -> GREEN: `cargo fmt --check`, `cargo clippy --all-targets
  --all-features -- -D warnings`, `cargo build --workspace --locked`,
  `cargo test --workspace --all-features --locked` all pass (13 passed, 4
  ignored — the ignored tests require the fetch-model runtime and are unrelated
  to this change).
- Worktree: SHA `13cac01` (uncommitted). `git status`:
  `M src/runtime.rs`, `?? specs/0.1-mvp/evidence/AC-005/`. No commits/pushes.
