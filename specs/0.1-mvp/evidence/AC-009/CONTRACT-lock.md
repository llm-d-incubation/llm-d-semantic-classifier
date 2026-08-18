# AC-009 CONTRACT-LOCK evidence — Convergence Slice 1: the wire contract

## Slice intent
The protobuf previously threw away everything the typed core knows: the response
was only `request_id` + `repeated string signals` — an ordered bag of signal names
with no scores, no revisions, and no status. This slice locks the wire contract so
the gRPC response faithfully carries the typed [`ClassificationResult`]
(`src/classify.rs`): request_id, classifier_id, model/tokenizer/taxonomy
revisions, a status, and ranked signals each with a label and a score.

## Locked schema (`proto/classify.proto`)
```proto
enum ClassificationStatus {
  CLASSIFICATION_STATUS_UNSPECIFIED = 0;
  OK = 1;
  ABSTAIN = 2;
  UNAVAILABLE = 3;
}

message RankedSignal {
  string label = 1;
  float score = 2;
}

message ClassifyResponse {
  string request_id = 1;
  string classifier_id = 2;
  string model_revision = 3;
  string tokenizer_revision = 4;
  string taxonomy_revision = 5;
  ClassificationStatus status = 6;
  repeated RankedSignal ranked = 7;
}
```
NO route/endpoint field (ADR-0001, AC-010): a route remains unrepresentable on
the wire. The `ClassifyRequest` is unchanged (request_id/session_id/context/
signals).

## Faithful mapping (`src/grpc/classify.rs`)
`ClassifyServiceImpl::classify` now maps the typed `ClassificationResult` to the
richer `ClassifyResponse`:
- `classifier_id` / `model_revision` / `tokenizer_revision` / `taxonomy_revision`
  copied verbatim from the typed result (the exact fingerprint that reproduces it).
- `status` mapped from `ClassifyStatus`:
  - `Ok -> OK`, `Abstain -> ABSTAIN`, `Error -> UNAVAILABLE`.
- `ranked` mapped from `result.ranked` as `RankedSignal { label: s.id, score: s.score as f32 }`
  — scores are now on the wire, not dropped.

## U-011: requested_signals is no longer dead (`src/grpc/classify.rs`)
`requested_signals` was passed through and never acted on. Now the handler
validates it: only `sensitivity` is accepted; any other signal is rejected with an
explicit `tonic::Status::invalid_argument` naming the unsupported signal. Never
silently ignored. (RED-first: `RED-U011.md` / `GREEN-U011.md`, recorded under
AC-009.)

## Existing gRPC/dummy-the AI Gateway tests strengthened (never weakened)
- `tests/grpc.rs` i001 (real round trip): now asserts non-empty `ranked`, every
  signal has a non-empty label and a finite score, all revision fingerprint
  fields are non-empty, and `status == OK`. i002 (persistent channel) similarly
  asserts labels + finite scores per turn.
- `src/dummy_gateway.rs` consumes the top ranked signal via `ranked[0].label`.
- `tests/restart.rs` I-045 and `tests/metrics.rs` I-080: updated to the `ranked`
  field and added `status == OK` assertions.
- `tests/schema.rs` U-010: updated to parse the richer `ClassifyResponse` block —
  still asserts NO route/endpoint/target field (ADR-0001, AC-010 invariant
  preserved) and now also asserts request_id, classifier_id, model_revision,
  tokenizer_revision, taxonomy_revision, status, and ranked are present.

## Commands & results (all green)
```
cargo test --locked --test grpc        # 6 passed; 0 failed
cargo test --locked --test schema      # 2 passed; 0 failed
./hack/test-impact ...                 # FULL SUITE (src/dummy_gateway.rs unknown surface)
./hack/spec-check 0.1-mvp              # OK; AC-009 5/8 (U-011 now green)
./hack/verify                          # EXIT 0 (fmt, clippy -D warnings, build, full suite)
```

## References
- `tests/TEST_MATRIX.md` U-010, U-011
- `docs/decisions/0001-no-route-field-in-response.md` (ADR-0001, decision (B))
- `specs/0.1-mvp/spec.md` AC-009/AC-010; `test-plan.md` (U-011 under AC-009)
