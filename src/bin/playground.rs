//! Interactive demo playground for the semantic cache (feature `playground`).
//!
//! This binary is NOT part of the served product — it exists to demo and poke at
//! the L1 exact / L2 semantic cache tiers from a browser. It hosts the REAL
//! production path in-process: it loads and warms one resident Candle classifier
//! per available taxonomy, binds the same [`ClassifyServer`] the service binary
//! uses for each (so the environment selects the `redis-semantic` L2 tier exactly
//! as in production), and drives them through the blocking [`ClassifyClient`].
//!
//! Multiple classifiers, one Redis: the two-tier cache namespaces every entry by
//! an identity tag that begins with the classifier id, so complexity, cost and
//! sensitivity share one Redis instance without ever reading each other's cached
//! labels. The UI picks which classifier a prompt is sent to.
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
//!   GET  /                    -> the single-page UI (embedded at build time)
//!   GET  /api/info            -> available classifiers (id + taxonomy + totals) + cache
//!   POST /api/classify        -> { "text": "...", "classifier": "..." }
//!                                -> ranked labels + cache tier + latency
//!   POST /v1/chat/completions -> OpenAI-compatible chat completion, served
//!                                through the semantic RESPONSE cache: a
//!                                near-duplicate prompt to the same model is
//!                                answered from Redis (header `x-praxis-cache:
//!                                hit`); a miss is forwarded to the upstream vLLM
//!                                (`LLM_D_SC_VLLM_URL`), returned, and cached
//!                                (`miss`). `stream:true` bypasses the cache
//!                                (`bypass`). Only active when both
//!                                `LLM_D_SC_VLLM_URL` and a Redis semantic cache
//!                                are configured.

use std::time::{Duration, Instant};

use llm_d_sc::cache::response::RedisResponseCache;
use llm_d_sc::classify::{CandleClassifier, ClassifierRuntime};
use llm_d_sc::config::CacheConfig;
use llm_d_sc::embedding::Embedder;
use llm_d_sc::grpc::classify::{ClassifyClient, ClassifyRequest, ClassifyServer};
use llm_d_sc::metrics::MetricsSnapshot;
use llm_d_sc::taxonomy::ClassifierDefinition;

/// The single-page UI, embedded so the binary is self-contained.
const INDEX_HTML: &str = include_str!("playground.html");

/// Base directory holding one ModelCar subdirectory per classifier.
const DEFAULT_MODELS_DIR: &str = "artifacts/models";
/// Classifiers offered when the environment does not name a set.
const DEFAULT_CLASSIFIERS: &str = "complexity,cost,sensitivity";
/// Default address the browser UI is served on.
const DEFAULT_UI_ADDR: &str = "127.0.0.1:8080";

/// One resident classifier hosted in-process: its own server, client, the
/// taxonomy it ranks, and the post-warmup baseline its totals are reported
/// against.
struct Classifier {
    /// Selector key exposed to the UI (the model subdirectory name).
    name: String,
    /// Identity the runtime reports (may differ from `name`).
    classifier_id: String,
    /// The full ranked taxonomy, captured from the warmup response.
    labels: Vec<String>,
    server: ClassifyServer,
    client: ClassifyClient,
    baseline: MetricsSnapshot,
}

/// State for the semantic RESPONSE-cache route (`/v1/chat/completions`): an
/// embedder to key completions by prompt similarity, the Redis-backed response
/// cache, and the upstream vLLM to forward misses to. Present only when both an
/// upstream and a Redis cache are configured.
struct ResponseCacheCtx {
    embedder: Embedder,
    cache: RedisResponseCache,
    vllm_url: String,
    threshold: f32,
    upstream_timeout: Duration,
}

