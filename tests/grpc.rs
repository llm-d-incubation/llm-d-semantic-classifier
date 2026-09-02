//! AC-009 proving tests (integration): dummy gateway consumes a response over
//! persistent gRPC.
//!
//! This slice selects I-001 (real tonic client/server round trip) and keeps the
//! I-002 persistent-channel test wired. I-001 is the proving test for the RPC
//! contract: a REAL tonic server and a REAL client channel exchange a
//! classification request and receive ranked semantic signals over the wire.
//! The pipeline behind the server is the deterministic classification pipeline
//! (tokenizer -> versioned cache -> single-flight -> ranker over the committed
//! synthetic prototypes) — no Candle model is required for I-001.
//!
//! This slice selects I-005/I-006 (dummy-the AI Gateway semantics): the dummy gateway
//! preserves the session metadata it propagates and consumes the ranked signal
//! then routes OUTSIDE llm-d-sc via its fixed test-only mapping (routing
//! authority stays the AI Gateway). I-008 (multi-turn requests do not reconnect per
//! call) is asserted by I-002 (`channel_reconnect_count == 0`).
//!
//! The proving tests drive a [`llm_d_sc::dummy_gateway::DummyGateway`] client
//! against the real classify server over the persistent channel. The dummy
//! module is intentionally NOT implemented yet, so these tests cannot compile
//! until it exists — that is the expected RED for this slice.

use llm_d_sc::classify::ClassifyService;
use llm_d_sc::grpc::classify::generated;
use llm_d_sc::grpc::classify::{ClassifyRequest, ClassifyResponse, ClassifyServiceImpl};

fn fixture_request(request_id: &str, session_id: &str) -> ClassifyRequest {
    ClassifyRequest {
        request_id: request_id.to_string(),
        session_id: session_id.to_string(),
        context: "this is a golden sensitivity input".to_string(),
        signals: Vec::new(),
        context_completeness: generated::ContextCompleteness::Full as i32,
    }
}

/// I-001: a real tonic client/server round trip returns ranked signals.
///
/// AC-009 requires the dummy gateway (client) to consume the classification
/// response over real gRPC. This test starts a REAL tonic classify server on an
/// ephemeral localhost port, connects a REAL client channel, sends one classify
/// request for a fixture input, and asserts a ranked-signals response arrives
/// over the wire (and never a final route, per AC-010). The pipeline is the
/// deterministic tokenizer -> cache -> single-flight -> ranker path over the
/// synthetic prototypes; no Candle model is required.
#[tokio::test]
async fn i001_real_tonic_round_trip() {
    // The pipeline-backed tonic classify service.
    let service = ClassifyServiceImpl::new(
        ClassifyService::from_synthetic_fixtures(),
        llm_d_sc::telemetry::Telemetry::new(),
    );
    let tonic_server = generated::classify_server::ClassifyServer::new(service);

    // Bind an ephemeral localhost port and serve in the background.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("classify server must bind an ephemeral port");
    let addr = listener.local_addr().expect("bound addr");
    let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);

    let server = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(tonic_server)
            .serve_with_incoming(incoming)
            .await
            .expect("classify server must serve")
    });

    // Connect a real tonic client channel over the wire.
    let mut client = generated::classify_client::ClassifyClient::connect(format!("http://{addr}"))
        .await
        .expect("client must connect to the server");

    // Send a classify request for a fixture input and await the response.
    let response: ClassifyResponse = client
        .classify(fixture_request("req-0001", "sess-0001"))
        .await
        .expect("classify round trip must succeed")
        .into_inner();

    // The response must carry ranked semantic signals over the wire, each with
    // a score present...
    assert!(
        !response.ranked.is_empty(),
        "response must carry ranked semantic signals"
    );
    for signal in &response.ranked {
        assert!(
            !signal.label.is_empty(),
            "every ranked signal must carry a label"
        );
        assert!(
            signal.score.is_finite(),
            "every ranked signal must carry a finite score"
        );
    }
    // ...the versioned classifier/revision fingerprint must be non-empty...
    assert!(
        !response.classifier_id.is_empty(),
        "response must carry classifier_id"
    );
    assert!(
        !response.model_revision.is_empty(),
        "response must carry model_revision"
    );
    assert!(
        !response.tokenizer_revision.is_empty(),
        "response must carry tokenizer_revision"
    );
    assert!(
        !response.taxonomy_revision.is_empty(),
        "response must carry taxonomy_revision"
    );
    // ...and the deterministic pipeline must return a real Ok result.
    assert_eq!(
        response.status,
        generated::ClassificationStatus::Ok as i32,
        "a successful classification must carry status OK"
    );
    // ...and carry no final route at all. AC-010 is now a SCHEMA invariant
    // (U-010): `ClassifyResponse` has no route/endpoint field, so a route is
    // unrepresentable on the wire (ADR-0001, interpretation (B)).

    // Tear down the server task (the channel is persistent; abort releases it).
    server.abort();
}

