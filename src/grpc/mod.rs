//! gRPC surface (AC-009).
//!
//! Exposes a blocking (synchronous) [`classify`](self::classify) surface used by
//! the dummy gateway client over a persistent HTTP/2 channel, plus the standard
//! [`health`](self::health) checking protocol (`grpc.health.v1`) that
//! orchestrators probe on the same port. Network I/O is owned by a private
//! Tokio runtime; the blocking wrappers hide it from callers so the crate's
//! public seam stays simple while the connection stays persistent across turns.

pub mod classify;
pub mod health;
