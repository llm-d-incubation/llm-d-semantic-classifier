# AC-001 RED — delta context abstention

Test: `U-048` (`u048_delta_context_abstains_without_forward_or_cache_access`).

Command:

```text
cargo test --locked u048_delta_context_abstains_without_forward_or_cache_access --lib
```

Expected failure before the short circuit:

```text
assertion `left == right` failed
  left: Ok
 right: Abstain
```

The test compiled after the additive request/core type scaffolding and failed
because `ServiceCore` still invoked normal classification for `DELTA` input.
This is the intended RED condition.
