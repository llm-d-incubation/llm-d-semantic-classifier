//! Pure (no-I/O) codec + query-shape helpers for the Redis semantic cache, so
//! the byte encoding, index name, and result serialization are unit-testable
//! without a live Redis.

use crate::classify::ClassificationResult;

/// The RediSearch index over the semantic-cache hash keys.
pub fn index_name() -> &'static str {
    "sc_semantic_idx"
}

/// Encode an embedding as the little-endian f32 blob RediSearch expects for a
/// FLOAT32 vector field.
pub fn vector_to_bytes(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for f in v {
        out.extend_from_slice(&f.to_le_bytes());
    }
    out
}

/// Serialize a classification result for storage.
pub fn encode_result(r: &ClassificationResult) -> String {
    serde_json::to_string(r).expect("ClassificationResult serializes")
}

/// Deserialize a stored result; `None` on any corruption (treated as a miss).
pub fn decode_result(s: &str) -> Option<ClassificationResult> {
    serde_json::from_str(s).ok()
}

/// RediSearch returns COSINE *distance* (0 = identical). Convert to a
/// similarity score in [0, 1] for threshold comparison.
pub fn cosine_score_from_distance(distance: f32) -> f32 {
    1.0 - distance
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::{ClassificationResult, ClassifyStatus, RankedSignal};

    #[test]
    fn vector_bytes_are_little_endian_f32() {
        let bytes = vector_to_bytes(&[1.0f32, 2.0f32]);
        assert_eq!(bytes.len(), 8);
        assert_eq!(&bytes[0..4], &1.0f32.to_le_bytes());
        assert_eq!(&bytes[4..8], &2.0f32.to_le_bytes());
    }

    #[test]
    fn result_round_trips_through_json() {
        let r = ClassificationResult {
            classifier_id: "complexity".into(),
            model_revision: "m".into(),
            tokenizer_revision: "t".into(),
            taxonomy_revision: "x".into(),
            status: ClassifyStatus::Ok,
            ranked: vec![RankedSignal {
                id: "SIMPLE".into(),
                score: 0.87,
            }],
        };
        let encoded = encode_result(&r);
        let decoded = decode_result(&encoded).expect("decode");
        assert_eq!(decoded, r);
    }

    #[test]
    fn cosine_score_is_one_minus_distance() {
        assert!((cosine_score_from_distance(0.1) - 0.9).abs() < 1e-6);
    }

    #[test]
    fn bad_json_decodes_to_none() {
        assert!(decode_result("not json").is_none());
    }

    #[test]
    fn index_name_is_stable() {
        assert_eq!(index_name(), "sc_semantic_idx");
    }
}
