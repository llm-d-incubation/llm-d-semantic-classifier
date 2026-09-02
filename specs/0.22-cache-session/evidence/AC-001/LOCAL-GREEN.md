# AC-001 LOCAL-GREEN — delta context abstention

Test: `U-048` (`u048_delta_context_abstains_without_forward_or_cache_access`).

Command:

```text
cargo test --locked u048_delta_context_abstains_without_forward_or_cache_access --lib
```

Result: passed. The core returns `ABSTAIN` with an empty ranking and leaves raw
forward count and exact-cache hit/miss/coalesced counters at zero.
