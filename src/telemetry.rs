//! Telemetry recording that never leaks raw prompt or session text (AC-014).
//!
//! AC-014 requires the service's default telemetry (logs, metrics, trace
//! capture) to never contain raw prompt or session text. This module provides a
//! small recorder whose default output surfaces request IDs and
//! context/session hashes (blake3) but NEVER the raw prompt text or the raw
//! session text. The raw context/session strings are consumed to derive hashes
//! and are not retained.

use std::sync::{Arc, Mutex};

/// A captured trace event: the request id plus context/session hashes.
///
/// No trace event ever carries the raw prompt or raw session text (AC-014).
#[derive(Debug, Clone)]
pub struct TraceEvent {
    /// The caller-supplied request id (an opaque identifier, never text).
    pub request_id: String,
    /// A blake3 hash of the context, prefixed `ctx_`.
    pub context_hash: String,
    /// A blake3 hash of the session id, prefixed `sess_`.
    pub session_hash: String,
}

/// One request fed to the telemetry recorder.
///
/// `context` and `session_id` are consumed to derive hashes and are never
/// retained in any default output.
#[derive(Debug, Clone)]
pub struct RequestEvent {
    /// The caller-supplied request id.
    pub request_id: String,
    /// The raw session id (hashed, never emitted verbatim).
    pub session_id: String,
    /// The raw context/prompt text (hashed, never emitted verbatim).
    pub context: String,
}

/// A telemetry recorder whose default output surfaces request ids and
/// context/session hashes but never raw prompt or session text (AC-014).
///
/// [`Clone`] shares the same underlying capture via [`Arc`]`<Mutex<_>>`, so the
/// pipeline and its server observe the same trace.
#[derive(Debug, Clone)]
pub struct Telemetry {
    events: Arc<Mutex<std::collections::VecDeque<TraceEvent>>>,
    capacity: usize,
}

impl Default for Telemetry {
    fn default() -> Self {
        Self::with_capacity(DEFAULT_TRACE_CAPACITY)
    }
}

/// Maximum retained trace events.
///
/// The capture is a bounded RING, not a log. `record_request` runs on the
/// production classify path for every request, so an unbounded store here grows
/// linearly with total requests served and the process eventually exhausts
/// memory while doing nothing wrong. The result cache being bounded does not
/// help: this is a separate allocation on the same path.
///
/// A ring is the right shape rather than a compromise. This capture exists to
/// answer "what did the last N requests look like", which is what an operator
/// debugging live traffic actually asks. Retaining everything answers a question
/// nobody asks and cannot be served from memory anyway.
pub const DEFAULT_TRACE_CAPACITY: usize = 1024;

/// Environment variable overriding the retained trace-event count.
pub const TRACE_CAPACITY_ENV: &str = "LLM_D_SC_TRACE_CAPACITY";

impl Telemetry {
    /// An empty telemetry recorder with the default bound.
    pub fn new() -> Self {
        let capacity = std::env::var(TRACE_CAPACITY_ENV)
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|n| *n > 0)
            .unwrap_or(DEFAULT_TRACE_CAPACITY);
        Self::with_capacity(capacity)
    }

    /// An empty recorder retaining at most `capacity` events.
    pub fn with_capacity(capacity: usize) -> Self {
        Telemetry {
            events: Arc::new(Mutex::new(std::collections::VecDeque::new())),
            capacity: capacity.max(1),
        }
    }

    /// Number of retained trace events. Never exceeds the configured capacity.
    pub fn len(&self) -> usize {
        self.events.lock().unwrap().len()
    }

    /// True when nothing has been recorded.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Record a request event, hashing the context and session so no raw text is
    /// retained in default output (AC-014).
    pub fn record_request(&self, event: RequestEvent) {
        // `request_id` is caller-supplied, so an unbounded id would let a client
        // control how much memory each retained event costs. Bounding the event
        // COUNT alone bounds growth; bounding the id too bounds the constant.
        let mut request_id = event.request_id;
        if request_id.len() > MAX_REQUEST_ID_BYTES {
            request_id.truncate(
                (0..=MAX_REQUEST_ID_BYTES)
                    .rev()
                    .find(|i| request_id.is_char_boundary(*i))
                    .unwrap_or(0),
            );
        }
        let mut events = self.events.lock().unwrap();
        while events.len() >= self.capacity {
            events.pop_front();
        }
        events.push_back(TraceEvent {
            request_id,
            context_hash: format!("ctx_{}", hash(&event.context)),
            session_hash: format!("sess_{}", hash(&event.session_id)),
        });
    }

    /// The default serialized logs/metrics output: request ids and hashes only,
    /// never raw prompt or session text.
    pub fn default_output(&self) -> String {
        let events = self.events.lock().unwrap();
        let mut out = String::new();
        for ev in events.iter() {
            out.push_str(&format!(
                "request_id={} context_hash={} session_hash={}\n",
                ev.request_id, ev.context_hash, ev.session_hash
            ));
        }
        out
    }

    /// A copy of the captured trace events.
    pub fn trace_capture(&self) -> Vec<TraceEvent> {
        self.events.lock().unwrap().iter().cloned().collect()
    }
}

/// Maximum retained bytes of a caller-supplied request id.
const MAX_REQUEST_ID_BYTES: usize = 256;

