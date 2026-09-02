#!/usr/bin/env python3
"""Round 2, phase 2: the llm-d inference gateway, same dimensions as Praxis.

Topology: Istio Gateway -> InferencePool -> EPP (endpoint picker) -> vllm-vcr.
The backends are the SAME pods Praxis routes to, so any difference between the
two campaigns is the gateway and nothing else.

One structural difference worth stating plainly: Praxis's `llm_d_sc` filter picks
a CLUSTER (which model tier should serve this), while llm-d's EPP picks an
ENDPOINT (which replica of a pool should serve this). They are complementary
decisions, not competing ones, so "llm-d vs Praxis" here compares gateway cost
and capacity, not routing quality.
"""
import json, os, subprocess, sys, time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import harness as H

LLMD = "http://llmd-bench-gw-istio.cnuland-dev.svc.cluster.local:80"
DIRECT = "http://vcr-small.cnuland-dev.svc.cluster.local:8000"

def run(label, target, conc, conns, cache_mode="hit", ctx=256, dur=20,
        keyspace=1, hit_ratio=1.0, warmup=None):
    a = H.KCTL + ["exec", "bench-driver", "--", "/work/bin/scbench", "--mode", "http",
                  "--target", target, "--concurrency", str(conc), "--connections", str(conns),
                  "--cache-mode", cache_mode, "--hit-ratio", str(hit_ratio),
                  "--keyspace", str(keyspace), "--context-bytes", str(ctx),
                  "--duration-secs", str(dur), "--run-id", str(int(time.time()) % 100000),
                  "--label", label, "--out", f"{H.RESULTS}/json/{label}.json"]
    if warmup is not None:
        a += ["--warmup", str(warmup)]
    r = H.sh(a)
    try:
        d = json.loads(r.stdout); l = d["latency"]
        print(f"  {label:<38} rps={d['throughput_rps']:>9,.0f} rpm={d['throughput_rpm']:>12,.0f} "
              f"p50={l['p50_ms']:>8.3f} p90={l['p90_ms']:>8.3f} p99={l['p99_ms']:>9.3f} "
              f"mean={l['mean_ms']:>8.3f} sd={l['stddev_ms']:>7.3f} err={d['errors']}", flush=True)
        return d
    except Exception:
        print(f"  {label:<38} FAILED {(r.stderr or r.stdout)[:150]}", flush=True)
        return None

def l1_ladder():
    """Offered-load ladder through the llm-d gateway, paired against the SAME
    backends reached directly. The delta is the gateway's cost."""
    print("\n== L1 llm-d gateway offered-load ladder ==", flush=True)
    rows = []
    for c in [8, 16, 32, 64, 128, 256, 512, 1024]:
        d = run(f"l1-llmd-c{c:04d}", LLMD, c, min(c, 128))
        if d: rows.append(d)
        run(f"l1-direct-c{c:04d}", DIRECT, c, min(c, 128))
    k = H.knee(rows)
    if k:
        print(f"  KNEE [llm-d]: concurrency {k['concurrency']} -> {k['throughput_rps']:,.0f} req/s "
              f"({k['throughput_rpm']:,.0f} req/min), p99 {k['latency']['p99_ms']:.2f} ms", flush=True)

def l2_context():
    print("\n== L2 llm-d gateway: context size ==", flush=True)
    for b in [64, 256, 1024, 4096, 16384, 65536]:
        run(f"l2-llmd-ctx{b:06d}", LLMD, 64, 64, ctx=b)

def l3_poolsize():
    """llm-d's routing dimension is POOL SIZE -- how many endpoints the EPP
    chooses between -- which is the closest analogue to Praxis's route table."""
    print("\n== L3 llm-d: InferencePool size (endpoints the EPP selects between) ==", flush=True)
    for n in [1, 2, 3, 6]:
        H.sh(H.KCTL + ["scale", "deploy/vcr-small", f"--replicas={n}"])
        H.sh(H.KCTL + ["rollout", "status", "deploy/vcr-small", "--timeout=300s"], t=360)
        time.sleep(10)
        run(f"l3-pool{n:02d}", LLMD, 128, 128)

def l4_soak():
    print("\n== L4 llm-d gateway soak (5 min) ==", flush=True)
    run("l4-llmd-soak", LLMD, 128, 128, dur=300)

PHASES = {"l1": l1_ladder, "l2": l2_context, "l3": l3_poolsize, "l4": l4_soak}

if __name__ == "__main__":
    want = sys.argv[1:] or ["l1", "l2", "l3"]
    print(f"ROUND 2 / LLM-D — phases {want}", flush=True)
    hd = H.backend_headroom(DIRECT)
    print(f"backend headroom (direct): {hd:,.0f} req/s", flush=True)
    t0 = time.time()
    for p in want:
        PHASES[p]()
    print(f"\nround2/llm-d {want} done in {time.time()-t0:.0f}s", flush=True)
