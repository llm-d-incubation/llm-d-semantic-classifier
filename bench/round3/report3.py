#!/usr/bin/env python3
"""Round 3 statistical report — generated from captured JSON, with CIs.

Differences from rounds 1-2, all of them consequences of the methodology review:

  * arms are REPLICATED, so every figure is a median over independent runs with a
    bootstrap 95% CI rather than a single observation;
  * arrival-rate arms are OPEN-LOOP, so offered and achieved rate are separate
    columns and the gap is itself the finding;
  * traffic is a frozen 200k-utterance corpus, so cache hits and misses coexist
    the way they do in production;
  * every arm carries its premise assertions, and any arm that failed them is
    reported as FAILED rather than silently included.
"""
import argparse, glob, json, os, statistics
from collections import defaultdict

def load(src):
    out = []
    for p in sorted(glob.glob(os.path.join(src, "json", "*.json"))):
        try:
            d = json.load(open(p))
            if d["label"].startswith("r3-"):
                out.append(d)
        except Exception:
            pass
    return out

def group(rows):
    """Collapse `<arm>-repN` into one entry with a median and a bootstrap CI."""
    g = defaultdict(list)
    for r in rows:
        base = r["label"].rsplit("-rep", 1)[0]
        g[base].append(r)
    return g

def ci95(xs, n=1500):
    import random
    if len(xs) < 2: return (float("nan"), float("nan"))
    random.seed(7)
    m = sorted(statistics.median(random.choices(xs, k=len(xs))) for _ in range(n))
    return m[int(.025*n)], m[int(.975*n)]

def table(g, prefix, title, keyfn, note=""):
    keys = sorted(k for k in g if k.startswith(prefix))
    if not keys: return ""
    o = [f"\n### {title}\n"]
    if note: o.append(note + "\n")
    o.append("| arm | n | req/s (median) | 95% CI | p50 ms | p90 ms | p95 ms | p99 ms | "
             "p99.9 ms | max ms | mean ms | stddev ms | errors | premises |")
    o.append("|---|--:|--:|:--|--:|--:|--:|--:|--:|--:|--:|--:|--:|:--|")
    med = statistics.median
    for k in keys:
        reps = g[k]
        rps = [r["throughput_rps"] for r in reps]
        lo, hi = ci95(rps)
        L = lambda f: med([r["latency"][f] for r in reps])
        errs = sum(r["errors"] for r in reps)
        okp = all(r.get("premises_passed", True) for r in reps)
        # p99.9 needs ~1000 samples to mean anything. Printing a number from 98
        # samples invites it to be quoted; the audit found 18 such arms.
        n_req = med([r["measured_requests"] for r in reps])
        p999 = f"{L('p999_ms'):.2f}" if n_req >= 1000 else "n/a"
        o.append(f"| {keyfn(k)} | {len(reps)} | {med(rps):,.0f} | "
                 f"{lo:,.0f}–{hi:,.0f} | {L('p50_ms'):.2f} | {L('p90_ms'):.2f} | "
                 f"{L('p95_ms'):.2f} | {L('p99_ms'):.2f} | {p999} | "
                 f"{L('max_ms'):.2f} | {L('mean_ms'):.2f} | {L('stddev_ms'):.2f} | "
                 f"{errs:,} | {'ok' if okp else '**FAILED**'} |")
    return "\n".join(o) + "\n"

