# llm-d-sc Test Matrix

Test IDs are evidence anchors. Each version's `test-plan.md` selects the required subset.

## Unit/component

| ID | Test | Phase |
|---|---|---:|
| U-001 | minimal valid configuration parses | 0.1 |
| U-002 | missing classifier config rejected | 0.1 |
| U-003 | unknown runtime backend rejected | 0.1 |
| U-004 | duplicate classifier ID rejected | 0.1 |
| U-005 | invalid model path rejected | 0.1 |
| U-006 | immutable revision required in production profile | 0.20 |
| U-010 | classification response schema contains no final route/endpoint | 0.1 |
| U-011 | unknown signal explicit error | 0.1 |
| U-012 | empty input follows contract | 0.1 |
| U-013 | oversize request rejected before model work | 0.20 |
| U-014 | Unicode input survives normalization/tokenization | 0.1 |
| U-015 | normalization idempotent | 0.20 |
| U-020 | readiness false before successful warmup | 0.1 |
| U-021 | model/tokenizer load once per active revision | 0.1 |
| U-022 | warmup failure keeps not-ready | 0.1 |
| U-023 | corrupt model/config produces actionable error, no panic | 0.1 |
| U-024 | manifest/model revision mismatch rejected | 0.20 |
| U-025 | unsupported tokenizer config rejected safely | 0.20 |
| U-030 | inference queue capacity is bounded | 0.1 |
| U-031 | full queue returns overload/resource exhausted | 0.1 |
| U-032 | expired queued job never executes forward | 0.20 |
| U-033 | cancelled queued job never executes forward | 0.20 |
| U-034 | worker failure resolves waiter with explicit error | 0.20 |
| U-035 | shutdown stops admission and drains configured in-flight work | 0.20 |
| U-036 | one runtime error cannot poison unrelated requests | 0.20 |
| U-040 | exact cache hit bypasses tokenizer and runtime | 0.1 |
| U-041 | identical concurrent misses coalesce/bound duplicate forwards | 0.1 |
| U-042 | cache key changes with model/classifier revision | 0.1 |
| U-043 | cache key changes with tokenizer revision | 0.1 |
| U-044 | cache key changes with taxonomy/prototype revision | 0.1 |
| U-045 | cache key changes with preprocessing contract | 0.20 |
| U-046 | cache capacity/eviction works under concurrency | 0.22 |
| U-047 | stale revision never returned after activation change | 0.22 |
| U-048 | insufficient context yields abstain, not benign label | 0.22 |
| U-049 | full-context recomputation after cache loss matches prior result tolerance | 0.22 |
| U-050 | optional session feature cache cannot become routing-state authority | 0.22 |
| U-060 | tokenizer golden token IDs match trusted reference | 0.1 |
| U-061 | pooling output matches trusted reference tolerance | 0.1 |
| U-062 | embedding dimension matches model contract | 0.1 |
| U-063 | embedding normalization matches classifier definition | 0.1 |
| U-064 | prototype/anchor ranking deterministic | 0.1 |
| U-065 | deterministic tie rule for top-k | 0.1 |
| U-066 | max-length truncation deterministic | 0.1 |
| U-067 | golden fixture output matches reference | 0.1 |
| U-068 | NaN/Inf output becomes error, never classification | 0.20 |
| U-070 | partial signal status independent | 0.23 |
| U-071 | failed sensitivity never serialized as `low` | 0.23 |
| U-072 | per-classifier deadline/config honored | 0.23 |
| U-073 | multi-signal result ordering deterministic | 0.23 |
| U-080 | queue/tokenize/forward/total metrics emitted | 0.1 |
| U-081 | cache hit/miss counters correct | 0.1 |
| U-082 | metric labels bounded | 0.20 |
| U-083 | session_id is never metric label | 0.20 |
| U-084 | request_id cannot create metric cardinality | 0.20 |
| U-085 | raw prompt absent from default logs/metrics | 0.1 |
| U-086 | per-stage percentiles separate workloads with identical means | 0.1 |
| U-087 | reported quantiles within documented bucket error | 0.1 |
| U-088 | histogram bucket index and lower bound are consistent | 0.1 |
| U-089 | empty latency stage reports zero, not a misleading value | 0.1 |
| U-090 | active model handle immutable | 0.24 |
| U-091 | candidate loads/warms before activation | 0.24 |
| U-092 | old in-flight request completes on old handle during swap | 0.24 |
| U-093 | failed candidate leaves previous active revision untouched | 0.24 |
| U-100 | every built-in classifier definition parses and validates | 0.1 |
| U-101 | the default classifier is a built-in | 0.1 |
| U-102 | a label with no anchors is rejected at load | 0.1 |
| U-103 | unknown classifier name rejected, listing available names | 0.1 |