/// A short blake3 hex digest of `s`, so telemetry never carries raw text.
fn hash(s: &str) -> String {
    blake3::hash(s.as_bytes()).to_hex()[..16].to_string()
}

#[cfg(test)]
mod bounded_tests {
    use super::*;

    /// U-130 (AC-014): trace capture must not grow without bound.
    ///
    /// `record_request` runs on the production classify path for every request,
    /// so an unbounded store grows linearly with total requests served and the
    /// process eventually exhausts memory while behaving correctly. No
    /// functional test can see this: every classification still returns the
    /// right answer. Only asserting on retention can.
    #[test]
    fn u130_trace_capture_is_bounded_under_sustained_load() {
        const CAPACITY: usize = 1024;
        const REQUESTS: usize = 100_000;
        let telemetry = Telemetry::with_capacity(CAPACITY);

        for i in 0..REQUESTS {
            telemetry.record_request(RequestEvent {
                request_id: format!("req-{i}"),
                session_id: format!("sess-{}", i % 7),
                context: format!("prompt number {i}"),
            });
        }

        assert_eq!(
            telemetry.len(),
            CAPACITY,
            "after {REQUESTS} requests the capture holds {} events with a bound of {CAPACITY}",
            telemetry.len()
        );
        assert_eq!(telemetry.trace_capture().len(), CAPACITY);
    }

    /// U-131: the ring retains the MOST RECENT events, not the first ones.
    ///
    /// A bound that kept the oldest entries would be bounded and useless: an
    /// operator debugging live traffic needs the last N requests, not the first
    /// N the process ever saw.
    #[test]
    fn u131_capture_retains_the_most_recent_events() {
        let telemetry = Telemetry::with_capacity(3);
        for i in 0..10 {
            telemetry.record_request(RequestEvent {
                request_id: format!("req-{i}"),
                session_id: "s".into(),
                context: "c".into(),
            });
        }
        let ids: Vec<String> = telemetry
            .trace_capture()
            .into_iter()
            .map(|e| e.request_id)
            .collect();
        assert_eq!(ids, vec!["req-7", "req-8", "req-9"]);
    }

    /// U-132: a caller-supplied request id cannot dictate retained memory.
    ///
    /// Bounding the event COUNT bounds growth; without also bounding the id, a
    /// client could still choose how many bytes each retained event costs.
    #[test]
    fn u132_oversized_request_id_is_truncated() {
        let telemetry = Telemetry::with_capacity(4);
        telemetry.record_request(RequestEvent {
            request_id: "x".repeat(1_000_000),
            session_id: "s".into(),
            context: "c".into(),
        });
        let ev = telemetry.trace_capture().pop().expect("one event");
        assert!(
            ev.request_id.len() <= MAX_REQUEST_ID_BYTES,
            "retained request id is {} bytes",
            ev.request_id.len()
        );
    }

    /// U-133: multibyte ids are truncated on a character boundary, not mid-glyph.
    #[test]
    fn u133_truncation_respects_character_boundaries() {
        let telemetry = Telemetry::with_capacity(2);
        telemetry.record_request(RequestEvent {
            // 3 bytes per character, so a byte-wise cut would split one.
            request_id: "\u{65e5}".repeat(1000),
            session_id: "s".into(),
            context: "c".into(),
        });
        let ev = telemetry.trace_capture().pop().expect("one event");
        assert!(ev.request_id.len() <= MAX_REQUEST_ID_BYTES);
        assert!(
            ev.request_id.chars().all(|c| c == '\u{65e5}'),
            "truncation must not corrupt a multibyte character"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{RequestEvent, Telemetry};

    /// U-085 (AC-014): the default output surfaces the request id and a context
    /// hash, but never the raw prompt or raw session text.
    #[test]
    fn u085_raw_prompt_absent_from_default_logs_metrics() {
        let telemetry = Telemetry::new();
        telemetry.record_request(RequestEvent {
            request_id: "req-085".to_string(),
            session_id: "sess-top-secret".to_string(),
            context: "this RAW secret prompt must never appear in default telemetry".to_string(),
        });

        let out = telemetry.default_output();
        assert!(out.contains("req-085"), "request id must appear");
        assert!(
            out.contains("ctx_"),
            "a context hash (ctx_...) must appear in default telemetry"
        );
        assert!(
            !out.contains("RAW secret prompt"),
            "default telemetry must not contain the raw prompt text"
        );
        assert!(
            !out.contains("sess-top-secret"),
            "default telemetry must not contain raw session text"
        );
    }

    /// The recorder must never retain raw text even after capture.
    #[test]
    fn u085_recorder_never_retains_raw_text() {
        let telemetry = Telemetry::new();
        telemetry.record_request(RequestEvent {
            request_id: "req-085".to_string(),
            session_id: "sess-top-secret".to_string(),
            context: "this RAW secret prompt must never appear in default telemetry".to_string(),
        });
        for ev in telemetry.trace_capture() {
            assert!(
                !ev.context_hash.contains("RAW secret prompt"),
                "trace context must be a hash, never the raw prompt"
            );
            assert!(
                !ev.session_hash.contains("sess-top-secret"),
                "trace session must be a hash, never the raw session text"
            );
        }
    }
}
