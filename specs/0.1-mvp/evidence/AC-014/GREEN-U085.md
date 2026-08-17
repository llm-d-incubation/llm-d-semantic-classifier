# AC-014 GREEN evidence — U-085 raw prompt absent from default logs/metrics

## Test ID
U-085 (`specs/0.1-mvp/test-plan.md` maps AC-014 to U-085, I-085).

## Test file
`tests/telemetry.rs` — `u085_raw_prompt_absent_from_default_logs_metrics`
(plain deterministic `#[test]`, offline).

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

## What this slice proves (U-085)
A telemetry recorder records a request event carrying a request id, a session id,
and the raw context text. The default serialized logs/metrics output surfaces the
request id (`req-085`) and a context hash (`ctx_...`) but NEVER the raw prompt
text (`RAW secret prompt`) or the raw session text (`sess-top-secret`).

## Smallest implementation change
- `src/telemetry.rs` (new): `Telemetry` (interior-mutability recorder via
  `Arc<Mutex<_>>`, `Clone` shares state), `RequestEvent`, `TraceEvent`. Recording
  hashes the context (`ctx_` prefix) and session (`sess_` prefix) with blake3 and
  never retains the raw text; `default_output()` emits `request_id=` +
  `context_hash=` + `session_hash=` lines only.
- `src/lib.rs`: added `pub mod telemetry;`.

## Worktree / SHA
- HEAD SHA: `259e707f8e5a2c3a030e84df9d9413295f5184e6`
- Working tree (uncommitted): `src/telemetry.rs` (new), `src/lib.rs` modified,
  `src/grpc/classify.rs` modified, `tests/telemetry.rs` untracked, AC-014
  evidence untracked. No commits/pushes.

## References
- `specs/0.1-mvp/test-plan.md` AC-014 -> U-085, I-085
- `tests/TEST_MATRIX.md` U-085 "raw prompt absent from default logs/metrics"
- `specs/0.1-mvp/spec.md` AC-014 "default telemetry contains no raw prompt/
  session text"
