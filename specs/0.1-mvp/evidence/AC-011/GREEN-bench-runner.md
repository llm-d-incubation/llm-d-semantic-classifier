# AC-011 GREEN — benchmark runner smoke test passes against the REAL model

## Proving test
`tests/bench_runner.rs::bench_runner_executes_a_tiny_matrix_against_the_real_model`
(`#[ignore]`, runs under `./hack/test-parity`).

## Command
```
cargo test --locked --test bench_runner -- --ignored --nocapture
```

## Result
PASSED (exit 0):
```
running 1 test
test bench_runner_executes_a_tiny_matrix_against_the_real_model ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 36.69s
```

## Why this proves GREEN
The compiled `bench-runner` binary launched against the real pinned sensitivity
model, executed a tiny 0.1 matrix (4 lengths x 2 cache modes x 2 concurrencies),
exited 0, printed a human-readable table + JSON path on stdout, and wrote a
valid JSON report whose manifest carries the HOMELAB.md fields
(backend=candle, topology=loopback, git sha, model dir + revision, tokenizer
revision, cpu model, warmup/measure counts) and a non-empty scenario list.
The runner's methodology self-check (miss -> measured cache misses, hit ->
measured cache hits) ran inside every scenario and did not abort.

## Worktree / SHA
- HEAD SHA: `2f1c66e8e1a69e1376b7d71f7a631bd66e8d02fa`
- Working tree: `src/bin/bench-runner.rs` + `tests/bench_runner.rs` present
  (untracked). No commits/pushes.

## See also
`BENCH-RUNNER.md` (full runner documentation and end-to-end run).
