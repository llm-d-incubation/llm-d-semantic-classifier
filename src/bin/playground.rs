//! Interactive demo playground for the semantic cache (feature `playground`).
//!
//! This binary is NOT part of the served product — it exists to demo and poke at
//! the L1 exact / L2 semantic cache tiers from a browser. It hosts the REAL
//! production path in-process: it loads and warms the resident Candle classifier,
//! binds the same [`ClassifyServer`] the service binary uses (so the environment
//! selects the `redis-semantic` L2 tier exactly as in production), and drives it
//! through the blocking [`ClassifyClient`].
//!
//! Why in-process: the gRPC response carries only ranked labels + revisions, and
//! there is no metrics HTTP endpoint. The ONLY way to tell whether a request was
//! an L1 exact hit, an L2 semantic hit, or a compute miss is to read the
//! in-process [`MetricsSnapshot`] counter deltas around each call. So the UI
//! backend must host the server to observe those counters. Requests are handled
//! one at a time (single-threaded loop), which keeps each delta attributable to
//! exactly one classify call.
//!
//! Served surface (plain HTTP over `tiny_http`, same-origin, no framework):
//!   GET  /              -> the single-page UI (embedded at build time)
//!   GET  /api/info      -> classifier id + cache strategy + running totals
//!   POST /api/classify  -> { "text": "..." } -> ranked labels + cache tier + latency

use std::time::Instant;

use llm_d_sc::classify::{load_and_warm_modelcar, ClassifierRuntime};
use llm_d_sc::config::CacheConfig;
use llm_d_sc::grpc::classify::{ClassifyClient, ClassifyRequest, ClassifyServer};
use llm_d_sc::metrics::MetricsSnapshot;

/// The single-page UI, embedded so the binary is self-contained.
const INDEX_HTML: &str = include_str!("playground.html");

/// Default per-classifier model directory (matches the CLI convention).
const DEFAULT_MODEL_DIR: &str = "artifacts/models/complexity";
/// Default address the browser UI is served on.
const DEFAULT_UI_ADDR: &str = "127.0.0.1:8080";

fn main() -> std::io::Result<()> {
    let model_dir =
        std::env::var("LLM_D_SC_MODEL_DIR").unwrap_or_else(|_| DEFAULT_MODEL_DIR.to_string());
    let ui_addr =
        std::env::var("LLM_D_SC_PLAYGROUND_ADDR").unwrap_or_else(|_| DEFAULT_UI_ADDR.to_string());

    // Load + warm the resident Candle classifier. ANY failure here (missing
    // ModelCar dir, bad layout, warmup forward failed) aborts with an
    // actionable error — a directory that merely exists must not serve.
    eprintln!("llm-d-sc playground: loading model from {model_dir} …");
    let classifier = load_and_warm_modelcar(&model_dir).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("could not load/warm model at {model_dir}: {e}\n\nRun ./hack/fetch-model --classifier complexity (or point LLM_D_SC_MODEL_DIR at a ModelCar dir)."),
        )
    })?;
    let meta = classifier.metadata();
    let classifier_id = meta.classifier_id.clone();

    // Bind the production server in-process. `bind_with_classifier` reads the
    // cache strategy from the environment (LLM_D_SC_CACHE=redis-semantic +
    // LLM_D_SC_REDIS_URL), and fails open to the exact cache if Redis is
    // unreachable or the feature is off — the UI keeps working either way.
    let server = ClassifyServer::bind_with_classifier("127.0.0.1:0", classifier)?;
    let grpc_addr = server.local_addr();
    let mut client = ClassifyClient::connect(&grpc_addr)?;

    // Snapshot of the cache config purely for display in the UI header.
    let cache_cfg = CacheConfig::from_env().unwrap_or_else(|_| CacheConfig {
        strategy: "exact".into(),
        redis_url: None,
        threshold: 0.90,
        ttl_secs: 86_400,
        timeout_ms: 50,
    });

    // Prime the semantic tier: RediSearch creates its vector index lazily on the
    // first insert, so the very first lookup hits a not-yet-existent index and
    // reads as DEGRADED (fail-open). A throwaway warmup classify creates the
    // index before any user request, and the post-warmup snapshot below becomes
    // the baseline the UI totals are reported against — so warmup activity stays
    // invisible and user-facing totals start at zero.
    let _ = client.classify(ClassifyRequest {
        request_id: "pg-warmup".to_string(),
        session_id: "playground".to_string(),
        context: "llm-d-sc playground warmup probe".to_string(),
        signals: Vec::new(),
    });
    let baseline = server.metrics_snapshot();

    let http = tiny_http::Server::http(&ui_addr).map_err(|e| {
        std::io::Error::other(format!("could not bind playground UI on {ui_addr}: {e}"))
    })?;

    eprintln!(
        "llm-d-sc playground: classifier '{classifier_id}', cache strategy '{}', gRPC {grpc_addr}",
        cache_cfg.strategy
    );
    eprintln!();
    eprintln!("    ▶  Playground UI ready:  http://{ui_addr}");
    eprintln!();

    let mut request_seq: u64 = 0;

    // Single-threaded request loop: handling one request at a time makes each
    // metrics delta attributable to exactly one classify call.
    for mut request in http.incoming_requests() {
        let method = request.method().clone();
        let url = request.url().to_string();

        let (status, content_type, body): (u16, &str, String) = match (method, url.as_str()) {
            (tiny_http::Method::Get, "/") => {
                (200, "text/html; charset=utf-8", INDEX_HTML.to_string())
            }
            (tiny_http::Method::Get, "/api/info") => (
                200,
                "application/json",
                info_json(
                    &classifier_id,
                    &cache_cfg,
                    &server.metrics_snapshot(),
                    &baseline,
                ),
            ),
            (tiny_http::Method::Post, "/api/classify") => {
                let mut raw = String::new();
                let _ = request.as_reader().read_to_string(&mut raw);
                match handle_classify(&mut client, &server, &raw, &mut request_seq, &baseline) {
                    Ok(json) => (200, "application/json", json),
                    Err(msg) => (200, "application/json", error_json(&msg)),
                }
            }
            _ => (404, "text/plain", "not found".to_string()),
        };

        let header = tiny_http::Header::from_bytes(&b"Content-Type"[..], content_type.as_bytes())
            .expect("static content-type header is valid");
        let response = tiny_http::Response::from_string(body)
            .with_status_code(status)
            .with_header(header);
        let _ = request.respond(response);
    }

    Ok(())
}

