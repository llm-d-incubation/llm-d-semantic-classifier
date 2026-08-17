//! llm-d-sc: a long-lived Rust semantic-classification runtime.
//!
//! This crate currently exposes configuration parsing and validation.
//! Networking, the gRPC API, the `ClassifierRuntime` abstraction, and the
//! Candle backend arrive in later acceptance criteria.

pub mod bench;
pub mod cache;
pub mod classify;
pub mod config;
pub mod dummy_praxis;
pub mod embedding;
pub mod grpc;
pub mod metrics;
pub mod queue;
pub mod ranker;
pub mod runtime;
pub mod tokenizer;