fn main() -> std::io::Result<()> {
    let models_dir =
        std::env::var("LLM_D_SC_MODELS_DIR").unwrap_or_else(|_| DEFAULT_MODELS_DIR.to_string());
    let names = std::env::var("LLM_D_SC_PLAYGROUND_CLASSIFIERS")
        .unwrap_or_else(|_| DEFAULT_CLASSIFIERS.to_string());
    let ui_addr =
        std::env::var("LLM_D_SC_PLAYGROUND_ADDR").unwrap_or_else(|_| DEFAULT_UI_ADDR.to_string());

    // Load + warm every named classifier whose weights are present. A missing
    // model dir is skipped with a warning rather than fatal, so the demo still
    // runs with whatever subset is downloaded.
    let mut classifiers: Vec<Classifier> = Vec::new();
    for name in names.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let model_dir = format!("{models_dir}/{name}");
        if !std::path::Path::new(&format!("{model_dir}/model.safetensors")).exists() {
            eprintln!("llm-d-sc playground: skipping '{name}' — no model at {model_dir}");
            continue;
        }
        match load_classifier(name, &model_dir) {
            Ok(c) => {
                eprintln!(
                    "llm-d-sc playground: loaded '{}' (id '{}', {} labels)",
                    c.name,
                    c.classifier_id,
                    c.labels.len()
                );
                classifiers.push(c);
            }
            Err(e) => eprintln!("llm-d-sc playground: could not load '{name}': {e}"),
        }
    }

    if classifiers.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!(
                "no classifier models found under {models_dir} (tried: {names}).\n\nRun ./hack/fetch-model --classifier complexity (and cost, sensitivity), or use ./hack/playground."
            ),
        ));
    }

    // Cache config snapshot, purely for display in the UI header.
    let cache_cfg = CacheConfig::from_env().unwrap_or_else(|_| CacheConfig {
        strategy: "exact".into(),
        redis_url: None,
        threshold: 0.90,
        ttl_secs: 86_400,
        timeout_ms: 50,
    });

    // Optional semantic RESPONSE cache in front of an upstream vLLM. Active only
    // when both LLM_D_SC_VLLM_URL and a Redis backend are configured; otherwise
    // the /v1/chat/completions route is disabled (404). Keys completions by
    // prompt similarity using its own embedder (the primary classifier's model).
    let response_cache = build_response_cache(&models_dir, &cache_cfg);
    match &response_cache {
        Some(ctx) => eprintln!(
            "llm-d-sc playground: response cache ENABLED -> upstream {} (threshold {:.2})",
            ctx.vllm_url, ctx.threshold
        ),
        None => eprintln!(
            "llm-d-sc playground: response cache disabled (set LLM_D_SC_VLLM_URL + a redis-semantic cache to enable)"
        ),
    }

    let http = tiny_http::Server::http(&ui_addr).map_err(|e| {
        std::io::Error::other(format!("could not bind playground UI on {ui_addr}: {e}"))
    })?;

    eprintln!(
        "llm-d-sc playground: {} classifier(s), cache strategy '{}'",
        classifiers.len(),
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

        // Extra response headers set by a route (e.g. the response-cache tier).
        let mut extra_headers: Vec<tiny_http::Header> = Vec::new();

        let (status, content_type, body): (u16, &str, String) = match (method, url.as_str()) {
            (tiny_http::Method::Get, "/") => {
                (200, "text/html; charset=utf-8", INDEX_HTML.to_string())
            }
            (tiny_http::Method::Get, "/api/info") => {
                (200, "application/json", info_json(&classifiers, &cache_cfg))
            }
            (tiny_http::Method::Post, "/api/classify") => {
                let mut raw = String::new();
                let _ = request.as_reader().read_to_string(&mut raw);
                match handle_classify(&mut classifiers, &raw, &mut request_seq) {
                    Ok(json) => (200, "application/json", json),
                    Err(msg) => (200, "application/json", error_json(&msg)),
                }
            }
            (tiny_http::Method::Post, "/v1/chat/completions") => {
                let mut raw = String::new();
                let _ = request.as_reader().read_to_string(&mut raw);
                match handle_chat(response_cache.as_ref(), &raw) {
                    Ok((code, cache_status, json)) => {
                        if let Ok(h) = tiny_http::Header::from_bytes(
                            &b"x-praxis-cache"[..],
                            cache_status.as_bytes(),
                        ) {
                            extra_headers.push(h);
                        }
                        (code, "application/json", json)
                    }
                    Err(msg) => (502, "application/json", error_json(&msg)),
                }
            }
            _ => (404, "text/plain", "not found".to_string()),
        };

        let header = tiny_http::Header::from_bytes(&b"Content-Type"[..], content_type.as_bytes())
            .expect("static content-type header is valid");
        let mut response = tiny_http::Response::from_string(body)
            .with_status_code(status)
            .with_header(header);
        for h in extra_headers {
            response = response.with_header(h);
        }
        let _ = request.respond(response);
    }

    Ok(())
}

