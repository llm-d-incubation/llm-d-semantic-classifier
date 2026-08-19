//! Cosine-similarity ranking of an embedding against a set of prototypes.
//!
//! AC-004 (U-064/U-065) proves the deterministic ranking mechanics: a fixed
//! synthetic embedding must rank identically across repeated runs, and exact
//! ties must be broken by a documented, deterministic rule (lexicographic by
//! id). This module is PURE MATH: it operates on plain `&[f32]` vectors and
//! never touches the model, tokenizer, or network.

use std::cmp::Ordering;

/// A prototype (anchor) embedding: an id plus its vector.
///
/// The id is the deterministic tie-break key; the vector is compared to an
/// input embedding via cosine similarity.
#[derive(Debug, Clone)]
pub struct Prototype {
    pub id: String,
    pub vector: Vec<f32>,
}

impl Prototype {
    /// Build a prototype from an id and its vector.
    pub fn new(id: impl Into<String>, vector: Vec<f32>) -> Self {
        Prototype {
            id: id.into(),
            vector,
        }
    }
}

/// Rank `prototypes` by cosine similarity to `embedding`, highest score first.
///
/// Ordering contract (documented):
/// 1. Primary: scores sorted descending (highest cosine similarity first).
/// 2. Secondary (exact ties): ids sorted ascending lexicographically
///    (byte-wise), so equal scores have a total, deterministic order.
///
/// Scores are `f64`. Non-finite scores (NaN/Inf) from degenerate inputs are
/// treated as equal for ordering purposes; the caller is responsible for
/// validating inputs (pure-math contract).
pub fn cosine_rank(embedding: &[f32], prototypes: &[Prototype]) -> Vec<(String, f64)> {
    let mut ranked: Vec<(String, f64)> = prototypes
        .iter()
        .map(|p| (p.id.clone(), cosine_similarity(embedding, &p.vector)))
        .collect();
    ranked.sort_by(|a, b| {
        // Primary: descending score. Non-finite comparisons fall back to Equal.
        let score_cmp = b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal);
        // Secondary: ascending id (lexicographic) for exact ties.
        score_cmp.then_with(|| a.0.cmp(&b.0))
    });
    ranked
}

/// A labelled anchor set: every anchor embedding that defines one label.
///
/// A label is not a single point but a REGION of embedding space. Averaging all
/// anchors into one centroid discards that shape, so a label whose examples are
/// legitimately spread (a broad tier) is penalised. Scoring by the mean of the
/// top-k nearest anchors keeps the region while staying robust to a single
/// unrepresentative anchor.
#[derive(Debug, Clone)]
pub struct AnchorSet {
    pub label: String,
    pub vectors: Vec<Vec<f32>>,
}

/// Rank labels by the mean cosine similarity of their `top_k` nearest anchors.
///
/// Ordering contract matches [`cosine_rank`]:
/// 1. Primary: score descending.
/// 2. Secondary (exact ties): label ascending lexicographically.
///
/// `top_k` is clamped to the number of anchors a label actually has, so a label
/// with fewer anchors than `top_k` is scored over all of them rather than
/// silently scoring lower than a label with more.
pub fn anchor_rank(embedding: &[f32], anchors: &[AnchorSet], top_k: usize) -> Vec<(String, f64)> {
    let mut ranked: Vec<(String, f64)> = anchors
        .iter()
        .map(|set| {
            let mut sims: Vec<f64> = set
                .vectors
                .iter()
                .map(|v| cosine_similarity(embedding, v))
                .collect();
            // Descending; non-finite compares Equal (pure-math contract).
            sims.sort_by(|a, b| b.partial_cmp(a).unwrap_or(Ordering::Equal));
            let k = top_k.clamp(1, sims.len().max(1));
            let score = if sims.is_empty() {
                0.0
            } else {
                sims[..k.min(sims.len())].iter().sum::<f64>() / k.min(sims.len()) as f64
            };
            (set.label.clone(), score)
        })
        .collect();
    ranked.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    ranked
}

