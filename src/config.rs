//! Service configuration: parsing and validation.
//!
//! The classifier registry is the authoritative list of resident
//! classifiers. Each entry names a runtime backend and a local model path.
//! Routing/stickiness/policy remain outside this crate.

use serde::Deserialize;
use std::collections::HashSet;

/// Runtime backends this crate can currently host.
pub const KNOWN_BACKENDS: &[&str] = &["candle"];

/// Cache strategies this crate can host.
pub const KNOWN_CACHE_STRATEGIES: &[&str] = &["exact", "redis-semantic"];

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

// Note: `Eq` is intentionally not derived here (unlike other simple error
// enums in this crate) because `InvalidThreshold` carries an `f32`, which
// has no `Eq` impl (NaN != NaN). `PartialEq` is sufficient for the
// `assert_eq!`/`match` usages in this crate's tests.
#[derive(Debug, Clone, PartialEq)]
pub enum ConfigError {
    MissingClassifiers,
    UnknownBackend(String),
    DuplicateClassifierId(String),
    InvalidModelPath(String),
    Parse(String),
    UnknownCacheStrategy(String),
    MissingRedisUrl,
    InvalidThreshold(f32),
}

/// L2 semantic-cache configuration, resolved from the environment.
///
/// Off by default: the default strategy is `"exact"`, which corresponds to
/// the existing L1-only, single-flight exact-match cache. Selecting
/// `"redis-semantic"` requires `LLM_D_SC_REDIS_URL`; this type only
/// validates that surface, it does not construct a Redis client.
#[derive(Debug, Clone, PartialEq)]
pub struct CacheConfig {
    pub strategy: String,
    pub redis_url: Option<String>,
    pub threshold: f32,
    pub ttl_secs: u64,
    pub timeout_ms: u64,
}

impl CacheConfig {
    /// Resolve from process environment variables.
    pub fn from_env() -> Result<CacheConfig, ConfigError> {
        Self::from_env_with(|k| std::env::var(k).ok())
    }

    /// Resolve from an arbitrary getter (injected for tests).
    pub fn from_env_with(get: impl Fn(&str) -> Option<String>) -> Result<CacheConfig, ConfigError> {
        let strategy = get("LLM_D_SC_CACHE").unwrap_or_else(|| "exact".to_string());
        if !KNOWN_CACHE_STRATEGIES.contains(&strategy.as_str()) {
            return Err(ConfigError::UnknownCacheStrategy(strategy));
        }
        let redis_url = get("LLM_D_SC_REDIS_URL").filter(|s| !s.trim().is_empty());
        if strategy == "redis-semantic" && redis_url.is_none() {
            return Err(ConfigError::MissingRedisUrl);
        }
        let threshold = get("LLM_D_SC_CACHE_THRESHOLD")
            .map(|s| {
                s.parse::<f32>()
                    .map_err(|_| ConfigError::InvalidThreshold(f32::NAN))
            })
            .transpose()?
            .unwrap_or(0.90);
        if !(0.0..=1.0).contains(&threshold) {
            return Err(ConfigError::InvalidThreshold(threshold));
        }
        let ttl_secs = get("LLM_D_SC_CACHE_TTL")
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(86_400);
        let timeout_ms = get("LLM_D_SC_CACHE_TIMEOUT_MS")
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(50);
        Ok(CacheConfig {
            strategy,
            redis_url,
            threshold,
            ttl_secs,
            timeout_ms,
        })
    }
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

    #[test]
    fn cache_config_defaults_to_exact() {
        let cfg = CacheConfig::from_env_with(|_| None).expect("defaults");
        assert_eq!(cfg.strategy, "exact");
        assert!(cfg.redis_url.is_none());
        assert!((cfg.threshold - 0.90).abs() < 1e-6);
        assert_eq!(cfg.ttl_secs, 86_400);
        assert_eq!(cfg.timeout_ms, 50);
    }

    #[test]
    fn redis_semantic_requires_url() {
        let get = |k: &str| match k {
            "LLM_D_SC_CACHE" => Some("redis-semantic".to_string()),
            _ => None,
        };
        match CacheConfig::from_env_with(get) {
            Err(ConfigError::MissingRedisUrl) => {}
            other => panic!("expected MissingRedisUrl, got {other:?}"),
        }
    }

    #[test]
    fn redis_semantic_with_url_parses() {
        let get = |k: &str| match k {
            "LLM_D_SC_CACHE" => Some("redis-semantic".to_string()),
            "LLM_D_SC_REDIS_URL" => Some("redis://localhost:6379".to_string()),
            _ => None,
        };
        let cfg = CacheConfig::from_env_with(get).expect("valid redis-semantic config");
        assert_eq!(cfg.strategy, "redis-semantic");
        assert_eq!(cfg.redis_url.as_deref(), Some("redis://localhost:6379"));
    }

    #[test]
    fn unknown_cache_strategy_rejected() {
        let get = |k: &str| match k {
            "LLM_D_SC_CACHE" => Some("memcached".to_string()),
            _ => None,
        };
        match CacheConfig::from_env_with(get) {
            Err(ConfigError::UnknownCacheStrategy(s)) => assert_eq!(s, "memcached"),
            other => panic!("expected UnknownCacheStrategy, got {other:?}"),
        }
    }

    #[test]
    fn threshold_out_of_range_rejected() {
        let get = |k: &str| match k {
            "LLM_D_SC_CACHE" => Some("exact".to_string()),
            "LLM_D_SC_CACHE_THRESHOLD" => Some("1.5".to_string()),
            _ => None,
        };
        match CacheConfig::from_env_with(get) {
            Err(ConfigError::InvalidThreshold(v)) => assert!((v - 1.5).abs() < 1e-6),
            other => panic!("expected InvalidThreshold, got {other:?}"),
        }
    }

    #[test]
    fn threshold_ttl_timeout_overrides_parse() {
        let get = |k: &str| match k {
            "LLM_D_SC_CACHE" => Some("exact".to_string()),
            "LLM_D_SC_CACHE_THRESHOLD" => Some("0.75".to_string()),
            "LLM_D_SC_CACHE_TTL" => Some("120".to_string()),
            "LLM_D_SC_CACHE_TIMEOUT_MS" => Some("25".to_string()),
            _ => None,
        };
        let cfg = CacheConfig::from_env_with(get).expect("valid overrides");
        assert!((cfg.threshold - 0.75).abs() < 1e-6);
        assert_eq!(cfg.ttl_secs, 120);
        assert_eq!(cfg.timeout_ms, 25);
    }

    #[test]
    fn non_numeric_threshold_rejected() {
        let get = |k: &str| match k {
            "LLM_D_SC_CACHE" => Some("exact".to_string()),
            "LLM_D_SC_CACHE_THRESHOLD" => Some("not-a-number".to_string()),
            _ => None,
        };
        match CacheConfig::from_env_with(get) {
            Err(ConfigError::InvalidThreshold(v)) => assert!(v.is_nan()),
            other => panic!("expected InvalidThreshold, got {other:?}"),
        }
    }
}
