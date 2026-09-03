#!/usr/bin/env python3
"""Measure CLASSIFICATION COVERAGE reproducibly, and emit it as evidence.

Coverage = requests that actually reached llm-d-sc / requests the gateway served.

It is the campaign's most important metric and was, until now, computed by
tailing llm-d-sc's periodic metrics line before and after a run. That races: the
line is emitted on a timer, so a "before" read can be stale by seconds and the
delta then spans a neighbouring arm. It produced impossible values -- 137% and
150% coverage -- which is how the defect was caught.

This pins the window instead:
  * llm-d-sc logs counters every 1s (LLM_D_SC_METRICS_LOG_SECS=1);
  * both reads WAIT for a line newer than the boundary, so the delta cannot
    include traffic from outside the arm;
  * an idle settle period before the run drains any in-flight predecessor;
  * the result is written next to the arm's JSON as a coverage sidecar.
"""
import json, os, re, subprocess, sys, time
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "round2"))
import harness as H

CNT = re.compile(r"served=(\d+) hits=(\d+) misses=(\d+)")

def pod():
    return H.sh(H.KCTL+["get","pods","-l","app=llm-d-sc","-o",
                        "jsonpath={.items[0].metadata.name}"]).stdout.strip()

def counters(p):
    """Latest counter line. Returns (served, hits, misses) or None."""
    out = H.sh(H.KCTL+["logs",p,"--tail=3"]).stdout
    ms = CNT.findall(out)
    return tuple(int(x) for x in ms[-1]) if ms else None

def fresh_counters(p, timeout=8.0):
    """Wait for a counter line that is NEWER than now, so the read is bounded."""
    base = counters(p)
    t0 = time.time()
    while time.time() - t0 < timeout:
        cur = counters(p)
        if cur and (base is None or cur[0] != base[0] or cur != base):
            return cur
        time.sleep(0.4)
    return counters(p)

def run_with_coverage(label, rate, dist="unique", extra=None, dur=25, conc=1024,
                      target="http://praxis.cnuland-dev.svc.cluster.local:8080",
                      model="tier-small", mode="http"):
    p = pod()
    time.sleep(6)                      # settle: drain any predecessor
    before = fresh_counters(p)
    a = H.KCTL+["exec","bench-driver","--","/work/bin/scbench","--mode",mode,
        "--target",target,"--rate",str(rate),"--arrival","poisson",
        "--concurrency",str(conc),"--connections","128","--model",model,
        "--corpus","/work/corpus.jsonl","--dist",dist,
        "--novel-salt",f"cov-{label}-{int(time.time())}",
        "--warmup","300","--duration-secs",str(dur),
        "--run-id",str(int(time.time()*1000)%100000),"--label",label,
        "--out",f"{H.RESULTS}/json/{label}.json"] + (extra or [])
    r = H.sh(a)
    after = fresh_counters(p)
    try:
        d = json.loads(r.stdout)
    except Exception:
        return None
    if not before or not after:
        return None
    d_served = after[0]-before[0]
    d_miss   = after[2]-before[2]
    n = d["measured_requests"] + d["warmup_requests"]
    cov = d_served / n * 100 if n else 0.0
    # Sanity gate: coverage above 105% means the window was not bounded and the
    # delta captured foreign traffic. Report it as invalid rather than publish it.
    valid = 0.0 <= cov <= 105.0
    ev = dict(label=label, offered_rate_rps=rate,
              gateway_requests=n, classifier_served_delta=d_served,
              classifier_misses_delta=d_miss,
              classification_coverage_pct=round(cov, 2),
              coverage_measurement_valid=valid,
              p50_ms=d["latency"]["p50_ms"], p90_ms=d["latency"]["p90_ms"],
              p99_ms=d["latency"]["p99_ms"], errors=d["errors"])
    # Write the sidecar LOCALLY. Piping a heredoc through `kubectl exec` failed
    # silently and produced zero files, which is exactly the sort of unverified
    # step this campaign keeps getting caught by -- so it is written where it can
    # be checked immediately.
    out_dir = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "results", "json")
    os.makedirs(out_dir, exist_ok=True)
    with open(os.path.join(out_dir, f"{label}.coverage.json"), "w") as f:
        json.dump(ev, f, indent=2)
    return ev
