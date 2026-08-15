# AC-003 RED evidence

## Acceptance criterion
AC-003 ModelCar supplies required files with no runtime HF fetch.

## Proving test(s)
- I-064 incomplete/corrupt ModelCar fails readiness (selected this iteration)
- I-060..I-063, S-010/S-051/S-053 (later integration/system phases)

## Command
```
cargo test --locked i064
```

## Worktree state
- Branch: agent/0.1-mvp
- HEAD: `eb591f81ea04c765736b8974cb7280d383eb36b3`
- `git status`:
  ```
  Changes not staged for commit:
  	modified:   src/runtime.rs
  ```
- Working tree has one test-only change: `src/runtime.rs` adds the I-064
  proving test to the `runtime::tests` module. No production code changed.

## Failure excerpt
```
$ cargo test --locked i064
   Compiling llm-d-sc v0.1.0 (/Users/cnuland/llm-d-sc-genesis)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.24s
     Running unittests src/lib.rs (target/debug/deps/llm_d_sc-8d2540db6cc19180)

running 1 test
test runtime::tests::i064_incomplete_modelcar_fails_readiness ... FAILED

failures:

---- runtime::tests::i064_incomplete_modelcar_fails_readiness stdout ----

thread 'runtime::tests::i064_incomplete_modelcar_fails_readiness' (17807965)
panicked at src/runtime.rs:122:14:
incomplete ModelCar must fail warmup: ()

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 7 filtered out
```

## Why this is the expected failure
AC-003 requires that a ModelCar supplies the required model files and that the
service starts solely from `/models` with no runtime Hugging Face fetch.
`tests/fixtures/modelcar/classifier-manifest.json` declares the required
resident files: `/models/model.safetensors`, `/models/tokenizer.json`, and
`/models/1_Pooling/config.json`.

The proving test I-064 exercises the failure branch: an incomplete ModelCar
(a model directory that exists but contains none of the required files) must
fail readiness and keep the runtime NOT ready. Today `Runtime::warmup`
(`src/runtime.rs:39`) validates only that the model *path* exists and is
readable, then unconditionally flips to ready. It performs no validation that
the ModelCar contains the required weights/tokenizer/pooling files.

Consequently, for an incomplete model directory `warmup` returns `Ok(())` and
sets readiness to ready. The test's `expect_err("incomplete ModelCar must fail
warmup")` panics at `src/runtime.rs:122` with `incomplete ModelCar must fail
warmup: ()` — i.e. the feature (ModelCar file-presence validation gating
readiness) does not exist yet. This is precisely the expected RED.

Note: `./hack/verify` is NOT run this iteration — per AGENTS.md steps 1-4 only
the RED proof and evidence are required before implementation. The
GREEN/implementation step will extend warmup/readiness to validate the
ModelCar's required files so I-064 compiles and passes, then the remaining
AC-003 tests (I-060..I-063, S-010/S-051/S-053) are exercised in later phases.
