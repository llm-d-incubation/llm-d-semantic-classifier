# AC-014 RED evidence — default telemetry contains no raw prompt/session text

## Criterion
AC-014 requires the service's default telemetry (logs, metrics, trace capture)
to never contain raw prompt or session text. Per `specs/0.1-mvp/test-plan.md`,
AC-014 maps to U-085 (raw prompt absent from default logs/metrics) and I-085
(trace capture has IDs/hashes but no raw prompt). No Kubernetes system test is
mapped to AC-014, so this slice covers the local deterministic mechanics
(U-085/I-085) only.

## Proving tests (this slice)
- U-085 `u085_raw_prompt_absent_from_default_logs_metrics`
- I-085 `i085_trace_capture_has_ids_hashes_no_raw_prompt`

Both in `tests/telemetry.rs` (integration, plain `#[test]`, offline — the
deterministic pipeline requires no model forward). U-085 drives a proposed
telemetry recorder (`llm_d_sc::telemetry::{Telemetry, RequestEvent}`) and
asserts its default serialized output contains the request id and a context
hash but never the raw prompt text or raw session text. I-085 binds a real
`ClassifyServer`, drives a classify request over the persistent channel, and
reads `server.trace_capture()` (`Vec<TraceEvent>`) asserting the trace carries
the request id and context/session hashes but never the raw prompt or raw
session text.

## RED state (no telemetry surface exists)
There is no telemetry/logging/trace infrastructure anywhere in the crate:
- no `telemetry` module (`src/lib.rs` registers only bench/cache/classify/
  config/dummy_gateway/embedding/grpc/metrics/queue/ranker/runtime/tokenizer);
- no tracing/logging dependency in `Cargo.toml` (only serde/serde_json/toml/
  candle/tonic/prost/tokio/blake3/tokenizers);
- `src/metrics.rs` records only latency stages and cache hit/miss counters — no
  request labels, no request id, no context hash;
- `ClassifyServer` exposes no `trace_capture`, and the classify pipeline
  (`src/grpc/classify.rs` -> `src/classify.rs`) records no request telemetry.

The proving tests reference `llm_d_sc::telemetry` and
`server.trace_capture()`, which are undefined, so they cannot compile.

## Command
```
cargo test --locked --test telemetry
```

## Result
FAILED. Expected RED reason: the telemetry surface (AC-014) does not exist yet,
so the proving tests cannot compile, let alone pass.

Failure excerpt:
```
error[E0432]: unresolved import `llm_d_sc::telemetry`
  --> tests/telemetry.rs:20:15
   |
20 | use llm_d_sc::telemetry::{RequestEvent, Telemetry, TraceEvent};
   |               ^^^^^^^^^ could not find `telemetry` in `llm_d_sc`

error[E0599]: no method named `trace_capture` found for struct `llm_d_sc::grpc::classify::ClassifyServer` in the current scope
  --> tests/telemetry.rs:73:41
   |
73 |     let trace: Vec<TraceEvent> = server.trace_capture();
   |                                         ^^^^^^^^^^^^^ method not found in `llm_d_sc::grpc::classify::ClassifyServer`

Some errors have detailed explanations: E0432, E0599.
For more information about an error, try `rustc --explain E0432`.
error: could not compile `llm-d-sc` (test "telemetry") due to 2 previous errors
```
Exit code: 101.

## Why this is the expected failure
AC-014 demands that default telemetry never carry raw prompt or session text.
No such capture surface exists: the crate has no `telemetry` module that
records request events, and `ClassifyServer` exposes no `trace_capture`. The
tests deliberately reference the proposed surface — a recorder whose default
output is request ids + context/session hashes (blake3), never the raw text —
and assert the absence of raw prompt/session text. Because
`llm_d_sc::telemetry` is undefined and `ClassifyServer::trace_capture` does not
exist, `cargo test --locked --test telemetry` fails at exit 101 with "could not
find `telemetry` in `llm_d_sc`" and "method not found". This is precisely the
expected RED: the feature (telemetry without raw prompt/session text) does not
exist, so the proving tests cannot run, let alone pass. The failure is
deterministic and confirms the tests are non-vacuous — once a `telemetry`
recorder whose default output carries request ids and context/session hashes
but never raw text (and a `ClassifyServer::trace_capture` surface) is
implemented, U-085/I-085 become selectable and must pass.

## Worktree / SHA
- HEAD SHA: `259e707f8e5a2c3a030e84df9d9413295f5184e6`
- Working tree: `tests/telemetry.rs` added (untracked); `src/` unchanged.
- `git status`:
  ```
   ?? tests/telemetry.rs
  ```
  No commits/pushes.

## References
- `specs/0.1-mvp/test-plan.md` AC-014 -> U-085, I-085
- `tests/TEST_MATRIX.md` U-085 / I-085
- `specs/0.1-mvp/spec.md` AC-014 "default telemetry contains no raw prompt/
  session text"
- AC-012 RED precedent: `specs/0.1-mvp/evidence/AC-012/RED.md` (undefined
  metrics module -> E0432/E0599 exit 101)
