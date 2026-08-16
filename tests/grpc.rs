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
//! I-005/I-006/I-008 (dummy-Praxis semantics) and S-001/S-002 (OpenShift system)
//! are deferred to later slices/phases within AC-009.

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
    // ...and never dictate a final route (AC-010).
    assert!(
        response.final_route.is_none(),
        "AC-010: llm-d-sc must never dictate a final route"
    );

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
