//! Semantic **response** cache (L2 for completions).
//!
//! The classification L2 cache ([`crate::cache::redis`]) stores a
//! `ClassificationResult` keyed by a prompt embedding. This module reuses the
//! same Redis Stack / RediSearch vector-KNN machinery to cache the far more
//! expensive artifact: the LLM's chat-completion response itself.
//!
//! It is deliberately kept in a **separate** RediSearch index
//! (`resp_semantic_idx`) and key namespace (`resp:`) so it never collides with
//! the classification cache — both can live in one Redis. The payload is the
//! raw upstream completion JSON, and the identity tag is the model name, so a
//! near-duplicate prompt to the *same* model can be served from cache while a
//! different model still misses.
//!
//! Everything here is best-effort and fail-open: any Redis error is a cache
//! miss (on lookup) or a silently-dropped write (on insert). Inference
//! correctness never depends on the cache being up.

use std::time::Duration;

use r2d2::Pool;
use redis::Client;

use crate::cache::redis_codec::{cosine_score_from_distance, vector_to_bytes};

/// RediSearch index for the response cache, distinct from the classification
/// index (`sc_semantic_idx`) so the two never share documents.
fn resp_index_name() -> &'static str {
    "resp_semantic_idx"
}

/// A semantic cache for full chat-completion responses.
pub struct RedisResponseCache {
    pool: Pool<Client>,
    threshold: f32,
    ttl_secs: u64,
    timeout: Duration,
}

impl RedisResponseCache {
    /// Connect and build a small connection pool. Returns an error only for a
    /// malformed URL or pool-construction failure; a *reachable* Redis is not
    /// required at construction time (operations fail open later).
    pub fn connect(
        redis_url: &str,
        threshold: f32,
        ttl_secs: u64,
        timeout_ms: u64,
    ) -> Result<Self, String> {
        let timeout = Duration::from_millis(timeout_ms.max(50));
        let client = Client::open(redis_url).map_err(|e| e.to_string())?;
        let pool = Pool::builder()
            .max_size(4)
            .min_idle(Some(0))
            .connection_timeout(timeout)
            .build(client)
            .map_err(|e| e.to_string())?;
        Ok(Self {
            pool,
            threshold,
            ttl_secs,
            timeout,
        })
    }

    fn conn(&self) -> Result<r2d2::PooledConnection<Client>, String> {
        let conn = self.pool.get().map_err(|e| e.to_string())?;
        conn.set_read_timeout(Some(self.timeout)).ok();
        conn.set_write_timeout(Some(self.timeout)).ok();
        Ok(conn)
    }

    /// Return a cached completion whose prompt embedding is within the cosine
    /// threshold for the same `model`. `None` on a miss or any error.
    pub fn lookup(&self, embedding: &[f32], model: &str) -> Option<String> {
        let blob = vector_to_bytes(embedding);
        let query = format!(
            "(@model:{{{}}})=>[KNN 1 @vec $BLOB AS dist]",
            escape_tag_value(model)
        );
        let mut conn = self.conn().ok()?;
        let reply: redis::Value = redis::cmd("FT.SEARCH")
            .arg(resp_index_name())
            .arg(&query)
            .arg("PARAMS")
            .arg(2)
            .arg("BLOB")
            .arg(blob.as_slice())
            .arg("SORTBY")
            .arg("dist")
            .arg("RETURN")
            .arg(2)
            .arg("dist")
            .arg("payload")
            .arg("DIALECT")
            .arg(2)
            .query(&mut *conn)
            .ok()?;
        match parse_knn_reply(&reply) {
            Some((distance, payload)) if cosine_score_from_distance(distance) >= self.threshold => {
                Some(payload)
            }
            _ => None,
        }
    }

    /// Store a completion `payload` under the prompt `embedding` + `model`.
    /// Best-effort: creates the index lazily on first write and drops any error.
    pub fn insert(&self, embedding: &[f32], model: &str, payload: &str) {
        let _ = self.try_insert(embedding, model, payload);
    }

