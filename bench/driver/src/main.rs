//! `scbench` — closed-loop load driver for llm-d-sc.
//!
//! Drives either path under test and captures the FULL latency distribution:
//!   * `grpc` — classify.Classify directly (isolates the classifier)
//!   * `http` — an OpenAI-shaped POST through a gateway (Praxis + llm_d_sc)
//!
//! Methodology (SPEC-BENCH section 0):
//!   * Nearest-rank percentiles, identical to `llm-d-sc/src/bench.rs::percentile`,
//!     so a number here is comparable to one from the in-tree harness.
//!   * Mean/stddev are reported ALONGSIDE the distribution, never instead of it
//!     (rule 1 forbids average-ONLY latency; a network engineer still needs the
//!     first two moments to reason about jitter).
//!   * Cache arms use disjoint key namespaces: MISS keys live in a per-run
//!     `measure-{run_id}-{i}` namespace that is never pre-warmed, so a request
//!     that is supposed to miss can never be silently served from cache.
//!   * Warmup requests are issued and DISCARDED before measurement starts.
//!   * Raw per-request samples are retained so anyone can recompute the summary.
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use clap::Parser;
use serde::Serialize;

pub mod pb {
    tonic::include_proto!("classify");
}
use pb::classify_client::ClassifyClient;

#[derive(Parser, Debug, Clone)]
#[command(name = "scbench")]
struct Args {
    /// Path under test: grpc (classify direct) or http (gateway chat-completions).
    #[arg(long, default_value = "grpc")]
    mode: String,
    /// Target endpoints, comma separated. grpc: http://host:50051 . http: http://host:8080
    /// More than one target = deterministic client-side fan-out (isolates ClusterIP).
    #[arg(long)]
    target: String,
    /// In-flight requests (closed-loop concurrency).
    #[arg(long, default_value_t = 64)]
    concurrency: usize,
    /// Transport connections. Concurrency is spread across these round-robin.
    #[arg(long, default_value_t = 32)]
    connections: usize,
    /// Measured requests. Takes precedence over --duration-secs when non-zero.
    #[arg(long, default_value_t = 0)]
    requests: u64,
    /// Measured duration when --requests is 0.
    #[arg(long, default_value_t = 30)]
    duration_secs: u64,
    /// Warmup requests, issued then discarded.
    #[arg(long, default_value_t = 2000)]
    warmup: u64,
    /// Prompt size in bytes (the context-window dimension).
    #[arg(long, default_value_t = 256)]
    context_bytes: usize,
    /// hit = one stable key (cache hits). miss = unique per-request keys.
    /// mixed = a weighted blend, controlled by --hit-ratio.
    #[arg(long, default_value = "miss")]
    cache_mode: String,
    /// In `mixed` mode, the fraction of requests drawn from the warm keyspace.
    /// 0.0 = every request novel, 1.0 = every request a repeat. Real workloads
    /// live in between, and this is the axis that decides whether caching pays.
    #[arg(long, default_value_t = 0.9)]
    hit_ratio: f64,
    /// Distinct prompt keys in hit mode; models a working set larger than one.
    #[arg(long, default_value_t = 1)]
    keyspace: u64,
    /// Run identifier; namespaces MISS keys so they can never collide with a
    /// previous run's cache contents.
    #[arg(long, default_value_t = 0)]
    run_id: u64,
    /// Write the JSON summary here.
    #[arg(long, default_value = "")]
    out: String,
    /// Write raw per-request samples (CSV: latency_ns,status) here.
    #[arg(long, default_value = "")]
    raw: String,
    /// Free-form label recorded in the manifest.
    #[arg(long, default_value = "")]
    label: String,
    /// gRPC ContextCompleteness: 0=UNSPECIFIED, 1=FULL, 2=DELTA.
    /// DELTA must short-circuit to ABSTAIN before any cache or model work, which
    /// is the behaviour PR #23 introduced and this flag exists to verify.
    #[arg(long, default_value_t = 0)]
    context_completeness: i32,
    /// Model name sent in http mode.
    #[arg(long, default_value = "router-model")]
    model: String,
}

