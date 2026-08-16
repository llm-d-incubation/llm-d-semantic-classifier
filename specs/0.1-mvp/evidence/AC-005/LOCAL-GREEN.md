# AC-005 GREEN evidence — model/tokenizer load once per active revision

## Criterion
AC-005: model/tokenizer load once per active revision.

`specs/0.1-mvp/test-plan.md` maps AC-005 to U-021 (unit) and I-012
(integration).

## Unit-level proof (U-021)
U-021 `u021_model_tokenizer_load_once_per_active_revision` (`src/runtime.rs`)
is the sole unit-level test for AC-005. It loads the resident tokenizer for the
active revision `43f21d21ac48134464f8510a9ac9c95bdac7ba86` ten times and asserts
`tokenizer_load_count() == 1` (exactly one load per active revision, not one per
call). It is offline (committed ModelCar fixture `tests/fixtures/modelcar/
tokenizer.json`, no `./hack/fetch-model`).

### Command
```
cargo test --locked u021
```

### Result
GREEN: `1 passed; 0 failed`.

```
running 1 test
test runtime::tests::u021_model_tokenizer_load_once_per_active_revision ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 16 filtered out; finished in 0.08s
```

## Implementation
`src/runtime.rs` now caches the resident tokenizer by active revision:
- `Runtime` holds `resident_tokenizer: Option<Tokenizer>` and
  `active_revision: Option<String>`.
- `load_tokenizer_once` reuses the resident tokenizer when the requested
  revision matches the active revision (no reload, no count increment); it
  reloads only when the active revision changes, incrementing the load count by
  one for the new revision.

## Suites / worktree
- `./hack/test-impact src/runtime.rs` -> unknown surface -> FULL SUITE required.
- `./hack/spec-check 0.1-mvp` -> OK.
- `./hack/verify` -> GREEN (fmt, clippy `-D warnings`, build, full test: 13
  passed, 4 ignored — the ignored tests require the fetch-model runtime and are
  unrelated to this change).
- Worktree: SHA `13cac01` (uncommitted). No commits/pushes.

## Note on I-012
I-012 (repeated calls do not reload model/tokenizer in the integration
environment) is an integration-environment test per the test plan and is out of
scope for this local unit turn. It remains open for the integration
environment.