    fn try_insert(&self, embedding: &[f32], model: &str, payload: &str) -> Result<(), String> {
        let blob = vector_to_bytes(embedding);
        let key = format!("resp:{}:{}", model, blake3::hash(&blob).to_hex());
        let mut conn = self.conn()?;

        // Lazily create the index; ignore "Index already exists".
        let _ = redis::cmd("FT.CREATE")
            .arg(resp_index_name())
            .arg("ON")
            .arg("HASH")
            .arg("PREFIX")
            .arg(1)
            .arg("resp:")
            .arg("SCHEMA")
            .arg("model")
            .arg("TAG")
            .arg("payload")
            .arg("TEXT")
            .arg("vec")
            .arg("VECTOR")
            .arg("FLAT")
            .arg(6)
            .arg("TYPE")
            .arg("FLOAT32")
            .arg("DIM")
            .arg(embedding.len())
            .arg("DISTANCE_METRIC")
            .arg("COSINE")
            .query::<redis::Value>(&mut *conn);

        redis::cmd("HSET")
            .arg(&key)
            .arg("model")
            .arg(model)
            .arg("payload")
            .arg(payload)
            .arg("vec")
            .arg(blob.as_slice())
            .query::<redis::Value>(&mut *conn)
            .map_err(|e| e.to_string())?;

        redis::cmd("EXPIRE")
            .arg(&key)
            .arg(self.ttl_secs)
            .query::<redis::Value>(&mut *conn)
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

/// Escape a TAG value so RediSearch treats it as one literal token (mirrors the
/// classification cache).
fn escape_tag_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
        } else {
            out.push('\\');
            out.push(c);
        }
    }
    out
}

fn as_string(v: &redis::Value) -> Option<String> {
    match v {
        redis::Value::BulkString(bytes) => Some(String::from_utf8_lossy(bytes).into_owned()),
        redis::Value::SimpleString(s) => Some(s.clone()),
        _ => None,
    }
}

/// Extract `(distance, payload)` from an `FT.SEARCH ... RETURN 2 dist payload`
/// reply. `None` for an empty result set or any unexpected shape (fail-open).
fn parse_knn_reply(value: &redis::Value) -> Option<(f32, String)> {
    // FT.SEARCH returns: [count, key, [field, val, field, val, ...], ...]
    if let redis::Value::Array(items) = value {
        if let Some(redis::Value::Array(fields)) = items.get(2) {
            let mut dist: Option<f32> = None;
            let mut payload: Option<String> = None;
            let mut i = 0;
            while i + 1 < fields.len() {
                let name = as_string(&fields[i]);
                let val = as_string(&fields[i + 1]);
                match name.as_deref() {
                    Some("dist") => dist = val.and_then(|s| s.parse::<f32>().ok()),
                    Some("payload") => payload = val,
                    _ => {}
                }
                i += 2;
            }
            if let (Some(d), Some(p)) = (dist, payload) {
                return Some((d, p));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resp_index_name_is_distinct_from_classification_index() {
        assert_eq!(resp_index_name(), "resp_semantic_idx");
        assert_ne!(resp_index_name(), crate::cache::redis_codec::index_name());
    }

    #[test]
    fn escape_tag_value_escapes_model_punctuation() {
        assert_eq!(escape_tag_value("llama"), "llama");
        assert_eq!(
            escape_tag_value("llama-32-3b-instruct"),
            "llama\\-32\\-3b\\-instruct"
        );
    }

    fn knn_reply(dist: &str, payload: &str) -> redis::Value {
        redis::Value::Array(vec![
            redis::Value::Int(1),
            redis::Value::BulkString(b"resp:m:abc".to_vec()),
            redis::Value::Array(vec![
                redis::Value::BulkString(b"dist".to_vec()),
                redis::Value::BulkString(dist.as_bytes().to_vec()),
                redis::Value::BulkString(b"payload".to_vec()),
                redis::Value::BulkString(payload.as_bytes().to_vec()),
            ]),
        ])
    }

    #[test]
    fn parse_knn_reply_extracts_distance_and_payload() {
        let (dist, payload) =
            parse_knn_reply(&knn_reply("0.03", "{\"choices\":[]}")).expect("must parse");
        assert!((dist - 0.03).abs() < 1e-6);
        assert_eq!(payload, "{\"choices\":[]}");
    }

    #[test]
    fn parse_knn_reply_is_none_on_empty_result_set() {
        let value = redis::Value::Array(vec![redis::Value::Int(0)]);
        assert!(parse_knn_reply(&value).is_none());
    }

    #[test]
    fn parse_knn_reply_is_none_on_unexpected_shape() {
        assert!(parse_knn_reply(&redis::Value::Okay).is_none());
        assert!(parse_knn_reply(&redis::Value::Nil).is_none());
    }

    #[test]
    fn threshold_gates_a_near_miss() {
        // A distance of 0.2 -> cosine score 0.8; below a 0.92 threshold it must
        // be rejected, above it accepted. This mirrors the lookup() gate without
        // needing a live Redis.
        let score = cosine_score_from_distance(0.2);
        assert!(score < 0.92);
        assert!(cosine_score_from_distance(0.05) >= 0.92);
    }
}
