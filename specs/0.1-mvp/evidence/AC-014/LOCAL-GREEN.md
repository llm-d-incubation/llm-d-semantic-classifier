# AC-014 LOCAL-GREEN evidence — default telemetry contains no raw prompt/session text

## Scope (this file)
The local deterministic telemetry mechanics for AC-014 are GREEN: a telemetry
recorder and a server trace-capture surface exist, and the tests mapped to AC-014
(U-085, I-085) pass locally. Per `specs/0.1-mvp/test-plan.md`, AC-014 maps to
U-085 and I-085 only — there is NO Kubernetes system test for AC-014, so this
LOCAL-GREEN covers the whole mapped set.

## Criterion
AC-014 requires the service's default telemetry (logs, metrics, trace capture) to
never contain raw prompt or session text. `specs/0.1-mvp/test-plan.md` maps
AC-014 to U-085 (unit) and I-085 (integration).

## What this evidence proves (local mechanics) — all GREEN
`cargo test --locked --test telemetry` — plain `#[test]`, offline (the
deterministic pipeline requires no model forward). Each test drives the real
telemetry surface:
- `u085_raw_prompt_absent_from_default_logs_metrics` (U-085): the default
  serialized logs/metrics output surfaces the request id and a context hash but
  never the raw prompt text or the raw session text.
- `i085_trace_capture_has_ids_hashes_no_raw_prompt` (I-085): binds a real
  `ClassifyServer`, drives a classify request over the persistent gRPC channel,
  and reads `server.trace_capture()` asserting every `TraceEvent` carries the
  request id and context/session hashes but never the raw prompt or session text.

## Implementation
The RED was that no telemetry surface existed. Smallest change:
- `src/telemetry.rs` (new): `Telemetry` (interior-mutability recorder via
  `Arc<Mutex<_>>`, `Clone` shares state), `RequestEvent`, `TraceEvent`. Recording
  hashes the context (`ctx_`) and session (`sess_`) with blake3 and never retains
  the raw text; `default_output()` emits `request_id=`/`context_hash=`/
  `session_hash=` lines only; `trace_capture()` returns the captured events.
- `src/grpc/classify.rs`: `ClassifyServiceImpl` holds a shared `Telemetry` and the
  classify handler records a `RequestEvent` (hashing context/session) before
  moving the request fields into the pipeline input; `ClassifyServer` holds a
  shared `Telemetry` and exposes `trace_capture() -> Vec<TraceEvent>`.
- `src/lib.rs`: added `pub mod telemetry;`.

## Command
```
cargo test --locked --test telemetry
```

## Result
PASSED:
```
running 2 tests
test u085_raw_prompt_absent_from_default_logs_metrics ... ok
test i085_trace_capture_has_ids_hashes_no_raw_prompt ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.05s
```
Exit code 0.

## Gate
`./hack/test-impact` and `./hack/spec-check 0.1-mvp` and `./hack/verify` are run
after this evidence (see the turn summary). No Kubernetes system tier is mapped to
AC-014, so PROMOTION-GREEN is not applicable and is not written by the worker.

## Worktree / SHA
- HEAD SHA: `259e707f8e5a2c3a030e84df9d9413295f5184e6`
- Working tree (uncommitted): `src/telemetry.rs` (new), `src/lib.rs` modified,
  `src/grpc/classify.rs` modified, `tests/telemetry.rs` untracked,
  `specs/0.1-mvp/evidence/AC-014/` untracked. No commits/pushes.

## References
- `specs/0.1-mvp/test-plan.md` AC-014 -> U-085, I-085
- `tests/TEST_MATRIX.md` U-085, I-085
- `specs/0.1-mvp/spec.md` AC-014 "default telemetry contains no raw prompt/
  session text"
