//! AC-011 benchmark runner: executes the HOMELAB.md benchmark protocol against
//! the REAL classifier and emits machine-readable results.
//!
//! The maintainer runs this binary directly (and later on Kubernetes unchanged).
//! It:
//!
//! 1. Builds a REAL [`CandleClassifier`] from `LLM_D_SC_MODEL_DIR` (default
//!    `artifacts/models/sensitivity`) and serves it via
//!    [`ClassifyServer::bind_with_classifier`] on an EPHEMERAL loopback port.
//!    If the model dir is absent the runner EXITS with a clear error — it NEVER
//!    silently falls back to the synthetic pipeline, because that would produce
//!    meaningless numbers (a benchmark must measure the real forward).
//! 2. Runs the 0.1 matrix via [`BenchmarkRun`] (src/bench.rs): cache modes Hit
//!    and Miss x input token lengths 32/64/128/256 x concurrency 1 and 4
//!    (P-020/P-021). Inputs are constructed so their tokenized length
//!    approximates each target; the ACTUAL token count is recorded.
//! 3. For every scenario records p50/p90/p95/p99/max, throughput req/s, error
//!    count, and the queue/tokenize/forward/total stage decomposition from the
//!    server's metrics surface (AC-012).
//! 4. Asserts the methodology exactly as the harness already does — miss
//!    scenarios must show measured-count cache misses, hit scenarios
//!    measured-count hits — and FAILS LOUDLY on any violation (the harness's
//!    own self-check returns an error, which aborts the run).
//! 5. Emits results as JSON to `artifacts/bench/<timestamp>.json` plus a
//!    human-readable table on stdout, including the HOMELAB.md manifest fields
//!    available locally (git sha, model dir + revision, tokenizer revision,
//!    backend=candle, topology=loopback, cpu model via std or env, concurrency,
//!    cache mode, sequence length, warmup/measure counts).
//!
//! Warmup/measure counts come from `BENCH_WARMUP` (default 200) and
//! `BENCH_MEASURE` (default 1000) so the maintainer can scale. Hit-mode warmup
//! is `max(BENCH_WARMUP, BENCH_MEASURE)` so every measured hit key is pre-warmed
//! (the harness's methodology self-check requires it).

use std::env;
use std::io;
use std::process::Command;

use llm_d_sc::bench::{BenchmarkRun, CacheMode, RttDistribution, Topology};
use llm_d_sc::classify::load_and_warm_modelcar;
use llm_d_sc::grpc::classify::ClassifyServer;
use llm_d_sc::metrics::MetricsSnapshot;
use llm_d_sc::tokenizer::Tokenizer;

/// Default ModelCar mount directory (matches the server binary's local default).
const DEFAULT_MODEL_DIR: &str = "artifacts/models/sensitivity";
/// Default warmup request count (HOMELAB.md starts with 1,000; 200 is the
/// runner's local default so the maintainer can scale via env).
const DEFAULT_WARMUP: u64 = 200;
/// Default measured request count.
const DEFAULT_MEASURE: u64 = 1000;
/// The 0.1 input-token-length matrix (HOMELAB.md).
const TOKEN_LENGTHS: [usize; 4] = [32, 64, 128, 256];
/// The 0.1 concurrency matrix (P-020 concurrency 1 / P-021 concurrency 4).
const CONCURRENCIES: [u64; 2] = [1, 4];
/// A repeated word whose tokens give a controllable sequence length.
const SEED_WORD: &str = "benchmark";

/// A single measured scenario's results.
struct Scenario {
    cache_mode: CacheMode,
    target_length: usize,
    actual_token_count: usize,
    concurrency: u64,
    warmup: u64,
    measured: u64,
    dist: RttDistribution,
    throughput: f64,
    errors: u64,
    stage: MetricsSnapshot,
}

