# AC-004 RED evidence — U-064

## Proving test
- U-064 `u064_prototype_ranking_deterministic` (`src/ranker.rs`, plain `#[test]`,
  pure math, NOT ignored)
- Loads the committed synthetic fixture
  (`tests/fixtures/modelcar/synthetic-prototypes.json`, label
  `synthetic_for_mechanics_only`, 4 orthogonal 384-dim unit vectors
  proto-a..proto-d), validates unit norms, then ranks the fixed synthetic
  embedding `proto-b` and asserts the top rank is `proto-b` at score ~1.0 and
  that the full ranking is byte-identical across 100 iterations.

## Command
```
cargo test --locked u06
```

## Result
FAILED (2 passed; 2 failed — U-064 and U-065; 3 ignored). Expected RED reason:
the `cosine_rank` stub computes scores but returns them in INPUT order
(`proto-a, proto-b, proto-c, proto-d`) — no descending sort, so `proto-b` does
not rank first.

Failure excerpt:
```
---- ranker::tests::u064_prototype_ranking_deterministic stdout ----
thread 'ranker::tests::u064_prototype_ranking_deterministic' panicked at src/ranker.rs:147:9:
assertion `left == right` failed: proto-b must rank first
  left: "proto-a"
 right: "proto-b"
```

## Worktree / SHA
- SHA: `7736ea9`
- `git status`: `M src/lib.rs` (module export), `?? src/ranker.rs` (new), `?? tests/fixtures/modelcar/synthetic-prototypes.json` (new). No commits/pushes.

## Why this is the expected failure
The proving test requires the documented ranking contract (highest cosine
similarity first). The stub omits the descending sort entirely, so the top rank
is the input-order first prototype (`proto-a`), not the correct `proto-b`.
