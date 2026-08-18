# How this project is developed

llm-d-sc is built with a **spec-driven, evidence-gated process**. This page
exists so contributors understand the artefacts they will encounter; you do not
need to adopt the method to contribute.

## Specification before implementation

Every substantial change owns a directory under `specs/`:

```
specs/<version>-<slug>/
  research.md     prior art, constraints, uncertainty destroyed
  spec.md         problem, behaviour, non-goals, acceptance criteria, rollback
  design.md       component boundaries and state ownership
  test-plan.md    every acceptance criterion mapped to test IDs
  acceptance.md   promotion checklist (including non-test gates)
  evidence/       per-criterion RED/GREEN records
```

Acceptance criteria are numbered (`AC-001`...) and each maps to test IDs from
`tests/TEST_MATRIX.md` (`U-` unit, `I-` integration, `S-` system, `P-`
performance, `R-` robustness). Test IDs are stable evidence anchors: they appear
in test function names, in evidence files, and in the ledger.

## Evidence, not assertion

`hack/spec-check <spec-id>` is the source of truth for status. It expands range
notation, cross-checks every required ID against the matrix, and derives status
from **execution results** (`hack/test-report`) rather than from the existence of
a test function.

Two tiers, deliberately separate:

- **LOCAL** — unit and local-integration evidence exists and was reviewed.
- **PROMOTION** — every required test ID for the version is green *and* the
  non-test gates in `acceptance.md` are met (CI, independent review, exact-SHA
  cluster validation, maintainer approval).

`spec-check --promotion` refuses to declare a version complete until both hold.
The practical effect is that "lots of passing unit tests" can never be mistaken
for "the system is proven".

## Working rules that show up in review

- A test must fail for the right reason before the implementation exists, and
  the RED evidence is recorded.
- Existing assertions are protected: changing or deleting one requires an
  explicit argument that the previous contract was wrong.
- No performance claim without comparable before/after p50/p95/p99 evidence and
  a recorded manifest (SHA, image digests, hardware profile, concurrency, cache
  mode, sequence length, sample counts). Average-only latency claims are
  rejected.
- Scope discipline: no opportunistic refactors bundled into functional changes.
- Architectural decisions that resolve a contradiction are recorded as ADRs in
  `docs/decisions/`.

## Why the specs are kept

The `specs/` tree is retained as the project's design record: what was intended,
what was proven, what was explicitly deferred, and why. It is more useful than a
changelog for understanding a pre-1.0 runtime, and it makes the pending work
auditable rather than folkloric.

Much of the implementation was produced by AI coding agents under this process,
with an independent model reviewing every change and a human maintainer as the
only merge authority. That is an implementation detail, not a standard: every
change upstream is held to the same evidence bar regardless of who or what
wrote it, and the submitting maintainer owns its correctness.