## Protocol/integration

| ID | Test | Phase |
|---|---|---:|
| I-001 | real tonic client/server round trip | 0.1 |
| I-002 | persistent HTTP/2 channel reused | 0.1 |
| I-003 | request deadline propagates | 0.20 |
| I-004 | gRPC status taxonomy matches contract | 0.20 |
| I-005 | dummy gateway preserves session metadata | 0.1 |
| I-006 | dummy gateway consumes signal then routes outside llm-d-sc | 0.1 |
| I-007 | llm-d-sc response cannot dictate endpoint | 0.1 |
| I-008 | multi-turn requests do not reconnect per call | 0.1 |
| I-010 | server not ready before artifact/warmup | 0.1 |
| I-011 | readiness true after warmup | 0.1 |
| I-012 | repeated calls do not reload model/tokenizer | 0.1 |
| I-013 | SIGTERM flips readiness before exit/drain | 0.20 |
| I-020 | real sensitivity artifact loads | 0.1 |
| I-021 | public-like golden fixture | 0.1 |
| I-022 | regulated-like golden fixture | 0.1 |
| I-023 | never-egress-like golden fixture | 0.1 |
| I-024 | adversarial/borderline fixture expected ordering | 0.1 |
| I-025 | Rust embedding agrees with pinned Python reference tolerance | 0.1 |
| I-030 | warmed result cache hit invokes zero model forwards | 0.1 |
| I-031 | 100 same-key simultaneous misses have bounded forward count | 0.1 |
| I-032 | model/classifier revision change invalidates cached result | 0.22 |
| I-033 | eviction + full context recomputes equivalent result | 0.22 |
| I-035 | saturation rejects rather than runaway queueing | 0.1 |
| I-036 | queued deadline expiry discards stale job | 0.20 |
| I-037 | caller cancellation does not leak waiter/job | 0.20 |
| I-040 | two replicas/independent caches return equivalent full-context result | 0.22 |
| I-045 | restart + full context recomputes correctly | 0.1 |
| I-046 | restart + weak delta/insufficient context abstains | 0.22 |
| I-047 | dummy gateway retains conservative fixture route after abstention | 0.22 |
| I-050 | one signal succeeds while another fails | 0.23 |
| I-051 | multi-signal deadline behavior deterministic | 0.23 |
| I-052 | one classifier queue cannot starve another | 0.23 |
| I-060 | ModelCar contains required files under `/models` | 0.1 |
| I-061 | artifact readable by arbitrary non-root UID | 0.1 |
| I-062 | artifact/model digest recorded | 0.1 |
| I-063 | service starts from OCI artifact with HF egress disabled | 0.1 |
| I-064 | incomplete/corrupt ModelCar fails readiness | 0.1 |
| I-065 | mutable `latest` rejected in production test profile | 0.20 |
| I-070 | active revision switch sends new calls to new handle | 0.24 |
| I-071 | old in-flight call drains on old revision | 0.24 |
| I-072 | served response carries real taxonomy labels and revisions | 0.1 |
| I-073 | served ranking is semantically correct, not merely populated | 0.1 |
| I-074 | custom classifier definition supplied by path is served | 0.1 |
| I-090 | executor workers run forwards in parallel, not merely admit them | 0.1 |
| I-091 | single-worker executor is observably serial (control for I-090) | 0.1 |
| I-092 | accept counter observes reconnection (control for I-002) | 0.1 |
| I-080 | latency decomposition metrics visible | 0.1 |
| I-081 | overload counter increments | 0.20 |
| I-085 | trace capture has IDs/hashes but no raw prompt | 0.1 |

## Kubernetes system

