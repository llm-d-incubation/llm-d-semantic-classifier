# Contributing to llm-d-sc

Thanks for your interest. llm-d-sc is pre-1.0 and moving quickly; small,
well-evidenced changes are much easier to land than large ones.

## Developer Certificate of Origin

All commits must carry a `Signed-off-by` line certifying the
[DCO](https://developercertificate.org/):

```bash
git commit -s -m "your message"
```

Commits without a signoff cannot be merged.

## Development loop

Requires a Rust toolchain and `protoc`.

```bash
./hack/verify       # format, lint (-D warnings), build, unit + local tests
./hack/test-parity  # fetches the pinned artifact, runs model-dependent tests
./hack/test-report  # writes per-test-ID execution results
./hack/spec-check 0.1-mvp   # the evidence ledger
```

`./hack/verify` must be green before a pull request is ready. `test-parity`
downloads a pinned model artifact, so it is not run on every push.

## What a good change looks like

1. **One coherent claim.** A change should assert one thing about the software
   that can be tested and reviewed on its own. Do not bundle a feature with an
   unrelated refactor.
2. **A test that failed first.** Write the test, watch it fail for the reason you
   expect, then implement. For bugs, the regression test comes before the fix.
3. **Evidence for behaviour claims.** Performance changes need comparable
   before/after p50/p95/p99 numbers with the measurement conditions recorded -
   hardware, concurrency, cache mode, input length, sample counts. Average-only
   latency claims are not accepted.
4. **No weakened assertions.** Changing or deleting an existing test assertion is
   a privileged change: explain in the pull request why the previous contract was
   wrong. "The new code produces a different value" is not a reason.
5. **Real boundaries over mocks.** Prefer real protobuf, real gRPC, real
   tokenizer and model fixtures in integration tests.

## Architectural rules that reviews enforce

These come from the project's design and are not negotiable in a patch:

- **llm-d-sc classifies; it does not route.** No routing policy, endpoint
  selection, session authority, or guardrail enforcement. The response type has
  no route field and must not gain one.
- **The async networking runtime is not the model scheduler.** Model forwards run
  on dedicated inference workers, never on a network worker.
- **Inference admission stays bounded.** Overload is rejected explicitly rather
  than queued without limit.
- **Cache identity is versioned.** A classifier, model, tokenizer, or taxonomy
  revision change must invalidate cached results.
- **No raw prompt or session text in logs, metrics, or traces.**
- **Never fabricate a label.** Insufficient context abstains; failure is an
  explicit error status.

## Specifications

Substantial changes are specification-first. Look under `specs/` for the format:
problem, observable behaviour, non-goals, acceptance criteria mapped to test IDs,
failure contract, and rollback. Test IDs (`U-`, `I-`, `S-`, `P-`, `R-`) come from
`tests/TEST_MATRIX.md` and are stable evidence anchors, reuse them rather than
inventing new numbering. See
[`docs/research/development-method.md`](docs/research/development-method.md).

## AI-assisted contributions

Portions of this project were implemented with AI coding agents under an
evidence-gated process. Contributions written with AI assistance are welcome and
are held to exactly the same standard as any other: the human submitting the
change owns its correctness, licensing, security implications, and design, and
must be able to explain it in review. AI tooling may not supply your DCO
signoff.

## Getting help

Ask in [`#sig-semantic-classifier`](https://llm-d.ai/slack) in the llm-d Slack,
or open an issue describing what you are trying to do. For security reports,
follow [SECURITY.md](SECURITY.md) instead.
