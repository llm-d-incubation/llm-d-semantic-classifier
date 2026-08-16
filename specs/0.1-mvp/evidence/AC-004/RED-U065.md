# AC-004 RED evidence — U-065

## Proving test
- U-065 `u065_exact_tie_ranks_by_documented_rule` (`src/ranker.rs`, plain
  `#[test]`, pure math, NOT ignored)
- Loads the committed synthetic fixture (4 orthogonal 384-dim unit vectors
  proto-a..proto-d), builds the fixed embedding `proto-a + proto-c` (so proto-a
  and proto-c tie exactly at cosine 1/√2; proto-b and proto-d tie at 0), and
  asserts the documented tie rule: exact ties ordered lexicographically by id
  (a < c, b < d).

## Command
```
cargo test --locked u06
```

## Result
FAILED. Expected RED reason: the `cosine_rank` stub returns scores in INPUT
order without any lexicographic tie-break, so the second rank is `proto-b`
instead of the tied `proto-c`.

Failure excerpt:
```
---- ranker::tests::u065_exact_tie_ranks_by_documented_rule stdout ----
thread 'ranker::tests::u065_exact_tie_ranks_by_documented_rule' panicked at src/ranker.rs:191:9:
assertion `left == right` failed
  left: "proto-b"
 right: "proto-c"
```

## Worktree / SHA
- SHA: `7736ea9`
- `git status`: `M src/lib.rs`, `?? src/ranker.rs`, `?? tests/fixtures/modelcar/synthetic-prototypes.json`. No commits/pushes.

## Why this is the expected failure
The tie-break rule (secondary sort ascending by id) is not implemented by the
stub, so equal scores are left in input order and the documented
id-lexicographic order is violated.
