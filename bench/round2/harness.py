#!/usr/bin/env python3
"""Round-2 benchmark harness: Praxis gateway, with premise validation.

Round 1 produced two wrong conclusions and one retracted finding. Every one of
them came from an UNVERIFIED ASSUMPTION rather than a bad measurement:

  * the driver counted a transport-level Ok as success without checking a
    classification came back;
  * the driver's own per-request lock made it the bottleneck;
  * Nagle on accepted sockets put a 40 ms delayed-ACK cluster in every p99;
  * warmup was counted in requests instead of keys covered, so a partially cold
    cache was measured as service latency.

So this harness refuses to report an arm whose premises it cannot confirm. Each
measurement carries the checks that make it meaningful, and a failed check marks
the arm rather than silently producing a plausible number.
"""
import csv, io, json, os, re, subprocess, sys, time

NS = "cnuland-dev"
KCTL = ["kubectl", "-n", NS]
RESULTS = "/work/results"
SC_SVC = "http://llm-d-sc.cnuland-dev.svc.cluster.local:50051"
PRAXIS = "http://praxis.cnuland-dev.svc.cluster.local"
HERE = os.path.dirname(os.path.abspath(__file__))

def sh(a, t=2400):
    return subprocess.run(a, capture_output=True, text=True, timeout=t)

# --------------------------------------------------------------------------
# environment control
# --------------------------------------------------------------------------
def configure_sc(workers=16, replicas=1, classifier="complexity", cache="exact", cpu=16):
    env = [
        {"name": "LLM_D_SC_INFERENCE_WORKERS", "value": str(workers)},
        {"name": "LLM_D_SC_CLASSIFIER", "value": classifier},
        {"name": "LLM_D_SC_CACHE", "value": cache},
        {"name": "LLM_D_SC_MODEL_DIR", "value": "/work/models"},
        {"name": "LLM_D_SC_LISTEN", "value": "0.0.0.0:50051"},
        {"name": "LLM_D_SC_REDIS_URL", "value": "redis://redis.cnuland-dev.svc.cluster.local:6379"},
    ]
    patch = {"spec": {"template": {"spec": {"containers": [{
        "name": "llm-d-sc", "env": env,
        "resources": {"requests": {"cpu": str(cpu), "memory": "8Gi"},
                      "limits": {"cpu": str(cpu), "memory": "32Gi"}}}]}}}}
    sh(KCTL + ["patch", "deployment", "llm-d-sc", "--type", "strategic", "-p", json.dumps(patch)])
    sh(KCTL + ["scale", "deployment", "llm-d-sc", f"--replicas={replicas}"])
    sh(KCTL + ["rollout", "status", "deployment/llm-d-sc", "--timeout=420s"], t=480)
    for _ in range(90):
        r = sh(KCTL + ["get", "endpoints", "llm-d-sc", "-o",
                       "jsonpath={.subsets[*].addresses[*].ip}"])
        if len(r.stdout.split()) == replicas:
            return replicas
        time.sleep(2)
    return len(sh(KCTL + ["get", "endpoints", "llm-d-sc", "-o",
                          "jsonpath={.subsets[*].addresses[*].ip}"]).stdout.split())

def apply_praxis_config(text, replicas=1):
    open("/tmp/r2-praxis.yaml", "w").write(text)
    p = sh(KCTL + ["create", "cm", "praxis-config",
                   "--from-file=praxis.yaml=/tmp/r2-praxis.yaml", "--dry-run=client", "-o", "yaml"])
    subprocess.run(["kubectl", "apply", "-n", NS, "-f", "-"], input=p.stdout,
                   capture_output=True, text=True)
    sh(KCTL + ["scale", "deployment", "praxis", f"--replicas={replicas}"])
    sh(KCTL + ["rollout", "restart", "deploy/praxis"])
    sh(KCTL + ["rollout", "status", "deploy/praxis", "--timeout=300s"], t=360)
    time.sleep(4)

def set_timeout(text, ms):
    return re.sub(r"timeout_ms: \d+", f"timeout_ms: {ms}", text)

def redis(*a):
    return sh(KCTL + ["exec", "deploy/redis", "--", "redis-cli", *a]).stdout.strip()