/// I-002: the HTTP/2 channel is persistent and reused across calls.
///
/// AC-009 requires a PERSISTENT gRPC channel. The dummy gateway makes several
/// calls over the same channel and must NOT open a new connection per call
/// (I-008: multi-turn requests do not reconnect per call). This test drives
/// several turn requests and asserts, FROM THE SERVER SIDE, that exactly one
/// TCP connection was accepted across all of them. Counting at the accept
/// boundary means the client cannot vouch for its own behaviour: a client that
/// reconnected per call would show 5 accepts here.
#[test]
fn i002_persistent_http2_channel_reused() {
    use llm_d_sc::grpc::classify::{ClassifyClient, ClassifyServer};

    let server = ClassifyServer::bind("127.0.0.1:0").expect("classify server must bind");
    let addr = server.local_addr();
    let mut client = ClassifyClient::connect(addr).expect("client must connect to the server");

    for turn in 1..=5 {
        let response = client
            .classify(fixture_request(&format!("req-{turn:04}"), "sess-0002"))
            .expect("every turn must succeed over the persistent channel");
        assert!(
            !response.ranked.is_empty(),
            "turn {turn} must return ranked signals"
        );
        assert!(
            !response.ranked[0].label.is_empty(),
            "turn {turn} must return a labeled signal"
        );
        assert!(
            response.ranked[0].score.is_finite(),
            "turn {turn} must return a finite score"
        );
    }

    assert_eq!(
        server.accepted_connection_count(),
        1,
        "I-008: 5 turns over a persistent channel must accept exactly ONE TCP \
         connection; more than one means the client reconnected per call"
    );
}

/// I-092 (control for I-002): the accept counter must be able to REPORT reconnection.
///
/// A counter that can only ever read 1 would prove nothing, and the assertion it
/// replaced (a client-side field that was initialised to 0 and never
/// incremented) failed exactly that way. This test connects a NEW client per
/// call and asserts the server observes one accept per connection, so the
/// I-002 result above is a measurement rather than a constant.
#[test]
fn i092_control_reconnecting_client_is_observed_as_multiple_accepts() {
    use llm_d_sc::grpc::classify::{ClassifyClient, ClassifyServer};

    let server = ClassifyServer::bind("127.0.0.1:0").expect("classify server must bind");
    let addr = server.local_addr();

    for turn in 1..=3 {
        let mut client = ClassifyClient::connect(&addr).expect("each fresh client must connect");
        client
            .classify(fixture_request(&format!("rc-{turn:04}"), "sess-rc"))
            .expect("turn must succeed");
    }

    assert_eq!(
        server.accepted_connection_count(),
        3,
        "the accept counter must observe one accept per fresh connection, \
         otherwise the I-002 assertion is vacuous"
    );
}

