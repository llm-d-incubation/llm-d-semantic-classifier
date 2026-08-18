# Specification-Driven Development (SDD)

## State machine

```text
IDEA
 -> RESEARCHED      research artifact exists
 -> SPECIFIED       observable behavior/non-goals/failures defined
 -> TEST-DESIGNED   every acceptance criterion mapped to test IDs
 -> IMPLEMENTING    exactly one acceptance criterion active
 -> LOCAL-GREEN     focused + impacted + required suites green
 -> INDEPENDENT-REVIEWED
 -> PUSHED
 -> GITHUB-CI-GREEN
 -> PROD-LIKE-VALIDATED
 -> MAINTAINER-APPROVED
 -> MERGED
 -> OBSERVED / LEARNED
```

The state machine is deterministic. An LLM never decides to skip a state.

## Required change artifact

Each substantial change/version owns:

```text
specs/<version>-<slug>/
  research.md
  spec.md
  design.md
  test-plan.md
  acceptance.md
  evidence/
```

### research.md

Must establish repository/current-runtime facts, verified assumptions, upstream constraints, prior art reused, and unresolved uncertainty. Research informs design; it does not make one implementation library the public architecture.

### spec.md

Mandatory fields:
- problem;
- observable desired behavior;
- in-scope;
- non-goals;
- public API/contract;
- state ownership;
- failure behavior;
- compatibility;
- security/privacy implications;
- performance objective and measurement method;
- acceptance criteria;
- rollback/disable path;
- open questions.

Acceptance criteria must be machine-verifiable where practical.

Bad: `Make classification fast.`

Good: `For a resident model/cache miss, queue, tokenization, model-forward and total service latency are independently measured; expired queued requests are discarded before inference.`

### design.md

For llm-d-sc, identify:
- protocol/API layer;
- classifier registry;
- exact-result/session-feature/model-residency state;
- bounded scheduler;
- runtime backend abstraction;
- model/tokenizer lifecycle;
- health/readiness;
- observability;
- failure boundaries;
- authoritative vs disposable state.

### test-plan.md

No implementation begins until every acceptance criterion maps to test IDs and environment.

Example:

| Acceptance criterion | Tests | Environment |
|---|---|---|
| readiness follows load/warmup | U-020, I-010, S-006 | unit/integration/Kubernetes |
| overload rejects instead of unbounded queue | U-031, I-035, P-023 | unit/integration/homelab |
| cache loss cannot silently downgrade | U-048, I-046, S-022 | unit/dummy gateway/Kubernetes |

### acceptance.md

Promotion checklist links each acceptance criterion to stored evidence. A prose assertion is not evidence.

## DeepSeek work slicing

Never assign `implement Phase 0.1` as one task. Give the worker one independently testable claim, e.g.:

```text
AC-001 protocol compiles
AC-002 server is not ready without a model
AC-003 ModelSpec validates
AC-004 Candle backend loads fixture
AC-005 warmup gates readiness
AC-006 classify returns ranked signal
AC-007 cache hit bypasses inference
AC-008 bounded queue rejects overload
AC-009 dummy gateway consumes response
AC-010 ModelCar is self-contained
AC-011 Kubernetes sidecar RTT evidence exists
```

## Spec drift

If implementation proves the written contract wrong:
1. stop;
2. record contradiction;
3. return to SPECIFIED;
4. update spec/test plan;
5. review the change;
6. resume.

The worker does not silently reinterpret intent.
