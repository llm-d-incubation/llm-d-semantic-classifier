#!/usr/bin/env python3
"""Generate a frozen, diverse utterance corpus for round 3.

Rounds 1 and 2 drove every arm with one filler sentence repeated to a target byte
length. That is excellent for deterministic tokenizer and context-size control and
useless as traffic: it exercises a single region of the embedding space, so
per-request inference variance -- the thing that actually drives queue behaviour --
never appears.

This emits a FROZEN corpus (fixed seed, committed to the repo) spanning the
domains a semantic router would really see, at realistic length variation. Frozen
matters: a corpus that changes between runs makes two campaigns incomparable.

Output: corpus.jsonl -- {"id", "domain", "expect", "bytes", "text"}
"""
import json, os, random, sys

SEED = 20260903
random.seed(SEED)
OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "corpus.jsonl")

# (domain, expected complexity tier, templates). `expect` is a HINT for
# distribution analysis, not ground truth -- the classifier's own taxonomy is
# authoritative and disagreement is itself a finding.
DOMAINS = {
"networking": ("MEDIUM", [
 "Why is BGP session flapping between {a} and {b} after the MTU change?",
 "Explain the difference between {a} and {b} for east-west traffic in a leaf-spine fabric.",
 "Our p99 latency doubled after enabling {a}; what would you check first?",
 "How do I read a tcpdump showing repeated TCP retransmits to {a}?",
 "What causes asymmetric routing when {a} and {b} advertise the same prefix?"]),
"kubernetes": ("MEDIUM", [
 "A pod is stuck in CrashLoopBackOff with exit code {n}; how do I diagnose it?",
 "Explain how {a} differs from {b} for scheduling GPU workloads.",
 "Why would a readiness probe pass while the service returns 503?",
 "How does the scheduler treat a pod with {n} CPU requests on a node at capacity?",
 "What happens to in-flight requests during a rolling update of {a}?"]),
"security": ("COMPLEX", [
 "Design a zero-trust access model for {a} spanning {b} and on-prem workloads.",
 "How should we rotate {a} credentials without downtime across {n} services?",
 "What is the blast radius if {a} is compromised in a shared {b} cluster?",
 "Review this policy: allow {a} to read {b} secrets in any namespace."]),
"code_small": ("SIMPLE", [
 "Write a function to reverse a string in {a}.",
 "What does the {a} keyword do in {b}?",
 "How do I sort a list of dicts by key in {a}?",
 "Fix this: for i in range(len(x)): print(x[i][{n}])",
 "What is the difference between == and === in {a}?"]),
"code_large": ("COMPLEX", [
 "Design a {a} service that ingests {n}k events/sec and writes to {b} with exactly-once semantics.",
 "Refactor a monolith into services around {a} and {b} without a big-bang cutover.",
 "Architect a multi-tenant {a} platform with per-tenant isolation and audit logging.",
 "How would you build a {a} pipeline that survives {b} partition and replays cleanly?"]),
"reasoning": ("REASONING", [
 "Prove that the sum of the first n odd integers equals n squared.",
 "Derive the worst-case complexity of {a} using the Master theorem, justifying each step.",
 "If {n} machines each fail independently with probability p, derive the expected time to first failure.",
 "Show that {a} is NP-complete by reduction from {b}."]),
"general_qa": ("SIMPLE", [
 "What is the capital of {a}?",
 "Convert {n} kilometres to miles.",
 "Who wrote the book {a}?",
 "What day comes after {a}?",
 "How many minutes are in {n} hours?"]),
"troubleshooting": ("MEDIUM", [
 "Disk usage on {a} jumped to {n}% overnight; where do I start?",
 "Requests to {a} intermittently return 502 but the backend logs look clean.",
 "Memory climbs steadily and never drops on {a}; is it a leak?",
 "The {a} job succeeds locally and fails in CI with exit {n}."]),
"conversation": ("SIMPLE", [
 "thanks, that helped",
 "can you rephrase that more simply?",
 "no, I meant the other one",
 "ok what about {a}?",
 "hmm, still not working"]),
"malformed": ("SIMPLE", [
 "{{{{", "  ", "SELECT * FROM;;;", "\\x00\\x01 binary-ish", "????????",
 '{"unterminated": ', "<<<<<<< HEAD"]),
"multilingual": ("MEDIUM", [
 "¿Cómo configuro el balanceo de carga en {a}?",
 "Wie behebe ich einen Speicherfehler in {a}?",
 "{a} のレイテンシが高いのはなぜですか?",
 "Comment déboguer une fuite mémoire dans {a} ?",
 "如何优化 {a} 的查询性能?"]),
"tool_json": ("MEDIUM", [
 '{"tool":"search","args":{"q":"{a}","limit":{n}}}',
 '{"function":"delete_resource","args":{"name":"{a}","force":true}}',
 'call {a} with parameters {{"target": "{b}", "retries": {n}}}']),
}
FILL_A = ["Envoy","Istio","Postgres","Kafka","Redis","Rust","Go","Python","Cilium","etcd",
          "Prometheus","vLLM","Kubernetes","BGP","gRPC","France","Japan","Brazil","merge sort",
          "quicksort","3-SAT","the vertex cover problem","S3","Kinesis","Terraform"]