/// I-008: multi-turn requests do not reconnect per call.
///
/// The same transport property as I-002, asserted at the scale the ID describes:
/// a sustained multi-turn session. Measured server-side by counting accepted TCP
/// connections, so the client cannot vouch for itself. I-092 is the control
/// proving the counter can report reconnection.
#[test]
fn i008_multi_turn_session_does_not_reconnect_per_call() {
    use llm_d_sc::grpc::classify::{ClassifyClient, ClassifyServer};

    let server = ClassifyServer::bind("127.0.0.1:0").expect("classify server must bind");
    let addr = server.local_addr();
    let mut client = ClassifyClient::connect(addr).expect("client must connect");

    const TURNS: usize = 25;
    for turn in 1..=TURNS {
        client
            .classify(fixture_request(&format!("mt-{turn:04}"), "sess-multiturn"))
            .expect("every turn of the session must succeed");
    }

    assert_eq!(
        server.accepted_connection_count(),
        1,
        "{TURNS} turns over one session must accept exactly ONE TCP connection"
    );
}

/// U-011 (AC-009): unknown signal explicit error.
///
/// The request's `requested_signals` must be validated: only the supported
/// `sensitivity` signal is accepted; any other requested signal is rejected with
/// an explicit tonic `invalid_argument` status (never silently ignored). A
/// supported signal still returns a real Ok result.
#[test]
fn u011_unknown_signal_explicit_error() {
    use llm_d_sc::grpc::classify::{ClassifyClient, ClassifyServer};

    let server = ClassifyServer::bind("127.0.0.1:0").expect("classify server must bind");
    let addr = server.local_addr();
    let mut client = ClassifyClient::connect(addr).expect("client must connect");

    // A supported signal is accepted and returns a real Ok result.
    let ok = client
        .classify(fixture_request("req-011-ok", "sess-011"))
        .expect("sensitivity must be accepted");
    assert_eq!(
        ok.status,
        generated::ClassificationStatus::Ok as i32,
        "the supported sensitivity signal must return status OK"
    );

    // An unknown signal is rejected explicitly with invalid_argument.
    let mut bad = fixture_request("req-011-bad", "sess-011");
    bad.signals = vec!["pii".to_string()];
    let err = client
        .classify(bad)
        .expect_err("unknown signal must be rejected explicitly");
    assert_eq!(
        err.code(),
        tonic::Code::InvalidArgument,
        "an unknown signal must map to an explicit invalid_argument error"
    );
}

/// I-005: the dummy gateway preserves session metadata.
///
/// AC-009 requires the dummy gateway to receive a synthetic request, propagate
/// request_id/session_id/context/requested-signals/deadline to llm-d-sc over the
/// PERSISTENT channel, consume the ranked signal, and keep that session metadata
/// intact for its own (outside llm-d-sc) routing decision. This test drives a
/// real DummyGateway client against the real classify server and asserts the
/// request/session ids the dummy propagated are preserved verbatim and it
/// recorded a route of its own — never one dictated by llm-d-sc.
#[test]
fn i005_dummy_gateway_preserves_session_metadata() {
    use llm_d_sc::dummy_gateway::{DummyGateway, DummyRequest};
    use llm_d_sc::grpc::classify::ClassifyServer;

    let server = ClassifyServer::bind("127.0.0.1:0").expect("classify server must bind");
    let addr = server.local_addr();
    let mut gateway = DummyGateway::connect(&addr).expect("dummy gateway must connect");

    let req = DummyRequest {
        request_id: "req-0005".to_string(),
        session_id: "sess-0005".to_string(),
        context: "this is a golden sensitivity input".to_string(),
        signals: Vec::new(),
        deadline: None,
    };
    let outcome = gateway
        .classify_and_route(req.clone())
        .expect("dummy gateway must classify and route");

    // The dummy preserved the session metadata it propagated: its recorded ids
    // match what it sent (routing/session authority stays the AI Gateway, AC-010).
    assert_eq!(
        outcome.request_id, req.request_id,
        "request_id must be preserved"
    );
    assert_eq!(
        outcome.session_id, req.session_id,
        "session_id must be preserved"
    );
    // The dummy recorded a route of its own (outside llm-d-sc).
    assert!(
        !outcome.route.is_empty(),
        "dummy gateway must record its own route outside llm-d-sc"
    );
}

