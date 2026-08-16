//! Dummy Praxis: consumes a classification response over persistent gRPC and
//! routes OUTSIDE llm-d-sc.
//!
//! AC-009 (I-005/I-006) proves the responsibility split: llm-d-sc returns ranked
//! semantic signals and NEVER a final route; routing/session authority stays with
//! Praxis (AC-010). This module is the dummy client that
//! 1. receives a synthetic [`DummyRequest`] with session metadata
//!    (request_id/session_id/context/requested-signals/deadline),
//! 2. propagates that metadata to llm-d-sc over the PERSISTENT channel,
//! 3. consumes the top ranked signal from the response,
//! 4. applies its own fixed test-only mapping
//!    (`NEVER_EGRESS_SIGNAL -> local-model`, otherwise -> `general-model`) and
//!    records the resulting route + classifier RTT.
//!
//! The route is chosen by the dummy (test-only policy), never by llm-d-sc. The
//! mapping is deliberately FIXED and deterministic so the tests are reproducible;
//! real routing policy is out of scope (spec non-goals).

use std::io;

use crate::grpc::classify::{ClassifyClient, ClassifyRequest};

/// The signal id that maps to the `local-model` route under the dummy's fixed
/// test-only policy (`NEVER_EGRESS -> local-model`). Any other consumed signal
/// maps to `general-model`.
const NEVER_EGRESS_SIGNAL: &str = "proto-a";

/// Route chosen when the consumed signal is the never-egress signal.
const ROUTE_LOCAL_MODEL: &str = "local-model";

/// Route chosen for every other consumed signal.
const ROUTE_GENERAL_MODEL: &str = "general-model";

/// A synthetic request the dummy Praxis receives.
///
/// The session metadata is propagated verbatim to llm-d-sc and kept intact for
/// the dummy's own (outside llm-d-sc) routing decision.
#[derive(Debug, Clone)]
pub struct DummyRequest {
    pub request_id: String,
    pub session_id: String,
    pub context: String,
    pub signals: Vec<String>,
    pub deadline: Option<std::time::SystemTime>,
}

/// The outcome of a dummy Praxis classify-and-route turn.
///
/// Carries the preserved session ids, the consumed ranked semantic signal, the
/// route the dummy chose itself (outside llm-d-sc), and the measured classifier
/// RTT.
#[derive(Debug, Clone)]
pub struct DummyOutcome {
    pub request_id: String,
    pub session_id: String,
    pub signal: String,
    pub route: String,
    pub rtt: std::time::Duration,
}

/// Dummy Praxis client over the persistent classify channel.
///
/// Connects once and reuses the single persistent channel (I-008: no reconnect
/// per call), consuming the ranked signal and routing outside llm-d-sc.
pub struct DummyPraxis {
    client: ClassifyClient,
}

impl DummyPraxis {
    /// Connect to the classify server at `addr` over a persistent channel.
    pub fn connect(addr: impl AsRef<str>) -> io::Result<DummyPraxis> {
        let client = ClassifyClient::connect(addr)?;
        Ok(DummyPraxis { client })
    }

    /// Propagate the request's session metadata to llm-d-sc, consume the ranked
    /// signal, and apply the dummy's fixed test-only mapping to route outside
    /// llm-d-sc. Returns the outcome with the measured classifier RTT.
    pub fn classify_and_route(&mut self, req: DummyRequest) -> Result<DummyOutcome, tonic::Status> {
        // Propagate the session metadata verbatim over the persistent channel.
        let classify_req = ClassifyRequest {
            request_id: req.request_id.clone(),
            session_id: req.session_id.clone(),
            context: req.context.clone(),
            signals: req.signals.clone(),
        };

        let start = std::time::Instant::now();
        let resp = self.client.classify(classify_req)?;
        let rtt = start.elapsed();

        // Consume the top ranked semantic signal (never a final route: AC-010).
        let signal = resp.signals.first().cloned().unwrap_or_default();

        // Routing authority stays Praxis: apply the dummy's fixed test-only
        // mapping, never a route dictated by llm-d-sc.
        let route = if signal == NEVER_EGRESS_SIGNAL {
            ROUTE_LOCAL_MODEL
        } else {
            ROUTE_GENERAL_MODEL
        };

        Ok(DummyOutcome {
            request_id: req.request_id,
            session_id: req.session_id,
            signal,
            route: route.to_string(),
            rtt,
        })
    }
}
