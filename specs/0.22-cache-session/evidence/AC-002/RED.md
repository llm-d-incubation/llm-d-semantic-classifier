# AC-002 RED — restart plus delta context

Test: `I-046` (`i046_restart_delta_context_abstains`).

Command:

```text
cargo test --locked --test restart i046_restart_delta_context_abstains
```

Expected failure before the short circuit:

```text
assertion `left == right` failed: a delta-only request after cache loss must abstain
  left: 1
 right: 2
```

The fresh server returned protobuf status `OK` (1), rather than `ABSTAIN` (2),
for a delta-only request. This is the intended RED condition.
