//! U-010 (AC-010): deterministic SCHEMA test.
//!
//! The classify response schema must contain NO final route/endpoint/target
//! field (ADR-0001 interpretation (B): the field is removed entirely, not merely
//! "never set"). Routing/session authority stays with Praxis, and the wire
//! contract should make the alternative unrepresentable.
//!
//! This is a plain `#[test]` (no network, no async) that reads the committed
//! `proto/classify.proto` schema and asserts the `ClassifyResponse` message
//! contains only `request_id` and `signals` — none of the forbidden route
//! field names. It is deterministic and RED while any route field exists.

use llm_d_sc::grpc::classify::generated::ClassifyResponse;

/// The forbidden route-like field names that must never appear in the response.
const FORBIDDEN: &[&str] = &["final_route", "route", "endpoint", "target"];

fn proto_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("proto")
        .join("classify.proto")
}

/// Extract the `ClassifyResponse` message block from the proto source.
fn classify_response_block(source: &str) -> &str {
    let start = source
        .find("message ClassifyResponse {")
        .unwrap_or_else(|| panic!("proto must declare `message ClassifyResponse`"));
    let open = &source[start..];
    // The message block ends at the first `}` after the opening brace.
    let brace = open.find('}').expect("ClassifyResponse message must close");
    &source[start..start + brace]
}

/// U-010: the `ClassifyResponse` schema carries request_id, classifier_id, the
/// revision fingerprint fields, status, and ranked signals — and no
/// route/endpoint/target field anywhere in the message (ADR-0001).
#[test]
fn u010_response_schema_has_no_route_field() {
    let source = std::fs::read_to_string(proto_root())
        .expect("committed proto/classify.proto must be readable");
    let block = classify_response_block(&source);

    // The response must carry request_id and the ranked signals.
    assert!(
        block.contains("request_id"),
        "ClassifyResponse must retain request_id"
    );
    assert!(
        block.contains("ranked"),
        "ClassifyResponse must retain ranked signals"
    );
    // The richer contract: classifier id, revision fingerprint, and status.
    assert!(
        block.contains("classifier_id"),
        "ClassifyResponse must carry classifier_id"
    );
    assert!(
        block.contains("model_revision"),
        "ClassifyResponse must carry model_revision"
    );
    assert!(
        block.contains("tokenizer_revision"),
        "ClassifyResponse must carry tokenizer_revision"
    );
    assert!(
        block.contains("taxonomy_revision"),
        "ClassifyResponse must carry taxonomy_revision"
    );
    assert!(
        block.contains("status"),
        "ClassifyResponse must carry ClassificationStatus"
    );

    // Parse the actual field declarations (`<type> <name> = <num>;`) and
    // assert none is a forbidden route/endpoint name.
    let field_names = block
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if let Some(stripped) = line.strip_suffix(';') {
                // `<type> <name> = <num>;` -> the field name is the second token.
                let mut tokens = stripped.split_whitespace();
                let _type = tokens.next()?;
                let name = tokens.next()?;
                Some(name.to_string())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    for name in FORBIDDEN {
        assert!(
            !field_names.iter().any(|f| f == name),
            "U-010: ClassifyResponse schema must not contain a `{name}` field (ADR-0001)"
        );
    }
}

/// U-010 surface check: the generated response type is still the wire message,
/// but it exposes no route field to consume (ADR-0001). We assert the generated
/// type exists and is a prost `Message`, and that the schema-level invariant
/// above already forbids any route field.
#[test]
fn u010_generated_response_type_exists() {
    // Compile-time surface: the generated message must implement prost Message.
    fn assert_message<M: prost::Message>() {}
    assert_message::<ClassifyResponse>();
}
