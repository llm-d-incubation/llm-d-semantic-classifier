#!/usr/bin/env python3
"""Round 3 campaign: Praxis, llm-d (IPP), and vLLM SR — all with llm-d-sc.

Same dimensions on every stack so the three are comparable:
  context size · route count · REQUEST RATE (open-loop) · cache configuration

Every arm: realistic corpus traffic, replicated with bootstrap CIs, premise
assertions persisted into the result JSON.

Usage: campaign3.py <stack> [phases...]
  stack  = praxis | llmd | vsr
"""
import json, os, random, statistics, subprocess, sys, time
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "round2"))
import harness as H

CORPUS = "/work/corpus.jsonl"
REPS   = 3
DUR    = 25

TARGETS = {
    "praxis":  ("http://praxis.cnuland-dev.svc.cluster.local:8080",
                "http://praxis.cnuland-dev.svc.cluster.local:8081"),
    "llmd":    ("http://ipp-envoy.cnuland-dev.svc.cluster.local:8080",
                "http://ipp-envoy.cnuland-dev.svc.cluster.local:8081"),
    "vsr":     ("http://llm-d-sc-vsr-adapter.cnuland-dev.svc.cluster.local:8080", None),
}
MODEL = {"praxis": "tier-small", "llmd": "tier-small", "vsr": "router-model"}

def ci95(xs, n=1500):
    if len(xs) < 2: return (float("nan"), float("nan"))
    m = sorted(statistics.median(random.choices(xs, k=len(xs))) for _ in range(n))
    return m[int(.025*n)], m[int(.975*n)]

def once(stack, label, target, *, rate=0.0, conc=64, conns=64, dist="zipf",
         ctx=0, rep=0, mode=None):
    m = mode or ("classify" if stack == "vsr" else "http")
    a = H.KCTL + ["exec","bench-driver","--","/work/bin/scbench","--mode",m,
        "--target",target,"--concurrency",str(conc),"--connections",str(conns),
        "--corpus",CORPUS,"--dist",dist,"--corpus-offset",str(rep*40000),
        "--model",MODEL[stack],"--warmup","1500","--duration-secs",str(DUR),
        "--run-id",str(int(time.time()*1000)%100000),"--label",label,
        "--out",f"{H.RESULTS}/json/{label}.json"]
    if rate > 0: a += ["--rate",str(rate),"--arrival","poisson"]
    if ctx > 0:  a += ["--context-bytes",str(ctx)]
    r = H.sh(a)
    try: return json.loads(r.stdout)
    except Exception: return None

def replicated(stack, base, target, **kw):
    """REPS independent runs -> median + bootstrap CI. Prints one line."""
    rps, p50, p90, p99, p999, mean, sd, errs, bad = [],[],[],[],[],[],[],[],[]
    for rep in range(REPS):
        d = once(stack, f"{base}-rep{rep}", target, rep=rep, **kw)
        if not d: continue
        l = d["latency"]
        rps.append(d["throughput_rps"]); p50.append(l["p50_ms"]); p90.append(l["p90_ms"])
        p99.append(l["p99_ms"]); p999.append(l["p999_ms"]); mean.append(l["mean_ms"])
        sd.append(l["stddev_ms"]); errs.append(d["errors"])
        if not d.get("premises_passed", True): bad.extend(d.get("premise_notes", []))
    if not rps:
        print(f"  {base:<44} FAILED", flush=True); return None
    lo, hi = ci95(rps)
    med = statistics.median
    flag = "  !!PREMISE" if bad else ""
    print(f"  {base:<44} rps={med(rps):>9,.0f} [{lo:>8,.0f}-{hi:>8,.0f}] "
          f"p50={med(p50):>8.2f} p90={med(p90):>8.2f} p99={med(p99):>9.2f} "
          f"p99.9={med(p999):>9.2f} mean={med(mean):>8.2f} sd={med(sd):>8.2f} "
          f"err={sum(errs)}{flag}", flush=True)
    if bad: print(f"     {bad[0][:150]}", flush=True)
    return dict(rps=med(rps), lo=lo, hi=hi, p50=med(p50), p90=med(p90),
                p99=med(p99), p999=med(p999), mean=med(mean), sd=med(sd),
                n=len(rps), errfree=(sum(errs) == 0 and not bad))

