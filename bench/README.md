# `bench/` — llm-d-sc v0.2-staging benchmark campaign

Multi-dimensional benchmark of llm-d-sc and its gateway integration, run on the
CoreWeave *waldorf* cluster (namespace `cnuland-dev`).

## What it answers

1. **Where is the bottleneck, and where does llm-d-sc perform?**
2. **How many connections per minute can it sustain?**
3. **What is the ideal scaling shape** (vertical: executor workers; horizontal: replicas)?
4. **When is the semantic cache worth turning on, and what does it buy?**
5. **Where does context size start to hurt?**

## Layout

| Path | Purpose |
|---|---|
| `driver/` | `scbench` — closed-loop load driver (gRPC classify + HTTP gateway) |
| `manifests/` | everything deployed on the cluster, in apply order |
| `praxis/` | Praxis gateway config: measured (`:8080`) and control (`:8081`) listeners |
| `run-campaign.py` | the campaign matrix; one dimension per phase |
| `placement-evidence.py` | records controlled same-node and cross-node ClusterIP observations |
| `report.py` | generates the statistical report FROM the captured JSON |

## Design notes that matter

**No registry in the loop.** Every binary (llm-d-sc, Praxis, `scbench`, vllm-vcr,
vllm-rs) is built in-cluster onto a ReadWriteMany volume and executed from there.
The nodes are amd64 and the dev laptop is arm64, so a local cross-build of Candle
would run under emulation; a 128-core node does it natively in minutes. It also
guarantees every replica runs a byte-identical binary.

**Driver and target never share a node.** A driver co-located with its target
competes for the same softirq and NIC budget, so the "target" latency it reports
is partly its own. This is the flaw Intel's corrected Arena campaign existed to
remove, and it is preserved here by `nodeSelector`.

**One variable per arm.** Each phase changes exactly one dimension. A "cost of X"
number is only meaningful as a delta between two runs differing solely in X --
which is why Praxis exposes a control listener with the `llm_d_sc` filter removed
and everything else identical.

**Cache arms use disjoint key namespaces.** MISS keys are namespaced by run id
*and* phase. Warmup and measurement both count from zero, so a shared namespace
would make the first N measured "misses" silent cache hits of the warmup's own
keys. That bug was present in the first draft of the driver and showed up exactly
as the house rules predict: an impossibly small minimum latency next to a large
mean (0.14 ms min against a 422 ms mean).

**Percentiles are nearest-rank**, identical to `src/bench.rs::percentile`, so a
figure here is directly comparable to one from the in-tree harness. Mean and
standard deviation are reported ALONGSIDE the distribution, never instead of it.

## Running

```sh
kubectl apply -f manifests/00-workspace.yaml
kubectl apply -f manifests/01-build-job.yaml      # llm-d-sc + model
kubectl apply -f manifests/03-driver-build-job.yaml
kubectl apply -f manifests/10-llm-d-sc.yaml
python3 run-campaign.py c1 c2 c3 c4 c5 c6
python3 report.py --src results --out STATISTICAL-REPORT.md
```

Raw per-request samples are retained under `results/raw/<label>.csv` so any
published figure can be recomputed independently.

## Placement evidence

The high-load campaign deliberately keeps its driver off the target node. The
placement arm is separate and low-rate: it runs two otherwise identical
observers through the same ClusterIP Service, one scheduled with the target and
one on a different node. It is therefore evidence about network placement, not
a replacement throughput figure.

### Before running

This arm is intentionally tied to a maintainer-owned cluster. Before applying
the observer manifest, confirm all of the following:

1. Exactly one `llm-d-sc` target Pod is Ready and the `llm-d-sc` Service has
   exactly that Pod as its endpoint.
2. `bench-placement-same` selects the node on which that target is scheduled.
3. `bench-placement-cross` selects a different schedulable node.
4. The `bench-workspace` PVC contains the built `scbench` binary and has space
   for the JSON summary and raw samples.

The manifest carries the node selectors used for the documented cluster; adjust
them to the observed target node and a distinct available node before running in
another context. A runner failure is a failed precondition or measurement run,
not evidence that either placement is faster or slower.

```sh
kubectl apply -f manifests/21-placement-observers.yaml
python3 placement-evidence.py
```

Before any traffic, the runner requires exactly one Ready target and one Service
endpoint. It then reads the scheduler's actual Pod nodes and fails rather than
mislabel a run if the `same-node` or `cross-node` relationship is not true. The
checked-in result is `results/json/placement-evidence.json`; its accompanying
raw samples remain on the campaign PVC under `results/raw/`.

`./hack/verify` runs an offline check of that placement guard. It verifies that
only the matching relationship is accepted and that a mismatched or unassigned
observer is refused before a result can be labelled.
