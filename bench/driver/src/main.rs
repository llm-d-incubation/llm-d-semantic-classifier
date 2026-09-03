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
    /// Path under test:
    ///   grpc     classify.Classify directly (isolates the classifier)
    ///   http     OpenAI chat-completions through a gateway
    ///   classify vLLM Semantic Router's http_classify contract (POST /classify)
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
    /// Frozen utterance corpus (JSONL with a `text` field). When set, prompts are
    /// drawn from it instead of being synthesised, so the workload exercises real
    /// semantic variance rather than one filler sentence at a target length.
    #[arg(long, default_value = "")]
    corpus: String,
    /// Start index into the corpus. Distinct offsets give repetitions DISJOINT
    /// prompt sets, which is required for a novel-prompt arm: repetitions share a
    /// process and therefore a warm L1 cache, so overlapping slices turn every
    /// repetition after the first into a cache-hit measurement.
    #[arg(long, default_value_t = 0)]
    corpus_offset: u64,
    /// How the corpus is sampled: uniform | zipf | hotset | unique.
    /// `unique` walks it once (worst case for caching); `hotset` sends 80% of
    /// traffic to 20% of utterances; `zipf` is heavy-tailed like real traffic.
    #[arg(long, default_value = "uniform")]
    dist: String,
    /// OPEN-LOOP: target arrival rate in requests/sec, independent of response
    /// latency. 0 = closed-loop (hold `--concurrency` outstanding).
    ///
    /// This distinction decides what the benchmark can observe. A closed-loop
    /// client slows down when latency rises, which HIDES queue explosion; an
    /// open-loop generator keeps offering work and lets the queue grow, which is
    /// the only way to see a true saturation knee.
    #[arg(long, default_value_t = 0.0)]
    rate: f64,
    /// Arrival process for open-loop mode: constant | poisson.
    #[arg(long, default_value = "poisson")]
    arrival: String,
    /// gRPC ContextCompleteness: 0=UNSPECIFIED, 1=FULL, 2=DELTA.
    /// DELTA must short-circuit to ABSTAIN before any cache or model work, which
    /// is the behaviour PR #23 introduced and this flag exists to verify.
    #[arg(long, default_value_t = 0)]
    context_completeness: i32,
    /// Model name sent in http mode.
    #[arg(long, default_value = "router-model")]
    model: String,
}

/// Load a frozen JSONL corpus. One `text` field per line.
fn load_corpus(path: &str) -> Vec<String> {
    let raw = std::fs::read_to_string(path).unwrap_or_default();
    raw.lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter_map(|v| v.get("text").and_then(|t| t.as_str()).map(|s| s.to_string()))
        .collect()
}