/// Deterministic prompt of `bytes` length. Real words so the tokenizer does
/// realistic work: a run of identical characters would tokenize unnaturally
/// cheaply and flatter the classifier.
fn make_prompt(key: &str, bytes: usize) -> String {
    const FILLER: &str = "the system processes distributed inference requests across heterogeneous \
compute tiers while maintaining throughput guarantees and bounded tail latency for each tenant ";
    let mut s = String::with_capacity(bytes + 64);
    s.push_str(key);
    s.push(' ');
    while s.len() < bytes {
        s.push_str(FILLER);
    }
    s.truncate(bytes.max(key.len() + 1));
    s
}

/// The key for request `index`.
///
/// MISS keys are namespaced by run_id AND by phase. The phase split is not
/// cosmetic: warmup and measurement both count from 0, so a shared namespace
/// makes the first N measured "misses" silent cache HITS of the warmup's own
/// keys. That is exactly the failure methodology rule 4 exists to prevent, and
/// it shows up as an impossibly small min latency next to a large mean.
///
/// HIT keys are deliberately SHARED across phases -- pre-warming them is the
/// whole point of a hit arm.
fn context_for(cache_mode: &str, run_id: u64, index: u64, keyspace: u64, measuring: bool) -> String {
    context_for_mix(cache_mode, run_id, index, keyspace, measuring, 1.0)
}

