#!/usr/bin/env python3
"""Generate the statistical report from captured campaign JSON.

Methodology rule 6: published numbers are GENERATED from the JSON. Nothing here
is transcribed by hand, so a figure in the report always has a raw file behind
it. Run after a campaign; point --src at the pulled results directory.
"""
import argparse, glob, json, os

def load(src):
    rows = []
    for p in sorted(glob.glob(os.path.join(src, "json", "*.json"))):
        try:
            rows.append(json.load(open(p)))
        except Exception:
            pass
    return rows

def fmt(n, d=0):
    return f"{n:,.{d}f}"

def table(rows, keycol, keyfn, title, note=""):
    """One dimension's table, in the units a network engineer reads:
    sessions/sec and sessions/min alongside the full latency distribution."""
    out = [f"\n### {title}\n"]
    if note:
        out.append(note + "\n")
    out.append(f"| {keycol} | req/s | req/min | p50 ms | p90 ms | p95 ms | p99 ms | p99.9 ms | max ms | mean ms | stddev ms | errors | err % |")
    out.append("|---:" * 13 + "|")
    for r in rows:
        l = r["latency"]
        out.append(
            f"| {keyfn(r)} | {fmt(r['throughput_rps'])} | {fmt(r['throughput_rpm'])} | "
            f"{l['p50_ms']:.3f} | {l['p90_ms']:.3f} | {l['p95_ms']:.3f} | {l['p99_ms']:.3f} | "
            f"{l['p999_ms']:.3f} | {l['max_ms']:.3f} | {l['mean_ms']:.3f} | {l['stddev_ms']:.3f} | "
            f"{r['errors']:,} | {r['error_rate_pct']:.3f} |")
    return "\n".join(out) + "\n"

def sel(rows, prefix):
    return sorted([r for r in rows if r["label"].startswith(prefix)], key=lambda r: r["label"])

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--src", default="results")
    ap.add_argument("--out", default="STATISTICAL-REPORT.md")
    a = ap.parse_args()
    rows = load(a.src)
    doc = ["# llm-d-sc v0.2-staging — statistical report", ""]
    doc.append(f"Generated from {len(rows)} captured runs. Every row is one measured arm; "
               "raw per-request samples for each are in `results/raw/<label>.csv`.")
    doc.append("")
    doc.append("Latency is wall-clock round-trip time measured at the driver, so it includes "
               "network transit, transport framing, admission, queue wait and service. "
               "Percentiles are nearest-rank, matching `llm-d-sc/src/bench.rs::percentile`. "
               "Mean and standard deviation are reported ALONGSIDE the distribution, never "
               "instead of it.")

    if sel(rows, "c1-"):
        doc.append(table(sel(rows, "c1-"), "offered concurrency",
                         lambda r: r["concurrency"],
                         "C1 — Offered-load ladder (1 replica, 4 workers, cache-hit, 256 B)",
                         "Closed-loop: `offered concurrency` is the number of simultaneously "
                         "outstanding requests, i.e. the session depth the service is asked to hold."))
    if sel(rows, "c2-"):
        for m in ("hit", "miss"):
            r2 = [r for r in sel(rows, "c2-") if r["cache_mode"] == m]
            if r2:
                doc.append(table(r2, "executor workers",
                                 lambda r: r["label"].split("-")[1].lstrip("w"),
                                 f"C2 — Vertical scale: executor workers ({m} path)",
                                 "CPU limit tracks worker count, so this is worker width, not CPU starvation."))
    if sel(rows, "c3-"):
        for m in ("hit", "miss"):
            r3 = [r for r in sel(rows, "c3-") if r["cache_mode"] == m]
            if r3:
                doc.append(table(r3, "replicas",
                                 lambda r: r["label"].split("-")[1].lstrip("r"),
                                 f"C3 — Horizontal scale: replicas ({m} path)"))
    if sel(rows, "c4-"):
        for m in ("hit", "miss"):
            r4 = [r for r in sel(rows, "c4-") if r["cache_mode"] == m]
            if r4:
                doc.append(table(r4, "context bytes", lambda r: r["context_bytes"],
                                 f"C4 — Context-window sensitivity ({m} path)"))
    if sel(rows, "c5-"):
        for m in ("hit", "miss"):
            r5 = [r for r in sel(rows, "c5-") if r["cache_mode"] == m]
            if r5:
                doc.append(table(r5, "taxonomy", lambda r: r["label"].split("-")[1],
                                 f"C5 — Route/taxonomy size ({m} path)"))
    if sel(rows, "c7-"):
        for m in ("miss", "hit"):
            r7 = [r for r in sel(rows, "c7-") if r["cache_mode"] == m]
            if r7:
                doc.append(table(r7, "anchors / cache",
                                 lambda r: r["label"].replace("c7-a", "").replace("-" + m, ""),
                                 f"C7 — Route count: synthetic taxonomies, 48-2000 anchors ({m} path)",
                                 "Ranking is anchor-topk-mean: one cosine similarity per ANCHOR. "
                                 "This sweep asks whether ranking cost ever becomes a meaningful "
                                 "fraction of a request, which is the precondition for the semantic "
                                 "cache to have anything worth saving."))
    if sel(rows, "c6-"):
        doc.append(table(sel(rows, "c6-"), "arm", lambda r: r["label"].replace("c6-", ""),
                         "C6 — Semantic cache: exact vs redis-semantic",
                         "`ks` is the key space: how many DISTINCT prompts the workload cycles "
                         "through, i.e. its repetition rate."))
    open(a.out, "w").write("\n".join(doc) + "\n")
    print(f"wrote {a.out} from {len(rows)} runs")

if __name__ == "__main__":
    main()
