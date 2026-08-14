# Test-Driven Development (TDD)

TDD is an evidence protocol, not a slogan.

```text
ACCEPTANCE CRITERION
 -> SELECT/WRITE PROVING TEST
 -> RED (expected failure proven)
 -> MINIMAL IMPLEMENTATION
 -> GREEN (focused)
 -> IMPACTED TESTS
 -> REQUIRED SUITE
 -> INDEPENDENT REVIEW
```

## RED evidence

Before implementation, record:
- test ID;
- command;
- base SHA/worktree state;
- failure excerpt;
- why this is the expected failure.

A test failing because the fixture is broken, a port is occupied, or the test itself is wrong does not count.

## GREEN evidence

Record the same test ID, command, result, and new SHA/worktree state. For performance-sensitive tests also record duration/distribution metadata.

## Feature TDD

Prefer contract/integration-first vertical slices:
1. executable API/behavior test;
2. expected failure;
3. smallest end-to-end implementation;
4. focused unit tests for internal invariants;
5. broader compatibility/negative cases.

Avoid building the entire service from mocks and discovering the real gRPC/model boundary later.

## Bug TDD

Reproduce -> failing regression -> prove expected failure -> minimal fix -> regression green -> impacted suite -> required suite.

## Existing tests are protected

Changing/deleting an existing assertion requires explicit evidence that the old contract was wrong. A worker may not update golden output simply because its code produced a new result.

## Performance TDD

```text
BASELINE
 -> single hypothesis
 -> one optimization
 -> same benchmark
 -> compare distributions
```

Required evidence:
- before/after SHA;
- hardware profile;
- service/model image digests;
- runtime backend;
- sequence length;
- concurrency;
- cache mode;
- warmup/sample count;
- p50/p95/p99/max;
- queue/tokenize/forward/end-to-end decomposition.

No performance PR may simultaneously change implementation, workload, and benchmark methodology without making the comparison explicit.

## Integration TDD: dummy Praxis first

The dummy service must accept a synthetic inference request, preserve session metadata, call llm-d-sc over the intended gRPC contract, consume semantic signals, apply a trivial test-only route outside llm-d-sc, and record callout RTT.

This ensures routing logic does not migrate into the classifier.

## Test impact

After focused GREEN, a deterministic helper selects impacted tests. The worker does not decide that a suite is probably unrelated.

## Flakes

A flaky functional test is a defect. Retries do not substitute for root cause. Performance experiments may run repeated independent trials by design.

## Mocks

Use mocks for internal trait invariants where valuable. Integration/system tests should prefer real protobuf, gRPC, scheduler, cache, tokenizer/model fixture, container images, and OpenShift networking.
