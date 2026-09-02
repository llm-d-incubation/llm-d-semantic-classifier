#!/usr/bin/env python3
"""Multi-dimensional benchmark campaign for llm-d-sc v0.2-staging.

Drives the matrix the goal asks for, one dimension at a time, changing exactly
ONE variable per arm (methodology rule 2: a "cost of X" number is only a delta
between two runs differing solely in X).

Dimensions
  concurrency   offered in-flight requests   -> saturation / throughput ceiling
  workers       executor threads per replica -> VERTICAL scale
  replicas      pod count                    -> HORIZONTAL scale
  context_bytes prompt size                  -> context-window sensitivity
  classifier    taxonomy (label/anchor count)-> "number of routes"
  cache         exact | redis-semantic       -> semantic-cache crossover
  keyspace      distinct keys in hit mode    -> working-set / repetition rate

Every arm writes:
  results/json/<arm>.json   full summary (percentiles + mean + stddev + statuses)
  results/raw/<arm>.csv     per-request samples, so anyone can recompute

Reconfiguration is done through the Deployment and waited on properly: a run
started before the rollout settles measures a mix of old and new pods.
"""
import json, os, subprocess, sys, time

NS = "cnuland-dev"
KCTL = ["kubectl", "-n", NS]
SC_SVC = "http://llm-d-sc.cnuland-dev.svc.cluster.local:50051"
RESULTS = "/work/results"

def sh(args, timeout=1200):
    return subprocess.run(args, capture_output=True, text=True, timeout=timeout)

def configure(workers=None, replicas=None, classifier=None, cache=None, cpu=None):
    """Patch the target Deployment, then wait for a CLEAN rollout."""
    env = []
    if workers   is not None: env.append({"name": "LLM_D_SC_INFERENCE_WORKERS", "value": str(workers)})
    if classifier is not None: env.append({"name": "LLM_D_SC_CLASSIFIER", "value": classifier})
    if cache     is not None: env.append({"name": "LLM_D_SC_CACHE", "value": cache})
    patch = {"spec": {"template": {"spec": {"containers": [{"name": "llm-d-sc"}]}}}}
    c = patch["spec"]["template"]["spec"]["containers"][0]
    if env: c["env"] = env
    if cpu is not None:
        # The worker pool cannot outrun its CPU allowance; if limits < workers the
        # arm measures CPU starvation, not worker width.
        c["resources"] = {"requests": {"cpu": str(cpu), "memory": "4Gi"},
                          "limits":   {"cpu": str(cpu), "memory": "16Gi"}}
    if env or cpu is not None:
        sh(KCTL + ["patch", "deployment", "llm-d-sc", "--type", "strategic",
                   "-p", json.dumps(patch)])
    if replicas is not None:
        sh(KCTL + ["scale", "deployment", "llm-d-sc", f"--replicas={replicas}"])
    sh(KCTL + ["rollout", "status", "deployment/llm-d-sc", "--timeout=300s"], timeout=360)
    # Ready endpoints must equal the requested replica count before traffic.
    want = replicas if replicas is not None else None
    for _ in range(60):
        r = sh(KCTL + ["get", "endpoints", "llm-d-sc", "-o",
                       "jsonpath={.subsets[*].addresses[*].ip}"])
        n = len(r.stdout.split())
        if n > 0 and (want is None or n == want):
            return n
        time.sleep(2)
    return 0

def run(label, concurrency, connections, cache_mode, context_bytes,
        duration=30, warmup=None, keyspace=1, run_id=None, requests=0, targets=SC_SVC):
    run_id = run_id if run_id is not None else int(time.time())
    if warmup is None:
        # A hit arm must fully populate the cache before measuring; a miss arm
        # only needs the connection pool and allocator settled.
        warmup = 2000 if cache_mode == "hit" else 100
    args = KCTL + ["exec", "bench-driver", "--", "/work/bin/scbench",
        "--mode", "grpc", "--target", targets,
        "--concurrency", str(concurrency), "--connections", str(connections),
        "--cache-mode", cache_mode, "--context-bytes", str(context_bytes),
        "--keyspace", str(keyspace), "--warmup", str(warmup),
        "--run-id", str(run_id), "--label", label,
        "--out", f"{RESULTS}/json/{label}.json",
        "--raw", f"{RESULTS}/raw/{label}.csv"]
    if requests: args += ["--requests", str(requests)]
    else:        args += ["--duration-secs", str(duration)]
    r = sh(args, timeout=1800)
    try:
        d = json.loads(r.stdout)
        l = d["latency"]
        print(f"  {label:<44} rps={d['throughput_rps']:>10,.0f}  rpm={d['throughput_rpm']:>12,.0f}  "
              f"p50={l['p50_ms']:>8.3f} p90={l['p90_ms']:>8.3f} p99={l['p99_ms']:>9.3f} "
              f"mean={l['mean_ms']:>8.3f} sd={l['stddev_ms']:>8.3f} err={d['errors']}", flush=True)
        return d
    except Exception:
        print(f"  {label:<44} FAILED: {(r.stderr or r.stdout)[:200]}", flush=True)
        return None


