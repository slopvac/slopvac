//! Sentence-level `requirement` and `enforced-invariant` taggers.
//!
//! Evidence: the detector prototype (`slopvac-8l4.2.14`) tagged 9% of sentences with each
//! tagger and lost no observed true positive. The paragraph-level invariant form was too
//! loose, so only the sentence-level form exists here.

use crate::{BlockKind, DocumentView, Error, Pattern, Tag, Tagger};

use super::text::clean;

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
                    evidence,
                });
            }
        }
    }
    Ok(out)
}

/// Tags sentences that state a requirement.
pub struct RequirementTagger {
    normative: Pattern,
    negated_imperative: Pattern,
    negated_literal: Pattern,
}

impl RequirementTagger {
    /// Compile the patterns.
    pub fn new() -> Result<Self, Error> {
        Ok(Self {
            normative: Pattern::new_caseless(NORMATIVE)?,
            negated_imperative: Pattern::new_caseless(NEGATED_IMPERATIVE)?,
            negated_literal: Pattern::new_caseless(NEGATED_LITERAL)?,
        })
    }
}

impl Tagger for RequirementTagger {
    fn id(&self) -> &'static str {
        "requirement"
    }

    fn tag(&self, doc: &dyn DocumentView) -> Result<Vec<Tag>, Error> {
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

/// Tags sentences that say how a guarantee is enforced.
pub struct EnforcedInvariantTagger {
    enforcement: Pattern,
    mechanism: Pattern,
}

impl EnforcedInvariantTagger {
    /// Compile the patterns.
    pub fn new() -> Result<Self, Error> {
        Ok(Self {
            enforcement: Pattern::new_caseless(ENFORCEMENT)?,
            mechanism: Pattern::new_caseless(MECHANISM)?,
        })
    }

    /// An enforcement verb that is not itself negated ("never rejects", "does not block").
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
}

impl Tagger for EnforcedInvariantTagger {
    fn id(&self) -> &'static str {
        "enforced-invariant"
    }

    fn tag(&self, doc: &dyn DocumentView) -> Result<Vec<Tag>, Error> {
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
