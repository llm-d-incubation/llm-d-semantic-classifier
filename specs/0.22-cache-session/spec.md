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

# 0.22 Cache/Session Optimization — Slice 2: Configurable Eviction

This specification covers the configurable FIFO/LRU exact-result-cache slice of
phase 0.22. Optional feature/session caching, cache-loss recovery, and
insufficient-context abstention remain separate 0.22 slices.

| Field | Answer |
|---|---|
| Problem | The bounded exact-result cache always uses FIFO eviction. Operators cannot opt into LRU for workloads with a repeatedly accessed hot set, even when they are willing to pay the recency-bookkeeping cost. |
| Upstream context | GitHub issue #9 records the FIFO/LRU tradeoff. `docs/VERSIONS.md` and `tests/TEST_MATRIX.md` assign cache eviction to phase 0.22. Making LRU opt-in preserves the measured and proven FIFO path while allowing deployments to evaluate LRU against their own traffic. |
| Existing behavior | `ExactCache` holds at most 50,000 entries by default in a `HashMap`, tracks insertion order in a `VecDeque`, and evicts the oldest inserted key. `SharedCache::new()` and `ServiceCore` always construct this FIFO cache. The production binary uses environment variables, while the TOML `Config` type is not currently wired into production startup. |
| Desired behavior | Introduce a typed cache eviction policy with `fifo` and `lru` values. Omitted configuration selects FIFO. Explicit LRU configuration updates recency on successful hits and evicts the least recently used key at capacity. Thread the selected policy through `SharedCache`, `ServiceCore`, and the production server without moving or duplicating the existing single-flight behavior. |
| Non-goals | Replacing FIFO as the default; choosing a policy automatically; changing the 50,000-entry default capacity; distributed or persistent caching; TTL expiration; semantic/approximate matching; runtime policy changes; changing cache keys, values, hit/miss semantics, or single-flight ownership; implementing the remaining phase 0.22 session/recovery/abstention work. |
| Compatibility | Existing constructors and configurations continue to select FIFO. Existing TOML remains valid. Add optional `[cache] eviction = "fifo" | "lru"`; add `LLM_D_SC_CACHE_EVICTION=fifo|lru` for the environment-configured production binary. Invalid explicit values fail parsing/startup instead of silently falling back. |
| Security impact | The new input is a closed enum and adds no network or data trust boundary. Cache keys remain versioned BLAKE3 fingerprints, raw prompts remain absent from cache identity telemetry, and capacity remains bounded. |
| Rollback | Unset the option or set it to `fifo` to recover existing behavior. The implementation can be reverted without migrating state because the cache is in-memory and disposable. |

## Design constraints

- Define one canonical `CacheEvictionPolicy` type, serialized as lowercase `fifo`
  and `lru`, with `Default` implemented as `Fifo`.
- Add a defaulted `ExactCacheConfig` to the top-level TOML configuration. The absent
  `[cache]` section and absent `eviction` key must both resolve to FIFO.
- Map the production environment setting to the same policy type; do not create
  separate parsing or policy semantics for TOML and environment configuration.
- Preserve the existing zero-argument constructors as FIFO-compatible wrappers.
  Add explicit-policy construction through `ExactCache`, `SharedCache`,
  `ServiceCore`, and the gRPC server construction path.
- Keep single-flight coordination in `SharedCache`. An eviction-policy library,
  if used, owns stored-entry eviction only and must not run or coalesce forwards.
- Do not implement LRU with an O(capacity) scan on every hit or with an
  ever-growing lazy recency queue. Use a bounded, established cache/container
  implementation or another design with bounded recency metadata and
  constant-time/amortized recency updates.
- A hit under LRU refreshes recency only after the key is found. Failed forwards
  remain uncached. Re-storing an existing key must not consume a second slot.

## Acceptance criteria (machine-verifiable; one worker turn each)

- [x] AC-1 (U-007) Default compatibility and validation: an omitted `[cache]`
      section, an omitted `cache.eviction` key, an absent production environment
      setting, and all existing zero-argument constructors select FIFO. TOML and
      production environment configuration accept exactly `fifo` and `lru`;
      invalid explicit values fail clearly and never silently select FIFO.
- [x] AC-2 (U-046) FIFO behavior: with capacity two, insert `A`, insert `B`, hit
      `A`, then insert `C`; `A` is evicted and `B` remains, matching current
      behavior.
- [x] AC-3 (U-046) LRU behavior: with capacity two, insert `A`, insert `B`, hit
      `A`, then insert `C`; `B` is evicted and `A` remains.
- [x] AC-4 (I-033) Runtime wiring: a server/core constructed with explicit LRU
      uses LRU for served requests; the default server/core uses FIFO. The test
      must exercise the construction path rather than only the standalone
      container.
- [x] AC-5 (U-046) Invariants under both policies: capacity is never exceeded; a
      hit bypasses tokenizer/model forward; version changes cannot return stale
      results; failures are not cached; and identical concurrent misses still
      coalesce into one forward with the existing hit/miss/coalesced metrics.
- [x] AC-6 Dependency and performance hygiene: any new cache dependency is
      locked and documented; LRU recency metadata remains bounded by cache
      capacity; and the existing FIFO cache-hit benchmark remains runnable so
      policy costs can be compared without changing FIFO's default contract.

## Negative cases (must continue to fail / remain unchanged)

- N-1 Unknown eviction-policy values must not be accepted or silently downgraded.
- N-2 Neither policy may allow the stored entries or recency metadata to grow
  without the configured capacity bound.
- N-3 Cache eviction must not cancel, duplicate, or assume ownership of in-flight
  classification work.
- N-4 A classifier, model, tokenizer, taxonomy, or preprocessing revision change
  must not serve a cached result produced under the prior revision.
- N-5 Opting into LRU must not change response schemas, ranking, abstention, or
  error behavior.

## Open questions

None blocking. The implementation uses the synchronous `lru` container for the
opt-in LRU storage and retains the existing map/queue for default FIFO storage.
If the existing TOML `Config` is later wired into production startup, the
environment variable can become a compatibility mapping into the exact-cache
configuration.
