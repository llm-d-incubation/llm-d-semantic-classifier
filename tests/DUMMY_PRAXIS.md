# Dummy Praxis Integration Contract

The dummy service is deliberately simple; it tests the integration seam without reimplementing Praxis.

## Responsibilities
- receive a synthetic incoming request;
- reuse a persistent gRPC channel to llm-d-sc;
- propagate request ID, session ID, context, requested signals, and deadline;
- measure monotonic start/end time around the classifier RPC;
- consume semantic signals/status;
- apply a fixed test-only mapping after classification;
- emit benchmark data including errors/drops.

## Forbidden behavior
- no semantic classifier of its own;
- no hidden retry loop;
- no shared in-process cache with llm-d-sc;
- no final route sent to llm-d-sc;
- no exclusion of failed/slow requests from latency statistics.

## Example

```text
fixture request
 -> dummy Praxis
 -> llm-d-sc classify
 <- ranked sensitivity signal
 -> dummy test policy
    NEVER_EGRESS -> local-model
    otherwise    -> general-model
 -> record route + classifier RTT
```

The mapping exists only to prove responsibility separation.

## Modes
- no-op/transport-floor (test build only, if implemented);
- classify/cache-hit;
- classify/cache-miss;
- classify/mixed;
- same-key burst;
- unique-key burst;
- deadline expiry;
- overload.
