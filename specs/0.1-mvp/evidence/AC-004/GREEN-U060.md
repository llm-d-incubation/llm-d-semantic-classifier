# AC-004 GREEN evidence — U-060 slice

## Acceptance criterion
AC-004 pinned sensitivity model matches trusted reference embedding/ranking
fixtures.

NOTE: AC-004 is NOT complete. This file proves only the U-060 slice. AC-004
remains open until U-061..U-067 and I-020..I-025 all pass; the whole-criterion
`GREEN.md` is only written once every mapped test for AC-004 is green.

## Proving test
- U-060 tokenizer golden token IDs match trusted reference (this iteration)

## Command
```
cargo test --locked u060
```

## Result
```
running 1 test
test tokenizer::tests::u060_tokenizer_golden_token_ids_match_trusted_reference ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 8 filtered out; finished in 0.07s
```

## Worktree state
- Branch: agent/0.1-mvp
- HEAD: `ce7768670e066930ee08fec335e682bbe8cc68d5`
- `git status`:
  ```
  Changes not staged for commit:
  	modified:   Cargo.lock
  	modified:   Cargo.toml
  	modified:   src/lib.rs
  Untracked files:
  	specs/0.1-mvp/evidence/AC-004/
  	src/tokenizer.rs
  	tests/fixtures/modelcar/golden-token-ids.json
  	tests/fixtures/modelcar/tokenizer.json
  ```
- `cargo build --locked` is clean (no warnings).

## What changed
- Added a resident `Tokenizer` in `src/tokenizer.rs` that loads a
  `tokenizer.json` (HuggingFace `tokenizers` library format) and reproduces the
  reference `BertNormalizer` + `BertPreTokenizer` + `WordPiece` +
  `TemplateProcessing` pipeline. No network access at runtime.
- Added `serde_json` (parse the tokenizer), `unicode-normalization` (NFD before
  accent stripping), and `unicode_categories` (exact control/punctuation/
  nonspacing-mark categories, the same crate the reference uses) to
  `Cargo.toml`.
- Committed the pinned ModelCar tokenizer fixture at
  `tests/fixtures/modelcar/tokenizer.json` (the real `/models/tokenizer.json`
  artifact) and the golden fixture at
  `tests/fixtures/modelcar/golden-token-ids.json`.
- U-060 now loads the tokenizer from the committed fixture (hermetic, no
  mounted `/models` needed) and asserts its token IDs equal the committed golden
  fixture produced by the pinned Python reference.

## Why this is the expected GREEN
The RED was exit 101: the `tokenizer` module had no `Tokenizer` type, so the
U-060 test could not compile. The resident tokenizer now compiles and, for the
golden input `this is a golden sensitivity input`, produces exactly the trusted
reference IDs `[101, 2023, 2003, 1037, 3585, 14639, 7953, 102]` (verified with
the pinned `tokenizers` library), so U-060 passes.

Full unit suite: 9 passed (U-060, U-020, U-022, I-064, U-001..U-005); build
clean. The implementation was also spot-checked against the reference for a
subword-splitting/CJK input (`don't 你好` → `[101, 2123, 1005, 1056, 100, 100,
102]`) and matched exactly.
