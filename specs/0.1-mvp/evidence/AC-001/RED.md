# AC-001 RED evidence

## Acceptance criterion
AC-001 clean Rust build/server lifecycle.

## Proving test(s)
- U-001 minimal valid configuration parses
- U-002 missing classifier config rejected
- U-003 unknown runtime backend rejected
- U-004 duplicate classifier ID rejected
- U-005 invalid model path rejected

## Command
```
cargo build
```

## Worktree state
- Branch: main
- HEAD: `6286ff70abecc707a9bdce23b8debe79c1afb20a`
- `git status`: clean (nothing to commit, working tree clean)
- Phase: scaffolding — no `Cargo.toml` anywhere in the workspace; `hack/build`
  is a Rust-aware no-op that prints `[build] no Cargo.toml yet`.

## Failure excerpt
```
$ cargo build
error: could not find `Cargo.toml` in `/Users/cnuland/llm-d-sc-genesis` or any parent directory
--- exit 101 ---
```

## Why this is the expected failure
AC-001 demands a clean Rust build and a server lifecycle proven by the
config-parsing unit tests U-001..U-005. At the current scaffolding phase no
Rust crate exists: there is no `Cargo.toml`, no source tree, and therefore no
unit-test targets for the configuration parser. Because the crate is absent,
`cargo build` cannot even locate a manifest and fails at exit 101, and none of
U-001..U-005 can be selected or run. This is precisely the expected RED: the
proving tests and the clean build are both impossible until a Rust crate is
introduced in the next (GREEN/implementation) step.

Note: `./hack/build` and `./hack/verify` exit 0 only because they detect the
scaffolding phase and skip Rust validation; they do NOT demonstrate a clean
Rust build, which is what AC-001 requires.
