//! Tokenizer residency and golden token-ID parity.
//!
//! AC-004 requires the pinned sensitivity model to match trusted reference
//! embedding/ranking fixtures. Tokenization is the first deterministic stage:
//! the resident tokenizer must reproduce the exact token IDs of the pinned
//! Python reference for the golden inputs.
//!
//! [`Tokenizer`] loads a `tokenizer.json` in the HuggingFace `tokenizers`
//! library format and reproduces the reference `BertNormalizer` +
//! `BertPreTokenizer` + `WordPiece` + `TemplateProcessing` pipeline. This is a
//! resident tokenizer: it reads only the committed ModelCar fixture and makes
//! no network call.

use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use unicode_categories::UnicodeCategories;
use unicode_normalization::UnicodeNormalization;

/// A resident BERT wordpiece tokenizer loaded from a `tokenizer.json`.
///
/// Tokenization follows the pinned reference (HuggingFace `tokenizers` library)
/// so downstream pooling and ranking parity can be proven against the same
/// reference fixtures.
pub struct Tokenizer {
    vocab: HashMap<String, u32>,
    unk_id: u32,
    cls_id: u32,
    sep_id: u32,
    continuing_subword_prefix: String,
    max_input_chars_per_word: usize,
    max_length: Option<usize>,
    lower: bool,
    strip_accents: bool,
}

/// Errors produced while loading or tokenizing.
#[derive(Debug)]
pub enum TokenizerError {
    Io(std::io::Error),
    Json(serde_json::Error),
    MissingField(&'static str),
}

impl std::fmt::Display for TokenizerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TokenizerError::Io(e) => write!(f, "tokenizer io error: {e}"),
            TokenizerError::Json(e) => write!(f, "tokenizer json error: {e}"),
            TokenizerError::MissingField(name) => write!(f, "tokenizer missing field: {name}"),
        }
    }
}

impl std::error::Error for TokenizerError {}

impl Tokenizer {
    /// Load a tokenizer from a `tokenizer.json` (tokenizers library format).
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Tokenizer, TokenizerError> {
        let raw = fs::read_to_string(path).map_err(TokenizerError::Io)?;
        let root: Value = serde_json::from_str(&raw).map_err(TokenizerError::Json)?;

        let mut vocab = HashMap::new();
        let vocab_obj = root
            .get("model")
            .and_then(|m| m.get("vocab"))
            .and_then(Value::as_object)
            .ok_or(TokenizerError::MissingField("model.vocab"))?;
        for (token, id) in vocab_obj {
            let id = id
                .as_u64()
                .ok_or(TokenizerError::MissingField("model.vocab id"))? as u32;
            vocab.insert(token.clone(), id);
        }

        let unk_token = root
            .get("model")
            .and_then(|m| m.get("unk_token"))
            .and_then(Value::as_str)
            .ok_or(TokenizerError::MissingField("model.unk_token"))?;
        let unk_id = vocab.get(unk_token).copied().unwrap_or(0);

        let continuing_subword_prefix = root
            .get("model")
            .and_then(|m| m.get("continuing_subword_prefix"))
            .and_then(Value::as_str)
            .unwrap_or("##")
            .to_string();

        let max_input_chars_per_word = root
            .get("model")
            .and_then(|m| m.get("max_input_chars_per_word"))
            .and_then(Value::as_u64)
            .unwrap_or(100) as usize;

        // [CLS]/[SEP] ids come from the TemplateProcessing post-processor.
        let cls_id = special_token_id(&root, "[CLS]").unwrap_or(101);
        let sep_id = special_token_id(&root, "[SEP]").unwrap_or(102);

        // Truncation (U-066): honor the fixture's `truncation.max_length`,
        // matching the pinned reference so over-length inputs are capped
        // deterministically. Absent when the tokenizer.json has no truncation.
        let max_length = root
            .get("truncation")
            .and_then(|t| t.get("max_length"))
            .and_then(Value::as_u64)
            .map(|v| v as usize);

        let lower = root
            .get("normalizer")
            .and_then(|n| n.get("lowercase"))
            .and_then(Value::as_bool)
            .unwrap_or(true);
        // The tokenizers library defaults `strip_accents` to the `lowercase`
        // value when unset (the sensitivity fixture ships `strip_accents: null`).
        let strip_accents = match root.get("normalizer").and_then(|n| n.get("strip_accents")) {
            Some(v) => v.as_bool().unwrap_or(lower),
            None => lower,
        };

        Ok(Tokenizer {
            vocab,
            unk_id,
            cls_id,
            sep_id,
            continuing_subword_prefix,
            max_input_chars_per_word,
            max_length,
            lower,
            strip_accents,
        })
    }

