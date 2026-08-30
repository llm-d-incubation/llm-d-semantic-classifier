//! Redis Stack (RediSearch) semantic cache. BEST-EFFORT and FAIL-OPEN: every
//! Redis interaction is wrapped so any error/timeout yields "no hit" (lookup)
//! or a dropped write (insert). Guarded by a circuit breaker so a dead Redis
//! costs one probe per cooldown, not one timeout per request.

use std::time::Duration;

use r2d2::Pool;
use redis::Client;

use crate::cache::breaker::CircuitBreaker;
use crate::cache::redis_codec::{
    cosine_score_from_distance, decode_result, encode_result, index_name, vector_to_bytes,
};
use crate::cache::SemanticCache;
use crate::classify::{ClassificationResult, Embedding};
use crate::config::CacheConfig;
use crate::metrics::Metrics;

pub struct RedisSemanticCache {
    pool: Pool<Client>,
    threshold: f32,
    ttl_secs: u64,
    timeout_ms: u64,
    breaker: CircuitBreaker,
    metrics: Metrics,
}

impl RedisSemanticCache {
    /// Connect, size the pool small (matches the inference pool width), set
    /// per-op timeouts, and ensure the vector index exists.
    pub fn connect(cfg: &CacheConfig, metrics: Metrics) -> Result<RedisSemanticCache, String> {
        let url = cfg
            .redis_url
            .as_ref()
            .ok_or("redis-semantic requires a URL")?;
        let client = Client::open(url.as_str()).map_err(|e| e.to_string())?;
        let pool = Pool::builder()
            .max_size(8)
            // Do not eagerly dial Redis at build time: a currently-unreachable
            // Redis must not prevent the service from starting (fail-open).
            // Connections are established lazily on the first `pool.get()`.
            .min_idle(Some(0))
            .connection_timeout(Duration::from_millis(cfg.timeout_ms))
            .build(client)
            .map_err(|e| e.to_string())?;
        let cache = RedisSemanticCache {
            pool,
            threshold: cfg.threshold,
            ttl_secs: cfg.ttl_secs,
            timeout_ms: cfg.timeout_ms,
            breaker: CircuitBreaker::new(5, Duration::from_secs(10)),
            metrics,
        };
        // Best-effort index creation; a missing index only means lookups miss.
        // The dimension is not known until the first insert (RediSearch
        // requires DIM at FT.CREATE time), so the index is actually created
        // lazily in `insert`. This call is kept for symmetry with `connect`
        // establishing everything it can up front.
        let _ = cache.ensure_index();
        Ok(cache)
    }

    /// Placeholder for eager index creation. See the comment in `connect`:
    /// the index needs a vector DIM that is only known once the first
    /// embedding is inserted, so real creation happens lazily in `insert`.
    fn ensure_index(&self) -> Result<(), String> {
        Ok(())
    }

    /// Check out a pooled connection, apply the per-op timeout, and run `f`.
    /// Any pool/connection/command error is collapsed to a `String` so the
    /// caller can treat it uniformly as "Redis is unavailable right now".
    fn with_timeout_conn<T>(
        &self,
        f: impl FnOnce(&mut redis::Connection) -> redis::RedisResult<T>,
    ) -> Result<T, String> {
        let mut conn = self.pool.get().map_err(|e| e.to_string())?;
        let timeout = Duration::from_millis(self.timeout_ms);
        conn.set_read_timeout(Some(timeout)).ok();
        conn.set_write_timeout(Some(timeout)).ok();
        f(&mut conn).map_err(|e| e.to_string())
    }
}

impl SemanticCache for RedisSemanticCache {
    fn lookup(&self, embedding: &Embedding, identity: &str) -> Option<ClassificationResult> {
        if !self.breaker.allow() {
            return None;
        }
        let blob = vector_to_bytes(&embedding.vector);
        // FT.SEARCH sc_semantic_idx "(@identity:{<tag>})=>[KNN 1 @vec $BLOB AS dist]"
        //   PARAMS 2 BLOB <blob> DIALECT 2 SORTBY dist RETURN 2 dist payload
        let query = format!(
            "(@identity:{{{}}})=>[KNN 1 @vec $BLOB AS dist]",
            escape_tag_value(identity)
        );
        let outcome = self.with_timeout_conn(|conn| {
            redis::cmd("FT.SEARCH")
                .arg(index_name())
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
                .query::<redis::Value>(conn)
        });
        match outcome {
            Ok(value) => {
                self.breaker.record_success();
                match parse_knn_reply(&value) {
                    Some((distance, payload))
                        if cosine_score_from_distance(distance) >= self.threshold =>
                    {
                        // A malformed stored payload is a corrupt reply, not a
                        // hit: recording a hit here would claim a result was
                        // served when `None` is actually returned.
                        match decode_result(&payload) {
                            Some(result) => {
                                self.metrics.record_l2_hit();
                                Some(result)
                            }
                            None => {
                                self.metrics.record_l2_degraded();
                                None
                            }
                        }
                    }
                    _ => {
                        self.metrics.record_l2_miss();
                        None
                    }
                }
            }
            Err(_) => {
                self.breaker.record_failure();
                self.metrics.record_l2_degraded();
                None
            }
        }
    }

