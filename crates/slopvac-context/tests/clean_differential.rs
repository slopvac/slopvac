//! The sentence taggers report what they reported before the single-pass text cleaning.
//!
//! The oracle is the previous code, copied unchanged: `strip_code`, `closing_run` and `clean`
//! with their rescans, and the two taggers that call `clean`. Both sides read the same random
//! text, so the comparison covers the spans, the confidence and the evidence the public
//! taggers return.
#![allow(clippy::unwrap_used, reason = "test code: a panic is a failed test")]

use proptest::prelude::*;

use slopvac_context::{
    EnforcedInvariantTagger, Error, PlainDocument, RequirementTagger, Tag, Tagger,
};

mod old {
    use slopvac_context::{BlockKind, DocumentView, Error, Pattern, Tag};

    pub(crate) fn strip_code(s: &str) -> String {
        let bytes = s.as_bytes();
        let mut out = String::with_capacity(s.len());
        let mut i = 0;
        let mut plain = 0;
        while i < bytes.len() {
            if bytes[i] != b'`' {
                i += 1;
                continue;
            }
            let run = bytes[i..].iter().take_while(|&&b| b == b'`').count();
            match closing_run(bytes, i + run, run) {
                Some(end) => {
                    out.push_str(&s[plain..i]);
                    out.push(' ');
                    i = end;
                    plain = end;
                }
                None => i += run,
            }
        }
        out.push_str(&s[plain..]);
        out
    }

    fn closing_run(bytes: &[u8], from: usize, len: usize) -> Option<usize> {
        let mut j = from;
        while j < bytes.len() {
            if bytes[j] == b'`' {
                let run = bytes[j..].iter().take_while(|&&b| b == b'`').count();
                if run == len {
                    return Some(j + run);
                }
                j += run;
            } else {
                j += 1;
            }
        }
        None
    }

