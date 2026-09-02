# 0.22 Cache/Session Optimization — Slice 1 Test Plan

## Strategy

| Criterion | Test | Kind | Fails-for-the-right-reason proof |
|---|---|---|---|
| AC-001 | U-048 | unit | A DELTA input is currently classified with labels and increments the cache-miss/forward path. |
| AC-002 | I-046 | integration | A fresh gRPC server currently returns `OK` for a delta-only follow-up. |

U-048 proves the core short circuit: `ABSTAIN`, empty ranking, zero raw forwards,
and unchanged cache counters. I-046 proves the wire mapping and restart/cache-loss
boundary. The integration control sends complete context to prove normal
classification remains available.

## Impact analysis

Expected change surface: protobuf, classification core, gRPC adapter, unit tests,
and gRPC integration tests. Run `./hack/test-impact` after the files are known,
then run its Required suite and the focused test commands.

## Mocks

U-048 uses a counting `ClassifierRuntime` test double to prove the core never
invokes a raw forward. I-046 uses a real tonic server/client and synthetic
classifier fixture.
