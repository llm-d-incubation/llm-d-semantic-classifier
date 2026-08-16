# AC-004 GREEN evidence — U-061

## Proving test
- U-061 pooling output matches trusted reference tolerance (#[ignore], hack/test-parity)

## Command
```
cargo test --locked -- --ignored u061
```

## Result
ok. 1 passed (dim0: rust 0.030966658 vs reference 0.03096645; first16 within 1e-4; L2 norm within 1e-3)

## Reviewer addendum (2026-08-15)
Initial reference fixture was INVALID: generated with tokenizer.json absent from the model
dir (fetch-model omission) -> AutoTokenizer degenerated to all-UNK ids [2,1,...,3]. U-061
correctly FAILED against it (dim0 got 0.030966658 vs garbage 0.0993). Reference regenerated
with the pinned tokenizer (ids [101,2023,...]); Rust output matches to ~1e-7 (dim0
0.03096645 ref vs 0.030966658 rust). Fixture provenance records the prior bug. fetch-model
patched to fetch tokenizer.json per the ModelCar manifest.
