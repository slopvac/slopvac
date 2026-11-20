//! Sentence-level `requirement` and `enforced-invariant` taggers.
//!
//! Evidence: the detector prototype (`slopvac-8l4.2.14`) tagged 9% of sentences with each
//! tagger and lost no observed true positive. The paragraph-level invariant form was too
//! loose, so only the sentence-level form exists here.

use crate::{BlockKind, DocumentView, Error, Pattern, Tag, TagSink, Tagger};

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
    sink: &mut TagSink,
) -> Result<(), Error> {
    for b in doc.blocks() {
        if matches!(b.kind, BlockKind::Heading | BlockKind::CodeBlock) {
            continue;
        }
        for s in doc.sentences(&b) {
            if let Some(evidence) = test(s.text)? {
                sink.push_with(|| Tag {
                    kind: kind.to_string(),
                    span: s.span,
                    confidence,
                    evidence,
                })?;
            }
        }
    }
    Ok(())
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
        let mut sink = TagSink::unbounded();
        self.tag_into(doc, &mut sink)?;
        Ok(sink.into_tags())
    }

    fn tag_into(&self, doc: &dyn DocumentView, sink: &mut TagSink) -> Result<(), Error> {
        sentence_tags(
            doc,
            "requirement",
            0.8,
            |s| {
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
            },
            sink,
        )
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
        let mut preceding = Preceding::new(text);
        while let Some(m) = self.enforcement.find(&text[from..])? {
            let start = from + m.start;
            let end = from + m.end;
            preceding.advance_to(start);
            if !preceding.negated() {
                return Ok(Some(text[start..end].to_lowercase()));
            }
            from = end;
        }
        Ok(None)
    }
}

const SEPARATORS: [char; 3] = [',', ';', ':'];

/// The last two whitespace-separated tokens before a position that only moves forward.
/// Asking about each of many positions in one text reads the text once in total, where
/// splitting the prefix again for each position reads a long token again every time.
struct Preceding<'a> {
    text: &'a str,
    /// Everything before this byte has been read.
    pos: usize,
    /// A token ends at `pos` without whitespace after it.
    in_token: bool,
    /// Where the open token's text starts after its leading separators. `None` while it has
    /// only separators.
    lead: Option<usize>,
    /// The last two finished tokens without surrounding separators, latest first.
    done: [&'a str; 2],
}

impl<'a> Preceding<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            text,
            pos: 0,
            in_token: false,
            lead: None,
            done: [""; 2],
        }
    }

    fn advance_to(&mut self, to: usize) {
        for (i, c) in self.text[self.pos..to].char_indices() {
            crate::work::negation(1);
            let at = self.pos + i;
            if c.is_whitespace() {
                if self.in_token {
                    let token = self.open_token(at);
                    self.done = [token, self.done[0]];
                    self.in_token = false;
                    self.lead = None;
                }
            } else {
                self.in_token = true;
                if self.lead.is_none() && !SEPARATORS.contains(&c) {
                    self.lead = Some(at);
                }
            }
        }
        self.pos = to;
    }

    /// The open token up to `end`, without separators.
    fn open_token(&self, end: usize) -> &'a str {
        let token = self.lead.map_or("", |l| &self.text[l..end]);
        let trimmed = token.trim_end_matches(SEPARATORS);
        crate::work::negation(token.len() - trimmed.len());
        trimmed
    }

    /// Whether one of the last two tokens negates what follows.
    fn negated(&self) -> bool {
        let (first, second) = if self.in_token {
            (self.open_token(self.pos), self.done[0])
        } else {
            (self.done[0], self.done[1])
        };
        is_negator(first) || is_negator(second)
    }
}

/// A negating word, ignoring case, or a word that ends in `n't`.
fn is_negator(token: &str) -> bool {
    const WORDS: [&str; 7] = ["never", "not", "no", "without", "cannot", "neither", "nor"];
    let ends = |suffix: &str| {
        let (t, s) = (token.as_bytes(), suffix.as_bytes());
        t.len() >= s.len() && t[t.len() - s.len()..].eq_ignore_ascii_case(s)
    };
    WORDS.iter().any(|w| token.eq_ignore_ascii_case(w)) || ends("n't") || ends("n’t")
}

impl Tagger for EnforcedInvariantTagger {
    fn id(&self) -> &'static str {
        "enforced-invariant"
    }

    fn tag(&self, doc: &dyn DocumentView) -> Result<Vec<Tag>, Error> {
        let mut sink = TagSink::unbounded();
        self.tag_into(doc, &mut sink)?;
        Ok(sink.into_tags())
    }

    fn tag_into(&self, doc: &dyn DocumentView, sink: &mut TagSink) -> Result<(), Error> {
        sentence_tags(
            doc,
            "enforced-invariant",
            0.7,
            |s| {
                let text = clean(s);
                if let Some(v) = self.enforcement_verb(&text)? {
                    return Ok(Some(format!("enforced-invariant: enforcement verb `{v}`")));
                }
                Ok(self
                    .mechanism
                    .find(s)?
                    .map(|m| format!("enforced-invariant: mechanism clause `{}`", s[m].trim())))
            },
            sink,
        )
    }
}