    /// Tokenize a single sequence, returning token IDs including `[CLS]` and
    /// `[SEP]` (matching the reference `TemplateProcessing`). Over-length inputs
    /// are truncated to the fixture's `max_length` (deterministically, dropping
    /// excess tokens from the right), matching the pinned reference.
    pub fn tokenize(&self, text: &str) -> Result<Vec<u32>, TokenizerError> {
        let normalized = self.normalize(text);
        let words = self.pre_tokenize(&normalized);
        let mut content = Vec::with_capacity(words.len());
        for w in words {
            content.extend(self.wordpiece(&w));
        }
        // Truncate content first, then wrap with [CLS]/[SEP]. The reference
        // caps total length (including the two special tokens) at max_length,
        // so the content budget is max_length - 2.
        if let Some(max_length) = self.max_length {
            content.truncate(max_length.saturating_sub(2));
        }
        let mut ids = Vec::with_capacity(content.len() + 2);
        ids.push(self.cls_id);
        ids.extend(content);
        ids.push(self.sep_id);
        Ok(ids)
    }

    /// Reference `BertNormalizer`: clean text, space out CJK chars, strip
    /// accents, lowercase.
    fn normalize(&self, text: &str) -> String {
        // clean_text: drop NUL/U+FFFD/control chars, map whitespace to ' '.
        let cleaned: String = text
            .chars()
            .filter(|c| !(*c as usize == 0 || *c == '\u{fffd}' || is_control(*c)))
            .map(|c| if is_whitespace(c) { ' ' } else { c })
            .collect();
        let spaced = handle_chinese_chars(&cleaned);
        let stripped = if self.strip_accents {
            spaced
                .as_str()
                .nfd()
                .filter(|c| !is_nonspacing_mark(*c))
                .collect::<String>()
        } else {
            spaced
        };
        if self.lower {
            stripped.to_lowercase()
        } else {
            stripped
        }
    }

    /// Reference `BertPreTokenizer`: split on whitespace (removed), then split
    /// on punctuation (isolated).
    fn pre_tokenize(&self, text: &str) -> Vec<String> {
        let whitespace_split: Vec<&str> = text
            .split(char::is_whitespace)
            .filter(|s| !s.is_empty())
            .collect();
        let mut tokens = Vec::new();
        for part in whitespace_split {
            let mut cur = String::new();
            for c in part.chars() {
                if is_bert_punc(c) {
                    if !cur.is_empty() {
                        tokens.push(std::mem::take(&mut cur));
                    }
                    tokens.push(c.to_string());
                } else {
                    cur.push(c);
                }
            }
            if !cur.is_empty() {
                tokens.push(cur);
            }
        }
        tokens
    }

    /// Reference `WordPiece` model: greedy longest-match-first with the
    /// continuing-subword prefix; any unmatched word collapses to `[UNK]`.
    fn wordpiece(&self, word: &str) -> Vec<u32> {
        if word.chars().count() > self.max_input_chars_per_word {
            return vec![self.unk_id];
        }
        let mut is_bad = false;
        let mut start = 0;
        let mut sub_tokens: Vec<u32> = Vec::new();
        while start < word.len() {
            let mut end = word.len();
            let mut cur_id: Option<u32> = None;
            while start < end {
                let sub: &str = &word[start..end];
                let candidate: String = if start > 0 {
                    format!("{}{}", self.continuing_subword_prefix, sub)
                } else {
                    sub.to_string()
                };
                if let Some(&id) = self.vocab.get(&candidate) {
                    cur_id = Some(id);
                    break;
                }
                end -= sub.chars().last().map_or(1, |c| c.len_utf8());
            }
            match cur_id {
                Some(id) => {
                    sub_tokens.push(id);
                    start = end;
                }
                None => {
                    is_bad = true;
                    break;
                }
            }
        }
        if is_bad {
            vec![self.unk_id]
        } else {
            sub_tokens
        }
    }
}

/// Pull the id of a named special token from the post-processor.
fn special_token_id(root: &Value, name: &str) -> Option<u32> {
    root.get("post_processor")
        .and_then(|p| p.get("special_tokens"))
        .and_then(|s| s.get(name))
        .and_then(|c| c.get("ids"))
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(Value::as_u64)
        .map(|id| id as u32)
}

/// Whether a character counts as whitespace per the reference `BertNormalizer`.
fn is_whitespace(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r') || c.is_whitespace()
}

/// Whether a character counts as control per the reference (the Cc/Cf/Cn/Co
/// "other" categories), mirroring the `unicode-categories` crate used by the
/// `tokenizers` library.
fn is_control(c: char) -> bool {
    match c {
        '\t' | '\n' | '\r' => false,
        _ => c.is_other(),
    }
}

/// Whether a character is a CJK ideograph per the reference `BasicTokenizer`.
fn is_chinese_char(c: char) -> bool {
    matches!(
        c as u32,
        0x4E00..=0x9FFF
            | 0x3400..=0x4DBF
            | 0x20000..=0x2A6DF
            | 0x2A700..=0x2B73F
            | 0x2B740..=0x2B81F
            | 0x2B920..=0x2CEAF
            | 0xF900..=0xFAFF
            | 0x2F800..=0x2FA1F
    )
}

/// Whether a character is punctuation per the reference `BertPreTokenizer`.
fn is_bert_punc(c: char) -> bool {
    c.is_ascii_punctuation() || c.is_punctuation()
}