# ---------------------------------------------------------------------------
# Phases. Each changes ONE dimension and holds the rest fixed.
# ---------------------------------------------------------------------------

def phase_concurrency():
    """C1 - saturation ladder. Answers 'how many connections/min' and locates
    the knee. Cache-HIT so the classifier's own forward is out of the way and we
    are measuring the SERVICE path (admission, transport, cache, dispatch)."""
    print("\n== C1 concurrency ladder (1 replica, 4 workers, hit, 256B) ==", flush=True)
    configure(workers=4, replicas=1, classifier="complexity", cache="exact", cpu=4)
    out = []
    for c in [1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024]:
        conns = max(1, min(c, 128))
        d = run(f"c1-conc{c:04d}", c, conns, "hit", 256, duration=25)
        if d: out.append(d)
    return out

def phase_vertical():
    """C2 - VERTICAL scale. default_worker_width() is available_parallelism().min(4),
    so the env var must be set explicitly; raising CPU alone cannot widen the pool.
    CPU limit tracks worker count so the arm isn't secretly CPU-starved."""
    print("\n== C2 vertical scale: executor workers (1 replica) ==", flush=True)
    out = []
    for w in [1, 2, 4, 8, 16, 32]:
        configure(workers=w, replicas=1, classifier="complexity", cache="exact", cpu=w)
        # Concurrency is held at HALF the 256-slot admission bound. At 256 a
        # 1-worker/1-CPU arm floods the queue and the run measures admission
        # shedding rather than worker width -- two different findings that must
        # not be conflated. C1 already located the shedding boundary separately.
        for mode, dur, conc in (("hit", 25, 128), ("miss", 40, 32)):
            d = run(f"c2-w{w:02d}-{mode}", conc, min(conc, 64), mode, 256, duration=dur)
            if d: out.append(d)
    return out

def phase_horizontal():
    """C3 - HORIZONTAL scale. Worker width held at 8 so each replica is identically
    shaped and the only variable is replica count."""
    print("\n== C3 horizontal scale: replicas (8 workers each) ==", flush=True)
    out = []
    for r in [1, 2, 4, 8]:
        n = configure(workers=8, replicas=r, classifier="complexity", cache="exact", cpu=8)
        print(f"  [ready endpoints: {n}/{r}]", flush=True)
        for mode, dur, conc in (("hit", 25, 128 * r), ("miss", 40, 32 * r)):
            d = run(f"c3-r{r:02d}-{mode}", conc, min(conc, 128), mode, 256, duration=dur)
            if d: out.append(d)
    return out

def phase_context():
    """C4 - context-window sensitivity. The hypothesis under test: small
    agent-turn contexts are cheap, whole-document contexts are not."""
    print("\n== C4 context size sweep ==", flush=True)
    configure(workers=8, replicas=1, classifier="complexity", cache="exact", cpu=8)
    out = []
    for b in [64, 256, 1024, 4096, 16384, 65536]:
        for mode, dur, conc in (("hit", 20, 128), ("miss", 40, 32)):
            d = run(f"c4-ctx{b:06d}-{mode}", conc, min(conc, 64), mode, b, duration=dur)
            if d: out.append(d)
    return out

def phase_routes():
    """C5 - route count. Ranking is anchor-topk-mean: a cosine similarity per
    ANCHOR, so cost tracks anchor count (complexity 48, cost 40, sensitivity 50),
    not label count."""
    print("\n== C5 route/taxonomy size ==", flush=True)
    out = []
    for clf in ["cost", "complexity", "sensitivity"]:
        configure(workers=8, replicas=1, classifier=clf, cache="exact", cpu=8)
        for mode, dur, conc in (("hit", 20, 128), ("miss", 40, 32)):
            d = run(f"c5-{clf}-{mode}", conc, min(conc, 64), mode, 256, duration=dur)
            if d: out.append(d)
    return out

def phase_semantic():
    """C6 - the semantic-cache crossover. keyspace is the lever: it sets how much
    the workload REPEATS. A large keyspace in miss mode is the adversarial case
    (every prompt novel); a small one is the friendly case."""
    print("\n== C6 semantic cache: exact vs redis-semantic ==", flush=True)
    out = []
    for cache in ["exact", "redis-semantic"]:
        configure(workers=8, replicas=1, classifier="complexity", cache=cache, cpu=8)
        for ks in [1, 100, 10000]:
            d = run(f"c6-{cache}-ks{ks:05d}-hit", 128, 64, "hit", 256, duration=25, keyspace=ks)
            if d: out.append(d)
        d = run(f"c6-{cache}-novel-miss", 32, 32, "miss", 256, duration=40)
        if d: out.append(d)
    return out

PHASES = {"c1": phase_concurrency, "c2": phase_vertical, "c3": phase_horizontal,
          "c4": phase_context, "c5": phase_routes, "c6": phase_semantic}

if __name__ == "__main__":
    want = sys.argv[1:] or list(PHASES)
    t0 = time.time()
    for p in want:
        if p not in PHASES:
            print(f"unknown phase {p}; known: {list(PHASES)}"); sys.exit(2)
        PHASES[p]()
    print(f"\ncampaign phases {want} done in {time.time()-t0:.0f}s", flush=True)