# --------------------------------------------------------------------------
# premise checks
# --------------------------------------------------------------------------
def check_cache_mode(expected):
    """The arm must actually be running the cache tier it claims."""
    r = sh(KCTL + ["get", "deploy", "llm-d-sc", "-o",
                   "jsonpath={.spec.template.spec.containers[0].env[?(@.name=='LLM_D_SC_CACHE')].value}"])
    actual = r.stdout.strip() or "exact"
    if actual != expected:
        return False, f"cache={actual} expected {expected}"
    if expected == "redis-semantic":
        # Configured is not the same as ENABLED: a build without the feature, or
        # an unreachable Redis, silently degrades to exact-only.
        log = sh(KCTL + ["logs", "-l", "app=llm-d-sc", "--tail=400"]).stdout
        if "semantic cache enabled" not in log:
            return False, "redis-semantic configured but never logged as enabled"
    return True, actual

def backend_headroom(target, conc=256):
    """Measure the backend directly so we can prove it is not the limiter."""
    r = sh(KCTL + ["exec", "bench-driver", "--", "/work/bin/scbench", "--mode", "http",
                   "--target", target, "--concurrency", str(conc), "--connections", "128",
                   "--cache-mode", "hit", "--keyspace", "1", "--context-bytes", "256",
                   "--warmup", "500", "--duration-secs", "10", "--run-id", "9999",
                   "--label", "headroom"])
    try:
        return json.loads(r.stdout)["throughput_rps"]
    except Exception:
        return 0.0

# --------------------------------------------------------------------------
# measurement
# --------------------------------------------------------------------------
def measure(label, port=8080, conc=64, conns=64, cache_mode="hit", keyspace=1,
            hit_ratio=1.0, ctx=256, dur=25, mode="http", warmup=None,
            expect_cache="exact", slow_tier_ms=None, quiet=False):
    """One arm. Returns the full record, including premise-check results."""
    ok_cache, cache_note = check_cache_mode(expect_cache)
    raw = f"{RESULTS}/raw/{label}.csv"
    tgt = f"{PRAXIS}:{port}" if mode == "http" else SC_SVC
    a = KCTL + ["exec", "bench-driver", "--", "/work/bin/scbench", "--mode", mode,
                "--target", tgt, "--concurrency", str(conc), "--connections", str(conns),
                "--cache-mode", cache_mode, "--hit-ratio", str(hit_ratio),
                "--keyspace", str(keyspace), "--context-bytes", str(ctx),
                "--duration-secs", str(dur), "--run-id", str(int(time.time()) % 100000),
                "--label", label, "--out", f"{RESULTS}/json/{label}.json", "--raw", raw]
    if warmup is not None:
        a += ["--warmup", str(warmup)]
    r = sh(a)
    try:
        d = json.loads(r.stdout)
    except Exception:
        print(f"  {label:<40} FAILED {(r.stderr or r.stdout)[:160]}", flush=True)
        return None

    # Routing coverage: with the slow tier at `slow_tier_ms`, a request's
    # destination is readable straight off its own latency, so the fraction that
    # was actually classified can be COUNTED rather than assumed.
    routed_pct = None
    if slow_tier_ms:
        g = sh(KCTL + ["exec", "bench-shell", "--", "cat", raw])
        lat = [int(x["latency_ns"]) / 1e6 for x in csv.DictReader(io.StringIO(g.stdout))]
        if lat:
            routed_pct = sum(1 for x in lat if x > slow_tier_ms / 2) / len(lat) * 100

    d["premise_cache_ok"] = ok_cache
    d["premise_cache_note"] = cache_note
    d["routed_pct"] = routed_pct
    with open("/tmp/r2-record.jsonl", "a") as f:
        f.write(json.dumps(d) + "\n")

    if not quiet:
        l = d["latency"]
        flag = "" if ok_cache else f"  !!PREMISE:{cache_note}"
        rp = f" routed={routed_pct:5.1f}%" if routed_pct is not None else ""
        print(f"  {label:<40} rps={d['throughput_rps']:>9,.0f} rpm={d['throughput_rpm']:>12,.0f} "
              f"p50={l['p50_ms']:>8.3f} p90={l['p90_ms']:>8.3f} p99={l['p99_ms']:>9.3f} "
              f"mean={l['mean_ms']:>8.3f} sd={l['stddev_ms']:>7.3f} err={d['errors']}{rp}{flag}", flush=True)
    return d

def knee(rows, key="throughput_rps"):
    """Locate the knee: the last point where a doubling of offered load still
    buys >=20% more throughput. Beyond it you are buying latency, not work."""
    best = None
    for i in range(1, len(rows)):
        prev, cur = rows[i-1], rows[i]
        gain = cur[key] / prev[key] - 1 if prev[key] else 0
        if gain >= 0.20:
            best = cur
    return best