| ID | Test | Phase |
|---|---|---:|
| S-001 | dummy gateway + llm-d-sc same Pod sidecar E2E | 0.1 |
| S-002 | separate Pods via ClusterIP E2E | 0.1 |
| S-003 | same-node service-to-service where schedulable | 0.21 |
| S-004 | cross-node service-to-service where possible | 0.21 |
| S-006 | readiness blocks traffic until warm model | 0.1 |
| S-010 | arbitrary non-root UID under a restricted security context | 0.1 |
| S-011 | model data read-only to runtime | 0.20 |
| S-012 | NetworkPolicy restricts expected traffic | 0.30 |
| S-020 | kill/restart llm-d-sc then full-context recompute | 0.1 |
| S-021 | warm cache lost on replacement, service recovers | 0.22 |
| S-022 | active session + cache loss + weak delta -> abstain | 0.22 |
| S-023 | dummy gateway preserves conservative route until context recovers | 0.22 |
| S-024 | crash loop never returns fabricated success | 0.30 |
| S-030 | scale 1 -> 3 replicas | 0.30 |
| S-031 | independent caches remain correctness-equivalent | 0.30 |
| S-032 | rolling service update | 0.30 |
| S-033 | rolling classifier revision reports new digest | 0.30 |
| S-034 | bad candidate rollout leaves known-good path available | 0.30 |
| S-040 | CPU limit causes bounded overload, not unbounded queue | 0.20 |
| S-041 | memory pressure failure/recovery observable | 0.30 |
| S-042 | graceful termination under active calls | 0.20 |
| S-050 | private registry pull secret | 0.30 |
| S-051 | ModelCar deployed/pulled by digest | 0.1 |
| S-052 | cached node image restart behavior | 0.30 |
| S-053 | egress to HF denied and service still starts | 0.1 |
| S-054 | artifact provenance metadata captured | 0.30 |
| S-060 | Prometheus scrapes metrics | 0.30 |
| S-061 | no high-cardinality user/session labels | 0.30 |
| S-080 | system evidence distinguishes RTT/queue/forward | 0.1 |

## Performance

| ID | Scenario | Phase |
|---|---|---:|
| P-001 | in-process exact-result cache hit | 0.1 |
| P-002 | gRPC localhost cache hit | 0.1 |
| P-003 | gRPC localhost cache miss | 0.1 |
| P-004 | same-key burst miss coalescing | 0.1 |
| P-005 | 0% hit workload | 0.21 |
| P-006 | 50% hit workload | 0.21 |
| P-007 | 90% hit workload | 0.21 |
| P-008 | 100% hit workload | 0.21 |
| P-010 | 32 tokens | 0.21 |
| P-011 | 64 tokens | 0.21 |
| P-012 | 128 tokens | 0.21 |
| P-013 | 256 tokens | 0.21 |
| P-014 | over-limit/truncation behavior | 0.21 |
| P-020 | concurrency 1 | 0.1 |
| P-021 | concurrency 4 | 0.1 |
| P-022 | concurrency 16 | 0.21 |
| P-023 | concurrency 32/saturation | 0.21 |
| P-030 | dummy gateway -> same-Pod cache-hit RTT | 0.1 |
| P-031 | dummy gateway -> same-Pod cache-miss RTT | 0.1 |
| P-032 | dummy gateway -> ClusterIP cache-hit RTT | 0.1 |
| P-033 | dummy gateway -> ClusterIP cache-miss RTT | 0.1 |
| P-034 | no-op transport floor sidecar | 0.21 |
| P-035 | no-op transport floor ClusterIP | 0.21 |
| P-036 | same-node vs cross-node delta | 0.21 |
| P-037 | container start -> readiness | 0.21 |
| P-038 | cold image pull vs node-cached image startup | 0.30 |
| P-040 | CPU worker/math-thread matrix | 0.21 |
| P-041 | tokenizer parallelism matrix | 0.21 |
| P-042 | GPU batch=1 if available | 0.21 |
| P-043 | micro-batch windows only if justified | 0.32 |
| P-050 | optimization before/after p99 | 0.32 |

## Robustness/property/fuzz

- R-001 fuzz HTTP/JSON parser if HTTP classification exists.
- R-002 protobuf boundary/unknown fields.
- R-003 Unicode normalization properties.
- R-004 randomized cancellation/timeout scheduling races.
- R-005 concurrent cache eviction/read.
- R-006 concurrent active-model swap/inference.
- R-007 malformed classifier manifest.
- R-008 artifact symlink/path-escape containment.
- R-009 oversized payload rejected before large allocation.
- R-010 compression/request-size abuse protections.
- R-011 API backward-compatible unknown-field handling.
- R-012 internal tensor/runtime error never leaks raw memory/details.
- R-013 randomized load never exceeds configured queue/in-flight bound.
- R-014 missing model files never report ready.
- R-015 repeated start/shutdown sequence does not deadlock.

## Coverage philosophy

Coverage percentage is diagnostic, not proof. Required instead:
- every acceptance criterion has test IDs;
- every bug has regression test;
- scheduler/cache/state transitions include negative branches;
- public behavior has integration tests;
- every explicit failure state is exercised;
- existing assertion changes receive privileged review.

## Adjudications
- **U-010 / AC-010**: resolved by `docs/decisions/0001-no-route-field-in-response.md` —
  the response schema must not contain a route/endpoint field at all (not merely
  "never set"). A schema-level test is the required proof.