# ---------------------------------------------------------------- phases ----
def ph_rate(stack):
    """OPEN-LOOP arrival-rate sweep. This is the saturation instrument: the
    offered rate is held independent of latency, so the queue is allowed to grow
    and the knee is observable."""
    c, ctl = TARGETS[stack]
    print(f"\n== [{stack}] A. open-loop arrival-rate sweep (Poisson) ==", flush=True)
    rows = []
    for rate in [250, 500, 1000, 2000, 4000, 8000, 16000]:
        r = replicated(stack, f"r3-{stack}-rate{rate:05d}", c, rate=rate, conc=1024, conns=128)
        if r: rows.append((rate, r))
    # TWO knees, and they are far apart. Reporting only the throughput knee is
    # misleading: the service keeps ACHIEVING the offered rate long after the tail
    # has collapsed, because a cache hit is ~2ms and a miss is ~300ms, so the
    # median stays flat while p90 explodes.
    tput_knee = None
    for rate, r in rows:
        if r["rps"] >= rate * 0.95 and r.get("errfree", True):
            tput_knee = (rate, r)
    lat_knee = None
    for i in range(1, len(rows)):
        prev, cur = rows[i-1][1], rows[i][1]
        if prev["p90"] > 0 and cur["p90"] / prev["p90"] >= 5.0:
            lat_knee = (rows[i-1][0], rows[i][0], prev["p90"], cur["p90"])
            break
    if lat_knee:
        print(f"  LATENCY KNEE [{stack}]: between {lat_knee[0]:,} and {lat_knee[1]:,} rps "
              f"offered -- p90 goes {lat_knee[2]:.2f}ms -> {lat_knee[3]:.2f}ms "
              f"({lat_knee[3]/lat_knee[2]:.0f}x) while p50 barely moves. THIS is the "
              f"operating limit.", flush=True)
    if tput_knee:
        print(f"  THROUGHPUT KNEE [{stack}]: still absorbs {tput_knee[0]:,} rps "
              f"(achieved {tput_knee[1]['rps']:,.0f}, p99 {tput_knee[1]['p99']:.2f}ms) "
              f"error-free, but well past the latency knee.", flush=True)

def ph_ctx(stack):
    c, _ = TARGETS[stack]
    print(f"\n== [{stack}] B. context size ==", flush=True)
    for b in [64, 256, 1024, 4096, 16384, 65536]:
        replicated(stack, f"r3-{stack}-ctx{b:06d}", c, ctx=b, conc=64, conns=64, dist="uniform")

def ph_cache(stack):
    """Cache configuration x traffic shape. Shape is the real lever: it decides
    the hit ratio the service actually sees."""
    c, _ = TARGETS[stack]
    print(f"\n== [{stack}] C. cache configuration x traffic distribution ==", flush=True)
    for cache in ["exact", "redis-semantic"]:
        H.redis("FLUSHALL")
        H.configure_sc(workers=16, replicas=1, cache=cache, cpu=16)
        time.sleep(4)
        for dist in ["unique", "uniform", "zipf", "hotset"]:
            replicated(stack, f"r3-{stack}-{cache}-{dist}", c, dist=dist, conc=32, conns=32)

def ph_routes(stack):
    if stack != "praxis":
        print(f"\n== [{stack}] D. route count — skipped (route table is Praxis-side) ==", flush=True); return
    print(f"\n== [praxis] D. route-table size ==", flush=True)
    c, _ = TARGETS["praxis"]
    here = os.path.dirname(os.path.abspath(__file__))
    for n in [2, 4, 8, 16, 32]:
        H.configure_sc(workers=16, replicas=1,
                       classifier=f"/work/classifiers/r2-taxonomy-{n}.json", cache="exact", cpu=16)
        H.apply_praxis_config(open(os.path.join(here,"..","round2",f"praxis-{n}.yaml")).read(), replicas=1)
        replicated("praxis", f"r3-praxis-routes{n:02d}", c, dist="zipf", conc=64, conns=64)

def ph_ab(stack):
    """The only defensible comparison: this path against ITSELF without
    classification."""
    c, ctl = TARGETS[stack]
    if not ctl:
        print(f"\n== [{stack}] E. A/B — no control listener for this stack ==", flush=True); return
    print(f"\n== [{stack}] E. classification cost (paired A/B) ==", flush=True)
    for conc in [32, 128, 512]:
        a = replicated(stack, f"r3-{stack}-ab-classified-c{conc:04d}", c, conc=conc, conns=min(conc,128), dist="zipf")
        b = replicated(stack, f"r3-{stack}-ab-control-c{conc:04d}", ctl, conc=conc, conns=min(conc,128), dist="zipf")
        if a and b and b["rps"]:
            print(f"     -> classification cost at c{conc}: {(a['rps']/b['rps']-1)*100:+.1f}% "
                  f"throughput, p99 {a['p99']:.2f} vs {b['p99']:.2f} ms", flush=True)

PHASES = {"a": ph_rate, "b": ph_ctx, "c": ph_cache, "d": ph_routes, "e": ph_ab}

if __name__ == "__main__":
    stack = sys.argv[1]
    want = sys.argv[2:] or ["a","b","c","d","e"]
    random.seed(20260903)
    print(f"ROUND 3 CAMPAIGN — stack={stack} phases={want} reps={REPS} dur={DUR}s", flush=True)
    t0=time.time()
    for p in want: PHASES[p](stack)
    print(f"\nround3/{stack} {want} done in {time.time()-t0:.0f}s", flush=True)
