//! Classifier definitions: the taxonomy a model is ranked against.
//!
//! A classifier definition is DATA, not code. It names the labels, supplies the
//! labelled anchor texts that define each label, and pins the revisions that
//! make a result reproducible. Three definitions are COMPILED INTO the binary
//! (`complexity`, `cost`, `sensitivity`), so every instance of llm-d-sc can rank
//! against a real taxonomy with no external file and no network fetch. A custom
//! definition may be supplied by path.
//!
//! Anchors are embedded at load time by the SAME resident model that embeds
//! requests, so a label is "the region of embedding space near these examples".
//! Changing anchors changes behaviour without retraining, which is the point:
//! the taxonomy is versioned data (`taxonomy_revision`) that participates in the
//! cache key, so a taxonomy change can never serve a stale classification.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use crate::embedding::Embedder;
use crate::ranker::AnchorSet;

/// The built-in classifier definitions, compiled into the binary.
const COMPLEXITY: &str = include_str!("../classifiers/complexity.json");
const COST: &str = include_str!("../classifiers/cost.json");
const SENSITIVITY: &str = include_str!("../classifiers/sensitivity.json");

/// The default built-in classifier when none is requested.
pub const DEFAULT_CLASSIFIER: &str = "complexity";

/// Environment variable selecting a built-in name or a path to a custom
/// definition JSON.
pub const CLASSIFIER_ENV: &str = "LLM_D_SC_CLASSIFIER";

/// Errors produced while resolving a classifier definition.
#[derive(Debug)]
pub enum TaxonomyError {
    Unknown(String),
    Io(std::io::Error),
    Json(serde_json::Error),
    Invalid(String),
}

impl std::fmt::Display for TaxonomyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaxonomyError::Unknown(n) => write!(
                f,
                "unknown classifier '{n}': expected one of {} or a path to a definition JSON",
                built_in_names().join(", ")
            ),
            TaxonomyError::Io(e) => write!(f, "classifier definition io error: {e}"),
            TaxonomyError::Json(e) => write!(f, "classifier definition json error: {e}"),
            TaxonomyError::Invalid(m) => write!(f, "invalid classifier definition: {m}"),
        }
    }
}

impl std::error::Error for TaxonomyError {}

/// A classifier definition: labels, their anchor texts, and pinned revisions.
#[derive(Debug, Clone, Deserialize)]
pub struct ClassifierDefinition {
    pub classifier_id: String,
    pub signal: String,
    pub taxonomy_revision: String,
    pub model_repo: String,
    pub model_revision: String,
    #[serde(default = "default_top_k")]
    pub top_k: usize,
    pub labels: Vec<String>,
    pub anchors: BTreeMap<String, Vec<String>>,
}

fn default_top_k() -> usize {
    3
}

/// The names of every built-in classifier definition.
pub fn built_in_names() -> Vec<&'static str> {
    vec!["complexity", "cost", "sensitivity"]
}

impl ClassifierDefinition {
    /// A built-in definition by name, or `None` if the name is not built in.
    pub fn built_in(name: &str) -> Option<Result<Self, TaxonomyError>> {
        let raw = match name {
            "complexity" => COMPLEXITY,
            "cost" => COST,
            "sensitivity" => SENSITIVITY,
            _ => return None,
        };
        Some(Self::from_str(raw))
    }

    /// Parse a definition from JSON text, validating its internal consistency.
    pub fn from_str(raw: &str) -> Result<Self, TaxonomyError> {
        let def: ClassifierDefinition =
            serde_json::from_str(raw).map_err(TaxonomyError::Json)?;
        def.validate()?;
        Ok(def)
    }

    /// Load a custom definition from a path.
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, TaxonomyError> {
        let raw = std::fs::read_to_string(path).map_err(TaxonomyError::Io)?;
        Self::from_str(&raw)
    }

    /// Resolve a built-in NAME or a PATH to a custom definition.
    ///
    /// A name that is not built in but exists as a file is loaded as a custom
    /// definition, so operators can ship their own taxonomy without rebuilding.
    pub fn resolve(spec: &str) -> Result<Self, TaxonomyError> {
        if let Some(built) = Self::built_in(spec) {
            return built;
        }
        if Path::new(spec).is_file() {
            return Self::load(spec);
        }
        Err(TaxonomyError::Unknown(spec.to_string()))
    }

