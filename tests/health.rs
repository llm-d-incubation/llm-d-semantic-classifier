//! Standard health-checking protocol tests.
//!
//! These close the tracked 0.3 gap "no health-checking endpoint": an
//! orchestrator (Kubernetes `grpc:` readiness probe, Envoy gRPC health
//! checker, grpcurl) can now observe the service's readiness over the STANDARD
//! `grpc.health.v1.Health` protocol, on the same port as classify.
//!
//! The proving tests bind the REAL [`ClassifyServer`] (deterministic
//! weight-free pipeline, but the same `serve` funnel the production binary
//! uses) and drive it with a real tonic health client over the wire.

use llm_d_sc::grpc::classify::{ClassifyServer, SERVICE_NAME};
use llm_d_sc::grpc::health::{HealthClient, ServingStatus};

/// The whole server and the classify service both report SERVING.
///
/// The protocol's empty service name is the whole server; the named service is
/// what a caller of the classify API would probe. Both must be SERVING on a
/// bound server, because a [`ClassifyServer`] only exists after the model is
/// loaded and warmed — a ModelCar that fails load or warmup never reaches the
/// bind, so readiness is never claimed for a directory that merely exists
/// (AC-002/AC-003).
#[test]
fn health_check_reports_serving_for_the_whole_server_and_the_classify_service() {
    let server = ClassifyServer::bind("127.0.0.1:0").expect("classify server must bind");
    let mut client =
        HealthClient::connect(server.local_addr()).expect("health client must connect");

    for service in ["", SERVICE_NAME] {
        let status = client.check(service).expect("Check must succeed");
        assert_eq!(
            status,
            ServingStatus::Serving,
            "a bound server must report SERVING for '{service}'"
        );
    }
}

/// An unknown service name is an explicit NOT_FOUND, never a guessed status.
///
/// The protocol requires Check to fail with NOT_FOUND for a service the server
/// does not know; a health client must never mistake "unknown service" for
/// "healthy" or "unhealthy".
#[test]
fn health_check_unknown_service_is_an_explicit_not_found() {
    let server = ClassifyServer::bind("127.0.0.1:0").expect("classify server must bind");
    let mut client =
        HealthClient::connect(server.local_addr()).expect("health client must connect");

    let err = client
        .check("no.such.Service")
        .expect_err("an unknown service name must be rejected");
    assert_eq!(
        err.code(),
        tonic::Code::NotFound,
        "the protocol requires NOT_FOUND for a service the server does not know"
    );
}

/// Watch streams the current status first, and never errors for an unknown
/// service (it streams SERVICE_UNKNOWN instead; NOT_FOUND is a Check-only
/// error).
#[test]
fn health_watch_streams_the_current_status_first() {
    let server = ClassifyServer::bind("127.0.0.1:0").expect("classify server must bind");
    let mut client =
        HealthClient::connect(server.local_addr()).expect("health client must connect");

    let status = client.watch("").expect("Watch must stream a status");
    assert_eq!(
        status,
        ServingStatus::Serving,
        "Watch must stream the current whole-server status first"
    );

    let status = client
        .watch("no.such.Service")
        .expect("Watch must stream a status, never fail, for an unknown service");
    assert_eq!(
        status,
        ServingStatus::ServiceUnknown,
        "Watch reports SERVICE_UNKNOWN for a service the server does not know"
    );
}
