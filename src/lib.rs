//! llm-d-sc: a long-lived Rust semantic-classification runtime.
//!
//! This crate currently exposes configuration parsing and validation.
//! Networking, the gRPC API, the ClassifierRuntime abstraction and the Candle
//! backend are IMPLEMENTED. See README for the remaining integration gaps.
//! Networking, the gRPC API, the ClassifierRuntime abstraction and the Candle
//! backend are IMPLEMENTED. See README for the remaining integration gaps.

pub mod bench;
pub mod cache;
pub mod classify;
pub mod config;
pub mod dummy_gateway;
pub mod embedding;
pub mod grpc;
pub mod handoff;
pub mod metrics;
pub mod queue;
pub mod ranker;
pub mod runtime;
pub mod telemetry;
pub mod taxonomy;
pub mod tokenizer;
