#!/usr/bin/env python3
"""Capture auditable same-node and cross-node ClusterIP observations.

This is intentionally a low-rate evidence arm, not a throughput benchmark.  The
normal campaign's high-capacity driver stays off the target node; here two tiny,
otherwise identical observers establish the placement fact that was left open by
v0.2.  A run fails closed if the observed scheduler placement is not the arm
named in its result.
"""
import datetime as dt
import json
import os
from pathlib import Path
import subprocess
import sys
import time

NAMESPACE = os.environ.get("NS", "cnuland-dev")
KUBECTL = os.environ.get("KUBECTL", "kubectl")
TARGET = f"llm-d-sc.{NAMESPACE}.svc.cluster.local:50051"
REMOTE_RESULTS = "/work/results"
OUT = Path(os.environ.get(
    "PLACEMENT_EVIDENCE_OUT",
    str(Path(__file__).resolve().parent / "results/json/placement-evidence.json"),
))
ARMS = (("same-node", "bench-placement-same"), ("cross-node", "bench-placement-cross"))


def run(args, *, input_text=None):
    completed = subprocess.run(
        args, input=input_text, text=True, stdout=subprocess.PIPE,
        stderr=subprocess.PIPE, check=False,
    )
    if completed.returncode:
        raise RuntimeError(
            f"command failed ({completed.returncode}): {' '.join(args)}\\n"
            f"{completed.stderr.strip()}"
        )
    return completed.stdout


def kctl(*args):
    return run([KUBECTL, "-n", NAMESPACE, *args])


def pod(name):
    return json.loads(kctl("get", "pod", name, "-o", "json"))


def node_record(name):
    item = json.loads(kctl("get", "node", name, "-o", "json"))
    labels = item.get("metadata", {}).get("labels", {})
    return {
        "name": item["metadata"]["name"],
        "uid": item["metadata"]["uid"],
        "instance_type": labels.get("node.kubernetes.io/instance-type")
        or labels.get("beta.kubernetes.io/instance-type"),
    }


def pod_record(item):
    statuses = item.get("status", {}).get("containerStatuses", [])
    node = item.get("spec", {}).get("nodeName")
    return {
        "name": item["metadata"]["name"],
        "uid": item["metadata"]["uid"],
        "node": node,
        "node_identity": node_record(node) if node else None,
        "pod_ip": item.get("status", {}).get("podIP"),
        "host_ip": item.get("status", {}).get("hostIP"),
        "image_ids": {s["name"]: s.get("imageID") for s in statuses},
    }


def ready_target():
    items = json.loads(kctl("get", "pods", "-l", "app=llm-d-sc", "-o", "json"))["items"]
    ready = []
    for item in items:
        states = item.get("status", {}).get("containerStatuses", [])
        if states and all(s.get("ready") for s in states):
            ready.append(item)
    if len(ready) != 1:
        raise RuntimeError(f"placement evidence needs exactly one Ready target pod, observed {len(ready)}")
    endpoints = kctl("get", "endpoints", "llm-d-sc", "-o", "jsonpath={.subsets[*].addresses[*].ip}").split()
    if len(endpoints) != 1 or endpoints[0] != ready[0].get("status", {}).get("podIP"):
        raise RuntimeError(
            "placement evidence needs exactly one Service endpoint for the Ready target; "
            f"observed {endpoints}"
        )
    return pod_record(ready[0]), endpoints[0]


def wait_ready(name):
    run([KUBECTL, "-n", NAMESPACE, "wait", "--for=condition=Ready", f"pod/{name}", "--timeout=180s"])
    return pod_record(pod(name))


def require_placement(arm, observer, target):
    same = observer["node"] == target["node"]
    if not observer["node"] or not target["node"]:
        raise RuntimeError(f"{arm}: scheduler has not assigned a node")
    if (arm == "same-node") != same:
        raise RuntimeError(
            f"{arm}: observed observer={observer['node']} target={target['node']}; refusing to label this run"
        )


def scbench(observer, arm):
    label = f"placement-{arm}"
    run_id = int(time.time_ns() % 2_000_000_000)
    result_path = f"{REMOTE_RESULTS}/json/{label}.json"
    raw_path = f"{REMOTE_RESULTS}/raw/{label}.csv"
    output = kctl(
        "exec", observer, "--", "/work/bin/scbench",
        "--mode", "grpc", "--target", TARGET,
        "--concurrency", "1", "--connections", "1",
        "--cache-mode", "hit", "--context-bytes", "256",
        "--keyspace", "1", "--warmup", "200", "--requests", "200",
        "--run-id", str(run_id), "--label", label,
        "--out", result_path, "--raw", raw_path,
    )
    try:
        summary = json.loads(output)
    except json.JSONDecodeError as exc:
        raise RuntimeError(f"{arm}: scbench did not emit JSON: {output[:400]}") from exc
    if summary.get("errors"):
        raise RuntimeError(f"{arm}: scbench reported {summary['errors']} errors")
    return {"arguments": {"target": TARGET, "concurrency": 1, "connections": 1,
                           "cache_mode": "hit", "context_bytes": 256,
                           "keyspace": 1, "warmup": 200, "requests": 200,
                           "run_id": run_id},
            "summary": summary, "raw_samples": raw_path, "summary_path": result_path}


def revision():
    return run(["git", "rev-parse", "HEAD"]).strip()


def main():
    target, endpoint = ready_target()
    observations = []
    for arm, observer_name in ARMS:
        observer = wait_ready(observer_name)
        require_placement(arm, observer, target)
        observations.append({"arm": arm, "observer": observer, "result": scbench(observer_name, arm)})
    evidence = {
        "schema_version": 1,
        "generated_at": dt.datetime.now(dt.timezone.utc).isoformat(),
        "git_revision": revision(),
        "namespace": NAMESPACE,
        "service": {"name": "llm-d-sc", "target": TARGET, "endpoint_ip": endpoint},
        "target": target,
        "observations": observations,
        "scope": "Low-rate ClusterIP placement evidence; not a replacement for the high-load campaign.",
    }
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(json.dumps(evidence, indent=2) + "\\n")
    print(f"[placement-evidence] wrote {OUT}")


if __name__ == "__main__":
    try:
        main()
    except RuntimeError as exc:
        print(f"[placement-evidence] FAIL: {exc}", file=sys.stderr)
        sys.exit(1)