fn main() -> io::Result<()> {
    // (1) Resolve the model dir and fail loudly if absent.
    let model_dir =
        env::var("LLM_D_SC_MODEL_DIR").unwrap_or_else(|_| DEFAULT_MODEL_DIR.to_string());
    if model_dir.trim().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "LLM_D_SC_MODEL_DIR must not be empty",
        ));
    }
    let model_path = std::path::Path::new(&model_dir);
    if !model_path.is_dir() {
        eprintln!(
            "FATAL: model dir '{model_dir}' not found. A benchmark must measure the REAL \
             forward, so the runner refuses to fall back to the synthetic pipeline. \
             Fetch the model (./hack/fetch-model) or set LLM_D_SC_MODEL_DIR."
        );
        std::process::exit(1);
    }

    // (2) Warmup/measure counts from env (maintainer-scalable).
    let warmup = parse_env("BENCH_WARMUP", DEFAULT_WARMUP)?;
    let measure = parse_env("BENCH_MEASURE", DEFAULT_MEASURE)?;
    if measure == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "BENCH_MEASURE must be >= 1",
        ));
    }

    // (3) Build the REAL classifier (layout validation + load + warmup forward).
    let classifier = load_and_warm_modelcar(model_path).map_err(|e| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("failed to load+warm the real classifier from {model_dir}: {e}"),
        )
    })?;

    // (4) Serve the real classifier on an ephemeral loopback port, sharing the
    // server's metrics handle so the harness proves its own methodology.
    let server = ClassifyServer::bind_with_classifier("127.0.0.1:0", classifier)?;
    let metrics = server.metrics();
    let addr = server.local_addr();

    // Load the resident tokenizer to construct/measure length-specific inputs.
    let tokenizer = Tokenizer::load(model_path.join("tokenizer.json")).map_err(io::Error::other)?;

    // Manifest fields available locally (HOMELAB.md).
    let git_sha = git_sha();
    let (model_revision, tokenizer_revision) = revisions();
    let cpu_model = cpu_model();

    // (5) Run the 0.1 matrix.
    let mut scenarios: Vec<Scenario> = Vec::new();
    for &target in &TOKEN_LENGTHS {
        // Build a base input whose tokenized length approximates the target.
        let seed = build_seed(target, &tokenizer);
        for cache_mode in [CacheMode::Hit, CacheMode::Miss] {
            for &concurrency in &CONCURRENCIES {
                let run = BenchmarkRun::with_metrics(
                    &addr,
                    Topology::Sidecar,
                    cache_mode,
                    metrics.clone(),
                )
                .map_err(io::Error::other)?
                .with_seed(seed.clone());
                // Hit mode must pre-warm at least the measured keys for the
                // harness's hit methodology self-check to hold.
                let warm_count = match cache_mode {
                    CacheMode::Hit => warmup.max(measure),
                    CacheMode::Miss => warmup,
                };
                // The ACTUAL token count of the text sent for this scenario
                // (seed + the harness's per-run measured suffix).
                let actual_context = format!("{seed}-measure-{}-{}", run.run_id(), 0);
                let actual_token_count = tokenizer
                    .tokenize(&actual_context)
                    .map(|ids| ids.len())
                    .unwrap_or(0);

                run.warmup(warm_count).map_err(io::Error::other)?;
                let before = metrics.snapshot();
                let start = std::time::Instant::now();
                let result = if concurrency == 1 {
                    run.measure(measure)
                } else {
                    run.measure_concurrent(measure, concurrency)
                };
                let elapsed = start.elapsed();

                // (4) The harness itself asserts the methodology; any violation
                // (or request failure) returns Err and we FAIL LOUDLY.
                let dist = result.map_err(|e| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "scenario cache_mode={cache_mode:?} len={target} \
                             concurrency={concurrency} aborted: {e}"
                        ),
                    )
                })?;
                let after = metrics.snapshot();
                let stage = stage_delta(before, after);
                let throughput = measure as f64 / elapsed.as_secs_f64();
                scenarios.push(Scenario {
                    cache_mode,
                    target_length: target,
                    actual_token_count,
                    concurrency,
                    warmup: warm_count,
                    measured: measure,
                    dist,
                    throughput,
                    errors: 0,
                    stage,
                });
            }
        }
    }

    // (5) Emit JSON + human-readable table.
    let out_dir = std::path::PathBuf::from(env::var("BENCH_OUT").unwrap_or_else(|_| {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("artifacts")
            .join("bench")
            .to_string_lossy()
            .to_string()
    }));
    std::fs::create_dir_all(&out_dir)?;
    let ts = timestamp();
    let json_path = out_dir.join(format!("{ts}.json"));

    let report = build_report(
        &ManifestInput {
            git_sha,
            model_dir: model_dir.to_string(),
            model_revision,
            tokenizer_revision,
            cpu_model,
            warmup,
            measure,
        },
        &scenarios,
    );
    std::fs::write(&json_path, serde_json::to_string_pretty(&report)?)?;
    println!("wrote JSON results to {}", json_path.display());
    print_table(&report);

    Ok(())
}

