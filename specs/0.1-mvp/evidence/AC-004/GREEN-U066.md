# AC-004 U-066 GREEN evidence

## Test
U-066 max-length truncation deterministic

## Command
```
cargo test --locked u066
```

## Result (GREEN)
```
running 1 test
test tokenizer::tests::u066_max_length_truncation_deterministic ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 9 filtered out
```

## Worktree state
- Branch: agent/0.1-mvp
- HEAD: `ce7768670e066930ee08fec335e682bbe8cc68d5` (committed baseline unchanged;
  changes uncommitted)
- `cargo build --locked` clean (no warnings).

## What changed
- `Tokenizer` now reads the fixture's `truncation.max_length` (256) into a new
  `max_length: Option<usize>` field (`src/tokenizer.rs`).
- `tokenize` builds the wordpiece content tokens first, truncates them from the
  right to `max_length - 2` (the two special tokens `[CLS]`/`[SEP]` that the
  reference `TemplateProcessing` adds for a single sequence), then wraps with
  `[CLS]` + content + `[SEP]`. This caps total length at `max_length`, preserves
  both special tokens, and drops excess tokens from the tail — matching the
  pinned `tokenizers` library.

## Why this is the expected GREEN
The RED failed because truncation was absent (405 IDs vs the fixture cap 256).
Implementing the deterministic right-truncation to `max_length - 2` content
tokens makes the test pass: total length 256, `[CLS]` first, `[SEP]` last, 254
content tokens, tail marker dropped, head kept, and two identical tokenizations
are equal (determinism).

## Reference parity (independent of the unit test)
A throwaway integration check encoded the identical over-length input
(`"golden " * 400 + "tailmarker"`) and asserted the Rust token IDs exactly equal
the pinned `tokenizers` 0.22.1 output for the same committed fixture:
```
ref len: 256, ref head: [101, 3585, 3585], ref tail: [3585, 3585, 102]
```
Rust output matched the reference ID array exactly. The throwaway check was
deleted after passing.

## Full suite
`cargo test --locked` -> 10 passed (U-066, U-060, U-020, U-022, I-064,
U-001..U-005); build clean. `hack/verify` -> GREEN, exit 0.
