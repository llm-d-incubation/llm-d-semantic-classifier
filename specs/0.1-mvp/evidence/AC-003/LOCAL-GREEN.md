# AC-003 GREEN evidence

## Acceptance criterion
AC-003 ModelCar supplies required files with no runtime HF fetch.

## Proving test
- I-064 incomplete/corrupt ModelCar fails readiness (this iteration)

## Command
```
cargo test --locked i064
```

## Result
```
running 1 test
test runtime::tests::i064_incomplete_modelcar_fails_readiness ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 7 filtered out; finished in 0.00s
```

## Worktree state
- Branch: agent/0.1-mvp
- HEAD: `eb591f81ea04c765736b8974cb7280d383eb36b3`
- `git status`:
  ```
  Changes not staged for commit:
  	modified:   src/runtime.rs
  ```
- Working tree production change: `src/runtime.rs` adds `warmup_modelcar`,
  which validates that every ModelCar required file (relative to the model
  path) is present before flipping readiness; plus the I-064 proving test.
- `cargo build --locked` is clean (no warnings).

## What changed
- Added `Runtime::warmup_modelcar(path, required_files)`. It first verifies
  that each required file exists under the model directory; any missing file
  returns `Err` and the runtime stays NOT ready. Only after all required files
  are present does it delegate to the existing `warmup`, which flips to READY.
- The existing AC-002 `warmup(path)` (path exists + readable) is unchanged, so
  U-020/U-022 remain green.
- The I-064 test supplies the ModelCar required-file contract via
  `MODELCAR_REQUIRED_FILES`, matching the manifest-declared resident files
  (`model.safetensors`, `tokenizer.json`, `1_Pooling/config.json`).

## Why this is the expected GREEN
Previously warmup only checked that the model path existed and was readable,
then unconditionally flipped to READY — so an incomplete ModelCar (an existing
but empty model dir) was accepted. `warmup_modelcar` now gates readiness on the
presence of the ModelCar's required resident files, so I-064's empty dir fails
warmup and leaves the runtime NOT ready. This is the AC-003 requirement that a
ModelCar supplies the required files before the runtime reports ready.

Full unit suite: 8 passed (U-020, U-022, I-064, U-001..U-005); build clean.