FILL_B = ["Calico","Linkerd","MySQL","Pulsar","Memcached","OSPF","HTTP/2","QUIC","Cassandra","Vault"]

def make(n_target=20000):
    """Emit n_target UNIQUE utterances.

    Uniqueness is enforced here so the corpus is a clean population; how often a
    given utterance RECURS is the driver's business (uniform / Zipfian / hot-set),
    not the corpus's. Baking repetition in would conflate "what the traffic says"
    with "how often it repeats", and those are separate axes.
    """
    rows, i, seen, stall = [], 0, set(), 0
    doms = list(DOMAINS)
    while len(rows) < n_target:
        dom = doms[i % len(doms)]
        expect, tpl = DOMAINS[dom]
        t = random.choice(tpl)
        text = (t.replace("{a}", random.choice(FILL_A))
                 .replace("{b}", random.choice(FILL_B))
                 .replace("{n}", str(random.choice([2,3,5,8,16,42,64,99,128,500]))))
        # Realistic length variation, including a long-context tail. Padding is
        # domain-flavoured rather than one global filler, so long prompts stay in
        # their own semantic neighbourhood instead of collapsing together.
        r = random.random()
        if r > 0.97:
            text = text + " " + " ".join(random.choice(tpl).replace("{a}", random.choice(FILL_A))
                                         .replace("{b}", random.choice(FILL_B))
                                         .replace("{n}", str(random.randint(1,999)))
                                         for _ in range(random.randint(40, 200)))
        elif r > 0.85:
            text = text + " " + " ".join(random.choice(tpl).replace("{a}", random.choice(FILL_A))
                                         .replace("{b}", random.choice(FILL_B))
                                         .replace("{n}", str(random.randint(1,99)))
                                         for _ in range(random.randint(3, 15)))
        if text in seen:
            # Short templates with no placeholders (e.g. "thanks, that helped")
            # exhaust quickly; give them a natural conversational variation
            # rather than an artificial unique-ifying suffix.
            stall += 1
            if stall % 3 == 0:
                text = text.rstrip("?. ") + random.choice(
                    [" please", " exactly?", " in production?", " for our setup?",
                     " and why?", " briefly", " step by step", " with an example"])
            if text in seen:
                i += 1
                if stall > n_target * 60:
                    break
                continue
        seen.add(text)
        stall = 0
        rows.append({"id": len(rows), "domain": dom, "expect": expect,
                     "bytes": len(text.encode()), "text": text})
        i += 1
    return rows

if __name__ == "__main__":
    # Default matches what the campaign actually ran. An earlier default of
    # 20000 disagreed with the docs, which said 200000, and the corpus is not
    # archived -- so the default IS the contract.
    n = int(sys.argv[1]) if len(sys.argv) > 1 else 200000
    rows = make(n)
    with open(OUT, "w") as f:
        for r in rows:
            f.write(json.dumps(r) + "\n")
    import statistics
    b = [r["bytes"] for r in rows]
    from collections import Counter
    print(f"wrote {len(rows):,} utterances (seed {SEED}) -> {OUT}")
    print(f"  bytes: min={min(b)} p50={statistics.median(b):.0f} p95={sorted(b)[int(len(b)*.95)]} max={max(b)}")
    print(f"  unique texts: {len({r['text'] for r in rows}):,}")
    print("  domains: " + ", ".join(f"{k}={v}" for k, v in sorted(Counter(r['domain'] for r in rows).items())))
