# AC-004 U-066 RED evidence

## Test
U-066 max-length truncation deterministic

## Command
```
cargo test --locked u066
```

## Result (RED)
```
running 1 test
test tokenizer::tests::u066_max_length_truncation_deterministic ... FAILED

failures:
---- tokenizer::tests::u066_max_length_truncation_deterministic stdout ----
thread 'tokenizer::tests::u066_max_length_truncation_deterministic' panicked at
src/tokenizer.rs:388:9:
assertion `left == right` failed: truncated output length must equal the fixture max_length
  left: 405
 right: 256
...
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 9 filtered out
```

## Worktree state
- Branch: agent/0.1-mvp
- HEAD: `ce7768670e066930ee08fec335e682bbe8cc68d5` (committed baseline unchanged)
- `git status` at RED: `src/tokenizer.rs` modified (test added), no truncation
  implemented yet.

## Why this is the expected failure
Before this slice the resident `Tokenizer::tokenize` applied no truncation, so an
over-length input (400 `golden` words + `tailmarker`, which wordpieces to
`tail/##mark/##er`) produced 405 IDs (403 content + `[CLS]` + `[SEP]`) instead of
the fixture-capped 256. The test asserts total length equals the fixture's
`truncation.max_length` (256), so it fails for the right reason: the max-length
truncation rule was simply not implemented yet.