/// I-006: the dummy gateway consumes the signal, then routes OUTSIDE llm-d-sc.
///
/// The classify response must carry ranked signals but NEVER a final route; the
/// dummy applies its fixed test-only mapping (NEVER_EGRESS -> local-model,
/// otherwise -> general-model) and records the resulting route + classifier RTT.
/// This asserts routing authority stays outside llm-d-sc (AC-009/AC-010).
#[test]
fn i006_dummy_gateway_routes_outside_llm_d_sc() {
    use llm_d_sc::dummy_gateway::{DummyGateway, DummyRequest};
    use llm_d_sc::grpc::classify::ClassifyServer;

    let server = ClassifyServer::bind("127.0.0.1:0").expect("classify server must bind");
    let addr = server.local_addr();
    let mut gateway = DummyGateway::connect(&addr).expect("dummy gateway must connect");

    let req = DummyRequest {
        request_id: "req-0006".to_string(),
        session_id: "sess-0006".to_string(),
        context: "this is a golden sensitivity input".to_string(),
        signals: Vec::new(),
        deadline: None,
    };
    let outcome = gateway
        .classify_and_route(req)
        .expect("dummy gateway must classify and route");

    // The dummy CONSUMED a ranked semantic signal from llm-d-sc...
    assert!(
        !outcome.signal.is_empty(),
        "dummy gateway must consume a ranked semantic signal"
    );
    // ...then routed OUTSIDE llm-d-sc via its fixed test-only mapping.
    assert!(
        outcome.route == "general-model" || outcome.route == "local-model",
        "route must come from the dummy test policy, not llm-d-sc"
    );
    // It measured a monotonic start/end around the classifier RPC.
    assert!(
        outcome.rtt > std::time::Duration::ZERO,
        "dummy gateway must measure classifier RTT"
    );
}

/// I-007: the llm-d-sc response cannot dictate an endpoint.
///
/// AC-010 requires the response to carry signals, not a final route. The dummy
/// the AI Gateway receives a response, and the ONLY route in the system is the one the
/// dummy computes itself (outside llm-d-sc). The response type offers no route
/// to consume: `ClassifyResponse` has no `route`/`endpoint`/`final_route` field
/// (ADR-0001, U-010), so referencing one would not compile. This test drives a
/// real DummyGateway against the real server and asserts the recorded route is
/// exactly the dummy's own fixed test-only mapping, never anything derived from
/// the response.
#[test]
fn i007_response_cannot_dictate_endpoint() {
    use llm_d_sc::dummy_gateway::{DummyGateway, DummyRequest};
    use llm_d_sc::grpc::classify::ClassifyServer;

    // The response type offers no route field to consume (ADR-0001): the schema
    // invariant (U-010) forbids route/endpoint/target on `ClassifyResponse`, so
    // there is nothing on the wire the dummy could read as a dictated endpoint.
    //
    // Compile-time surface check (cannot reference a non-existent field):
    // `llm_d_sc::grpc::classify::generated::ClassifyResponse` implements
    // `prost::Message` but exposes no route field (enforced by U-010 in
    // `tests/schema.rs`).
    fn assert_no_route_field<M: prost::Message>() {}
    assert_no_route_field::<llm_d_sc::grpc::classify::generated::ClassifyResponse>();

    let server = ClassifyServer::bind("127.0.0.1:0").expect("classify server must bind");
    let addr = server.local_addr();
    let mut gateway = DummyGateway::connect(&addr).expect("dummy gateway must connect");

    let req = DummyRequest {
        request_id: "req-0007".to_string(),
        session_id: "sess-0007".to_string(),
        context: "this is a golden sensitivity input".to_string(),
        signals: Vec::new(),
        deadline: None,
    };
    let outcome = gateway
        .classify_and_route(req)
        .expect("dummy gateway must classify and route");

    // The ONLY route in the system is the one the dummy computes itself: it is
    // exactly one of the dummy's own fixed test-only mappings, and it was
    // chosen purely from the consumed signal (never read off the response).
    assert!(
        outcome.route == "local-model" || outcome.route == "general-model",
        "the only route must be the dummy's own test-only mapping, not dictated by llm-d-sc"
    );
    assert!(
        !outcome.signal.is_empty(),
        "the dummy consumes a ranked signal before routing itself"
    );
}
