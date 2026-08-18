# AC-009 GREEN evidence — U-011 unknown signal explicit error

## Test ID
U-011 (unknown signal explicit error).

## Test file
`tests/grpc.rs` — `u011_unknown_signal_explicit_error` (integration, real tonic
classify server/client over the persistent channel).

## Command
```
cargo test --locked --test grpc u011_unknown_signal_explicit_error
```

## Worktree / SHA
- HEAD SHA: `5d67e52d4f610e87e6e360d74b474d3fe687752f` (no commits; working-tree
  changes only).

## Result: GREEN
```
test u011_unknown_signal_explicit_error ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out
```

## Implementation (smallest change)
In `src/grpc/classify.rs` `ClassifyServiceImpl::classify`, after recording request
telemetry and before building the pipeline input, validate `req.signals`:
- the supported signal `sensitivity` is accepted;
- any other requested signal returns `tonic::Status::invalid_argument` with a
  message naming the unsupported signal — never silently ignored.

The test asserts both sides: a supported `sensitivity` request returns status
`OK`, and an unknown signal (`pii`) is rejected with `tonic::Code::InvalidArgument`.

## References
- `tests/TEST_MATRIX.md` U-011
- `specs/0.1-mvp/test-plan.md` AC-009 -> U-011
- `specs/0.1-mvp/evidence/AC-009/RED-U011.md` (RED-first evidence)
