//! Service configuration: parsing and validation.
//!
//! The classifier registry is the authoritative list of resident
//! classifiers. Each entry names a runtime backend and a local model path.
//! Routing/stickiness/policy remain outside this crate.

use serde::Deserialize;
use std::collections::HashSet;

/// Runtime backends this crate can currently host.
pub const KNOWN_BACKENDS: &[&str] = &["candle"];

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub classifiers: Vec<ClassifierConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_listen")]
    pub listen: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ClassifierConfig {
    pub id: String,
    pub backend: String,
    pub model_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    MissingClassifiers,
    UnknownBackend(String),
    DuplicateClassifierId(String),
    InvalidModelPath(String),
    Parse(String),
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            listen: default_listen(),
        }
    }
}

fn default_listen() -> String {
    "0.0.0.0:50051".to_string()
}

impl Config {
    /// Parse a TOML config string and validate its invariants.
    pub fn parse(raw: &str) -> Result<Config, ConfigError> {
        let cfg: Config = toml::from_str(raw).map_err(|e| ConfigError::Parse(e.to_string()))?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Validate cross-field invariants.
    fn validate(&self) -> Result<(), ConfigError> {
        if self.classifiers.is_empty() {
            return Err(ConfigError::MissingClassifiers);
        }

        let mut seen: HashSet<&str> = HashSet::new();
        for c in &self.classifiers {
            if !KNOWN_BACKENDS.contains(&c.backend.as_str()) {
                return Err(ConfigError::UnknownBackend(c.backend.clone()));
            }
            if c.model_path.trim().is_empty() {
                return Err(ConfigError::InvalidModelPath(c.id.clone()));
            }
            if !seen.insert(c.id.as_str()) {
                return Err(ConfigError::DuplicateClassifierId(c.id.clone()));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_toml() -> &'static str {
        r#"
        [server]
        listen = "0.0.0.0:50051"

        [[classifiers]]
        id = "sensitivity"
        backend = "candle"
        model_path = "/models/sensitivity"
        "#
    }

    #[test]
    fn u001_minimal_valid_configuration_parses() {
        let cfg = Config::parse(minimal_toml()).expect("minimal valid config should parse");
        assert_eq!(cfg.server.listen, "0.0.0.0:50051");
        assert_eq!(cfg.classifiers.len(), 1);
        assert_eq!(cfg.classifiers[0].id, "sensitivity");
        assert_eq!(cfg.classifiers[0].backend, "candle");
        assert_eq!(cfg.classifiers[0].model_path, "/models/sensitivity");
    }

    #[test]
    fn u002_missing_classifier_config_rejected() {
        let toml = r#"
        [server]
        listen = "0.0.0.0:50051"
        "#;
        match Config::parse(toml) {
            Err(ConfigError::MissingClassifiers) => {}
            other => panic!("expected MissingClassifiers, got {other:?}"),
        }
    }

    #[test]
    fn u003_unknown_runtime_backend_rejected() {
        let toml = r#"
        [server]
        listen = "0.0.0.0:50051"

        [[classifiers]]
        id = "sensitivity"
        backend = "vllm"
        model_path = "/models/sensitivity"
        "#;
        match Config::parse(toml) {
            Err(ConfigError::UnknownBackend(b)) => assert_eq!(b, "vllm"),
            other => panic!("expected UnknownBackend, got {other:?}"),
        }
    }

    #[test]
    fn u004_duplicate_classifier_id_rejected() {
        let toml = r#"
        [[classifiers]]
        id = "sensitivity"
        backend = "candle"
        model_path = "/models/sensitivity-a"

        [[classifiers]]
        id = "sensitivity"
        backend = "candle"
        model_path = "/models/sensitivity-b"
        "#;
        match Config::parse(toml) {
            Err(ConfigError::DuplicateClassifierId(id)) => assert_eq!(id, "sensitivity"),
            other => panic!("expected DuplicateClassifierId, got {other:?}"),
        }
    }

    #[test]
    fn u005_invalid_model_path_rejected() {
        let toml = r#"
        [[classifiers]]
        id = "sensitivity"
        backend = "candle"
        model_path = ""
        "#;
        match Config::parse(toml) {
            Err(ConfigError::InvalidModelPath(id)) => assert_eq!(id, "sensitivity"),
            other => panic!("expected InvalidModelPath, got {other:?}"),
        }
    }
}
