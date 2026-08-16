# AC-004 GREEN evidence — U-065

## Proving test
- U-065 `u065_exact_tie_ranks_by_documented_rule` (`src/ranker.rs`, plain
  `#[test]`, pure math)
- Ranks the fixed embedding `proto-a + proto-c` so proto-a and proto-c tie
  exactly at 1/√2 and proto-b/proto-d tie at 0; asserts ties are ordered
  lexicographically by id (a < c, b < d) per the documented rule.

## Smallest change
`src/ranker.rs`: `cosine_rank` sort key — primary descending score, secondary
ascending id (`score_cmp.then_with(|| a.0.cmp(&b.0))`). Documented in the
function doc ("ties broken lexicographically by id").

## Command
```
cargo test --locked u06
```

## Result
```
test ranker::tests::u065_exact_tie_ranks_by_documented_rule ... ok
test result: ok. 4 passed; 0 failed; 3 ignored
```

## Worktree / SHA
- SHA: `7736ea9` (uncommitted)
- No commits/pushes.
