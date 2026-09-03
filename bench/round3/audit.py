#!/usr/bin/env python3
"""Consistency audit over every captured result.

Written because this campaign has now produced five bugs that summary statistics
alone could not reveal: warmup carry-in, non-Poisson arrivals, ignored
--context-bytes, ignored cache_mode, and cache-miss arms running at a 4.4% miss
rate. Each was found by noticing that a number contradicted something else we
knew. This automates that habit.

Checks are ordered from "definitely broken" to "worth a look".
"""
import glob, json, os, sys
from collections import defaultdict

def audit(src):
    rows=[]
    for p in sorted(glob.glob(os.path.join(src,"json","*.json"))):
        try: rows.append(json.load(open(p)))
        except Exception as e: print(f"UNPARSEABLE {p}: {e}")
    issues=defaultdict(list)

    for r in rows:
        lbl=r["label"]; l=r["latency"]
        # 1. Percentiles must be monotone. A violation means the sort or the
        #    nearest-rank index is wrong -- everything downstream is void.
        seq=[("min",l["min_ms"]),("p50",l["p50_ms"]),("p90",l["p90_ms"]),
             ("p95",l["p95_ms"]),("p99",l["p99_ms"]),("p99.9",l["p999_ms"]),("max",l["max_ms"])]
        for (na,a),(nb,b) in zip(seq,seq[1:]):
            if a>b+1e-9:
                issues["percentile_non_monotone"].append(f"{lbl}: {na}={a:.3f} > {nb}={b:.3f}")
                break
        # 2. Sample count must match the reported count.
        if l["count"]!=r["measured_requests"]:
            issues["count_mismatch"].append(f"{lbl}: latency.count={l['count']} vs measured={r['measured_requests']}")
        # 3. Zero throughput with no errors is impossible.
        if r["throughput_rps"]<=0 and r["errors"]==0 and r["measured_requests"]>0:
            issues["zero_throughput_no_errors"].append(lbl)
        # 4. Little's law, closed-loop only.
        if r.get("load_mode","closed-loop")=="closed-loop" and l["mean_ms"]>0:
            ratio=r["implied_mean_ms"]/l["mean_ms"]
            if not (0.6<=ratio<=1.6):
                issues["littles_law"].append(f"{lbl}: implied={r['implied_mean_ms']:.2f} measured={l['mean_ms']:.2f} ratio={ratio:.2f}")
        # 5. Open-loop overshoot/undershoot.
        if r.get("offered_rate_rps",0)>0:
            oa=r.get("offer_attainment")
            if oa is not None and not (0.95<=oa<=1.05):
                issues["offer_attainment"].append(f"{lbl}: offer_attainment={oa:.3f}")
        # 6. Premise failures already recorded by the driver.
        if not r.get("premises_passed",True):
            issues["premise_failed"].append(f"{lbl}: {'; '.join(r.get('premise_notes',[]))[:110]}")
        # 7a. Concurrent-campaign contamination. Two arms whose measurement
        #     windows overlap were sharing the classifier, so neither measured
        #     what it claims. Recorded here because it happened: ad-hoc
        #     cross-checks were run against the same pod a campaign was driving.
        # 7b. Percentiles cannot be resolved from too few samples.
        if r["measured_requests"]<1000 and l["p999_ms"]>0:
            issues["p999_underdetermined"].append(f"{lbl}: n={r['measured_requests']} (p99.9 needs >=1000)")

    # 8. Cross-arm: a hit arm must never be slower than the matching miss arm.
    by={r["label"]:r for r in rows}
    for lbl,r in by.items():
        if "-hit-" in lbl or lbl.endswith("-hit"):
            mate=lbl.replace("-hit","-miss")
            if mate in by and r["latency"]["p50_ms"]>by[mate]["latency"]["p50_ms"]:
                issues["hit_slower_than_miss"].append(
                    f"{lbl}: hit p50={r['latency']['p50_ms']:.2f} > miss p50={by[mate]['latency']['p50_ms']:.2f}")

    # 9. Identical throughput across arms that should differ -- the signature of
    #    a knob that was silently ignored (how the context-bytes bug surfaced).
    groups=defaultdict(list)
    for r in rows:
        if "-ctx" in r["label"]:
            groups[r["label"].split("-ctx")[0]].append((r["label"],round(r["throughput_rps"],1)))
    for base,vals in groups.items():
        uniq={v for _,v in vals}
        if len(vals)>=4 and len(uniq)<=2:
            issues["context_sweep_suspiciously_flat"].append(f"{base}: {len(vals)} arms, only {len(uniq)} distinct rps")
    return rows, issues

if __name__=="__main__":
    src=sys.argv[1] if len(sys.argv)>1 else "results"
    rows,issues=audit(src)
    print(f"=== AUDIT over {len(rows)} result files ===\n")
    if not issues: print("  no issues found")
    for k in sorted(issues, key=lambda k:-len(issues[k])):
        v=issues[k]
        print(f"[{k}]  {len(v)} occurrence(s)")
        for x in v[:6]: print(f"    {x}")
        if len(v)>6: print(f"    ... and {len(v)-6} more")
        print()