/// Load one classifier, bind it in-process, capture its taxonomy, and snapshot
/// the post-warmup baseline.
///
/// The model weights and the taxonomy are independent: every shipped model is an
/// embedder ranked against anchor definitions, and the taxonomy (its labels and
/// `classifier_id`) is selected by name — NOT by the model directory. So each
/// model dir is paired with its matching built-in definition via
/// [`ClassifierDefinition::resolve`]; without this every classifier would default
/// to the same taxonomy and share one cache identity.
///
/// The warmup classify does double duty: it primes the RediSearch index (created
/// lazily on first insert, so the very first lookup would otherwise read as
/// DEGRADED) and its ranked response reveals the full taxonomy. The baseline
/// taken afterwards is what UI totals are reported against, so warmup activity
/// stays invisible and user-facing totals start at zero.
fn load_classifier(name: &str, model_dir: &str) -> std::io::Result<Classifier> {
    eprintln!("llm-d-sc playground: loading model from {model_dir} (taxonomy '{name}') …");
    let definition = ClassifierDefinition::resolve(name).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("no taxonomy definition for '{name}': {e}"),
        )
    })?;
    let classifier =
        CandleClassifier::from_modelcar_with(std::path::Path::new(model_dir), definition).map_err(
            |e| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("could not load model at {model_dir}: {e}"),
                )
            },
        )?;
    let classifier_id = classifier.metadata().classifier_id.clone();

    let server = ClassifyServer::bind_with_classifier("127.0.0.1:0", classifier)?;
    let grpc_addr = server.local_addr();
    let mut client = ClassifyClient::connect(&grpc_addr)?;

    let warm = client.classify(ClassifyRequest {
        request_id: format!("pg-warmup-{name}"),
        session_id: "playground".to_string(),
        context: "llm-d-sc playground warmup probe".to_string(),
        signals: Vec::new(),
    });
    let labels = warm
        .map(|r| r.ranked.into_iter().map(|s| s.label).collect())
        .unwrap_or_default();
    let baseline = server.metrics_snapshot();

    Ok(Classifier {
        name: name.to_string(),
        classifier_id,
        labels,
        server,
        client,
        baseline,
    })
}