def knees(g, stack):
    """Latency knee vs throughput knee. Reporting only the second is how a
    saturated service gets described as healthy."""
    rows = []
    for k in sorted(g):
        if not k.startswith(f"r3-{stack}-rate"): continue
        rate = int(k.split("rate")[1])
        reps = g[k]
        rows.append((rate,
                     statistics.median(r["throughput_rps"] for r in reps),
                     statistics.median(r["latency"]["p50_ms"] for r in reps),
                     statistics.median(r["latency"]["p90_ms"] for r in reps),
                     statistics.median(r["latency"]["p99_ms"] for r in reps),
                     sum(r["errors"] for r in reps)))
    if not rows: return ""
    rows.sort()
    o = [f"\n**{stack} — open-loop arrival-rate sweep**\n",
         "| offered rps | achieved rps | p50 ms | p90 ms | p99 ms | errors |",
         "|--:|--:|--:|--:|--:|--:|"]
    for rate, ach, p50, p90, p99, e in rows:
        o.append(f"| {rate:,} | {ach:,.0f} | {p50:.2f} | {p90:.2f} | {p99:.2f} | {e:,} |")
    lat = None
    for i in range(1, len(rows)):
        if rows[i-1][3] > 0 and rows[i][3] / rows[i-1][3] >= 5.0:
            lat = (rows[i-1][0], rows[i][0], rows[i-1][3], rows[i][3]); break
    tput = None
    for rate, ach, *_rest, e in rows:
        if ach >= rate * 0.95 and e == 0: tput = (rate, ach)
    if lat:
        o.append(f"\n* **Latency knee: between {lat[0]:,} and {lat[1]:,} rps offered** — "
                 f"p90 goes {lat[2]:.2f} → {lat[3]:.2f} ms ({lat[3]/lat[2]:.0f}×) while p50 "
                 f"barely moves. This is the operating limit.")
    if tput:
        o.append(f"* Throughput knee: absorbs {tput[0]:,} rps error-free "
                 f"({tput[1]:,.0f} achieved) — far past the latency knee, and misleading "
                 f"on its own.")
    return "\n".join(o) + "\n"

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--src", default="../results")
    ap.add_argument("--out", default="ROUND3-STATISTICAL-REPORT.md")
    a = ap.parse_args()
    rows = load(a.src); g = group(rows)
    d = ["# Round 3 — statistical report", "",
         f"Generated from {len(rows)} captured runs across {len(g)} distinct arms, "
         "each replicated. Every figure is a median over independent repetitions with a "
         "bootstrap 95% CI. `p99.9` reads `n/a` where an arm has fewer than 1,000 "
         "measured requests, because it cannot be resolved from that sample.", "",
         "**Traffic:** frozen 200,000-utterance corpus (seed `20260903`), 12 domains, "
         "2 B–18 KB. **Config:** llm-d-sc at 16 workers / `RAYON_NUM_THREADS=1` / 16 CPU — "
         "Rayon pinned rather than left to track the CPU limit. **Arrival-rate arms are "
         "OPEN-LOOP** (Poisson), so offered rate is independent of response latency.", ""]
    d.append("## Knees\n")
    for s in ["praxis", "llmd", "vsr"]:
        d.append(knees(g, s))
    for s, name in [("praxis","Praxis + llm-d-sc"), ("llmd","llm-d IPP + llm-d-sc"),
                    ("vsr","vLLM SR adapter + llm-d-sc")]:
        d.append(f"\n## {name}\n")
        d.append(table(g, f"r3-{s}-rate", f"{name} — open-loop arrival rate",
                       lambda k: k.split("rate")[1].lstrip("0") + " rps offered"))
        d.append(table(g, f"r3-{s}-ctx", f"{name} — context size",
                       lambda k: k.split("ctx")[1].lstrip("0") + " B"))
        d.append(table(g, f"r3-{s}-exact", f"{name} — cache: exact, by traffic shape",
                       lambda k: k.split("exact-")[1]))
        d.append(table(g, f"r3-{s}-redis-semantic", f"{name} — cache: redis-semantic, by traffic shape",
                       lambda k: k.split("redis-semantic-")[1]))
        d.append(table(g, f"r3-{s}-routes", f"{name} — route-table size",
                       lambda k: k.split("routes")[1] + " routes"))
        d.append(table(g, f"r3-{s}-ab", f"{name} — classification cost (paired A/B)",
                       lambda k: k.split("ab-")[1]))
    open(a.out, "w").write("\n".join(x for x in d if x) + "\n")
    print(f"wrote {a.out}: {len(rows)} runs, {len(g)} arms")

if __name__ == "__main__":
    main()
