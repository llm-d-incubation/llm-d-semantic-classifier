# AC-004 GREEN evidence — whole criterion

## Acceptance criterion
AC-004 pinned sensitivity model matches trusted reference embedding/ranking
fixtures.

## Status: GREEN for the unit slice (U-060..U-067)
All eight unit/component slices mapped to AC-004 by `specs/0.1-mvp/test-plan.md`
(U-060..U-067) are GREEN. Integration slices I-020..I-025 remain for the
integration environment (see below).

## Slices

| ID | Test | Where proven | Result |
|----|------|--------------|--------|
| U-060 | tokenizer golden token IDs match trusted reference | `cargo test` (plain) | GREEN |
| U-061 | pooling output matches trusted reference tolerance | `./hack/test-parity` (`#[ignore]`) | GREEN |
| U-062 | embedding dimension matches model contract | `./hack/test-parity` (`#[ignore]`) | GREEN |
| U-063 | embedding normalization matches classifier definition | `./hack/test-parity` (`#[ignore]`) | GREEN |
| U-064 | prototype/anchor ranking deterministic | `cargo test` (plain, pure math) | GREEN |
| U-065 | deterministic tie rule for top-k | `cargo test` (plain, pure math) | GREEN |
| U-066 | max-length truncation deterministic | `cargo test` (plain) | GREEN |
| U-067 | golden fixture output matches reference | `./hack/test-parity` (`#[ignore]`) | GREEN |

## Model-dependent slices (U-061/U-062/U-063/U-067)
These require the resident sensitivity model weights (gitignored) and are
`#[ignore]`d by default; they are proven via the parity suite:

```
./hack/test-parity
```
(which runs `./hack/fetch-model` then `cargo test --locked -- --ignored`).
Result: `4 passed; 0 failed` — U-061, U-062, U-063, U-067 all green. U-060,
U-064, U-065, U-066 are plain tests proven by `cargo test` (12 passed).

## Golden fixtures committed
- `tests/fixtures/modelcar/golden-token-ids.json` (U-060)
- `tests/fixtures/modelcar/golden-embedding.json` (U-061 reference embedding,
  full module stack Transformer + Pooling + 2_Normalize at pinned rev 43f21d2)
- `tests/fixtures/modelcar/synthetic-prototypes.json` (U-064/U-065/U-067,
  label `synthetic_for_mechanics_only` — NOT a real taxonomy)
- `tests/fixtures/modelcar/golden-ranking.json` (U-067 golden pipeline fixture,
  provenance embedded; SYNTHETIC prototypes, not a real taxonomy)

## Remaining for the integration environment (I-020..I-025)
Per `specs/0.1-mvp/test-plan.md`, AC-004 additionally requires these integration
tests, which remain for the integration environment:
- I-020 real sensitivity artifact loads
- I-021 public-like golden fixture
- I-022 regulated-like golden fixture
- I-023 never-egress-like golden fixture
- I-024 adversarial/borderline fixture expected ordering
- I-025 Rust embedding agrees with pinned Python reference tolerance

These are NOT proven by this unit slice and are intentionally out of scope here.

## Commands (all GREEN)
```
./hack/test-all            # 12 passed; 4 ignored (unit + ranker + tokenizer)
./hack/test-parity         # 4 passed (ignored model-dependent: U-061/062/063/067)
./hack/verify              # exit 0 (fmt, clippy -D warnings, build --locked, cargo test)
```

## Worktree / SHA
- SHA: `7736ea9` (uncommitted changes)
- No commits/pushes.