/// Run one classify call and describe the outcome, including which cache tier
/// served it (derived from the metrics counter delta around the call).
fn handle_classify(
    client: &mut ClassifyClient,
    server: &ClassifyServer,
    raw_body: &str,
    seq: &mut u64,
    baseline: &MetricsSnapshot,
) -> Result<String, String> {
    let text =
        parse_text(raw_body).ok_or_else(|| "request body must be {\"text\": \"…\"}".to_string())?;
    if text.trim().is_empty() {
        return Err("prompt text is empty".to_string());
    }

    *seq += 1;
    let request = ClassifyRequest {
        request_id: format!("pg-{seq}"),
        session_id: "playground".to_string(),
        context: text,
        // Empty = no signal constraint, so this works against any served taxonomy.
        signals: Vec::new(),
    };

    let before = server.metrics_snapshot();
    let started = Instant::now();
    let response = client
        .classify(request)
        .map_err(|s| format!("classify failed: {} ({})", s.message(), s.code()))?;
    let latency_ms = started.elapsed().as_secs_f64() * 1000.0;
    let after = server.metrics_snapshot();

    let tier = classify_tier(&before, &after);
    Ok(response_json(&response, tier, latency_ms, &after, baseline))
}

/// Partition a served request into a cache tier from the counter delta.
///
/// | tier            | Δcache_hits | Δl2_hits | Δl2_degraded |
/// |-----------------|-------------|----------|--------------|
/// | L1_EXACT_HIT    | +1          | 0        | 0            |
/// | L2_SEMANTIC_HIT | 0           | +1       | 0            |
/// | DEGRADED        | 0           | 0        | +1           |
/// | COMPUTE_MISS    | 0           | 0        | 0            |
fn classify_tier(before: &MetricsSnapshot, after: &MetricsSnapshot) -> &'static str {
    if after.cache_hits > before.cache_hits {
        "L1_EXACT_HIT"
    } else if after.l2_hits > before.l2_hits {
        "L2_SEMANTIC_HIT"
    } else if after.l2_degraded > before.l2_degraded {
        "DEGRADED"
    } else {
        "COMPUTE_MISS"
    }
}

/// Extract the `text` field from a `{"text": "..."}` JSON body.
fn parse_text(raw: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(raw).ok()?;
    v.get("text")?.as_str().map(|s| s.to_string())
}

/// Human-readable name for a proto `ClassificationStatus` discriminant.
fn status_name(status: i32) -> &'static str {
    match status {
        1 => "OK",
        2 => "ABSTAIN",
        3 => "UNAVAILABLE",
        _ => "UNSPECIFIED",
    }
}

/// Running totals since serving began, i.e. the live snapshot minus the
/// post-warmup baseline (so index-priming activity is not shown to the user).
fn totals_json(snap: &MetricsSnapshot, baseline: &MetricsSnapshot) -> serde_json::Value {
    serde_json::json!({
        "cache_hits": snap.cache_hits.saturating_sub(baseline.cache_hits),
        "cache_misses": snap.cache_misses.saturating_sub(baseline.cache_misses),
        "l2_hits": snap.l2_hits.saturating_sub(baseline.l2_hits),
        "l2_misses": snap.l2_misses.saturating_sub(baseline.l2_misses),
        "l2_degraded": snap.l2_degraded.saturating_sub(baseline.l2_degraded),
    })
}

fn response_json(
    response: &llm_d_sc::grpc::classify::ClassifyResponse,
    tier: &str,
    latency_ms: f64,
    totals: &MetricsSnapshot,
    baseline: &MetricsSnapshot,
) -> String {
    let ranked: Vec<serde_json::Value> = response
        .ranked
        .iter()
        .map(|r| serde_json::json!({ "label": r.label, "score": r.score }))
        .collect();
    serde_json::json!({
        "tier": tier,
        "latency_ms": format!("{latency_ms:.1}"),
        "status": status_name(response.status),
        "classifier_id": response.classifier_id,
        "model_revision": response.model_revision,
        "tokenizer_revision": response.tokenizer_revision,
        "taxonomy_revision": response.taxonomy_revision,
        "ranked": ranked,
        "totals": totals_json(totals, baseline),
    })
    .to_string()
}

fn info_json(
    classifier_id: &str,
    cfg: &CacheConfig,
    totals: &MetricsSnapshot,
    baseline: &MetricsSnapshot,
) -> String {
    serde_json::json!({
        "classifier_id": classifier_id,
        "cache_strategy": cfg.strategy,
        "redis_url": cfg.redis_url,
        "threshold": cfg.threshold,
        "totals": totals_json(totals, baseline),
    })
    .to_string()
}

fn error_json(message: &str) -> String {
    serde_json::json!({ "error": message }).to_string()
}
