//! `Tagger::from_model_bytes` on small hand-built models in prose's flat
//! format: what it accepts, and what it must reject without panicking or
//! reserving memory a hostile header asks for.

use slopvac_pos::{ModelError, Tagger};

/// Appends a `u16`-prefixed string.
fn str16(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&u16::try_from(s.len()).unwrap().to_le_bytes());
    out.extend_from_slice(s.as_bytes());
}

/// A valid one-class, zero-feature model with the given metadata and
/// `declared_words` as the unambiguous-word count (no records follow it).
fn model(meta: &[(&str, &str)], declared_words: u32) -> Vec<u8> {
    let mut out = b"PROSETAG".to_vec();
    out.extend_from_slice(&2_u32.to_le_bytes()); // version
    out.extend_from_slice(&1_u32.to_le_bytes()); // classes
    str16(&mut out, "NN");
    out.extend_from_slice(&0_u32.to_le_bytes()); // features
    out.extend_from_slice(&0_u32.to_le_bytes()); // pairs
    out.extend_from_slice(&0_u32.to_le_bytes()); // blob length
    out.extend_from_slice(&0_u32.to_le_bytes()); // feature offsets: [0]
    out.extend_from_slice(&0_u32.to_le_bytes()); // pair offsets: [0]
    out.extend_from_slice(&declared_words.to_le_bytes());
    if declared_words == 0 {
        out.extend_from_slice(&u32::try_from(meta.len()).unwrap().to_le_bytes());
        for (k, v) in meta {
            str16(&mut out, k);
            str16(&mut out, v);
        }
    }
    out
}

const SUPPORTED: [(&str, &str); 2] = [("template", "1"), ("tagset", "penn")];

#[test]
fn accepts_a_model_with_the_supported_template_and_tagset() {
    let tagger = Tagger::from_model_bytes(&model(&SUPPORTED, 0)).unwrap();
    // One class and no features: every word gets that class.
    let tags: Vec<&str> = tagger.tag_text("any words").iter().map(|t| t.tag).collect();
    assert_eq!(tags, ["NN", "NN"]);
}

#[test]
fn rejects_the_byte_template() {
    let bytes = model(&[("template", "0"), ("tagset", "penn")], 0);
    assert_eq!(
        Tagger::from_model_bytes(&bytes).unwrap_err(),
        ModelError::UnsupportedTemplate("0".to_owned())
    );
}

#[test]
fn rejects_a_model_without_template_metadata() {
    let bytes = model(&[("tagset", "penn")], 0);
    assert_eq!(
        Tagger::from_model_bytes(&bytes).unwrap_err(),
        ModelError::UnsupportedTemplate(String::new())
    );
}

#[test]
fn rejects_the_universal_tag_set() {
    let bytes = model(&[("template", "1"), ("tagset", "upos")], 0);
    assert_eq!(
        Tagger::from_model_bytes(&bytes).unwrap_err(),
        ModelError::UnsupportedTagset("upos".to_owned())
    );
}

/// A header that declares four billion words in a few dozen bytes must fail
/// as truncated; it must not reserve a table for them first.
#[test]
fn an_oversized_word_count_is_truncated_not_allocated() {
    let bytes = model(&SUPPORTED, u32::MAX);
    assert!(bytes.len() < 100);
    assert_eq!(
        Tagger::from_model_bytes(&bytes).unwrap_err(),
        ModelError::Truncated
    );
}

#[test]
fn an_oversized_metadata_count_is_truncated() {
    let mut bytes = model(&[], 0);
    let n = bytes.len();
    bytes[n - 4..].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(
        Tagger::from_model_bytes(&bytes).unwrap_err(),
        ModelError::Truncated
    );
}
