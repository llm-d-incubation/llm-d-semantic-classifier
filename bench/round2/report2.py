#!/usr/bin/env python3
"""Round-2 statistical report: Praxis and llm-d, generated from captured JSON.

Rule 6: published numbers are GENERATED, never transcribed. Every table below is
built from results/json/*.json, so any figure in prose has a raw file behind it.
"""
import argparse, glob, json, os

def load(src):
    out = []
    for p in sorted(glob.glob(os.path.join(src, "json", "*.json"))):
        try:
            out.append(json.load(open(p)))
        except Exception:
            pass
    return out

def sel(rows, pref):
    return sorted([r for r in rows if r["label"].startswith(pref)],
                  key=lambda r: r["label"])

def table(rows, keycol, keyfn, title, note=""):
    o = [f"\n### {title}\n"]
    if note: o.append(note + "\n")
    hdr = (f"| {keycol} | req/s | req/min | p50 ms | p90 ms | p95 ms | p99 ms | p99.9 ms "
           f"| max ms | mean ms | stddev ms | errors | err % |")
    o.append(hdr); o.append("|---:" * 13 + "|")
    for r in rows:
        l = r["latency"]
        o.append(f"| {keyfn(r)} | {r['throughput_rps']:,.0f} | {r['throughput_rpm']:,.0f} | "
                 f"{l['p50_ms']:.3f} | {l['p90_ms']:.3f} | {l['p95_ms']:.3f} | {l['p99_ms']:.3f} | "
                 f"{l['p999_ms']:.3f} | {l['max_ms']:.3f} | {l['mean_ms']:.3f} | {l['stddev_ms']:.3f} | "
                 f"{r['errors']:,} | {r['error_rate_pct']:.3f} |")
    return "\n".join(o) + "\n"

def paired(rows, a_pref, b_pref, a_name, b_name, title, keyfn):
    """A/B table -- the only honest way to state a 'cost of X'."""
    A = {keyfn(r): r for r in sel(rows, a_pref)}
    B = {keyfn(r): r for r in sel(rows, b_pref)}
    keys = [k for k in A if k in B]
    if not keys: return ""
    o = [f"\n### {title}\n",
         f"| key | {a_name} req/s | {b_name} req/s | cost | {a_name} p99 | {b_name} p99 |",
         "|---:|---:|---:|---:|---:|---:|"]
    for k in keys:
        a, b = A[k], B[k]
        cost = (a["throughput_rps"] / b["throughput_rps"] - 1) * 100 if b["throughput_rps"] else 0
        o.append(f"| {k} | {a['throughput_rps']:,.0f} | {b['throughput_rps']:,.0f} | "
                 f"{cost:+.1f} % | {a['latency']['p99_ms']:.2f} ms | {b['latency']['p99_ms']:.2f} ms |")
    return "\n".join(o) + "\n"

def knee_line(rows, name):
    rows = sorted(rows, key=lambda r: r["concurrency"])
    best = None
    for i in range(1, len(rows)):
        if rows[i]["throughput_rps"] / rows[i-1]["throughput_rps"] - 1 >= 0.20:
            best = rows[i]
    peak = max(rows, key=lambda r: r["throughput_rps"]) if rows else None
    s = ""
    if best:
        s += (f"* **{name} knee:** concurrency {best['concurrency']} -> "
              f"{best['throughput_rps']:,.0f} req/s ({best['throughput_rpm']:,.0f} req/min), "
              f"p99 {best['latency']['p99_ms']:.2f} ms\n")
    if peak:
        s += (f"* **{name} peak:** concurrency {peak['concurrency']} -> "
              f"{peak['throughput_rps']:,.0f} req/s ({peak['throughput_rpm']:,.0f} req/min)\n")
    return s

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--src", default="../results")
    ap.add_argument("--out", default="ROUND2-STATISTICAL-REPORT.md")
    a = ap.parse_args()
    rows = load(a.src)
    d = ["# Round 2 — statistical report (Praxis and llm-d)", "",
         f"Generated from {len(rows)} captured runs. Raw per-request samples are retained "
         "alongside each arm.", "",
         "Latency is wall-clock round-trip time at the driver: it includes network transit, "
         "transport framing, gateway processing, any classification hop, and the backend. "
         "Percentiles are nearest-rank, matching `src/bench.rs::percentile`. Mean and standard "
         "deviation are reported alongside the distribution, never instead of it.", ""]

    d.append("## Knees and peaks\n")
    for pref, name in [("p1-classified-", "Praxis classified"), ("p1-control-", "Praxis control"),
                       ("l1-llmd-", "llm-d gateway"), ("l1-direct-", "backend direct")]:
        r = sel(rows, pref)
        if r: d.append(knee_line(r, name))

    d.append(paired(rows, "p1-classified-", "p1-control-", "classified", "control",
                    "P1 — Praxis: cost of classification across the ladder",
                    lambda r: r["concurrency"]))
    d.append(paired(rows, "l1-llmd-", "l1-direct-", "llm-d", "direct",
                    "L1 — llm-d: cost of the gateway across the ladder",
                    lambda r: r["concurrency"]))

    for pref, title, keyf, note in [
        ("p1-", "P1 — Praxis offered-load ladder", lambda r: r["label"].replace("p1-", ""), ""),
        ("p2-", "P2 — Praxis context size", lambda r: r["label"].replace("p2-", ""), ""),
        ("p3-", "P3 — Praxis route-table size (2..32 clusters)",
         lambda r: r["label"].replace("p3-", ""),
         "Route-table size is the number of clusters the gateway selects between. "
         "All clusters point at the same backend, so only the table size varies."),
        ("p4-", "P4 — Praxis cache configuration x workload",
         lambda r: r["label"].replace("p4-", ""), ""),
        ("p5-", "P5 — Praxis gateway horizontal scale",
         lambda r: r["label"].replace("p5-", ""), ""),
        ("l1-", "L1 — llm-d offered-load ladder", lambda r: r["label"].replace("l1-", ""), ""),
        ("l2-", "L2 — llm-d context size", lambda r: r["label"].replace("l2-", ""), ""),
        ("l3-", "L3 — llm-d InferencePool size",
         lambda r: r["label"].replace("l3-", ""),
         "How many endpoints the EPP selects between."),
        ("l4-", "L4 — llm-d soak", lambda r: r["label"].replace("l4-", ""), ""),
        ("v1-", "V1 — vLLM Semantic Router adapter (http_classify), cached",
         lambda r: r["label"].replace("v1-vsr-cached-", ""),
         "llm-d-sc served over vLLM SR's `http_classify` contract. Every response is "
         "validated to carry the declared labels with scores summing to ~1.0 -- the "
         "router rejects anything else, so an unnormalised 200 is counted as a failure."),
        ("v2-", "V2 — vLLM Semantic Router adapter, novel prompts",
         lambda r: r["label"].replace("v2-vsr-novel-", ""), ""),
    ]:
        rr = sel(rows, pref)
        if rr: d.append(table(rr, "arm", keyf, title, note))

    open(a.out, "w").write("\n".join(d) + "\n")
    print(f"wrote {a.out} from {len(rows)} runs")

if __name__ == "__main__":
    main()