/// Parse an env var as a positive integer, returning a clear error otherwise.
fn parse_env(name: &str, default: u64) -> io::Result<u64> {
    match env::var(name) {
        Ok(v) => v.trim().parse::<u64>().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{name} must be a positive integer, got '{v}'"),
            )
        }),
        Err(_) => Ok(default),
    }
}

/// Build a base text whose tokenized length approximates `target` (recording the
/// actual length is done by the caller on the full sent context).
fn build_seed(target: usize, tokenizer: &Tokenizer) -> String {
    let mut base = String::new();
    loop {
        base.push_str(SEED_WORD);
        base.push(' ');
        let len = tokenizer
            .tokenize(base.trim_end())
            .map(|ids| ids.len())
            .unwrap_or(0);
        if len >= target {
            break;
        }
    }
    base.trim_end().to_string()
}

/// The stage-decomposition delta between two snapshots (the measured window).
fn stage_delta(before: MetricsSnapshot, after: MetricsSnapshot) -> MetricsSnapshot {
    MetricsSnapshot {
        queue: after.queue.saturating_sub(before.queue),
        tokenize: after.tokenize.saturating_sub(before.tokenize),
        forward: after.forward.saturating_sub(before.forward),
        total: after.total.saturating_sub(before.total),
        cache_hits: after.cache_hits.saturating_sub(before.cache_hits),
        cache_misses: after.cache_misses.saturating_sub(before.cache_misses),
    }
}

/// The reviewed git SHA of the running worktree.
fn git_sha() -> String {
    if let Ok(out) = Command::new("git").args(["rev-parse", "HEAD"]).output() {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return s;
            }
        }
    }
    "unknown".to_string()
}

/// The model and tokenizer revisions from the committed classifier manifest.
fn revisions() -> (String, String) {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("modelcar")
        .join("classifier-manifest.json");
    if let Ok(raw) = std::fs::read_to_string(path) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
            if let Some(rev) = v
                .get("source")
                .and_then(|s| s.get("revision"))
                .and_then(|r| r.as_str())
            {
                // The tokenizer ships in the same pinned source revision.
                return (rev.to_string(), rev.to_string());
            }
        }
    }
    ("unknown".to_string(), "unknown".to_string())
}