/// Whether a character is a nonspacing combining mark (Mn) per the reference.
fn is_nonspacing_mark(c: char) -> bool {
    c.is_mark_nonspacing()
}

/// Surround CJK ideographs with spaces so they split into single-character
/// tokens (reference `handle_chinese_chars`).
fn handle_chinese_chars(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    for c in text.chars() {
        if is_chinese_char(c) {
            out.push(' ');
            out.push(c);
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOLDEN_INPUT: &str = "this is a golden sensitivity input";

    fn fixture(name: &str) -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("modelcar")
            .join(name)
    }

    fn load_golden_token_ids() -> Vec<u32> {
        let raw = std::fs::read_to_string(fixture("golden-token-ids.json"))
            .expect("golden fixture readable");
        let v: Value = serde_json::from_str(&raw).expect("golden fixture valid json");
        v["token_ids"]
            .as_array()
            .expect("golden token_ids array")
            .iter()
            .map(|x| x.as_u64().expect("token id numeric") as u32)
            .collect()
    }

    #[test]
    fn u060_tokenizer_golden_token_ids_match_trusted_reference() {
        // U-060 (AC-004): the resident tokenizer must reproduce the trusted
        // reference token IDs for the golden input so downstream pooling and
        // ranking parity can be proven against the same pinned reference. The
        // tokenizer is loaded from the committed ModelCar fixture (no network),
        // and the expected IDs are the committed golden fixture produced by the
        // pinned Python reference (HuggingFace `tokenizers` library).
        let tokenizer = Tokenizer::load(fixture("tokenizer.json"))
            .expect("tokenizer must load from the committed ModelCar fixture");
        let ids = tokenizer
            .tokenize(GOLDEN_INPUT)
            .expect("golden input must tokenize");
        assert_eq!(
            ids,
            load_golden_token_ids(),
            "tokenizer token IDs must match the trusted reference"
        );
    }

    #[test]
    fn u066_max_length_truncation_deterministic() {
        // U-066 (AC-004): when the input exceeds the fixture's configured max
        // length, the resident tokenizer must truncate deterministically and
        // exactly as the pinned reference does: total length (including [CLS]
        // and [SEP]) is capped at `truncation.max_length`, the [CLS] and [SEP]
        // special tokens are preserved, and excess tokens are dropped from the
        // right (the tail), not the head.
        let tokenizer = Tokenizer::load(fixture("tokenizer.json"))
            .expect("tokenizer must load from the committed ModelCar fixture");

        // The fixture pins truncation.max_length; the resident tokenizer must
        // honor that same value so parity with the reference is maintained.
        let raw =
            std::fs::read_to_string(fixture("tokenizer.json")).expect("tokenizer fixture readable");
        let root: Value = serde_json::from_str(&raw).expect("tokenizer fixture valid json");
        let max_length = root["truncation"]["max_length"]
            .as_u64()
            .expect("fixture truncation.max_length present") as usize;

        // Far more than max_length tokens: 400 "golden" words plus a trailing
        // marker that must be dropped by right truncation.
        let mut long_input = "golden ".repeat(400);
        long_input.push_str("tailmarker");

        let first = tokenizer
            .tokenize(&long_input)
            .expect("long input must tokenize");
        let second = tokenizer
            .tokenize(&long_input)
            .expect("long input must tokenize deterministically");

        // Determinism: repeated tokenization of the same input is identical.
        assert_eq!(first, second, "max-length truncation must be deterministic");

        // Total length is capped at the fixture's max_length.
        assert_eq!(
            first.len(),
            max_length,
            "truncated output length must equal the fixture max_length"
        );

        // [CLS] and [SEP] are preserved at the boundaries.
        assert_eq!(
            first.first(),
            Some(&101),
            "truncated output must keep [CLS] first"
        );
        assert_eq!(
            first.last(),
            Some(&102),
            "truncated output must keep [SEP] last"
        );

        // Content budget is max_length minus the two special tokens, matching
        // the pinned reference.
        let content_len = first.len() - 2;
        let golden_id = load_golden_token_ids()[4]; // id of "golden" = 3585
        assert_eq!(
            first.iter().filter(|&&i| i == golden_id).count(),
            content_len,
            "content tokens must be kept up to max_length - 2"
        );

        // Right truncation: the trailing marker must be dropped, the head kept.
        let marker_id = tokenizer.tokenize("tailmarker").expect("marker tokenizes");
        let marker_content = marker_id
            .iter()
            .skip(1)
            .take_while(|&&i| i != 102)
            .copied()
            .collect::<Vec<u32>>();
        assert!(
            !marker_content.is_empty(),
            "marker must tokenize to a real content token"
        );
        let marker_content_id = marker_content[0];
        assert!(
            !first.contains(&marker_content_id),
            "right truncation must drop the tail marker token"
        );
        assert!(
            first.contains(&golden_id),
            "right truncation must keep the head content tokens"
        );
    }
}