/// Run one classify call against the requested classifier and describe the
/// outcome, including which cache tier served it (derived from that server's
/// metrics counter delta around the call).
fn handle_classify(
    classifiers: &mut [Classifier],
    raw_body: &str,
    seq: &mut u64,
) -> Result<String, String> {
    let (text, which) = parse_request(raw_body)?;
    if text.trim().is_empty() {
        return Err("prompt text is empty".to_string());
    }
    let entry = classifiers
        .iter_mut()
        .find(|c| c.name == which)
        .ok_or_else(|| format!("unknown classifier '{which}'"))?;

    *seq += 1;
    let request = ClassifyRequest {
        request_id: format!("pg-{seq}"),
        session_id: "playground".to_string(),
        context: text,
        // Empty = no signal constraint, so this works against any served taxonomy.
        signals: Vec::new(),
    };

    let before = entry.server.metrics_snapshot();
    let started = Instant::now();
    let response = entry
        .client
        .classify(request)
        .map_err(|s| format!("classify failed: {} ({})", s.message(), s.code()))?;
    let latency_ms = started.elapsed().as_secs_f64() * 1000.0;
    let after = entry.server.metrics_snapshot();

    let tier = classify_tier(&before, &after);
    Ok(response_json(
        &entry.name,
        &response,
        tier,
        latency_ms,
        &after,
        &entry.baseline,
    ))
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

/// Extract `text` (required) and `classifier` (defaults to the first built-in)
/// from a `{"text": "...", "classifier": "..."}` JSON body.
fn parse_request(raw: &str) -> Result<(String, String), String> {
    let v: serde_json::Value =
        serde_json::from_str(raw).map_err(|_| "request body must be JSON".to_string())?;
    let text = v
        .get("text")
        .and_then(|t| t.as_str())
        .ok_or_else(|| "request body must be {\"text\": \"…\"}".to_string())?
        .to_string();
    let which = v
        .get("classifier")
        .and_then(|c| c.as_str())
        .unwrap_or("complexity")
        .to_string();
    Ok((text, which))
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
    classifier: &str,
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
        "classifier": classifier,
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

fn info_json(classifiers: &[Classifier], cfg: &CacheConfig) -> String {
    let list: Vec<serde_json::Value> = classifiers
        .iter()
        .map(|c| {
            serde_json::json!({
                "name": c.name,
                "classifier_id": c.classifier_id,
                "labels": c.labels,
                "totals": totals_json(&c.server.metrics_snapshot(), &c.baseline),
            })
        })
        .collect();
    serde_json::json!({
        "cache_strategy": cfg.strategy,
        "redis_url": cfg.redis_url,
        "threshold": cfg.threshold,
        "classifiers": list,
    })
    .to_string()
}

fn error_json(message: &str) -> String {
    serde_json::json!({ "error": message }).to_string()
}

/// Build the response-cache context if both an upstream vLLM URL and a Redis
/// semantic backend are configured. Any load/connect failure logs and disables
/// the route (returns `None`) rather than aborting the playground.
fn build_response_cache(models_dir: &str, cache_cfg: &CacheConfig) -> Option<ResponseCacheCtx> {
    let vllm_url = std::env::var("LLM_D_SC_VLLM_URL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())?;
    let redis_url = cache_cfg.redis_url.clone()?;

    let threshold = std::env::var("LLM_D_SC_RESP_CACHE_THRESHOLD")
        .ok()
        .and_then(|s| s.parse::<f32>().ok())
        .unwrap_or(0.92);
    let ttl_secs = std::env::var("LLM_D_SC_RESP_CACHE_TTL")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(cache_cfg.ttl_secs);
    // The classification cache's timeout (LLM_D_SC_TIMEOUT_MS, default 50ms) is
    // too tight for a vector-KNN FT.SEARCH: the first (cold) search or a fresh
    // pooled connection can exceed it, and a timed-out lookup fails open to a
    // false miss. Use a dedicated, more generous bound (warm lookups are still
    // single-digit ms; this only caps the cold/slow path).
    let timeout_ms = std::env::var("LLM_D_SC_RESP_CACHE_TIMEOUT_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(1000);

    // The response cache keys completions by SEMANTIC similarity, so it needs a
    // general sentence-embedding model — NOT a task-tuned classifier model.
    // Classifier embeddings are anisotropic: unrelated prompts score ~0.97 and a
    // true paraphrase can score *lower* than an unrelated one, so an absolute
    // cosine threshold is meaningless there. Default to the baked all-MiniLM-L6-v2
    // baseline (clean separation: paraphrase ~0.99, unrelated ~0.75); override
    // with LLM_D_SC_RESP_EMBED_DIR.
    let embed_dir = std::env::var("LLM_D_SC_RESP_EMBED_DIR")
        .unwrap_or_else(|_| format!("{models_dir}/baseline-minilm"));
    let embedder = Embedder::load(
        format!("{embed_dir}/config.json"),
        format!("{embed_dir}/model.safetensors"),
        format!("{embed_dir}/tokenizer.json"),
        format!("{embed_dir}/1_Pooling/config.json"),
    )
    .map_err(|e| {
        eprintln!(
            "llm-d-sc playground: response cache disabled — embedder load failed at {embed_dir}: {e:?}"
        )
    })
    .ok()?;

    let cache = RedisResponseCache::connect(&redis_url, threshold, ttl_secs, timeout_ms)
        .map_err(|e| {
            eprintln!("llm-d-sc playground: response cache disabled — redis connect failed: {e}")
        })
        .ok()?;

    Some(ResponseCacheCtx {
        embedder,
        cache,
        vllm_url,
        threshold,
        upstream_timeout: Duration::from_secs(120),
    })
}

/// Serve one chat-completion request through the semantic response cache.
///
/// Returns `(http_status, cache_status, body_json)` where `cache_status` is
/// "hit" (served from Redis), "miss" (forwarded upstream then cached), or
/// "bypass" (streaming or non-embeddable prompt: forwarded, not cached).
fn handle_chat(
    ctx: Option<&ResponseCacheCtx>,
    raw: &str,
) -> Result<(u16, &'static str, String), String> {
    let ctx = ctx.ok_or("response cache route is disabled (LLM_D_SC_VLLM_URL not set)")?;
    let req: serde_json::Value =
        serde_json::from_str(raw).map_err(|_| "request body must be JSON".to_string())?;

    let stream = req.get("stream").and_then(|s| s.as_bool()).unwrap_or(false);
    let model = req
        .get("model")
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_string();

    // Cache only non-streaming requests whose last user message is a plain
    // string we can embed. Everything else is forwarded transparently.
    if !stream {
        if let Some(text) = last_user_message(&req) {
            if let Ok(vector) = ctx.embedder.embed(&text) {
                if let Some(hit) = ctx.cache.lookup(&vector, &model) {
                    return Ok((200, "hit", hit));
                }
                let (code, body) = forward_upstream(&ctx.vllm_url, raw, ctx.upstream_timeout)?;
                if code == 200 {
                    ctx.cache.insert(&vector, &model, &body);
                }
                return Ok((code, "miss", body));
            }
        }
    }

    let (code, body) = forward_upstream(&ctx.vllm_url, raw, ctx.upstream_timeout)?;
    Ok((code, "bypass", body))
}

/// Forward a chat-completion request body to the upstream vLLM and return
/// `(status, body)`. HTTP error statuses (4xx/5xx) are returned as-is so the
/// caller sees the real upstream error; only transport failures are `Err`.
fn forward_upstream(base: &str, body: &str, timeout: Duration) -> Result<(u16, String), String> {
    let url = format!("{}/v1/chat/completions", base.trim_end_matches('/'));
    match ureq::post(&url)
        .timeout(timeout)
        .set("Content-Type", "application/json")
        .send_string(body)
    {
        Ok(r) => {
            let code = r.status();
            let text = r
                .into_string()
                .map_err(|e| format!("reading upstream body: {e}"))?;
            Ok((code, text))
        }
        Err(ureq::Error::Status(code, r)) => Ok((code, r.into_string().unwrap_or_default())),
        Err(e) => Err(format!("upstream request failed: {e}")),
    }
}

/// The most recent `role:"user"` message's string content, if present. Array /
/// multimodal content is not embeddable here and yields `None` (bypass).
fn last_user_message(req: &serde_json::Value) -> Option<String> {
    req.get("messages")?
        .as_array()?
        .iter()
        .rev()
        .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("user"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_user_message_picks_the_final_user_turn() {
        let req = serde_json::json!({
            "messages": [
                {"role": "system", "content": "be brief"},
                {"role": "user", "content": "first"},
                {"role": "assistant", "content": "reply"},
                {"role": "user", "content": "second"}
            ]
        });
        assert_eq!(last_user_message(&req).as_deref(), Some("second"));
    }

    #[test]
    fn last_user_message_is_none_without_user_turn() {
        let req = serde_json::json!({"messages": [{"role": "system", "content": "x"}]});
        assert!(last_user_message(&req).is_none());
    }

    #[test]
    fn last_user_message_is_none_for_array_content() {
        // Multimodal content is an array, not a string, and is not embeddable.
        let req = serde_json::json!({
            "messages": [{"role": "user", "content": [{"type": "text", "text": "hi"}]}]
        });
        assert!(last_user_message(&req).is_none());
    }

    #[test]
    fn handle_chat_errors_when_disabled() {
        let out = handle_chat(None, "{}");
        assert!(out.is_err());
    }
}
