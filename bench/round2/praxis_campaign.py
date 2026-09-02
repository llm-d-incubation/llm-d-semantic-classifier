#!/usr/bin/env python3
"""Round 2, phase 1: the Praxis gateway across every dimension.

Dimensions: offered load (the RPS knee), context size, route-table size, cache
configuration, and gateway replicas. Each phase varies ONE of them.

Every arm is paired against a control listener that is identical except that
cluster selection is a static `router` instead of the classifier, so a "cost of
classification" number is always a delta between two runs differing solely in
that filter.
"""
import json, os, subprocess, sys, time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import harness as H

HERE = os.path.dirname(os.path.abspath(__file__))

# Base config: two tiers, deliberately latency-separated so every request's
# DESTINATION is readable from its own latency and routing coverage can be
# counted instead of assumed.
# DIFFERENTIATED tiers: destination readable from latency -> routing coverage.
BASE = open(os.path.join(HERE, "praxis-base.yaml")).read()
# UNIFORM-fast tiers: gateway is the limiter -> capacity and knee.
UNIFORM = open(os.path.join(HERE, "praxis-uniform.yaml")).read()

def p1_load_ladder():
    print("\n== P1 offered-load ladder: where is the gateway knee? ==", flush=True)
    H.configure_sc(workers=16, replicas=1, cache="exact", cpu=16)
    H.apply_praxis_config(H.set_timeout(UNIFORM, 1000), replicas=1)
    rows_c, rows_b = [], []
    for c in [8, 16, 32, 64, 128, 256, 512, 1024]:
        d = H.measure(f"p1-classified-c{c:04d}", port=8080, conc=c, conns=min(c,128),
                      cache_mode="hit", keyspace=1, ctx=256, dur=20)
        if d: rows_c.append(d)
        d = H.measure(f"p1-control-c{c:04d}", port=8081, conc=c, conns=min(c,128),
                      cache_mode="hit", keyspace=1, ctx=256, dur=20)
        if d: rows_b.append(d)
    for name, rows in (("classified", rows_c), ("control", rows_b)):
        k = H.knee(rows)
        if k:
            print(f"  KNEE [{name}]: concurrency {k['concurrency']} -> "
                  f"{k['throughput_rps']:,.0f} req/s ({k['throughput_rpm']:,.0f} req/min), "
                  f"p99 {k['latency']['p99_ms']:.2f} ms", flush=True)

def p2_context():
    print("\n== P2 context size through the gateway ==", flush=True)
    H.configure_sc(workers=16, replicas=1, cache="exact", cpu=16)
    H.apply_praxis_config(H.set_timeout(UNIFORM, 1000), replicas=1)
    for b in [64, 256, 1024, 4096, 16384, 65536]:
        H.measure(f"p2-cached-ctx{b:06d}", port=8080, conc=64, conns=64,
                  cache_mode="hit", keyspace=1, ctx=b, dur=20)
        H.measure(f"p2-novel-ctx{b:06d}", port=8080, conc=16, conns=16,
                  cache_mode="miss", ctx=b, dur=25)

def p3_routes():
    print("\n== P3 route-table size: 2..32 clusters at the gateway ==", flush=True)
    for n in [2, 4, 8, 16, 32]:
        tax = f"/work/classifiers/r2-taxonomy-{n}.json"
        H.configure_sc(workers=16, replicas=1, classifier=tax, cache="exact", cpu=16)
        cfg = open(os.path.join(HERE, f"praxis-{n}.yaml")).read()
        H.apply_praxis_config(cfg, replicas=1)
        H.measure(f"p3-routes{n:02d}-cached", port=8080, conc=64, conns=64,
                  cache_mode="hit", keyspace=1, ctx=256, dur=20)
        H.measure(f"p3-routes{n:02d}-novel", port=8080, conc=16, conns=16,
                  cache_mode="miss", ctx=256, dur=25)

def p4_cache():
    print("\n== P4 cache configuration x workload ==", flush=True)
    H.apply_praxis_config(H.set_timeout(UNIFORM, 1000), replicas=1)
    for cache in ["exact", "redis-semantic"]:
        H.redis("FLUSHALL")
        H.configure_sc(workers=16, replicas=1, cache=cache, cpu=16)
        time.sleep(5)
        for hr, name in [(1.0, "cached"), (0.9, "mix90"), (0.5, "mix50"), (0.0, "novel")]:
            H.measure(f"p4-{cache}-{name}", port=8080, conc=32, conns=32,
                      cache_mode="mixed", hit_ratio=hr, keyspace=200, ctx=256,
                      dur=25, expect_cache=cache)

def p5_gateway_scale():
    print("\n== P5 gateway horizontal scale ==", flush=True)
    H.configure_sc(workers=16, replicas=2, cache="exact", cpu=16)
    for r in [1, 2, 4]:
        H.apply_praxis_config(H.set_timeout(UNIFORM, 1000), replicas=r)
        H.measure(f"p5-praxis{r}-classified", port=8080, conc=256, conns=128,
                  cache_mode="hit", keyspace=1, ctx=256, dur=20)
        H.measure(f"p5-praxis{r}-control", port=8081, conc=256, conns=128,
                  cache_mode="hit", keyspace=1, ctx=256, dur=20)

PHASES = {"p1": p1_load_ladder, "p2": p2_context, "p3": p3_routes,
          "p4": p4_cache, "p5": p5_gateway_scale}

if __name__ == "__main__":
    want = sys.argv[1:] or list(PHASES)
    print(f"ROUND 2 / PRAXIS — phases {want}", flush=True)
    hd = H.backend_headroom("http://vcr-small.cnuland-dev.svc.cluster.local:8000")
    print(f"backend headroom (vcr-small direct): {hd:,.0f} req/s "
          f"{'OK' if hd > 40000 else '** LOW: backend may be the limiter **'}", flush=True)
    t0 = time.time()
    for p in want:
        PHASES[p]()
    print(f"\nround2/praxis {want} done in {time.time()-t0:.0f}s", flush=True)
