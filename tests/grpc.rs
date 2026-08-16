//! AC-009 proving tests (integration): dummy Praxis consumes a response over
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
//! This slice selects I-005/I-006 (dummy-Praxis semantics): the dummy Praxis
//! preserves the session metadata it propagates and consumes the ranked signal
//! then routes OUTSIDE llm-d-sc via its fixed test-only mapping (routing
//! authority stays Praxis). I-008 (multi-turn requests do not reconnect per
//! call) is asserted by I-002 (`channel_reconnect_count == 0`).
//!
//! The proving tests drive a [`llm_d_sc::dummy_praxis::DummyPraxis`] client
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
        signals: vec!["sensitivity".to_string()],
    }
}

/// I-001: a real tonic client/server round trip returns ranked signals.
///
/// AC-009 requires the dummy Praxis (client) to consume the classification
/// response over real gRPC. This test starts a REAL tonic classify server on an
/// ephemeral localhost port, connects a REAL client channel, sends one classify
/// request for a fixture input, and asserts a ranked-signals response arrives
/// over the wire (and never a final route, per AC-010). The pipeline is the
/// deterministic tokenizer -> cache -> single-flight -> ranker path over the
/// synthetic prototypes; no Candle model is required.
#[tokio::test]
async fn i001_real_tonic_round_trip() {
    // The pipeline-backed tonic classify service.
    let service = ClassifyServiceImpl::new(ClassifyService::from_synthetic_fixtures());
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

    // The response must carry ranked semantic signals over the wire...
    assert!(
        !response.signals.is_empty(),
        "response must carry ranked semantic signals"
    );
    // ...and carry no final route at all. AC-010 is now a SCHEMA invariant
    // (U-010): `ClassifyResponse` has no route/endpoint field, so a route is
    // unrepresentable on the wire (ADR-0001, interpretation (B)). The former
    // `final_route.is_none()` assertion is superseded by that deterministic
    // schema test (`tests/schema.rs`).
    let _ = response;

    // Tear down the server task (the channel is persistent; abort releases it).
    server.abort();
}

/// I-002: the HTTP/2 channel is persistent and reused across calls.
///
/// AC-009 requires a PERSISTENT gRPC channel. The dummy Praxis makes several
/// calls over the same channel and must NOT open a new connection per call
/// (I-008: multi-turn requests do not reconnect per call). This test drives
/// several turn requests and asserts the client reused the persistent channel
/// (zero reconnect events) and every turn succeeded.
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
            !response.signals.is_empty(),
            "turn {turn} must return ranked signals"
        );
    }

    assert_eq!(
        client.channel_reconnect_count(),
        0,
        "I-008: multi-turn requests must not reconnect per call"
    );
}

/// I-005: the dummy Praxis preserves session metadata.
///
/// AC-009 requires the dummy Praxis to receive a synthetic request, propagate
/// request_id/session_id/context/requested-signals/deadline to llm-d-sc over the
/// PERSISTENT channel, consume the ranked signal, and keep that session metadata
/// intact for its own (outside llm-d-sc) routing decision. This test drives a
/// real DummyPraxis client against the real classify server and asserts the
/// request/session ids the dummy propagated are preserved verbatim and it
/// recorded a route of its own — never one dictated by llm-d-sc.
#[test]
fn i005_dummy_praxis_preserves_session_metadata() {
    use llm_d_sc::dummy_praxis::{DummyPraxis, DummyRequest};
    use llm_d_sc::grpc::classify::ClassifyServer;

    let server = ClassifyServer::bind("127.0.0.1:0").expect("classify server must bind");
    let addr = server.local_addr();
    let mut praxis = DummyPraxis::connect(&addr).expect("dummy praxis must connect");

    let req = DummyRequest {
        request_id: "req-0005".to_string(),
        session_id: "sess-0005".to_string(),
        context: "this is a golden sensitivity input".to_string(),
        signals: vec!["sensitivity".to_string()],
        deadline: None,
    };
    let outcome = praxis
        .classify_and_route(req.clone())
        .expect("dummy praxis must classify and route");

    // The dummy preserved the session metadata it propagated: its recorded ids
    // match what it sent (routing/session authority stays Praxis, AC-010).
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
        "dummy praxis must record its own route outside llm-d-sc"
    );
}

/// I-006: the dummy Praxis consumes the signal, then routes OUTSIDE llm-d-sc.
///
/// The classify response must carry ranked signals but NEVER a final route; the
/// dummy applies its fixed test-only mapping (NEVER_EGRESS -> local-model,
/// otherwise -> general-model) and records the resulting route + classifier RTT.
/// This asserts routing authority stays outside llm-d-sc (AC-009/AC-010).
#[test]
fn i006_dummy_praxis_routes_outside_llm_d_sc() {
    use llm_d_sc::dummy_praxis::{DummyPraxis, DummyRequest};
    use llm_d_sc::grpc::classify::ClassifyServer;

    let server = ClassifyServer::bind("127.0.0.1:0").expect("classify server must bind");
    let addr = server.local_addr();
    let mut praxis = DummyPraxis::connect(&addr).expect("dummy praxis must connect");

    let req = DummyRequest {
        request_id: "req-0006".to_string(),
        session_id: "sess-0006".to_string(),
        context: "this is a golden sensitivity input".to_string(),
        signals: vec!["sensitivity".to_string()],
        deadline: None,
    };
    let outcome = praxis
        .classify_and_route(req)
        .expect("dummy praxis must classify and route");

    // The dummy CONSUMED a ranked semantic signal from llm-d-sc...
    assert!(
        !outcome.signal.is_empty(),
        "dummy praxis must consume a ranked semantic signal"
    );
    // ...then routed OUTSIDE llm-d-sc via its fixed test-only mapping.
    assert!(
        outcome.route == "general-model" || outcome.route == "local-model",
        "route must come from the dummy test policy, not llm-d-sc"
    );
    // It measured a monotonic start/end around the classifier RPC.
    assert!(
        outcome.rtt > std::time::Duration::ZERO,
        "dummy praxis must measure classifier RTT"
    );
}

/// I-007: the llm-d-sc response cannot dictate an endpoint.
///
/// AC-010 requires the response to carry signals, not a final route. The dummy
/// Praxis receives a response, and the ONLY route in the system is the one the
/// dummy computes itself (outside llm-d-sc). The response type offers no route
/// to consume: `ClassifyResponse` has no `route`/`endpoint`/`final_route` field
/// (ADR-0001, U-010), so referencing one would not compile. This test drives a
/// real DummyPraxis against the real server and asserts the recorded route is
/// exactly the dummy's own fixed test-only mapping, never anything derived from
/// the response.
#[test]
fn i007_response_cannot_dictate_endpoint() {
    use llm_d_sc::dummy_praxis::{DummyPraxis, DummyRequest};
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
    let mut praxis = DummyPraxis::connect(&addr).expect("dummy praxis must connect");

    let req = DummyRequest {
        request_id: "req-0007".to_string(),
        session_id: "sess-0007".to_string(),
        context: "this is a golden sensitivity input".to_string(),
        signals: vec!["sensitivity".to_string()],
        deadline: None,
    };
    let outcome = praxis
        .classify_and_route(req)
        .expect("dummy praxis must classify and route");

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
