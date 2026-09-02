# AC-002 LOCAL-GREEN — restart plus delta context

Commands:

```text
cargo test --locked --test restart i046_restart_delta_context_abstains
cargo test --locked --test restart i045_restart_full_context_recomputes_correctly
cargo test --locked
```

Results: `I-046` and the existing full-context restart control (`I-045`) passed.
The required full suite passed; tests requiring local model artifacts or explicit
performance runs remained intentionally ignored.