    fn insert(&self, embedding: &Embedding, result: &ClassificationResult, identity: &str) {
        if !self.breaker.allow() {
            return;
        }
        let blob = vector_to_bytes(&embedding.vector);
        let dim = embedding.dim();
        let payload = encode_result(result);
        let key = format!("sc:{identity}:{}", blake3::hash(&blob).to_hex());
        let ttl = self.ttl_secs;
        let identity = identity.to_string();
        let outcome = self.with_timeout_conn(move |conn| {
            // Create the index lazily now that the vector dimension is known.
            // Ignore the result: an "already exists" error (or any other
            // trouble) here only means lookups may miss, never that the
            // caller's request fails.
            let _ = redis::cmd("FT.CREATE")
                .arg(index_name())
                .arg("ON")
                .arg("HASH")
                .arg("PREFIX")
                .arg(1)
                .arg("sc:")
                .arg("SCHEMA")
                .arg("identity")
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
                .arg(dim)
                .arg("DISTANCE_METRIC")
                .arg("COSINE")
                .query::<redis::Value>(conn);
            redis::cmd("HSET")
                .arg(&key)
                .arg("identity")
                .arg(&identity)
                .arg("payload")
                .arg(&payload)
                .arg("vec")
                .arg(blob.as_slice())
                .query::<redis::Value>(conn)?;
            redis::cmd("EXPIRE")
                .arg(&key)
                .arg(ttl)
                .query::<redis::Value>(conn)
        });
        match outcome {
            Ok(_) => self.breaker.record_success(),
            Err(_) => {
                self.breaker.record_failure();
                self.metrics.record_l2_degraded();
            }
        }
    }
}

/// Escape characters RediSearch treats as query syntax inside a TAG value, so
/// an identity tag containing '.', '-', or similar (e.g. a semver revision)
/// is matched literally rather than parsed as query syntax.
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

/// Extract (distance, payload) from a RediSearch FT.SEARCH reply, or None if the
/// reply shape is empty/unexpected. Fail-open: any parse surprise is a miss.
fn parse_knn_reply(value: &redis::Value) -> Option<(f32, String)> {
    // FT.SEARCH returns: [count, key, [field, val, field, val, ...], ...]
    if let redis::Value::Array(items) = value {
        // items[0] = count; the field array is the 3rd element (index 2).
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

fn as_string(v: &redis::Value) -> Option<String> {
    match v {
        redis::Value::BulkString(bytes) => Some(String::from_utf8_lossy(bytes).into_owned()),
        redis::Value::SimpleString(s) => Some(s.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A well-formed FT.SEARCH KNN-1 reply: `[count, key, [field, val, ...]]`.
    fn knn_reply(dist: &str, payload: &str) -> redis::Value {
        redis::Value::Array(vec![
            redis::Value::Int(1),
            redis::Value::BulkString(b"sc:some-key".to_vec()),
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
        let value = knn_reply("0.05", "{\"ok\":true}");
        let (dist, payload) = parse_knn_reply(&value).expect("well-formed reply must parse");
        assert!((dist - 0.05).abs() < 1e-6);
        assert_eq!(payload, "{\"ok\":true}");
    }

    #[test]
    fn parse_knn_reply_is_none_on_empty_result_set() {
        // FT.SEARCH with no matches replies just `[0]` — a clean miss, not a
        // malformed reply.
        let value = redis::Value::Array(vec![redis::Value::Int(0)]);
        assert!(parse_knn_reply(&value).is_none());
    }

    #[test]
    fn parse_knn_reply_is_none_on_unexpected_shape() {
        // Fail-open: any reply shape parse_knn_reply doesn't recognize is a miss.
        assert!(parse_knn_reply(&redis::Value::Okay).is_none());
        assert!(parse_knn_reply(&redis::Value::Nil).is_none());
    }

    #[test]
    fn escape_tag_value_escapes_non_alphanumeric_characters() {
        assert_eq!(escape_tag_value("complexity"), "complexity");
        assert_eq!(
            escape_tag_value("complexity|m-1.0|t|x"),
            "complexity\\|m\\-1\\.0\\|t\\|x"
        );
    }
}
