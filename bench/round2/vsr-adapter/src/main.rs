//! `llm-d-sc-vsr-adapter` — serves llm-d-sc over vLLM Semantic Router's
//! `http_classify` contract.
//!
//! vLLM Semantic Router classifies in-process via its own Candle binding, but it
//! also supports remote heads: a classifier declared `type: sequence_classifier`
//! backed by an external model whose endpoint speaks a small HTTP contract
//! (`src/semantic-router/pkg/classification/http_classifier.go`):
//!
//!   POST /classify        Authorization: Bearer <access_key>
//!   request   {"inputs": "<text>"}
//!   response  [{"label": "...", "score": 0.93}, ...]
//!
//! llm-d-sc already produces exactly that shape -- `Classify` returns `ranked`
//! as (label, score) pairs over a versioned taxonomy -- so no translation of
//! meaning is required, only of transport. That is the whole adapter: gRPC in,
//! JSON out, labels and scores passed through untouched.
//!
//! Deliberately NOT re-normalising or re-ranking. The router requires the
//! declared labels with their full distribution; inventing scores here would
//! make llm-d-sc's taxonomy revision meaningless on the far side.
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

use http_body_util::BodyExt;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;

pub mod pb {
    tonic::include_proto!("classify");
}
use pb::classify_client::ClassifyClient;

#[derive(Deserialize)]
struct ClassifyReq {
    inputs: String,
}

#[derive(Serialize)]
struct LabelScore {
    label: String,
    score: f32,
}

type Body = http_body_util::Full<bytes::Bytes>;

fn json(status: StatusCode, v: &impl Serialize) -> Response<Body> {
    let b = serde_json::to_vec(v).unwrap_or_else(|_| b"[]".to_vec());
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(http_body_util::Full::new(bytes::Bytes::from(b)))
        .unwrap()
}