    /// Resolve from the environment, falling back to the default built-in.
    pub fn from_env() -> Result<Self, TaxonomyError> {
        let spec =
            std::env::var(CLASSIFIER_ENV).unwrap_or_else(|_| DEFAULT_CLASSIFIER.to_string());
        Self::resolve(&spec)
    }

    /// Every label must have at least one anchor, and every anchor label must be
    /// a declared label. A definition that ranks against nothing is rejected at
    /// load rather than silently returning an empty ranking at request time.
    fn validate(&self) -> Result<(), TaxonomyError> {
        if self.labels.is_empty() {
            return Err(TaxonomyError::Invalid("no labels declared".into()));
        }
        if self.top_k == 0 {
            return Err(TaxonomyError::Invalid("top_k must be at least 1".into()));
        }
        for label in &self.labels {
            match self.anchors.get(label) {
                None => {
                    return Err(TaxonomyError::Invalid(format!(
                        "label '{label}' has no anchors"
                    )))
                }
                Some(a) if a.is_empty() => {
                    return Err(TaxonomyError::Invalid(format!(
                        "label '{label}' has an empty anchor list"
                    )))
                }
                Some(_) => {}
            }
        }
        for key in self.anchors.keys() {
            if !self.labels.contains(key) {
                return Err(TaxonomyError::Invalid(format!(
                    "anchor group '{key}' is not a declared label"
                )));
            }
        }
        Ok(())
    }

    /// Total number of anchor texts across every label.
    pub fn anchor_count(&self) -> usize {
        self.anchors.values().map(Vec::len).sum()
    }

    /// Embed every anchor with the resident model, producing the ranked anchor
    /// sets. Runs ONCE at load time, never per request.
    pub fn embed_anchors(&self, embedder: &Embedder) -> Result<Vec<AnchorSet>, TaxonomyError> {
        self.labels
            .iter()
            .map(|label| {
                let vectors = self.anchors[label]
                    .iter()
                    .map(|text| {
                        embedder.embed(text).map_err(|e| {
                            TaxonomyError::Invalid(format!(
                                "failed to embed anchor for '{label}': {e}"
                            ))
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(AnchorSet {
                    label: label.clone(),
                    vectors,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn u100_every_built_in_definition_parses_and_validates() {
        // Every definition compiled into the binary must be loadable with no
        // external file: an instance of llm-d-sc always has a real taxonomy.
        for name in built_in_names() {
            let def = ClassifierDefinition::built_in(name)
                .unwrap_or_else(|| panic!("{name} must be built in"))
                .unwrap_or_else(|e| panic!("{name} must validate: {e}"));
            assert_eq!(def.classifier_id, name);
            assert!(
                def.labels.len() >= 4,
                "{name} must declare at least 4 labels, got {}",
                def.labels.len()
            );
            assert!(
                def.anchor_count() >= def.labels.len(),
                "{name} must have at least one anchor per label"
            );
        }
    }

    #[test]
    fn u101_default_classifier_is_a_built_in() {
        ClassifierDefinition::built_in(DEFAULT_CLASSIFIER)
            .expect("default must be built in")
            .expect("default must validate");
    }

    #[test]
    fn u102_definition_with_a_label_that_has_no_anchors_is_rejected() {
        // A taxonomy that cannot rank one of its own labels is a configuration
        // error, and must fail at load rather than at request time.
        let raw = r#"{
          "classifier_id":"x","signal":"x","taxonomy_revision":"v1",
          "model_repo":"r","model_revision":"s","top_k":3,
          "labels":["A","B"],
          "anchors":{"A":["only a"]}
        }"#;
        let err = ClassifierDefinition::from_str(raw).expect_err("must reject missing anchors");
        assert!(
            format!("{err}").contains("has no anchors"),
            "error must name the unanchored label, got: {err}"
        );
    }

    #[test]
    fn u103_unknown_classifier_name_is_rejected_with_the_available_names() {
        let err = ClassifierDefinition::resolve("nope").expect_err("must reject unknown name");
        let msg = format!("{err}");
        for name in built_in_names() {
            assert!(msg.contains(name), "error must list '{name}': {msg}");
        }
    }
}
