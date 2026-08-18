# AC-011 RED — benchmark runner binary does not exist

## Criterion / slice
AC-011 (OpenShift sidecar/ClusterIP RTT distributions captured) benchmark
MEASUREMENT INFRASTRUCTURE: a `src/bin/bench-runner.rs` binary the maintainer
executes (also unchanged on OpenShift) that runs the HOMELAB.md 0.1 protocol
against the REAL classifier and emits machine-readable results.

## Proving test (this slice)
`tests/bench_runner.rs::bench_runner_executes_a_tiny_matrix_against_the_real_model`
(`#[ignore]`, runs under `./hack/test-parity`). It launches the compiled
`bench-runner` binary with a TINY matrix against the real model dir, asserts it
exits 0, prints a human-readable table, and writes a valid JSON report carrying
the HOMELAB.md manifest fields (backend=candle, topology=loopback, git sha,
model dir + revision, tokenizer revision, cpu model, concurrency, cache mode,
sequence length, warmup/measure counts).

## RED state (the runner does not exist)
`src/bin/bench-runner.rs` did not exist, so there is no `bench-runner` bin target
and Cargo does not define `CARGO_BIN_EXE_bench-runner` for the integration test.
The proving test therefore cannot compile.

## Command
```
cargo test --locked --test bench_runner
```

## Result
FAILED. Expected RED reason: the benchmark-runner binary (AC-011 measurement
infrastructure) does not exist, so the smoke test cannot compile, let alone pass.

Failure excerpt:
```
error: environment variable `CARGO_BIN_EXE_bench-runner` not defined at compile time
  --> tests/bench_runner.rs:12:19
   |
12 | const BIN: &str = env!("CARGO_BIN_EXE_bench-runner");
   |                   ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
   |
   = help: `CARGO_BIN_EXE_bench-runner` may not be available for the current Cargo target
error: could not compile `llm-d-sc` (test "bench_runner") due to 1 previous error
```

## Why this is the expected failure
The slice's deliverable is the benchmark runner binary. Before it exists, the
smoke test that launches it cannot compile (no `bench-runner` target -> the
`CARGO_BIN_EXE_bench-runner` env var is undefined). This is precisely the
expected RED: the measurement infrastructure does not exist, so the proving
test cannot run. The failure is deterministic and confirms the test is
non-vacuous — once `src/bin/bench-runner.rs` is implemented (and the pinned
model is fetched), the smoke test must launch it, exit 0, and emit a valid JSON
report.

## Worktree / SHA
- HEAD SHA: `c3149e5a345adffb39b3cbf8deaed5b7c87f17e6`
- Working tree (RED state): `tests/bench_runner.rs` present (untracked);
  `src/bin/bench-runner.rs` ABSENT (the bin target did not exist).
- No commits/pushes.

## References
- `specs/0.1-mvp/test-plan.md` AC-011 -> P-030..P-033, S-001/S-002
- `tests/HOMELAB.md` benchmark protocol + manifest fields
- AGENTS.md (percentile latency evidence, never average-only)
