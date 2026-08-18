# AC-011 BENCH-RUNNER.md — benchmark measurement infrastructure (maintainer-executable)

## Deliverable
`src/bin/bench-runner.rs` — the benchmark runner binary that executes the
`tests/HOMELAB.md` 0.1 protocol against the REAL classifier and emits
machine-readable results. The maintainer runs it directly; it must also run
unchanged on OpenShift later (only `LLM_D_SC_MODEL_DIR` / `BENCH_OUT` / CPU env
differ, never the code).

## Criterion
AC-011 (OpenShift sidecar/ClusterIP RTT distributions captured). This runner is
the measurement infrastructure the maintainer executes. The criterion's OWN
tests (P-030..P-033, S-001/S-002) remain PENDING cluster measurement (see
`LOCAL-GREEN.md`); this file proves the runner itself executes the protocol.

## Requirements satisfied

1. **Real classifier, never synthetic fallback.** Builds a real `CandleClassifier`
   via `load_and_warm_modelcar` from `LLM_D_SC_MODEL_DIR` (default
   `artifacts/models/sensitivity`) and serves it with
   `ClassifyServer::bind_with_classifier` on an EPHEMERAL loopback port. If the
   model dir is absent or empty the runner EXITS with a clear FATAL error and
   exit code 1 — it NEVER silently falls back to the synthetic pipeline, because
   that would produce meaningless numbers.
2. **0.1 matrix via `src/bench.rs`.** Cache modes Hit and Miss x input token
   lengths 32/64/128/256 x concurrency 1 and 4 (P-020/P-021). Inputs are built
   with a length-specific seed (`build_seed`, repeating `benchmark`) so the
   tokenized length approximates each target; the ACTUAL token count of the sent
   text is recorded per scenario. `BenchmarkRun::with_seed` keeps the harness's
   disjoint warmup/measured namespaces intact (new unit test
   `seed_aware_namespaces_preserve_methodology` in `src/bench.rs`).
3. **Full per-scenario metrics.** p50/p90/p95/p99/max, throughput req/s, error
   count, and the queue/tokenize/forward/total stage decomposition from the
   server's metrics surface (`MetricsSnapshot` delta over the measured window,
   AC-012).
4. **Methodology self-check asserted.** The runner calls the harness's OWN
   `BenchmarkRun::measure`/`measure_concurrent`, whose `verify_metrics` asserts
   miss scenarios show `measured_count` cache misses and hit scenarios
   `measured_count` hits. Any violation (or any request error) returns `Err`
   and the runner FAILS LOUDLY (non-zero exit) with the offending
   mode/length/concurrency. Hit-mode warmup is `max(BENCH_WARMUP, BENCH_MEASURE)`
   so every measured hit key is pre-warmed.
5. **Machine-readable output.** Emits JSON to
   `artifacts/bench/<timestamp>.json` (BENCH_OUT override) plus a human-readable
   table on stdout. The manifest carries every HOMELAB.md field available
   locally: git sha, model dir + revision, tokenizer revision, backend=candle,
   topology=loopback, cpu model (via `CPU_MODEL` env or `sysctl`/`/proc/cpuinfo`),
   concurrency, cache mode, sequence length, warmup/measure counts.
6. **Smoke test.** `tests/bench_runner.rs::bench_runner_executes_a_tiny_matrix_against_the_real_model`
   (`#[ignore]`, runs under `./hack/test-parity`) launches the compiled binary
   with a TINY matrix, asserts exit 0, a JSON path + human-readable table on
   stdout, a valid JSON report, and the required manifest fields.

## GREEN proof — smoke test against the REAL model

The pinned sensitivity model is present locally
(`artifacts/models/sensitivity/model.safetensors`), so the smoke test was run
against the real forward.

### Command
```
cargo test --locked --test bench_runner -- --ignored --nocapture
```

### Result
PASSED (exit 0):
```
running 1 test
test bench_runner_executes_a_tiny_matrix_against_the_real_model ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 36.69s
```

### What the smoke test asserts
- Runner exits 0 with `LLM_D_SC_MODEL_DIR` set to the real model dir.
- It prints `wrote JSON results to <path>` and the `== llm-d-sc benchmark runner` table.
- The announced JSON exists and is valid JSON.
- It carries a `manifest` with `git_sha`, `model_dir`, `model_revision`,
  `tokenizer_revision`, `backend=candle`, `topology=loopback`, `cpu_model`,
  `warmup_requests`, `measured_requests`, and a non-empty `scenarios` array.
- Smoke artifact is removed afterward (no accumulation in gitignored
  `artifacts/bench/`).

## End-to-end run (maintainer-invoked)

A tiny manual run (`BENCH_WARMUP=1 BENCH_MEASURE=2`) exercised all 16 scenarios
and produced real forward numbers (e.g. miss seq=256 conc=1 p50 1206ms, hit
seq=32 conc=1 p50 0.33ms), confirming the runner builds the real classifier and
records real tokenize/forward stage times with 0 errors.

## RED → GREEN
- RED: `specs/0.1-mvp/evidence/AC-011/RED-bench-runner.md` — the bin target did
  not exist, so the smoke test could not compile
  (`CARGO_BIN_EXE_bench-runner` undefined).
- GREEN: this file — the binary exists, compiles, and the smoke test passes
  against the real model.

## Worktree / SHA
- HEAD SHA: `2f1c66e8e1a69e1376b7d71f7a631bd66e8d02fa`
- Working tree (uncommitted): `src/bin/bench-runner.rs`,
  `tests/bench_runner.rs`; modifications to `src/bench.rs`, `src/classify.rs`,
  `src/embedding.rs`, `src/grpc/classify.rs`; rustfmt normalization of
  pre-existing `tests/floor*.rs` (rustfmt 1.9.0 drift, mechanical only). No
  commits/pushes.

## Gates
- `./hack/test-impact <slice files>` -> FULL SUITE (unknown surface; verify runs
  the whole suite).
- `./hack/spec-check 0.1-mvp` -> OK; AC-011 LOCAL-GREEN (harness + runner),
  P-030..P-033/S-001/S-002 pending cluster measurement by design.
- `./hack/verify` -> exit 0, GREEN **without weights** (the smoke test is
  `#[ignore]` and does not run in verify; it runs under `./hack/test-parity`).
