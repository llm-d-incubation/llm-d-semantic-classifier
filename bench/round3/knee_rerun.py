#!/usr/bin/env python3
"""Round 3b: the knee experiment, rerun with the scheduler corrected.

The previous knee sweep is void for two reasons, both found in external review:

  1. Warmup carry-in. Workers claimed future arrival slots during warmup and
     those crossed into the measurement window, inflating every rate by exactly
     concurrency/window (1024/25s = 40.96 rps). 250 offered measured 290.9.
  2. Zipf traffic at a fixed duration confounds rate with cache warming: a
     higher-rate arm populates more of the exact cache inside the same window,
     so the hit ratio is not held constant across the sweep. That is why p90 went
     2ms -> 194ms -> 80ms -> 33ms, which no queue-saturation curve should do.

So the knee is now measured in THREE regimes with the hit ratio pinned by
construction, and Zipf is demoted to workload characterisation rather than being
the instrument for a service-rate knee.

  miss  every request novel        -> classifier COMPUTE capacity
  hit   one prewarmed key          -> service/transport capacity
  mix80 80% warm keyspace / 20% novel -> a controlled blend
"""
import json, os, statistics, subprocess, sys, time, random
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "round2"))
import harness as H

REPS, DUR = 3, 25
RATES = [250, 500, 1000, 2000, 4000, 8000]
TARGETS = {"praxis": "http://praxis.cnuland-dev.svc.cluster.local:8080",
           "llmd":   "http://ipp-envoy.cnuland-dev.svc.cluster.local:8080",
           "vsr":    "http://llm-d-sc-vsr-adapter.cnuland-dev.svc.cluster.local:8080"}
MODEL = {"praxis": "tier-small", "llmd": "tier-small", "vsr": "router-model"}
REGIMES = {
    # dist,       keyspace, hit_ratio, cache_mode
    "miss":  dict(dist="unique",  extra=["--novel-salt","MISS"]),
    "hit":   dict(dist="uniform", extra=["--cache-mode","hit","--keyspace","1"]),
    "mix80": dict(dist="uniform", extra=["--cache-mode","mixed","--hit-ratio","0.8","--keyspace","2000"]),
    # NOTE: mix80's novel fraction is drawn from a corpus region disjoint from its
    # warm slice, but that region can still have been cached by an earlier run, so
    # its effective hit ratio is >= 0.8 rather than exactly 0.8. Reported as such.
}

def ci95(xs, n=1500):
    if len(xs) < 2: return (float("nan"),)*2
    random.seed(11); m=sorted(statistics.median(random.choices(xs,k=len(xs))) for _ in range(n))
    return m[int(.025*n)], m[int(.975*n)]

def run(stack, regime, rate, rep):
    r = dict(REGIMES[regime])
    r["extra"] = [f"{stack}-{regime}-{rate}-{rep}-{int(time.time())}" if e == "MISS" else e
                  for e in r["extra"]]
    lbl = f"r3b-{stack}-{regime}-rate{rate:05d}-rep{rep}"
    a = H.KCTL+["exec","bench-driver","--","/work/bin/scbench",
        "--mode", "classify" if stack=="vsr" else "http",
        "--target",TARGETS[stack],"--rate",str(rate),"--arrival","poisson",
        "--concurrency","1024","--connections","128","--model",MODEL[stack],
        "--corpus","/work/corpus.jsonl","--dist",r["dist"],
        "--corpus-offset",str(rep*50000),
        "--warmup","3000","--duration-secs",str(DUR),
        "--run-id",str(int(time.time()*1000)%100000),"--label",lbl,
        "--out",f"{H.RESULTS}/json/{lbl}.json"] + r["extra"]
    out = H.sh(a)
    try: return json.loads(out.stdout)
    except Exception: return None

if __name__ == "__main__":
    stacks = sys.argv[1:] or ["praxis","llmd","vsr"]
    H.configure_sc(workers=16, replicas=1, cache="exact", cpu=16)
    print(f"ROUND 3b — corrected knee sweep, stacks={stacks}, {len(RATES)} rates x "
          f"{len(REGIMES)} regimes x {REPS} reps", flush=True)
    for stack in stacks:
        for regime in REGIMES:
            print(f"\n== [{stack}/{regime}] ==", flush=True)
            rows=[]
            for rate in RATES:
                reps=[run(stack,regime,rate,i) for i in range(REPS)]
                reps=[d for d in reps if d]
                if not reps:
                    print(f"  {rate:>6,} FAILED", flush=True); continue
                med=statistics.median
                comp=[d["completed_rps"] for d in reps]
                lo,hi=ci95(comp)
                offer=med([d["offer_attainment"] for d in reps])
                ok=all(d.get("premises_passed",True) for d in reps)
                L=lambda f: med([d["latency"][f] for d in reps])
                print(f"  offered={rate:>6,} completed={med(comp):>9,.1f} [{lo:>8,.1f}-{hi:>8,.1f}] "
                      f"offer_att={offer:.3f} p50={L('p50_ms'):>8.2f} p90={L('p90_ms'):>8.2f} "
                      f"p99={L('p99_ms'):>9.2f} rej={med([d['rejected_rps'] for d in reps]):>7,.1f} "
                      f"{'ok' if ok else 'PREMISE-FAIL'}", flush=True)
                rows.append((rate, med(comp), L('p50_ms'), L('p90_ms'), ok))
            # Knee: first rate where COMPLETION falls behind the OFFER (true
            # saturation), reported alongside the first big p90 step.
            sat=None
            for rate,comp,_,_,ok in rows:
                if comp < rate*0.95: sat=rate; break
            lat=None
            for i in range(1,len(rows)):
                if rows[i-1][3]>0 and rows[i][3]/rows[i-1][3]>=5.0:
                    lat=(rows[i-1][0],rows[i][0],rows[i-1][3],rows[i][3]); break
            if sat: print(f"  SATURATION [{stack}/{regime}]: completion falls behind offer at {sat:,} rps", flush=True)
            else:   print(f"  SATURATION [{stack}/{regime}]: not reached by {RATES[-1]:,} rps", flush=True)
            if lat: print(f"  LATENCY KNEE [{stack}/{regime}]: {lat[0]:,} -> {lat[1]:,} rps, p90 "
                          f"{lat[2]:.2f} -> {lat[3]:.2f} ms ({lat[3]/lat[2]:.0f}x)", flush=True)
