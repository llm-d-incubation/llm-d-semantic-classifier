# 0.22 Cache/Session Optimization — Slice 1

## Problem

A follow-up request may be meaningful only with preceding conversation context.
The exact-result cache is disposable, so after a restart or cache loss the
classifier must not turn a context-free delta into a confident semantic label.

## Context

Long-running conversations benefit from continuity, but llm-d-sc remains a
semantic-evidence service. Session continuity, model selection, and routing
state remain the AI Gateway's responsibility.

## Existing behavior

The API accepts a context string and a session id, but cannot distinguish a
complete context from a delta-only follow-up. The `ABSTAIN` wire status exists,
but the serving path never emits it.

## Desired behavior

The request explicitly declares whether its supplied context is complete or a
delta. A delta-only request returns `ABSTAIN`, with no ranked labels, before
cache lookup or classifier work. A request whose completeness is absent or full
keeps the current classification behavior for wire compatibility.

## Non-goals

- Model/endpoint selection, stickiness, tool-loop locking, or routing policy.
- Durable session storage or cross-replica session coordination.
- An optional session feature cache; that is a later 0.22 slice.
- Inferring completeness from session id, prompt text, or cache contents.

## Compatibility

This is an additive protobuf change. `CONTEXT_COMPLETENESS_UNSPECIFIED` retains
the current behavior so existing clients classify as before. Gateways that send
delta-only context must set `DELTA` and handle `ABSTAIN`.

## State ownership

The gateway authoritatively owns session history, construction of complete
context, and the routing decision. llm-d-sc owns only resident runtime state and
disposable caches. No cache entry may become a routing or session-state source
of truth.

## Failure behavior

- `DELTA` context -> `ABSTAIN`, empty ranking, no forward and no cache access.
- complete/unspecified context -> existing classify/error behavior.
- cache loss + complete context -> recompute; cache loss + delta context ->
  abstain.

## Security/privacy

The service continues to hash session identifiers in telemetry and does not
persist raw conversation context beyond the existing bounded, disposable cache.

## Performance and measurement

Delta abstention is a constant-time short circuit. Tests prove it performs zero
model forwards and does not alter exact-cache counters.

## Rollback

The additive request field can be left unset by clients. Reverting the serving
logic restores the previous behavior without invalidating existing requests.

## Acceptance criteria

- [ ] AC-001 Explicit `DELTA` context returns `ABSTAIN` with no ranked labels,
      model forward, or exact-cache interaction (U-048).
- [ ] AC-002 A fresh server receiving a delta-only follow-up abstains, while a
      complete context on the same fresh server classifies normally (I-046).

## Negative cases

- N-1 A response contains no model, route, endpoint, or target field (U-010).
- N-2 Unspecified and full context retain the existing successful behavior.
- N-3 A session id alone never changes classification behavior.

## Open questions

The representation and invalidation rules for a future optional session feature
cache are intentionally deferred until this abstention boundary is proven.
