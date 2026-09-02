#!/usr/bin/env python3
"""Round 3, priority 1: RAYON_NUM_THREADS x LLM_D_SC_INFERENCE_WORKERS, replicated.

Rounds 1 and 2 varied executor width and the CPU limit together while Rayon's
intra-op pool silently tracked the CPU limit -- three variables at once. This
controls all three explicitly and REPLICATES each cell, because a single 20-second
run is one experiment however many requests it contains.

Reports median and a bootstrap 95% CI over independent repetitions, with the
repetition ORDER randomised so drift or thermal effects do not align with the
matrix.
"""
import json, random, statistics, subprocess, sys, time, os
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "round2"))
import harness as H

CPU = 16            # held constant across every cell
REPS = 5
DUR  = 35

def configure(workers, rayon):
    env = [
        {"name":"LLM_D_SC_INFERENCE_WORKERS","value":str(workers)},
        {"name":"LLM_D_SC_CLASSIFIER","value":"complexity"},
        {"name":"LLM_D_SC_CACHE","value":"exact"},
        {"name":"LLM_D_SC_MODEL_DIR","value":"/work/models"},
        {"name":"LLM_D_SC_LISTEN","value":"0.0.0.0:50051"},
        {"name":"RAYON_NUM_THREADS","value":str(rayon)},
    ]
    patch={"spec":{"template":{"spec":{"containers":[{
        "name":"llm-d-sc","env":env,
        "resources":{"requests":{"cpu":str(CPU),"memory":"8Gi"},
                     "limits":{"cpu":str(CPU),"memory":"32Gi"}}}]}}}}
    H.sh(H.KCTL+["patch","deployment","llm-d-sc","--type","strategic","-p",json.dumps(patch)])
    H.sh(H.KCTL+["rollout","status","deployment/llm-d-sc","--timeout=420s"], t=480)
    time.sleep(5)

def one(label, conc, rep=0):
    """One repetition. Corpus/unique = every request novel: the pure forward path,
    which is what Rayon actually affects."""
    a = H.KCTL+["exec","bench-driver","--","/work/bin/scbench","--mode","grpc",
        "--target",H.SC_SVC,"--concurrency",str(conc),"--connections",str(min(conc,64)),
        "--corpus","/work/corpus.jsonl","--dist","unique",
        # Disjoint slice per repetition. Repetitions share a process and so a warm
        # L1 cache; overlapping slices make every repetition after the first a
        # cache-HIT measurement, which showed up as a CI of [491, 154948].
        "--corpus-offset",str(rep*30000),
        "--warmup","800","--duration-secs",str(DUR),
        "--run-id",str(int(time.time()*1000)%100000),"--label",label,
        "--out",f"{H.RESULTS}/json/{label}.json"]
    r=H.sh(a)
    try:
        return json.loads(r.stdout)
    except Exception:
        return None

def ci95(xs, n=2000):
    """Bootstrap 95% CI of the median -- no normality assumption, which matters
    because these distributions are strongly bimodal."""
    if len(xs) < 2: return (float('nan'), float('nan'))
    meds=[]
    for _ in range(n):
        meds.append(statistics.median(random.choices(xs, k=len(xs))))
    meds.sort()
    return meds[int(0.025*n)], meds[int(0.975*n)]

if __name__ == "__main__":
    WORKERS = [1, 4, 16]
    RAYON   = [1, 4]
    conc    = int(sys.argv[1]) if len(sys.argv)>1 else 32
    cells = [(w, rt) for w in WORKERS for rt in RAYON]
    # Randomised cell order: drift must not align with the matrix.
    random.seed(20260903); random.shuffle(cells)
    print(f"== R3 Rayon x workers, CPU={CPU} fixed, conc={conc}, {REPS} reps/cell, "
          f"corpus/unique ==", flush=True)
    print(f"   {len(cells)} cells x {REPS} reps = {len(cells)*REPS} runs\n", flush=True)
    results={}
    for (w, rt) in cells:
        configure(w, rt)
        rps, p50, p99 = [], [], []
        for rep in range(REPS):
            d = one(f"r3-w{w:02d}-rt{rt:02d}-rep{rep}", conc, rep)
            if d:
                rps.append(d['throughput_rps']); p50.append(d['latency']['p50_ms']); p99.append(d['latency']['p99_ms'])
        if not rps: continue
        lo,hi = ci95(rps)
        results[(w,rt)] = dict(rps=statistics.median(rps), lo=lo, hi=hi,
                               p50=statistics.median(p50), p99=statistics.median(p99), n=len(rps))
        print(f"  W{w:<3} RT{rt:<3} rps={statistics.median(rps):>8,.1f} "
              f"[95% CI {lo:>8,.1f} - {hi:>8,.1f}]  p50={statistics.median(p50):>8.2f}ms "
              f"p99={statistics.median(p99):>8.2f}ms  n={len(rps)}", flush=True)
    print("\n== matrix (median req/s) ==", flush=True)
    print("  W\\RT " + "".join(f"{rt:>12}" for rt in RAYON), flush=True)
    for w in WORKERS:
        row = "".join(f"{results[(w,rt)]['rps']:>12,.1f}" if (w,rt) in results else f"{'-':>12}" for rt in RAYON)
        print(f"  {w:<5}" + row, flush=True)
    best = max(results.items(), key=lambda kv: kv[1]['rps'])
    print(f"\n  BEST: W{best[0][0]} RT{best[0][1]} -> {best[1]['rps']:,.1f} req/s "
          f"[{best[1]['lo']:,.1f} - {best[1]['hi']:,.1f}]", flush=True)
    json.dump({f"w{k[0]}_rt{k[1]}":v for k,v in results.items()},
              open("rayon_matrix_results.json","w"), indent=2)