/// The CPU model via env (`CPU_MODEL`) or std/platform probes.
fn cpu_model() -> String {
    if let Ok(v) = env::var("CPU_MODEL") {
        if !v.trim().is_empty() {
            return v;
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Ok(out) = Command::new("sysctl")
            .args(["-n", "machdep.cpu.brand_string"])
            .output()
        {
            if out.status.success() {
                let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !s.is_empty() {
                    return s;
                }
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(contents) = std::fs::read_to_string("/proc/cpuinfo") {
            for line in contents.lines() {
                if let Some(v) = line.strip_prefix("model name") {
                    let v = v.trim_start_matches(':').trim();
                    if !v.is_empty() {
                        return v.to_string();
                    }
                }
            }
        }
    }
    env::consts::ARCH.to_string()
}

/// A compact local timestamp (via the platform `date`, else epoch seconds).
fn timestamp() -> String {
    if let Ok(out) = Command::new("date").args(["+%Y%m%d-%H%M%S"]).output() {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return s;
            }
        }
    }
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

// ---- machine-readable report (serde) ----

#[derive(serde::Serialize)]
struct Report {
    manifest: Manifest,
    scenarios: Vec<ScenarioReport>,
}

/// The HOMELAB.md manifest fields the runner can capture locally, passed as a
/// single unit into [`build_report`].
struct ManifestInput {
    git_sha: String,
    model_dir: String,
    model_revision: String,
    tokenizer_revision: String,
    cpu_model: String,
    warmup: u64,
    measure: u64,
}

#[derive(serde::Serialize)]
struct Manifest {
    git_sha: String,
    model_dir: String,
    model_revision: String,
    tokenizer_revision: String,
    backend: &'static str,
    topology: &'static str,
    cpu_model: String,
    warmup_requests: u64,
    measured_requests: u64,
}

#[derive(serde::Serialize)]
struct ScenarioReport {
    cache_mode: String,
    sequence_length: usize,
    actual_token_count: usize,
    concurrency: u64,
    warmup_requests: u64,
    measured_requests: u64,
    throughput_req_per_s: f64,
    errors: u64,
    latency_ms: LatencyMs,
    stages_ms: StagesMs,
}

#[derive(serde::Serialize)]
struct LatencyMs {
    p50: f64,
    p90: f64,
    p95: f64,
    p99: f64,
    max: f64,
}

#[derive(serde::Serialize)]
struct StagesMs {
    queue: f64,
    tokenize: f64,
    forward: f64,
    total: f64,
}

fn ms(d: std::time::Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn build_report(m: &ManifestInput, scenarios: &[Scenario]) -> Report {
    let manifest = Manifest {
        git_sha: m.git_sha.clone(),
        model_dir: m.model_dir.clone(),
        model_revision: m.model_revision.clone(),
        tokenizer_revision: m.tokenizer_revision.clone(),
        backend: "candle",
        topology: "loopback",
        cpu_model: m.cpu_model.clone(),
        warmup_requests: m.warmup,
        measured_requests: m.measure,
    };
    let scenarios = scenarios
        .iter()
        .map(|s| ScenarioReport {
            cache_mode: match s.cache_mode {
                CacheMode::Hit => "hit".to_string(),
                CacheMode::Miss => "miss".to_string(),
            },
            sequence_length: s.target_length,
            actual_token_count: s.actual_token_count,
            concurrency: s.concurrency,
            warmup_requests: s.warmup,
            measured_requests: s.measured,
            throughput_req_per_s: s.throughput,
            errors: s.errors,
            latency_ms: LatencyMs {
                p50: ms(s.dist.p50()),
                p90: ms(s.dist.p90()),
                p95: ms(s.dist.p95()),
                p99: ms(s.dist.p99()),
                max: ms(s.dist.max()),
            },
            stages_ms: StagesMs {
                queue: ms(s.stage.queue),
                tokenize: ms(s.stage.tokenize),
                forward: ms(s.stage.forward),
                total: ms(s.stage.total),
            },
        })
        .collect();
    Report {
        manifest,
        scenarios,
    }
}

/// Print a human-readable table on stdout.
fn print_table(report: &Report) {
    let m = &report.manifest;
    println!("== llm-d-sc benchmark runner (AC-011) ==");
    println!(
        "git_sha={} backend={} topology={} cpu={}",
        m.git_sha, m.backend, m.topology, m.cpu_model
    );
    println!(
        "model_dir={} model_revision={} tokenizer_revision={}",
        m.model_dir, m.model_revision, m.tokenizer_revision
    );
    println!(
        "warmup_requests={} measured_requests={}\n",
        m.warmup_requests, m.measured_requests
    );

    println!(
        "{:<6} {:<6} {:<8} {:<8} {:<8} {:<8} {:<8} {:<8} {:<8} {:<8} {:<9} {:<7} {:<6} {:<9} {:<10} {:<9} {:<9}",
        "mode",
        "seq",
        "actual",
        "conc",
        "warmup",
        "measure",
        "p50ms",
        "p90ms",
        "p95ms",
        "p99ms",
        "maxms",
        "req/s",
        "errors",
        "queue_ms",
        "tok_ms",
        "fwd_ms",
        "tot_ms",
    );
    for s in &report.scenarios {
        println!(
            "{:<6} {:<6} {:<8} {:<8} {:<8} {:<8} {:<8.2} {:<8.2} {:<8.2} {:<8.2} {:<9.2} {:<7.1} {:<6} {:<9.3} {:<10.3} {:<9.3} {:<9.3}",
            s.cache_mode,
            s.sequence_length,
            s.actual_token_count,
            s.concurrency,
            s.warmup_requests,
            s.measured_requests,
            s.latency_ms.p50,
            s.latency_ms.p90,
            s.latency_ms.p95,
            s.latency_ms.p99,
            s.latency_ms.max,
            s.throughput_req_per_s,
            s.errors,
            s.stages_ms.queue,
            s.stages_ms.tokenize,
            s.stages_ms.forward,
            s.stages_ms.total,
        );
    }
}
