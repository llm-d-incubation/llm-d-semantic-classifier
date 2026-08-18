# AC-009 RED evidence — U-011 unknown signal explicit error

## Test ID
U-011 (unknown signal explicit error).

## Why it lives under AC-009 (decision recorded)
AC-009 is "dummy Praxis consumes response over persistent gRPC" — its proving
domain is the request/response contract carried over the persistent channel,
including the `requested_signals` the dummy Praxis sends in the request. U-011
validates those requested signals (accept `sensitivity`, reject anything else),
which is a request-side contract concern. AC-010 is strictly about the response
schema not carrying a route; U-011 is request validation, not response schema.
The convergence slice is locking the wire contract, and the request-side
validation belongs to AC-009. The test-plan maps U-011 to AC-009.

## Test file
`tests/grpc.rs` — `u011_unknown_signal_explicit_error` (integration, drives the
real tonic classify server/client over the persistent channel).

## Command
```
cargo test --locked --test grpc u011_unknown_signal_explicit_error
```

## Worktree / SHA
- HEAD SHA: `5d67e52d4f610e87e6e360d74b474d3fe687752f` (no commits; working-tree
  changes only).
- Working tree: the wire contract slice (richer `ClassifyResponse` with
  classifier_id/revisions/status/ranked) is applied, but the handler does NOT yet
  validate requested signals.

## Result: RED (expected)
```
test u011_unknown_signal_explicit_error ... FAILED

---- u011_unknown_signal_explicit_error stdout ----
thread 'u011_unknown_signal_explicit_error' panicked at tests/grpc.rs:197:10:
unknown signal must be rejected explicitly: ClassifyResponse { request_id: "req-011-bad", classifier_id: "sensitivity-synthetic", model_revision: "synthetic-for-mechanics-only", tokenizer_revision: "tokenizer-fixture", taxonomy_revision: "synthetic-prototypes", status: Ok, ranked: [...] }

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 5 filtered out
```

## Why this is the expected failure
The handler currently passes `requested_signals` through into the pipeline input
verbatim and never validates them. A request asking for an unknown signal
(`pii`) is therefore silently accepted and returns a successful `Ok` response —
the exact behavior U-011 forbids. The test asserts `expect_err` with
`tonic::Code::InvalidArgument`; it panics because the unknown signal is accepted.
The failure is caused by the missing requested-signal validation, matching the
criterion.

## References
- `tests/TEST_MATRIX.md` U-011
- `specs/0.1-mvp/test-plan.md` AC-009 -> U-011
