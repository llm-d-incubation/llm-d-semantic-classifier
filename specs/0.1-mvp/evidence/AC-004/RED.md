# AC-004 RED evidence

## Acceptance criterion
AC-004 pinned sensitivity model matches trusted reference embedding/ranking
fixtures.

## Proving test(s)
- U-060 tokenizer golden token IDs match trusted reference (selected this
  iteration)
- U-061..U-067, I-020..I-025 (later iterations/phases)

## Command
```
cargo test --locked u060
```

## Worktree state
- Branch: agent/0.1-mvp
- HEAD: `ce7768670e066930ee08fec335e682bbe8cc68d5`
- `git status`:
  ```
  Changes not staged for commit:
  	modified:   src/lib.rs
  Untracked files:
  	src/tokenizer.rs
  ```
- Working tree has two test-scaffold changes only: `src/lib.rs` registers a
  new `tokenizer` module; `src/tokenizer.rs` holds the U-060 proving test.
  No production code changed.

## Failure excerpt
```
$ cargo test --locked u060
   Compiling llm-d-sc v0.1.0 (/Users/cnuland/llm-d-sc-genesis)
error[E0432]: unresolved import `super::Tokenizer`
  --> src/tokenizer.rs:10:9
   |
10 |     use super::Tokenizer;
   |         ^^^^^^^^^^^^^^^^ no `Tokenizer` in `tokenizer`

For more information about this error, try `rustc --explain E0432`.
error: could not compile `llm-d-sc` (lib test) due to 1 previous error
--- exit 101 ---
```

## Why this is the expected failure
AC-004 requires that the pinned sensitivity model match trusted reference
embedding/ranking fixtures. The first deterministic stage of that parity is
tokenization: the resident tokenizer must reproduce the exact golden token IDs
of the pinned Python reference for the golden inputs (U-060). The proving test
therefore selects a `Tokenizer` type (`Tokenizer::load("/models/tokenizer.json")`
then `.tokenize(...)`) and asserts the produced token IDs equal trusted
reference `GOLDEN_TOKEN_IDS`.

Today the crate ships only `src/config.rs` (configuration, AC-001) and
`src/runtime.rs` (readiness/lifecycle, AC-002/AC-003). No tokenizer/embedding
code exists: the `tokenizer` module has no `Tokenizer` type at all. Because
`super::Tokenizer` is undefined, the U-060 test cannot compile — `cargo test
--locked u060` fails at exit 101 with "no `Tokenizer` in `tokenizer`". This is
precisely the expected RED: the feature (tokenizer golden token-ID parity
against the pinned reference) does not exist, so the proving test cannot run,
let alone pass.

The trusted reference is the pinned Python implementation for the sensitivity
model at HF revision `43f21d21ac48134464f8510a9ac9c95bdac7ba86`
(`tests/fixtures/modelcar/classifier-manifest.json`). Per
`tests/fixtures/modelcar/README.md`, golden token IDs / embedding parity must be
generated with that pinned reference and committed as fixtures; the GREEN step
will add the minimal resident `Tokenizer` and the committed golden reference
values so U-060 compiles and passes.

Note: `./hack/verify` is NOT run this iteration — per AGENTS.md steps 1-4 only
the RED proof and evidence are required before implementation. The
GREEN/implementation step will add the tokenizer so U-060 compiles and passes,
then the remaining AC-004 tests (U-061..U-067, I-020..I-025) are exercised in
later phases.
