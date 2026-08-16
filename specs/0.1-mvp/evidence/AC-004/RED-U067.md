# AC-004 RED evidence — U-067

## Proving test
- U-067 `u067_golden_fixture_ranking_matches_reference` (`src/embedding.rs`,
  `#[ignore]`d, run with `-- --ignored` after `./hack/fetch-model`).
- Loads the resident embedder, embeds the golden input
  `"this is a golden sensitivity input"`, cosine-ranks the committed SYNTHETIC
  prototypes (`tests/fixtures/modelcar/synthetic-prototypes.json`,
  label `synthetic_for_mechanics_only`), and asserts (a) the rank ORDER matches
  the golden-ranking fixture exactly and (b) each score is within `1e-4` of the
  fixture.

## Why this is a golden verification test, not a fresh implementation
`embed` (U-062/U-063) and `cosine_rank` (U-064/U-065) already exist, so U-067 is
a composition test against the new golden fixture. To prove the assertion is
non-vacuous (that it actually catches a discrepancy), RED was demonstrated by
temporarily corrupting the fixture's expected order (swapping the two top-ranked
ids), running the test, observing the failure, then restoring the correct
fixture.

## Command
```
python3 -c "<temporarily swapped the two top-ranked ids in golden-ranking.json>"
cargo test --locked -- --ignored u067_golden_fixture_ranking_matches_reference
```

## Result
FAILED. Expected RED reason: the fixture's expected order is wrong (proto-d
swapped ahead of proto-a), so the assertion that the Rust ranking order matches
the fixture exactly fails.

Failure excerpt:
```
---- embedding::tests::u067_golden_fixture_ranking_matches_reference stdout ----
thread '...' panicked at src/embedding.rs:385:9:
assertion `left == right` failed: ranking order must match the golden fixture exactly
  left: ["proto-a", "proto-d", "proto-c", "proto-b"]
 right: ["proto-d", "proto-a", "proto-c", "proto-b"]
```

## Why this is the expected failure
The golden test's core assertion is an exact-order match against a trusted
reference fixture. A wrong expected order (or, equivalently, an embed/cosine_rank
output that diverges from the reference) must fail the order assertion rather
than pass vacuously. The failure is at the first (order) assertion, confirming
the test guards the ranking contract. The fixture was restored immediately after
capturing this evidence.

## Worktree / SHA
- SHA: `7736ea9` (uncommitted)
- No commits/pushes. Fixture restored to the correct reference order.