/// Pick an index into the corpus under the requested distribution.
///
/// Deterministic in `i` (a hash, not an RNG) so two runs with identical
/// parameters draw the identical sequence and are directly comparable.
fn corpus_index(dist: &str, i: u64, n: u64, offset: u64) -> u64 {
    if n == 0 {
        return 0;
    }
    let i = i.wrapping_add(offset);
    let mut z = i.wrapping_mul(0x9E3779B97F4A7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    let u = ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64;
    match dist {
        // Walk the corpus once: every request novel, the adversarial cache case.
        // NOTE: this wraps at the corpus size. A run that issues more requests
        // than the corpus holds stops being novel and silently becomes a
        // cache-hit measurement, so the caller must size the corpus above the
        // expected request count (enforced as a premise below).
        "unique" => i % n,
        // 80% of traffic into 20% of the corpus.
        "hotset" => {
            let hot = (n as f64 * 0.2).max(1.0) as u64;
            if u < 0.8 { z % hot } else { hot + (z % (n - hot).max(1)) }
        }
        // Heavy-tailed: rank ~ n^u gives a Zipf-like skew without a table.
        "zipf" => {
            let r = (n as f64).powf(u);
            (r as u64).min(n - 1)
        }
        _ => z % n,
    }
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
    /// closed-loop | open-loop-constant | open-loop-poisson
    load_mode: String,
    /// Open-loop only: requested arrival rate.
    offered_rate_rps: f64,
    /// Open-loop, kept SEPARATE because they answer different questions:
    ///   offer_attainment  did the GENERATOR meet the target? (scheduler health)
    ///   completion_rate   did the TARGET keep up? (saturation)
    /// Conflating them makes "my driver was too slow" indistinguishable from
    /// "the service saturated", which are opposite conclusions.
    scheduled_requests: u64,
    sent_requests: u64,
    actual_offer_rps: f64,
    offer_attainment: f64,
    completed_rps: f64,
    rejected_rps: f64,
    rate_attainment: f64,
    corpus: String,
    corpus_count: u64,
    corpus_sha256: String,
    dist: String,
    /// Premise assertions, persisted HERE in the canonical result rather than in
    /// a side file, so the archive can prove what passed. Report generation
    /// refuses any arm with premises_passed=false.
    premises_passed: bool,
    premise_notes: Vec<String>,
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

    // Corpus identity travels WITH the result. The corpus is too large to
    // archive and is regenerated from a seed, so a hash in each record is the
    // only way a reader can confirm two arms saw the same population.
    let corpus_hash = if args.corpus.is_empty() {
        String::new()
    } else {
        let bytes = std::fs::read(&args.corpus).unwrap_or_default();
        let mut h: u64 = 0xcbf29ce484222325;
        for b in &bytes {
            h ^= *b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        format!("fnv1a64:{h:016x}")
    };
    let corpus: Arc<Vec<String>> = Arc::new(if args.corpus.is_empty() {
        Vec::new()
    } else {
        let c = load_corpus(&args.corpus);
        eprintln!("scbench: corpus {} -> {} utterances, dist={}", args.corpus, c.len(), args.dist);
        c
    });

    // OPEN-LOOP: a shared ticket dispenser paced by wall clock. Workers take a
    // ticket only when the schedule says a request is DUE, so the offered rate is
    // independent of how long responses take. In closed-loop mode this is unused
    // and each worker simply issues the next request on completion.
    let open_loop = args.rate > 0.0;
    // Counters kept SEPARATE. Conflating them is how "the generator could not
    // keep up" and "the target rejected the load" become indistinguishable.
    let scheduled = Arc::new(AtomicU64::new(0));
    let sent = Arc::new(AtomicU64::new(0));
    let (sched_tx, sched_rx_raw) = tokio::sync::mpsc::channel::<()>(1);
    let sched_rx = Arc::new(tokio::sync::Mutex::new(sched_rx_raw));

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
        let corpus = corpus.clone();
        let sched_rx = sched_rx.clone();
        let sent = sent.clone();

        handles.push(tokio::spawn(async move {
            let mut grpc = chan.map(ClassifyClient::new);
            let mut local: Vec<(u64, u8)> = Vec::with_capacity(1 << 16);
            let mut local_status: std::collections::BTreeMap<String, u64> = Default::default();
            loop {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                if open_loop {
                    // Take the next arrival from the CENTRAL schedule. The
                    // schedule is generated by one task (below) rather than by
                    // each worker computing its own slot: a per-worker slot
                    // scheme lets workers claim future slots during warmup, and
                    // those carry across the warmup boundary as a burst. That
                    // bug inflated every measured rate by exactly
                    // concurrency/window (1024/25s = 40.96 rps) and is why
                    // 250 rps offered measured 290.9.
                    if sched_rx.lock().await.recv().await.is_none() {
                        break;
                    }
                    sent.fetch_add(1, Ordering::Relaxed);
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
                let (ctx, prompt) = if !corpus.is_empty() {
                    // Corpus mode: the utterance IS the prompt. Cache behaviour
                    // then comes from the sampling distribution rather than from
                    // a synthetic key, which is the realistic arrangement.
                    let ci = corpus_index(&args.dist, idx, corpus.len() as u64, args.corpus_offset) as usize;
                    let mut text = corpus[ci].clone();
                    if args.context_bytes > 0 {
                        // Pad with FURTHER corpus utterances rather than filler:
                        // a long prompt should still look like language, and
                        // repeating one filler sentence collapses every long
                        // prompt into the same embedding neighbourhood.
                        let mut k = ci;
                        while text.len() < args.context_bytes {
                            k = (k + 7919) % corpus.len();
                            text.push(' ');
                            text.push_str(&corpus[k]);
                        }
                        if text.len() > args.context_bytes {
                            let mut cut = args.context_bytes;
                            while cut > 0 && !text.is_char_boundary(cut) { cut -= 1; }
                            text.truncate(cut);
                        }
                    }
                    (format!("corpus-{ci}"), text)
                } else {
                    let c = context_for_mix(&args.cache_mode, args.run_id, idx,
                                            args.keyspace, is_measuring, args.hit_ratio);
                    let p = make_prompt(&c, args.context_bytes);
                    (c, p)
                };
                let _ = &ctx;

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
                } else if args.mode == "classify" {
                    // vLLM SR http_classify: {"inputs": text} -> [{label, score}]
                    // A 200 whose scores do not sum to ~1.0 is a FAILURE: the
                    // router rejects exactly that, so counting it as success
                    // would measure something the router would never accept.
                    let body = serde_json::json!({"inputs": prompt});
                    let req = hyper::Request::builder()
                        .method("POST")
                        .uri(format!("{target}/classify"))
                        .header("content-type", "application/json")
                        .body(http_body_util::Full::new(bytes::Bytes::from(
                            serde_json::to_vec(&body).unwrap())))
                        .unwrap();
                    match http_client.request(req).await {
                        Ok(resp) => {
                            let st = resp.status();
                            let b = http_body_util::BodyExt::collect(resp.into_body())
                                .await.map(|x| x.to_bytes()).unwrap_or_default();
                            if !st.is_success() {
                                (false, format!("HTTP_{}", st.as_u16()))
                            } else {
                                match serde_json::from_slice::<Vec<serde_json::Value>>(&b) {
                                    Ok(v) if !v.is_empty() => {
                                        let sum: f64 = v.iter()
                                            .filter_map(|x| x.get("score").and_then(|s| s.as_f64()))
                                            .sum();
                                        if (sum - 1.0).abs() > 0.01 {
                                            (false, "SCORES_NOT_NORMALISED".into())
                                        } else {
                                            let top = v[0].get("label")
                                                .and_then(|l| l.as_str()).unwrap_or("?");
                                            (true, top.to_string())
                                        }
                                    }
                                    _ => (false, "BAD_CLASSIFY_BODY".into()),
                                }
                            }
                        }
                        Err(e) => (false, format!("TRANSPORT_{e}")),
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

    // ---- central arrival scheduler ----------------------------------------
    //
    // ONE task owns the arrival process. It emits a token per arrival; workers
    // block on the channel, so a worker never runs ahead of the schedule and
    // nothing can be claimed early and carried across a phase boundary.
    //
    // Poisson is generated as a genuine point process: the next arrival time is
    // the previous one PLUS an exponential deviate (cumulative inter-arrival
    // times). The previous implementation placed arrivals on an evenly spaced
    // grid and jittered each one independently, which has the right mean rate
    // but is not a Poisson process -- its counts are far less dispersed and it
    // cannot produce the clustering that drives real queue growth.
    let sched_handle = if open_loop {
        let rate = args.rate;
        let arrival = args.arrival.clone();
        let scheduled_c = scheduled.clone();
        let stop_c = stop.clone();
        Some(tokio::spawn(async move {
            let mean_gap = 1.0 / rate;
            let mut next = 0.0f64;
            let mut seed: u64 = 0x2545F4914F6CDD1D;
            let origin = Instant::now();
            loop {
                if stop_c.load(Ordering::Relaxed) {
                    break;
                }
                let gap = if arrival == "constant" {
                    mean_gap
                } else {
                    // Inverse-transform exponential: -ln(U)/lambda.
                    seed = seed
                        .wrapping_mul(6364136223846793005)
                        .wrapping_add(1442695040888963407);
                    let z = seed ^ (seed >> 33);
                    let u = ((z >> 11) as f64 / (1u64 << 53) as f64).max(1e-12);
                    -u.ln() * mean_gap
                };
                next += gap;
                let target_t = Duration::from_secs_f64(next);
                let now = origin.elapsed();
                if target_t > now {
                    tokio::time::sleep(target_t - now).await;
                }
                scheduled_c.fetch_add(1, Ordering::Relaxed);
                if sched_tx.send(()).await.is_err() {
                    break;
                }
            }
        }))
    } else {
        None
    };

    // ---- warmup ----
    let warm_target = args.warmup;
    while (warm_done.load(Ordering::Relaxed) as u64) < warm_target {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // Let warmup traffic DRAIN before the measurement window opens. Without
    // this, in-flight warmup requests complete inside the measured window and
    // are attributed to it.
    tokio::time::sleep(Duration::from_millis(400)).await;

    // ---- measure ----
    // Reset the arrival accounting at the phase boundary so offered-rate
    // attainment is computed over the measurement window alone.
    scheduled.store(0, Ordering::Relaxed);
    sent.store(0, Ordering::Relaxed);
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
    if let Some(sh) = sched_handle {
        sh.abort();
    }

    let sched_n = scheduled.load(Ordering::Relaxed);
    let sent_n = sent.load(Ordering::Relaxed);
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
        load_mode: if open_loop {
            format!("open-loop-{}", args.arrival)
        } else {
            "closed-loop".to_string()
        },
        offered_rate_rps: args.rate,
        scheduled_requests: sched_n,
        sent_requests: sent_n,
        actual_offer_rps: sched_n as f64 / wall.max(1e-9),
        offer_attainment: if open_loop && args.rate > 0.0 {
            (sched_n as f64 / wall.max(1e-9)) / args.rate
        } else { 1.0 },
        completed_rps: ok_count as f64 / wall.max(1e-9),
        rejected_rps: errs as f64 / wall.max(1e-9),
        rate_attainment: if open_loop && args.rate > 0.0 {
            (ok_count as f64 / wall.max(1e-9)) / args.rate
        } else { 1.0 },
        corpus: args.corpus.clone(),
        corpus_count: corpus.len() as u64,
        corpus_sha256: corpus_hash.clone(),
        dist: args.dist.clone(),
        premises_passed: true,
        premise_notes: Vec::new(),
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

    // Enforce the premises the DRIVER can verify, and record them in the
    // canonical JSON. Round 2 computed these into a side file that was never
    // archived, so the evidence could not demonstrate what had been validated.
    let mut report = report;
    let mut notes: Vec<String> = Vec::new();
    // Overshoot is as disqualifying as undershoot: an arm that offered 16% MORE
    // than its target was not measuring the rate it claims. The old check only
    // looked for undershoot, so 250 rps offered / 291 achieved passed cleanly.
    if open_loop && !(0.98..=1.02).contains(&report.offer_attainment) {
        notes.push(format!(
            "offer attainment {:.3} outside [0.98, 1.02]: the GENERATOR scheduled \
             {:.1} rps against a {:.0} rps target, so this arm did not offer the \
             load it claims",
            report.offer_attainment, report.actual_offer_rps, args.rate));
    }
    if open_loop && report.rate_attainment < 0.98 {
        if report.error_rate_pct > 5.0 {
            notes.push(format!(
                "rate attainment {:.3} with {:.1}% errors ({:?}): the target REJECTED \
                 the load -- diagnose the errors, not the generator",
                report.rate_attainment, report.error_rate_pct, report.status_counts));
        } else {
            notes.push(format!(
                "rate attainment {:.3} < 0.98 at {:.0}% errors: the generator could not \
                 sustain {} rps, so this arm measures the DRIVER, not the target",
                report.rate_attainment, report.error_rate_pct, args.rate));
        }
    }
    if !open_loop {
        // Little's law cross-check: concurrency/throughput must track the
        // measured mean. A large divergence means the driver was the limiter.
        let m = report.latency.mean_ms;
        if m > 0.0 {
            let ratio = report.implied_mean_ms / m;
            if !(0.7..=1.4).contains(&ratio) {
                notes.push(format!(
                    "Little's law divergence: implied mean {:.3}ms vs measured {:.3}ms \
                     (ratio {:.2}) -- driver may be the limiter",
                    report.implied_mean_ms, m, ratio));
            }
        }
    }
    if args.dist == "unique" && !corpus.is_empty()
        && report.measured_requests > corpus.len() as u64 {
        notes.push(format!(
            "dist=unique issued {} requests against a {}-utterance corpus: it \
             wrapped {:.1}x, so most requests were REPEATS and this arm measures \
             the cache-hit path, not novel prompts",
            report.measured_requests, corpus.len(),
            report.measured_requests as f64 / corpus.len() as f64));
    }
    if report.measured_requests < 200 {
        notes.push(format!("only {} measured requests: percentiles beyond p90 are \
                            not determined", report.measured_requests));
    }
    report.premises_passed = notes.is_empty();
    report.premise_notes = notes;
    if !report.premises_passed {
        for n in &report.premise_notes {
            eprintln!("scbench: PREMISE FAILED: {n}");
        }
    }

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
