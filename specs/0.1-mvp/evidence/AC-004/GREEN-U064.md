# AC-004 GREEN evidence — U-064

## Proving test
- U-064 `u064_prototype_ranking_deterministic` (`src/ranker.rs`, plain `#[test]`,
  pure math)
- Loads the committed synthetic fixture (`synthetic_for_mechanics_only`, 4
  orthogonal 384-dim unit vectors proto-a..proto-d), ranks the fixed synthetic
  embedding `proto-b`, and asserts `proto-b` is top at score ~1.0 and the full
  ranking is identical across 100 iterations.

## Smallest change
`src/ranker.rs`: replaced the RED stub (input-order scores) with a real
`cosine_rank` that sorts by descending cosine score and breaks exact ties by
ascending id (lexicographic). Ordering contract documented in the function doc.

## Command
```
cargo test --locked u06
```

## Result
```
test ranker::tests::u064_prototype_ranking_deterministic ... ok
test result: ok. 4 passed; 0 failed; 3 ignored
```

## Worktree / SHA
- SHA: `7736ea9` (implementation and fixture uncommitted)
- `git status`: `M src/lib.rs`, `?? src/ranker.rs`, `?? tests/fixtures/modelcar/synthetic-prototypes.json`. No commits/pushes.