/// As [`context_for`], with a `mixed` mode that blends warm and novel keys.
///
/// The split is DETERMINISTIC on the request index (a hash, not an RNG) so two
/// runs with the same parameters draw the same sequence and are comparable.
fn context_for_mix(cache_mode: &str, run_id: u64, index: u64, keyspace: u64,
                   measuring: bool, hit_ratio: f64) -> String {
    if cache_mode == "mixed" {
        // splitmix64-style scramble: cheap, and decorrelated from `index % k`
        // so the hit/miss choice is not aliased to the keyspace stride.
        let mut z = index.wrapping_mul(0x9E3779B97F4A7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        let r = ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64;
        return if r < hit_ratio {
            // Warm keys are SHARED across phases on purpose -- warmup exists to
            // populate them.
            format!("hitkey-{run_id}-{}", index % keyspace.max(1))
        } else if measuring {
            format!("measure-{run_id}-{index}")
        } else {
            // Novel keys stay phase-namespaced here too, or warmup's novel keys
            // become silent hits for the measured novel keys of the same index.
            format!("warm-{run_id}-{index}")
        };
    }
    if cache_mode == "hit" {
        format!("hitkey-{run_id}-{}", index % keyspace.max(1))
    } else if measuring {
        format!("measure-{run_id}-{index}")
    } else {
        format!("warm-{run_id}-{index}")
    }
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    let idx = rank.saturating_sub(1).min(sorted.len() - 1);
    sorted[idx]
}

#[derive(Serialize, Default)]
struct Stats {
    count: usize,
    min_ms: f64,
    mean_ms: f64,
    stddev_ms: f64,
    p50_ms: f64,
    p90_ms: f64,
    p95_ms: f64,
    p99_ms: f64,
    p999_ms: f64,
    max_ms: f64,
}

impl Stats {
    fn from_nanos(mut v: Vec<u64>) -> Stats {
        if v.is_empty() {
            return Stats::default();
        }
        v.sort_unstable();
        let n = v.len();
        let ms = |x: u64| x as f64 / 1_000_000.0;
        let mean = v.iter().map(|&x| x as f64).sum::<f64>() / n as f64;
        let var = v.iter().map(|&x| (x as f64 - mean).powi(2)).sum::<f64>() / n as f64;
        Stats {
            count: n,
            min_ms: ms(v[0]),
            mean_ms: mean / 1_000_000.0,
            stddev_ms: var.sqrt() / 1_000_000.0,
            p50_ms: ms(percentile(&v, 50.0)),
            p90_ms: ms(percentile(&v, 90.0)),
            p95_ms: ms(percentile(&v, 95.0)),
            p99_ms: ms(percentile(&v, 99.0)),
            p999_ms: ms(percentile(&v, 99.9)),
            max_ms: ms(v[n - 1]),
        }
    }
}

#[derive(Serialize)]
struct Report {
    schema_version: u32,
    label: String,
    mode: String,
    targets: Vec<String>,
    concurrency: usize,
    connections: usize,
    context_bytes: usize,
    cache_mode: String,
    hit_ratio: f64,
    keyspace: u64,
    run_id: u64,
    warmup_requests: u64,
    measured_requests: u64,
    wall_secs: f64,
    /// Successful requests per second.
    throughput_rps: f64,
    /// The network-engineer framing: successful requests per minute, and the
    /// offered session rate the transport actually sustained.
    throughput_rpm: f64,
    offered_concurrency: usize,
    /// Little's law check: concurrency / throughput. Should track mean latency;
    /// a large divergence means the driver, not the target, was the limiter.
    implied_mean_ms: f64,
    errors: u64,
    error_rate_pct: f64,
    status_counts: std::collections::BTreeMap<String, u64>,
    latency: Stats,
    started_unix: u64,
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = Args::parse();
    let targets: Vec<String> = args.target.split(',').map(|s| s.trim().to_string()).collect();

    // Warmup must be expressed in KEYS COVERED, not requests issued.
    //
    // With a keyspace of K, warmup requests are spread across all K keys, so a
    // warmup of only a few multiples of K leaves part of the working set cold.
    // Those stragglers then take a full model forward INSIDE the measurement
    // window, which looks exactly like a service defect: elevated latency and,
    // behind a gateway with a short classify timeout, apparent fail-open
    // routing. That mistake produced a since-retracted finding in this
    // campaign's first Praxis pass, so the floor is enforced here rather than
    // left to the caller to remember.
    if args.cache_mode != "miss" {
        let floor = args.keyspace.saturating_mul(40).max(2000);
        if args.warmup < floor {
            eprintln!(
                "scbench: raising warmup {} -> {} to cover a keyspace of {} \
                 (>=40 requests per key; a partially warm cache is measured as \
                 service latency)",
                args.warmup, floor, args.keyspace
            );
            args.warmup = floor;
        }
    }
    let args = args;

    let counter = Arc::new(AtomicU64::new(0));
    let errors = Arc::new(AtomicU64::new(0));
    // Merged once per worker at the end. A per-request lock on a shared map
    // serialises every thread through one mutex and makes the DRIVER the
    // bottleneck -- which shows up as a throughput ceiling that has nothing to do
    // with the service under test.
    let statuses: Arc<std::sync::Mutex<std::collections::BTreeMap<String, u64>>> =
        Arc::new(std::sync::Mutex::new(Default::default()));
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let measuring = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let warm_done = Arc::new(AtomicUsize::new(0));

    // Build the connection pool up front so connect cost is never inside a
    // measured sample.
    let mut grpc_chans = Vec::new();
    if args.mode == "grpc" {
        for i in 0..args.connections {
            let t = targets[i % targets.len()].clone();
            let ch = tonic::transport::Channel::from_shared(t)?
                .connect()
                .await?;
            grpc_chans.push(ch);
        }
    }
    let http_client = hyper_util::client::legacy::Client::builder(
        hyper_util::rt::TokioExecutor::new(),
    )
    .pool_max_idle_per_host(args.connections)
    .build_http::<http_body_util::Full<bytes::Bytes>>();

    let mut handles = Vec::new();
    let samples: Arc<std::sync::Mutex<Vec<(u64, u8)>>> =
        Arc::new(std::sync::Mutex::new(Vec::with_capacity(1 << 20)));

    for w in 0..args.concurrency {
        let args = args.clone();
        let targets = targets.clone();
        let counter = counter.clone();
        let errors = errors.clone();
        let statuses = statuses.clone();
        let stop = stop.clone();
        let measuring = measuring.clone();
        let warm_done = warm_done.clone();
        let samples = samples.clone();
        let chan = if args.mode == "grpc" {
            Some(grpc_chans[w % grpc_chans.len()].clone())
        } else {
            None
        };
        let http_client = http_client.clone();
        let target = targets[w % targets.len()].clone();

        handles.push(tokio::spawn(async move {
            let mut grpc = chan.map(ClassifyClient::new);
            let mut local: Vec<(u64, u8)> = Vec::with_capacity(1 << 16);
            let mut local_status: std::collections::BTreeMap<String, u64> = Default::default();
            loop {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                let is_measuring = measuring.load(Ordering::Relaxed);
                // In mixed mode the warmup must POPULATE the warm keyspace, or
                // the "hit" fraction would not actually be warm.
                let idx = if is_measuring {
                    let c = counter.fetch_add(1, Ordering::Relaxed);
                    if args.requests > 0 && c >= args.requests {
                        break;
                    }
                    c
                } else {
                    warm_done.fetch_add(1, Ordering::Relaxed) as u64
                };
                let ctx = context_for_mix(&args.cache_mode, args.run_id, idx, args.keyspace, is_measuring, args.hit_ratio);
                let prompt = make_prompt(&ctx, args.context_bytes);

                let t0 = Instant::now();
                let (ok, status): (bool, String) = if let Some(c) = grpc.as_mut() {
                    let req = pb::ClassifyRequest {
                        request_id: format!("scbench-{idx}"),
                        session_id: String::new(),
                        context: prompt,
                        signals: vec![],
                        context_completeness: args.context_completeness,
                    };
                    match c.classify(req).await {
                        Ok(resp) => {
                            // Rule 3: the harness must verify its own premise. A
                            // transport-level Ok proves only that bytes came back;
                            // it does NOT prove a classification happened. Without
                            // this check a service that returned an empty ranking
                            // at wire speed would look like record throughput.
                            let r = resp.into_inner();
                            let top = r.ranked.iter().max_by(|a, b| {
                                a.score.partial_cmp(&b.score).unwrap_or(std::cmp::Ordering::Equal)
                            });
                            match (r.status, top) {
                                // 1 == ClassificationStatus::OK
                                (1, Some(t)) if !t.label.is_empty() => {
                                    // Label is interned per worker by the tally
                                    // map, so this allocation happens once per
                                    // distinct label, not once per request.
                                    (true, t.label.clone())
                                }
                                (1, _) => (false, "INVALID_EMPTY_RANKING".into()),
                                // 2 == ABSTAIN. Correct, not a failure, when the
                                // request declared DELTA context.
                                (2, _) if args.context_completeness == 2 => {
                                    (true, "ABSTAIN".into())
                                }
                                (st, _) => (false, format!("STATUS_{st}")),
                            }
                        }
                        Err(e) => (false, format!("{:?}", e.code())),
                    }
                } else {
                    let body = serde_json::json!({
                        "model": args.model,
                        "messages": [{"role": "user", "content": prompt}],
                        "max_tokens": 16,
                        "temperature": 0
                    });
                    let req = hyper::Request::builder()
                        .method("POST")
                        .uri(format!("{target}/v1/chat/completions"))
                        .header("content-type", "application/json")
                        .body(http_body_util::Full::new(bytes::Bytes::from(
                            serde_json::to_vec(&body).unwrap(),
                        )))
                        .unwrap();
                    match http_client.request(req).await {
                        Ok(resp) => {
                            let st = resp.status();
                            // Drain so the connection is reusable, and confirm the
                            // gateway actually produced a completion: a 200 with an
                            // empty `choices` is a failure that would otherwise be
                            // scored as success.
                            let body = http_body_util::BodyExt::collect(resp.into_body())
                                .await
                                .map(|b| b.to_bytes())
                                .unwrap_or_default();
                            if !st.is_success() {
                                (false, format!("HTTP_{}", st.as_u16()))
                            } else {
                                match serde_json::from_slice::<serde_json::Value>(&body) {
                                    Ok(v) if v.get("choices")
                                              .and_then(|c| c.as_array())
                                              .map(|a| !a.is_empty()) == Some(true) =>
                                        (true, "HTTP_200".into()),
                                    _ => (false, "HTTP_200_EMPTY_CHOICES".into()),
                                }
                            }
                        }
                        Err(e) => (false, format!("TRANSPORT_{e}")),
                    }
                };
                let dt = t0.elapsed().as_nanos() as u64;

                if is_measuring {
                    local.push((dt, if ok { 0 } else { 1 }));
                    if !ok {
                        errors.fetch_add(1, Ordering::Relaxed);
                    }
                    // Cheap in the common case: only allocate a new key the first
                    // time a given status is seen by this worker.
                    if let Some(v) = local_status.get_mut(status.as_str()) {
                        *v += 1;
                    } else {
                        local_status.insert(status, 1);
                    }
                }
            }
            samples.lock().unwrap().extend(local);
            {
                let mut m = statuses.lock().unwrap();
                for (k, v) in local_status {
                    *m.entry(k).or_insert(0) += v;
                }
            }
        }));
    }

    // ---- warmup ----
    let warm_target = args.warmup;
    while (warm_done.load(Ordering::Relaxed) as u64) < warm_target {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // ---- measure ----
    let started = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let t_start = Instant::now();
    measuring.store(true, Ordering::Relaxed);
    if args.requests > 0 {
        while counter.load(Ordering::Relaxed) < args.requests && !handles.iter().all(|h| h.is_finished())
        {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    } else {
        tokio::time::sleep(Duration::from_secs(args.duration_secs)).await;
    }
    stop.store(true, Ordering::Relaxed);
    let wall = t_start.elapsed().as_secs_f64();
    for h in handles {
        let _ = h.await;
    }

    let all = std::mem::take(&mut *samples.lock().unwrap());
    let measured = all.len() as u64;
    let errs = errors.load(Ordering::Relaxed);
    let lat = Stats::from_nanos(all.iter().map(|(d, _)| *d).collect());
    let ok_count = measured.saturating_sub(errs);
    let rps = ok_count as f64 / wall.max(1e-9);

    let report = Report {
        schema_version: 1,
        label: args.label.clone(),
        mode: args.mode.clone(),
        targets: targets.clone(),
        concurrency: args.concurrency,
        connections: args.connections,
        context_bytes: args.context_bytes,
        cache_mode: args.cache_mode.clone(),
        hit_ratio: args.hit_ratio,
        keyspace: args.keyspace,
        run_id: args.run_id,
        warmup_requests: args.warmup,
        measured_requests: measured,
        wall_secs: wall,
        throughput_rps: rps,
        throughput_rpm: rps * 60.0,
        offered_concurrency: args.concurrency,
        implied_mean_ms: if rps > 0.0 {
            (args.concurrency as f64 / rps) * 1000.0
        } else {
            0.0
        },
        errors: errs,
        error_rate_pct: if measured > 0 {
            errs as f64 / measured as f64 * 100.0
        } else {
            0.0
        },
        status_counts: statuses.lock().unwrap().clone(),
        latency: lat,
        started_unix: started,
    };

    let json = serde_json::to_string_pretty(&report)?;
    if args.out.is_empty() {
        println!("{json}");
    } else {
        std::fs::write(&args.out, &json)?;
        println!("{json}");
    }
    if !args.raw.is_empty() {
        use std::io::Write;
        let f = std::fs::File::create(&args.raw)?;
        let mut bw = std::io::BufWriter::new(f);
        writeln!(bw, "latency_ns,err")?;
        for (d, e) in &all {
            writeln!(bw, "{d},{e}")?;
        }
    }
    Ok(())
}