/// Cosine similarity between two equal-length vectors: dot / (|a| * |b|).
///
/// Degenerate (zero-norm) vectors score 0.0 against everything.
fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    let dot: f64 = a
        .iter()
        .zip(b.iter())
        .map(|(x, y)| f64::from(*x) * f64::from(*y))
        .sum();
    let na: f64 = a
        .iter()
        .map(|x| f64::from(*x) * f64::from(*x))
        .sum::<f64>()
        .sqrt();
    let nb: f64 = b
        .iter()
        .map(|x| f64::from(*x) * f64::from(*x))
        .sum::<f64>()
        .sqrt();
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na * nb)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    const PROTOTYPES_FIXTURE: &str = "tests/fixtures/modelcar/synthetic-prototypes.json";

    fn fixture_prototypes() -> Vec<Prototype> {
        let raw =
            std::fs::read_to_string(PROTOTYPES_FIXTURE).expect("prototype fixture must exist");
        let root: Value = serde_json::from_str(&raw).expect("prototype fixture must be valid JSON");
        assert_eq!(
            root.get("label").and_then(Value::as_str),
            Some("synthetic_for_mechanics_only"),
            "fixture must be labeled synthetic_for_mechanics_only"
        );
        assert_eq!(
            root.get("dim").and_then(Value::as_u64),
            Some(384),
            "fixture dim must be 384"
        );
        let arr = root
            .get("prototypes")
            .and_then(Value::as_array)
            .expect("fixture must have a prototypes array");
        arr.iter()
            .map(|obj| {
                let id = obj
                    .get("id")
                    .and_then(Value::as_str)
                    .expect("prototype id")
                    .to_string();
                let vector: Vec<f32> = obj
                    .get("vector")
                    .and_then(Value::as_array)
                    .expect("prototype vector")
                    .iter()
                    .map(|v| v.as_f64().expect("vector value") as f32)
                    .collect();
                Prototype::new(id, vector)
            })
            .collect()
    }

    fn assert_unit_vectors(prototypes: &[Prototype]) {
        for p in prototypes {
            let norm: f64 = p
                .vector
                .iter()
                .map(|x| f64::from(*x) * f64::from(*x))
                .sum::<f64>()
                .sqrt();
            assert!(
                (norm - 1.0).abs() < 1e-6,
                "prototype {} must be a unit vector (got norm {norm})",
                p.id
            );
        }
    }

    #[test]
    fn u064_prototype_ranking_deterministic() {
        // U-064 (AC-004): ranking of a fixed synthetic embedding must be
        // identical across 100 iterations — pure-math determinism (no RNG, no
        // float jitter, no state). Uses the committed synthetic fixture.
        let prototypes = fixture_prototypes();
        assert_unit_vectors(&prototypes);

        // Fixed synthetic embedding: proto-b (a unit basis vector).
        let embedding: Vec<f32> = prototypes
            .iter()
            .find(|p| p.id == "proto-b")
            .expect("proto-b must exist")
            .vector
            .clone();

        let first = cosine_rank(&embedding, &prototypes);
        // Sanity: proto-b must be the top rank at score ~1.0.
        assert_eq!(first[0].0, "proto-b", "proto-b must rank first");
        assert!((first[0].1 - 1.0).abs() < 1e-9, "proto-b must score ~1.0");

        for iter in 1..100 {
            let again = cosine_rank(&embedding, &prototypes);
            assert_eq!(
                again, first,
                "iteration {iter} ranking must be identical to the first"
            );
        }
    }

    #[test]
    fn u065_exact_tie_ranks_by_documented_rule() {
        // U-065 (AC-004): exact-tie scores must be ordered lexicographically by
        // id (documented rule). Build embedding = proto-a + proto-c: proto-a and
        // proto-c tie exactly (each cosine = 1/sqrt(2)); proto-b and proto-d are
        // orthogonal so they tie at 0.
        let prototypes = fixture_prototypes();
        assert_unit_vectors(&prototypes);

        let a = prototypes
            .iter()
            .find(|p| p.id == "proto-a")
            .expect("proto-a")
            .vector
            .clone();
        let c = prototypes
            .iter()
            .find(|p| p.id == "proto-c")
            .expect("proto-c")
            .vector
            .clone();
        let dim = a.len();
        let mut embedding = vec![0.0f32; dim];
        for i in 0..dim {
            embedding[i] = a[i] + c[i];
        }

        let ranked = cosine_rank(&embedding, &prototypes);
        let ids: Vec<&str> = ranked.iter().map(|(id, _)| id.as_str()).collect();

        // Exact tie between proto-a and proto-c: id-lexicographic order (a < c).
        assert_eq!(ids[0], "proto-a", "tie must order proto-a before proto-c");
        assert_eq!(ids[1], "proto-c");
        let score = ranked[0].1;
        assert!(
            (score - ranked[1].1).abs() < 1e-9,
            "tie scores must be equal (got {} vs {})",
            score,
            ranked[1].1
        );
        assert!(
            (score - (1.0 / 2.0f64.sqrt())).abs() < 1e-9,
            "tie score must be 1/sqrt(2) (got {score})"
        );

        // The remaining orthogonal pair tie at 0, also id-lexicographic (b < d).
        assert_eq!(ids[2], "proto-b");
        assert_eq!(ids[3], "proto-d");
        assert!(ranked[2].1.abs() < 1e-6, "proto-b must score ~0");
        assert!(ranked[3].1.abs() < 1e-6, "proto-d must score ~0");
    }
}
