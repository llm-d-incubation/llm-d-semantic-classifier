# AC-014 GREEN evidence — I-085 trace capture has IDs/hashes but no raw prompt

## Test ID
I-085 (`specs/0.1-mvp/test-plan.md` maps AC-014 to U-085, I-085).

## Test file
`tests/telemetry.rs` — `i085_trace_capture_has_ids_hashes_no_raw_prompt`
(plain deterministic `#[test]`, integration over a real gRPC server, offline).

## Command
```
cargo test --locked --test telemetry
```

## Result: GREEN
```
running 2 tests
test u085_raw_prompt_absent_from_default_logs_metrics ... ok
test i085_trace_capture_has_ids_hashes_no_raw_prompt ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.05s
```
Exit code 0.

## What this slice proves (I-085)
Driving a real classify request over the persistent gRPC channel leaves a
captured trace (`server.trace_capture()`) containing a `TraceEvent` with the
request id (`req-085`). Every trace event carries a non-empty context hash
(`ctx_...`) and a session hash that never contain the raw prompt text
(`TRACE secret prompt`) or the raw session text (`sess-trace-secret`).

## Smallest implementation change
- `src/telemetry.rs` (new): `TraceEvent { request_id, context_hash,
  session_hash }` and `Telemetry::trace_capture()`.
- `src/grpc/classify.rs`: `ClassifyServiceImpl` now holds a shared `Telemetry`;
  the classify handler records a `RequestEvent` (hashing context/session) before
  moving the request fields into the pipeline input. `ClassifyServer` holds a
  shared `Telemetry` and exposes `trace_capture() -> Vec<TraceEvent>`.

## Worktree / SHA
- HEAD SHA: `259e707f8e5a2c3a030e84df9d9413295f5184e6`
- Working tree (uncommitted): `src/telemetry.rs` (new), `src/lib.rs` modified,
  `src/grpc/classify.rs` modified, `tests/telemetry.rs` untracked, AC-014
  evidence untracked. No commits/pushes.

## References
- `specs/0.1-mvp/test-plan.md` AC-014 -> U-085, I-085
- `tests/TEST_MATRIX.md` I-085 "trace capture has IDs/hashes but no raw prompt"
- `specs/0.1-mvp/spec.md` AC-014 "default telemetry contains no raw prompt/
  session text"
