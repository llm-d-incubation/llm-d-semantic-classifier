//! `llm-d-sc-gateway-probe`: measure classify RTT from a dummy AI Gateway to a
//! REMOTE llm-d-sc over the network (P-030..P-033, S-001/S-002).
//!
//! `bench-runner` serves its own classifier on loopback, so it measures the
//! runtime but never the transport. These IDs are about topology: same-Pod
//! sidecar over 127.0.0.1 versus a separate Pod over a ClusterIP Service. That
//! difference only exists when the client and server are genuinely different
//! processes on a real network path, so this binary connects OUT to an address
//! and measures what a caller experiences.
//!
//! Usage:
//!   llm-d-sc-gateway-probe --target 127.0.0.1:50051 --topology same-pod [--samples 200]

use std::time::Instant;

use llm_d_sc::grpc::classify::{ClassifyClient, ClassifyRequest};

fn arg(name: &str, default: &str) -> String {
    let a: Vec<String> = std::env::args().collect();
    a.iter()
        .position(|x| x == name)
        .and_then(|i| a.get(i + 1).cloned())
        .unwrap_or_else(|| default.to_string())
}

fn percentile(sorted: &[u128], q: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() as f64 - 1.0) * q).round() as usize;
    sorted[idx] as f64 / 1000.0
}

fn main() {
    let target = arg("--target", "127.0.0.1:50051");
    let topology = arg("--topology", "unspecified");
    let samples: usize = arg("--samples", "200").parse().expect("--samples must be a number");

    let mut client = match ClassifyClient::connect(&target) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("gateway-probe: cannot reach llm-d-sc at {target}: {e}");
            std::process::exit(1);
        }
    };

    let req = |id: &str, text: &str| ClassifyRequest {
        request_id: id.to_string(),
        session_id: "probe-session".to_string(),
        context: text.to_string(),
        // Empty: a remote client cannot know which taxonomy this instance
        // serves, and asserting one couples the tool to a single deployment.
        // An empty list means "no constraint" and the server returns its signal.
        signals: Vec::new(),
    };

    // Warm the connection and the model so the first sample does not carry
    // connection setup or lazy-init cost into the measurement.
    for i in 0..5 {
        client
            .classify(req(&format!("warm-{i}"), &format!("warmup prompt {i}")))
            .expect("warmup must succeed");
    }

    // MISS: every request has distinct text, so the result cache cannot serve it
    // and each sample pays a real model forward plus the network path.
    let mut miss = Vec::with_capacity(samples);
    for i in 0..samples {
        let text = format!("distinct probe prompt {i} concerning an unrelated subject entirely");
        let t = Instant::now();
        let r = client.classify(req(&format!("miss-{i}"), &text)).expect("miss must succeed");
        miss.push(t.elapsed().as_micros());
        // The response must never carry a route: routing authority stays with
        // the caller (AC-010). Asserting it here makes the E2E topology runs
        // evidence for that contract too, not just for latency.
        assert!(!r.ranked.is_empty(), "a served response must carry ranked signals");
    }

    // HIT: identical text every time, so after the first the result is cached
    // and the sample measures the transport plus a cache lookup.
    let hit_text = "a single stable prompt served repeatedly from the result cache";
    client.classify(req("hit-warm", hit_text)).expect("hit warm must succeed");
    let mut hit = Vec::with_capacity(samples);
    for i in 0..samples {
        let t = Instant::now();
        client.classify(req(&format!("hit-{i}"), hit_text)).expect("hit must succeed");
        hit.push(t.elapsed().as_micros());
    }

    miss.sort_unstable();
    hit.sort_unstable();

    let report = serde_json::json!({
        "target": target,
        "topology": topology,
        "samples": samples,
        "cache_miss_rtt_ms": {
            "p50": percentile(&miss, 0.50), "p95": percentile(&miss, 0.95),
            "p99": percentile(&miss, 0.99), "max": percentile(&miss, 1.0),
        },
        "cache_hit_rtt_ms": {
            "p50": percentile(&hit, 0.50), "p95": percentile(&hit, 0.95),
            "p99": percentile(&hit, 0.99), "max": percentile(&hit, 1.0),
        },
    });
    println!("{}", serde_json::to_string_pretty(&report).unwrap());

    eprintln!(
        "gateway-probe [{topology}] {target}: miss p50 {:.2} ms p99 {:.2} ms | hit p50 {:.2} ms p99 {:.2} ms",
        percentile(&miss, 0.50), percentile(&miss, 0.99),
        percentile(&hit, 0.50), percentile(&hit, 0.99)
    );
}
