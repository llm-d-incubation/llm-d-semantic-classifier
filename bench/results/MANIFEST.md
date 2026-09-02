# Campaign manifest

Methodology rule 5: every result carries a manifest. This is it.

| Field | Value |
|---|---|
| date | 2026-09-02 |
| cluster | CoreWeave *waldorf* (`api.6787d4-33de361c.k8s.us-east-04a.coreweave.com`) |
| namespace | `cnuland-dev` |
| llm-d-sc branch | `v0.2-staging` |
| llm-d-sc commit | `9eb7b58` (pre-fix arms) / `f57d4ef` (post-tcp_nodelay arms, prefix v2/c8-c13) |
| build features | `--release --features redis-semantic` |
| model | `cnuland/llm-d-sc-complexity` @ `c5f55ef419d268ba843c544dc00988d1e9878044` |
| taxonomies | complexity (4 labels / 48 anchors), cost (4/40), sensitivity (5/50), plus synthetic 48/200/800/2000-anchor variants |
| Praxis | 0.5.2 vendored snapshot + `llm-d-sc-praxis-filter` (branch `poc/phase-a-external`) |
| endpoints | vllm-vcr `d01e542e72b6ed2dd29a4ee0fd771dce7e9a5d11` + vLLM Rust frontend `vllm-project/vllm@6e448d0` |
| llm-d gateway | Istio GatewayClass + InferencePool + EPP `ghcr.io/llm-d/llm-d-router-endpoint-picker:v0.9.0`, same vllm-vcr backends |
| vLLM SR adapter | `bench/round2/vsr-adapter` serving llm-d-sc over the http_classify contract |
| llm-d IPP + llm-d-sc | llm-d-ipp-scorer POC (llm-d-inference-payload-processor#299), Envoy v1.34 ext_proc FULL_DUPLEX_STREAMED -> IPP -> llm-d-sc |
| Redis | `redis/redis-stack-server:7.4.0-v1` (RediSearch `search` module present) |
| target node | `gf41fb2` (128 vCPU, amd64, Ubuntu 24.04) |
| driver node | `gd91fda` (128 vCPU) — always a DIFFERENT node than targets |
| gateway node | `gf48cf2` · endpoints node | `gf49e9c` |
| driver | `scbench` (this repo, `bench/driver`), closed-loop |
| percentiles | nearest-rank, identical to `src/bench.rs::percentile` |
| runs captured | 311 |

## Raw data

Full per-request samples (`latency_ns,err` per request, ~332 MB gzipped, 58
files) are retained on the campaign's PersistentVolume, which uses a **Retain**
reclaim policy so it survives the namespace.

    PVC: bench-workspace (cnuland-dev), path /work/results/raw/

To fetch:

    kubectl -n cnuland-dev exec bench-shell -- tar cz -C /work/results raw > raw.tgz

They are excluded from git only for size; every published figure can be
recomputed from them.
