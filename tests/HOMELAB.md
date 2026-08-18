# Kubernetes Homelab System and Performance Validation

The homelab is the prod-like evidence environment. GitHub is the clean-room reproducibility environment. Do not expose a trusted homelab as a general self-hosted runner for untrusted public PR code.

## Immutable test inputs

For every promotion run record:
- exact reviewed git SHA;
- llm-d-sc image digest;
- dummy gateway image digest;
- model OCI image digest;
- classifier revision;
- cluster version;
- node/CPU/GPU profile;
- resource requests/limits;
- runtime backend and configuration.

## Topology A — same-Pod sidecar

```text
Pod
+------------------------------------------------+
| dummy-gateway                                   |
|   persistent gRPC -> 127.0.0.1:50051          |
|                                                |
| llm-d-sc                                       |
|   resident classifier                         |
+------------------------------------------------+
```

Measure from the dummy gateway process:
- no-op RPC transport floor (test service/build only if implemented);
- exact-result cache hit;
- cache miss/real inference;
- mixed distribution;
- same-key burst;
- unique-key burst;
- queue delay;
- total callout RTT.

This directly answers the sidecar network-hop question.

## Topology B — ClusterIP

```text
dummy gateway Pod
      |
      | persistent HTTP/2 gRPC
      v
llm-d-sc Service -> llm-d-sc Pod(s)
```

Use identical model, resources, requests, concurrency, warmup and sample count. Where possible test same-node and cross-node placement.

## Benchmark protocol

Every benchmark emits a machine-readable manifest:

```yaml
git_sha:
service_image_digest:
model_image_digest:
classifier_revision:
cluster_version:
node:
cpu_model:
gpu_model:
cpu_request:
cpu_limit:
memory_limit:
runtime_backend:
sequence_length:
concurrency:
cache_mode:
warmup_requests:
measured_requests:
topology:
```

Initial method:
1. deploy exact SHA;
2. wait for readiness/warmup;
3. run sufficient warmup (start with 1,000 requests, adjust with evidence);
4. run measured workload (start with 10,000 requests for stable percentiles);
5. repeat at least 3 independent trials;
6. store p50/p90/p95/p99/max, throughput, errors/drops;
7. separately store queue/tokenize/forward/total service time and dummy-the AI Gateway RTT;
8. retain raw histogram output.

## 0.1 matrix

Input lengths: 32, 64, 128, 256 tokens.

Cache: 0% and 100% hit initially; add realistic mixed traffic once expected hit distribution is known.

Concurrency: 1 and 4 for MVP. 0.21 expands to 2/8/16/32 and CPU/GPU/thread matrices.

## Recovery tests

### Restart with complete context
- warm cache;
- kill llm-d-sc;
- replacement loads/warms;
- send same complete-context request;
- classification matches expected tolerance;
- evidence shows cold cache/recompute rather than hidden persistence.

### Cache loss with weak delta — 0.22
- dummy gateway stores a test session already using a high-capability route;
- llm-d-sc had optional session feature state;
- restart llm-d-sc;
- send only `continue`/weak delta without enough context;
- classifier returns `abstain`/`insufficient_context`;
- dummy gateway preserves conservative route;
- once context is re-established, normal classification resumes.

### Saturation
- constrain CPU;
- exceed measured service capacity;
- queue stays bounded;
- overload responses become visible;
- stale expired jobs do not execute later;
- admitted-request p99 and error rates are preserved in evidence;
- service recovers when load ends.

### Graceful rollout
- sustain requests;
- trigger deployment update;
- old Pod becomes not-ready before exit;
- configured in-flight requests drain/fail according to contract;
- replacement warms before traffic;
- no fabricated classifications.

## OCI / disconnected model test

1. Build classifier from pinned Hugging Face revision into ModelCar.
2. Push it to a private/internal OCI registry.
3. Resolve and deploy by immutable digest.
4. Deny namespace egress to Hugging Face.
5. restart the workload.
6. verify model materializes and service becomes ready.
7. prove request path performs no external model download.

## Kubernetes security/runtime tests

- random non-root UID works;
- model files are readable without root and mounted read-only to llm-d-sc;
- NetworkPolicy only permits required classifier callout/metrics traffic;
- prompts/session IDs never appear as metric labels;
- CPU/memory limits are intentional and recorded.

## Performance gating

### 0.1
Latency is required evidence, not yet a universal hard SLA. Fail promotion if:
- cache hit still runs model forward;
- queue is unbounded;
- sidecar benchmark cannot separate transport/cache/inference;
- p99 is missing;
- benchmark parameters are not recorded.

### 0.21
Approve named repeatable hardware profiles (for example `cpu-homelab-4c-256t`). Only then encode absolute latency thresholds such as the desired ~20 ms uncached budget for that exact profile.