    pub(crate) fn clean(s: &str) -> String {
        let no_code = strip_code(s);
        let mut out = String::with_capacity(no_code.len());
        let chars: Vec<char> = no_code.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if c == ']'
                && chars.get(i + 1) == Some(&'(')
                && let Some(close) = chars[i + 2..].iter().position(|&x| x == ')')
            {
                i += 2 + close + 1;
                continue;
            }
            if !matches!(c, '[' | '*' | '_') {
                out.push(c);
            }
            i += 1;
        }
        out.trim().to_string()
    }

    const NORMATIVE: &str = r"\b(?:must(?:\s+not)?|required|requires?|cannot|can['’]t|can\s+not|shall(?:\s+not)?|should\s+not)\b|(?<![\w-])only(?![\w-])|(?:^\W*|[.!?:;]\s+)never\s+(?=[a-z])";
    const NEGATED_IMPERATIVE: &str = r"^\W*(?:do not|don['’]t|avoid|never)\b";
    const NEGATED_LITERAL: &str = r"\b(?:not|no|never|without|neither|nor)\s+(?:\w+\s+){0,3}(?:`[^`]+`|--?[a-z][\w-]+|(?:\.{0,2}/)[\w./-]+)";
    const ENFORCEMENT: &str = r"\b(?:rejects?|rejected|refus(?:es?|ed)|den(?:y|ies|ied)|blocks?|blocked|validat(?:es?|ed)|enforc(?:es?|ed)|prevents?|prevented|requires?|drops?|dropped|ignores?|ignored|fails?|failed|errors?|returns?\s+an?\s+error)\b";
    const MECHANISM: &str = r"\b(?:because|since)\b|,\s*so\s|;\s*so\s|\bso that\b";

    fn sentence_tags(
        doc: &dyn DocumentView,
        kind: &str,
        confidence: f32,
        test: impl Fn(&str) -> Result<Option<String>, Error>,
    ) -> Result<Vec<Tag>, Error> {
        let mut out = Vec::new();
        for b in doc.blocks() {
            if matches!(b.kind, BlockKind::Heading | BlockKind::CodeBlock) {
                continue;
            }
            for s in doc.sentences(&b) {
                if let Some(evidence) = test(s.text)? {
                    out.push(Tag {
                        kind: kind.to_string(),
                        span: s.span,
                        confidence,
                        evidence: evidence.into(),
                    });
                }
            }
        }
        Ok(out)
    }

    pub(crate) struct Requirement {
        normative: Pattern,
        negated_imperative: Pattern,
        negated_literal: Pattern,
    }

    impl Requirement {
        pub(crate) fn new() -> Result<Self, Error> {
            Ok(Self {
                normative: Pattern::new_caseless(NORMATIVE)?,
                negated_imperative: Pattern::new_caseless(NEGATED_IMPERATIVE)?,
                negated_literal: Pattern::new_caseless(NEGATED_LITERAL)?,
            })
        }

        pub(crate) fn tag(&self, doc: &dyn DocumentView) -> Result<Vec<Tag>, Error> {
            sentence_tags(doc, "requirement", 0.8, |s| {
                let cleaned = clean(s);
                if let Some(m) = self.normative.find(&cleaned)? {
                    let word = cleaned[m]
                        .trim_matches(|c: char| !c.is_alphanumeric())
                        .to_lowercase();
                    return Ok(Some(format!("requirement: normative marker `{word}`")));
                }
                if self.negated_imperative.is_match(&cleaned)? {
                    return Ok(Some("requirement: negated imperative".to_string()));
                }
                Ok(self
                    .negated_literal
                    .find(s)?
                    .map(|m| format!("requirement: negated literal `{}`", &s[m])))
            })
        }
    }

    pub(crate) struct Enforced {
        enforcement: Pattern,
        mechanism: Pattern,
    }

    impl Enforced {
        pub(crate) fn new() -> Result<Self, Error> {
            Ok(Self {
                enforcement: Pattern::new_caseless(ENFORCEMENT)?,
                mechanism: Pattern::new_caseless(MECHANISM)?,
            })
        }

        fn enforcement_verb(&self, text: &str) -> Result<Option<String>, Error> {
            let mut from = 0;
            while let Some(m) = self.enforcement.find(&text[from..])? {
                let start = from + m.start;
                let end = from + m.end;
                let negated = text[..start].split_whitespace().rev().take(2).any(|w| {
                    let w = w.trim_matches([',', ';', ':']).to_lowercase();
                    matches!(
                        w.as_str(),
                        "never" | "not" | "no" | "without" | "cannot" | "neither" | "nor"
                    ) || w.ends_with("n't")
                        || w.ends_with("n’t")
                });
                if !negated {
                    return Ok(Some(text[start..end].to_lowercase()));
                }
                from = end;
            }
            Ok(None)
        }

        pub(crate) fn tag(&self, doc: &dyn DocumentView) -> Result<Vec<Tag>, Error> {
            sentence_tags(doc, "enforced-invariant", 0.7, |s| {
                let text = clean(s);
                if let Some(v) = self.enforcement_verb(&text)? {
                    return Ok(Some(format!("enforced-invariant: enforcement verb `{v}`")));
                }
                Ok(self
                    .mechanism
                    .find(s)?
                    .map(|m| format!("enforced-invariant: mechanism clause `{}`", s[m].trim())))
            })
        }
    }
}

/// Words that trigger the taggers, Markdown syntax that `clean` rewrites, and filler.
const PIECES: &[&str] = &[
    "must", "never", "only", "do not", "rejects", "blocks", "because", "not", "`", "``", "](", ")",
    "[", "]", "*", "_", " ", " ", "a", "--force", "./x", ". ", "é",
];

fn text() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(PIECES), 0..40).prop_map(|p| p.concat())
}

type Observed = Vec<(String, std::ops::Range<usize>, u32, String)>;

fn observed(tags: Result<Vec<Tag>, Error>) -> Observed {
    tags.unwrap()
        .into_iter()
        .map(|t| {
            (
                t.kind,
                t.span,
                t.confidence.to_bits(),
                t.evidence.to_string(),
            )
        })
        .collect()
}

proptest! {
    #[test]
    fn the_requirement_tagger_matches_the_old_algorithm(t in text()) {
        let doc = PlainDocument::from_markdown("a.md", &format!("# T\n\nx {t}\n"));
        let new = RequirementTagger::new().unwrap();
        let old = old::Requirement::new().unwrap();
        prop_assert_eq!(observed(new.tag(&doc)), observed(old.tag(&doc)));
    }

    #[test]
    fn the_enforced_invariant_tagger_matches_the_old_algorithm(t in text()) {
        let doc = PlainDocument::from_markdown("a.md", &format!("# T\n\nx {t}\n"));
        let new = EnforcedInvariantTagger::new().unwrap();
        let old = old::Enforced::new().unwrap();
        prop_assert_eq!(observed(new.tag(&doc)), observed(old.tag(&doc)));
    }
}
