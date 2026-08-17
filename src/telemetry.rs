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
#[derive(Debug, Clone, Default)]
pub struct Telemetry {
    events: Arc<Mutex<Vec<TraceEvent>>>,
}

impl Telemetry {
    /// An empty telemetry recorder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a request event, hashing the context and session so no raw text is
    /// retained in default output (AC-014).
    pub fn record_request(&self, event: RequestEvent) {
        self.events.lock().unwrap().push(TraceEvent {
            request_id: event.request_id,
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
        self.events.lock().unwrap().clone()
    }
}

/// A short blake3 hex digest of `s`, so telemetry never carries raw text.
fn hash(s: &str) -> String {
    blake3::hash(s.as_bytes()).to_hex()[..16].to_string()
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