async fn handle(
    req: Request<hyper::body::Incoming>,
    chan: Arc<tonic::transport::Channel>,
    token: Arc<Option<String>>,
) -> Result<Response<Body>, Infallible> {
    if req.uri().path() == "/health" {
        return Ok(json(StatusCode::OK, &serde_json::json!({"status":"ok"})));
    }
    if req.method() != hyper::Method::POST || req.uri().path() != "/classify" {
        return Ok(json(StatusCode::NOT_FOUND, &serde_json::json!({"error":"not found"})));
    }
    // The router sends a bearer token when `access_key` is configured. Enforce it
    // only when one is set, so the adapter is usable unauthenticated in a lab.
    if let Some(expected) = token.as_ref() {
        let got = req
            .headers()
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .unwrap_or("");
        if got != expected {
            return Ok(json(StatusCode::UNAUTHORIZED, &serde_json::json!({"error":"unauthorized"})));
        }
    }

    let body = match req.into_body().collect().await {
        Ok(b) => b.to_bytes(),
        Err(_) => return Ok(json(StatusCode::BAD_REQUEST, &serde_json::json!({"error":"bad body"}))),
    };
    let parsed: ClassifyReq = match serde_json::from_slice(&body) {
        Ok(p) => p,
        Err(e) => {
            return Ok(json(StatusCode::BAD_REQUEST,
                           &serde_json::json!({"error": format!("bad request: {e}")})))
        }
    };

    let mut client = ClassifyClient::new((*chan).clone());
    let rpc = pb::ClassifyRequest {
        request_id: format!("vsr-{}", std::process::id()),
        session_id: String::new(),
        context: parsed.inputs,
        signals: vec![],
        // FULL: the router hands us a complete prompt, never a turn delta, so
        // declaring DELTA here would make llm-d-sc ABSTAIN on every request.
        context_completeness: 1,
    };
    match client.classify(rpc).await {
        Ok(resp) => {
            let r = resp.into_inner();
            // status 1 == OK. ABSTAIN/UNAVAILABLE carry no ranking, and the
            // router's contract requires the declared labels, so surface those
            // as an error rather than an empty distribution it cannot align.
            if r.status != 1 || r.ranked.is_empty() {
                return Ok(json(StatusCode::SERVICE_UNAVAILABLE,
                               &serde_json::json!({"error":"classifier returned no ranking",
                                                   "status": r.status})));
            }
            // llm-d-sc emits COSINE SIMILARITIES in [-1, 1] over its taxonomy.
            // vLLM SR's http_classify contract requires a probability
            // distribution: alignScoresToMapping rejects any response whose
            // scores do not sum to ~1.0, and the docs are explicit that "sigmoid
            // multi-label outputs and label subsets are rejected". Passing the
            // raw ranking through therefore fails validation outright -- the two
            // systems agree on the RANKING but not on the score semantics.
            //
            // Softmax bridges them: it is monotonic, so llm-d-sc's ordering and
            // its argmax are preserved exactly, and the result sums to 1 by
            // construction. VSR_SOFTMAX_TEMPERATURE sharpens or flattens it.
            //
            // Be clear about what this is NOT: these are softmaxed similarities,
            // not calibrated probabilities. Any downstream threshold in the
            // router (`gte: 0.5` and friends) is a threshold on THIS transform,
            // and must be tuned against it rather than carried over from a model
            // that emits real posteriors.
            let temp: f32 = std::env::var("VSR_SOFTMAX_TEMPERATURE")
                .ok()
                .and_then(|v| v.parse().ok())
                .filter(|t: &f32| *t > 0.0)
                .unwrap_or(1.0);
            let raw: Vec<(String, f32)> =
                r.ranked.into_iter().map(|s| (s.label, s.score as f32)).collect();
            // Subtract the max before exponentiating: standard guard against
            // overflow at low temperature.
            let maxv = raw.iter().map(|(_, v)| *v).fold(f32::NEG_INFINITY, f32::max);
            let exps: Vec<f32> = raw.iter().map(|(_, v)| ((v - maxv) / temp).exp()).collect();
            let sum: f32 = exps.iter().sum();
            let out: Vec<LabelScore> = raw
                .into_iter()
                .zip(exps)
                .map(|((label, _), e)| LabelScore { label, score: e / sum })
                .collect();
            Ok(json(StatusCode::OK, &out))
        }
        Err(e) => Ok(json(StatusCode::BAD_GATEWAY,
                          &serde_json::json!({"error": format!("llm-d-sc: {}", e.code() as i32)}))),
    }
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let sc = std::env::var("LLM_D_SC_ENDPOINT")
        .unwrap_or_else(|_| "http://llm-d-sc:50051".to_string());
    let listen: SocketAddr = std::env::var("VSR_ADAPTER_LISTEN")
        .unwrap_or_else(|_| "0.0.0.0:8080".to_string())
        .parse()?;
    let token = Arc::new(std::env::var("VSR_ACCESS_KEY").ok().filter(|s| !s.is_empty()));

    // One long-lived channel, established up front: connect cost must never land
    // inside a classification the router is timing (its default budget is 5 s,
    // but a cold TLS/H2 handshake under load is exactly how a routing decision
    // gets silently lost).
    let chan = Arc::new(
        tonic::transport::Endpoint::from_shared(sc.clone())?
            .tcp_nodelay(true)
            .connect()
            .await?,
    );
    eprintln!("llm-d-sc-vsr-adapter: {listen} -> {sc}; POST /classify (http_classify contract); READY");

    let listener = TcpListener::bind(listen).await?;
    loop {
        let (stream, _) = listener.accept().await?;
        let _ = stream.set_nodelay(true);
        let chan = chan.clone();
        let token = token.clone();
        tokio::spawn(async move {
            let io = hyper_util::rt::TokioIo::new(stream);
            let _ = hyper::server::conn::http1::Builder::new()
                .serve_connection(io, service_fn(move |r| handle(r, chan.clone(), token.clone())))
                .await;
        });
    }
}
