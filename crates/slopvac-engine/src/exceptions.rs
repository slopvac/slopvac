//! Exception detectors: the code that drops a finding for a named reason.
//!
//! A rule lists its exceptions by name. An exception with a `detector` drops a finding on
//! its own, with no annotation in the document. [`Detectors`] resolves the detector name:
//!
//! - `quotation`, `code-span` and `negated-antecedent` are built in.
//! - A name registered with [`Detectors::register`] runs that detector.
//! - Any other name is a context tag, so `requirement` and `enforced-invariant` come from
//!   the taggers in `slopvac-context`, and a pack's own tag works the same way. The finding
//!   is dropped when that tag covers its span.

use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::LazyLock;

use slopvac_model::Span;
use slopvac_parse::{Document, Projection};
use slopvac_text::{Pattern, split_sentences};

/// Where a [`MatchSite`]'s text sits in the projection it was cut from.
///
/// The projection is the exact one the engine matched in, so `base` locates a match in it
/// without going back through source bytes. A source span is lossy where one source token
/// projects to many (an MDX string literal), and it cannot tell two identical sentences
/// apart inside one token.
#[derive(Debug, Clone, Copy)]
pub struct TextOrigin<'a> {
    /// The projection that holds the site's text.
    pub projection: &'a Projection,
    /// Byte offset of the site's text in `projection.text()`.
    pub base: usize,
}

/// A candidate finding, as a detector sees it.
#[derive(Debug, Clone, Copy)]
pub struct MatchSite<'a> {
    /// The text the rule matched in: a block, a sentence or a line of projected prose.
    pub text: &'a str,
    /// The match, as byte offsets into `text`.
    pub range: &'a Range<usize>,
    /// The match in the source.
    pub span: Span,
    /// Where `text` sits in its projection, when the text is projected prose. A detector
    /// that reads text before the match needs it; `None` means `text` is all there is.
    pub origin: Option<TextOrigin<'a>>,
    /// The parsed document.
    pub document: &'a Document,
    /// Context tags that cover `span`.
    pub tags: &'a [&'a str],
}

/// A detector for one exception name.
pub trait Detector: Send + Sync {
    /// The exception `detector` name this answers to.
    fn name(&self) -> &str;
    /// Whether the exception holds at `site`, so the finding is dropped.
    fn detects(&self, site: &MatchSite<'_>) -> bool;
}

/// A detector name was registered twice.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{}", crate::messages::EngineText::DuplicateDetector { name: .0.clone() })]
pub struct DuplicateDetector(String);

/// A match wholly inside a quotation on its line. A document that bans a phrase has to
/// print the phrase. The scan is line-scoped and capped at 200 characters, because a quote
/// mark is also an apostrophe, a unit of inches and a shell quote. An unbounded scan would
/// pair marks that were never a quotation and silence half a document. There is no
/// straight single-quote branch for the same reason.
struct Quotation;

// Port of `engine._QUOTED_SPAN`. The `\w` classes are rewritten by `slopvac-text`.
static QUOTED_SPAN: LazyLock<Pattern> = LazyLock::new(|| {
    Pattern::new(
        r#"(?<![\w"”])"[^"\n]{1,200}(?<!\s)"(?!\w)|(?<![\w“])“[^”\n]{1,200}”|(?<![\w‘])‘[^’\n]{1,200}’"#,
        false,
    )
    .expect("the quotation pattern compiles")
});

impl Detector for Quotation {
    fn name(&self) -> &'static str {
        "quotation"
    }

    fn detects(&self, site: &MatchSite<'_>) -> bool {
        // A pattern failure leaves the finding in place: the exception only ever removes.
        QUOTED_SPAN.find_all(site.text).is_ok_and(|quotes| {
            quotes.iter().any(|q| {
                // One byte for each quote mark would be wrong for the curly marks, so the
                // bound is the mark's own width, found from the text.
                let open = site.text[q.start..]
                    .chars()
                    .next()
                    .map_or(1, char::len_utf8);
                let close = site.text[..q.end]
                    .chars()
                    .next_back()
                    .map_or(1, char::len_utf8);
                q.start + open <= site.range.start && site.range.end + close <= q.end
            })
        })
    }
}

/// A match inside an inline code span.
struct CodeSpan;

impl Detector for CodeSpan {
    fn name(&self) -> &'static str {
        "code-span"
    }

    fn detects(&self, site: &MatchSite<'_>) -> bool {
        site.document
            .code_spans
            .iter()
            .any(|c| c.start() <= site.span.start() && site.span.end() <= c.end())
    }
}

/// How many sentences back `negated-antecedent` looks.
const ANTECEDENT_SENTENCES: usize = 2;

/// Words that never name the negated thing: articles, determiners, prepositions, pronouns
/// and intensifiers. Lowercase.
const FUNCTION_WORDS: &[&str] = &[
    "a", "all", "also", "always", "an", "and", "any", "anymore", "are", "as", "at", "be", "by",
    "each", "even", "ever", "every", "for", "from", "here", "his", "her", "in", "into", "is", "it",
    "its", "itself", "just", "more", "most", "much", "never", "of", "on", "one", "only", "onto",
    "other", "our", "over", "same", "some", "still", "such", "that", "the", "their", "them",
    "there", "these", "they", "this", "those", "to", "under", "very", "with", "yet", "your",
];

/// Words that end the negated clause: a conjunction or subordinator opens a new clause, so
/// what follows no longer belongs to the negated phrase.
const CLAUSE_OPENERS: &[&str] = &[
    "and", "because", "but", "if", "or", "so", "than", "that", "unless", "when", "where", "which",
    "while", "who", "whom", "whose",
];

/// Punctuation that ends the negated phrase: it opens a new clause, a list or an aside.
///
/// Quote marks are not here: they wrap a quoted object (`it does not read "YAML"`) and
/// [`words`] drops them.
const CLAUSE_PUNCTUATION: &[char] = &[';', ':', ',', '—', '–', '(', ')', '[', ']', '{', '}'];

/// The words of `text`, lowercase: runs of letters and digits, with inner `-` and `_`.
fn words(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_'))
        .map(|w| w.trim_matches(['-', '_']))
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
}

/// A word reduced to a comparison key: a plural `-s` is dropped (`servers` and `server`
/// agree), and nothing else is stemmed.
fn stem(word: &str) -> &str {
    match word.strip_suffix('s') {
        Some(rest) if rest.len() >= 3 && !rest.ends_with('s') => rest,
        _ => word,
    }
}

/// The negated term of a contrastive-appositive match, as a comparison key.
///
/// The match has the form `X does A; it does not B`. The negated term is the last content
/// word of `B`, the head of its closing noun phrase: `yaml` in `it does not read YAML`,
/// `judge` in `it does not judge`. `B` ends at the first conjunction or subordinator, so
/// `it does not confirm that it started` yields `confirm`. `B` also ends at the first
/// punctuation that opens another clause or an aside ([`CLAUSE_PUNCTUATION`]), so in
/// `it does not accept YAML; it emits XML` the term is `yaml`. A content word is any word
/// that is not in [`FUNCTION_WORDS`]. A match with no `;` or `,` followed by `not` has no
/// negated term.
fn negated_term(matched: &str) -> Option<String> {
    let (_, negation) = matched.split_once([';', ','])?;
    let negation = negation
        .split(CLAUSE_PUNCTUATION)
        .next()
        .unwrap_or(negation);
    let mut tokens = words(negation);
    // The negator is `not`, alone or after a subject and an auxiliary: `it does not`.
    tokens.by_ref().take(3).find(|w| w == "not")?;
    tokens
        .take_while(|w| !CLAUSE_OPENERS.contains(&w.as_str()))
        .filter(|w| !FUNCTION_WORDS.contains(&w.as_str()))
        .last()
        .map(|w| stem(&w).to_owned())
}

/// A contrastive-appositive match whose negated term already appeared in the previous two
/// sentences. In `The server accepts sockets. Pipes are slower. It reads JSON; it does not
/// read sockets.` the reader has met the term, so the negation answers something already
/// said and the contrast is not an invented foil. A term is repeated when a word of those
/// sentences has the same lowercase form, ignoring a plural `-s`.
///
/// The sentences come from the text before the match in its own block, then, when fewer
/// than [`ANTECEDENT_SENTENCES`] are there, from the nearest earlier block that has prose.
/// A sentence three or more back does not count, and neither does a block before that.
struct NegatedAntecedent;

impl NegatedAntecedent {
    /// The last [`ANTECEDENT_SENTENCES`] sentences before the match, nearest last.
    fn preceding(site: &MatchSite<'_>) -> Option<Vec<String>> {
        let blocks = &site.document.blocks;
        // The projection's own offset locates the match, so a repeated sentence, a
        // sentence-scoped text and an opaque token all resolve to the right place.
        let before = match site.origin {
            Some(o) => o.projection.text().get(..o.base + site.range.start)?,
            None => site.text.get(..site.range.start)?,
        };
        let owner = site.origin.and_then(|o| {
            blocks.iter().position(|b| {
                b.projection
                    .as_ref()
                    .is_some_and(|p| std::ptr::eq(p, o.projection))
            })
        });
        let mut sentences = split_sentences(before).ok()?;
        if sentences.len() < ANTECEDENT_SENTENCES
            && let Some(i) = owner
            && let Some(previous) = blocks[..i].iter().rev().find_map(|b| b.projection.as_ref())
        {
            let mut earlier = split_sentences(previous.text()).ok()?;
            earlier.append(&mut sentences);
            sentences = earlier;
        }
        let keep = sentences.len().saturating_sub(ANTECEDENT_SENTENCES);
        Some(
            sentences
                .split_off(keep)
                .into_iter()
                .map(|s| s.text)
                .collect(),
        )
    }
}

impl Detector for NegatedAntecedent {
    fn name(&self) -> &'static str {
        "negated-antecedent"
    }

    fn detects(&self, site: &MatchSite<'_>) -> bool {
        // Any failure leaves the finding in place: the exception only ever removes.
        let Some(term) = site.text.get(site.range.clone()).and_then(negated_term) else {
            return false;
        };
        Self::preceding(site)
            .is_some_and(|sentences| sentences.iter().any(|s| words(s).any(|w| stem(&w) == term)))
    }
}

/// The detectors of a run.
pub struct Detectors {
    by_name: BTreeMap<String, Box<dyn Detector>>,
}

impl Default for Detectors {
    fn default() -> Self {
        Self::builtin()
    }
}

impl Detectors {
    /// The built-in detectors: `quotation`, `code-span` and `negated-antecedent`.
    #[must_use]
    pub fn builtin() -> Self {
        let mut detectors = Self {
            by_name: BTreeMap::new(),
        };
        for detector in [
            Box::new(Quotation) as Box<dyn Detector>,
            Box::new(CodeSpan),
            Box::new(NegatedAntecedent),
        ] {
            detectors
                .register(detector)
                .expect("built-in detector names are distinct");
        }
        detectors
    }

    /// Add a detector.
    pub fn register(&mut self, detector: Box<dyn Detector>) -> Result<(), DuplicateDetector> {
        let name = detector.name().to_owned();
        if self.by_name.contains_key(&name) {
            return Err(DuplicateDetector(name));
        }
        self.by_name.insert(name, detector);
        Ok(())
    }

    /// Registered names, sorted.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.by_name.keys().map(String::as_str)
    }

    /// Whether the exception `detector` holds at `site`. A name that is not registered is
    /// a context tag.
    #[must_use]
    pub fn detects(&self, detector: &str, site: &MatchSite<'_>) -> bool {
        self.by_name
            .get(detector)
            .map_or_else(|| site.tags.contains(&detector), |d| d.detects(site))
    }
}

#[cfg(test)]
mod tests {
    use slopvac_parse::{parse_markdown, parse_mdx};

    use super::*;

    const NEEDLE: &str = "The parser reads JSON; it does not read YAML";

    /// Whether `negated-antecedent` drops the `nth` match (from 0) of `needle` in the prose
    /// of `document`, counting across blocks in order. As the engine does for a sentence
    /// rule, the site's text is the sentence that holds the match, and its origin is the
    /// block's projection.
    fn detects_in(document: &Document, needle: &str, nth: usize) -> bool {
        let mut seen = 0;
        for block in &document.blocks {
            let Some(projection) = &block.projection else {
                continue;
            };
            let text = projection.text();
            let found = text.match_indices(needle).nth(nth - seen);
            let Some((at, _)) = found else {
                seen += text.matches(needle).count();
                continue;
            };
            let sentence = split_sentences(text)
                .unwrap()
                .into_iter()
                .find(|s| s.start <= at && at < s.end)
                .expect("a sentence holds the match");
            let range = at - sentence.start..at - sentence.start + needle.len();
            let site = MatchSite {
                text: &text[sentence.start..sentence.end],
                range: &range,
                span: block.span,
                origin: Some(TextOrigin {
                    projection,
                    base: sentence.start,
                }),
                document,
                tags: &[],
            };
            return Detectors::builtin().detects("negated-antecedent", &site);
        }
        panic!("the document has no match number {nth} of {needle:?}");
    }

    fn detects_nth(source: &str, needle: &str, nth: usize) -> bool {
        let document = parse_markdown(source, Span::new(0, source.len()).unwrap()).unwrap();
        detects_in(&document, needle, nth)
    }

    fn detects(source: &str, needle: &str) -> bool {
        detects_nth(source, needle, 0)
    }

    #[test]
    fn a_term_in_the_previous_sentence_of_the_same_block_is_detected() {
        let doc = format!("The module loads YAML files. {NEEDLE}.\n");
        assert!(detects(&doc, NEEDLE));
    }

    #[test]
    fn a_plural_and_a_different_case_count_as_the_same_term() {
        let doc = "Teams edit yaml by hand. The parser reads JSON; it does not read YAMLs.\n";
        assert!(detects(
            doc,
            "The parser reads JSON; it does not read YAMLs"
        ));
    }

    #[test]
    fn a_term_two_sentences_back_is_detected() {
        let doc = format!("We ship YAML. It is small. {NEEDLE}.\n");
        assert!(detects(&doc, NEEDLE));
    }

    #[test]
    fn a_term_three_sentences_back_is_not_detected() {
        let doc = format!("We ship YAML. It is small. It is fast. {NEEDLE}.\n");
        assert!(!detects(&doc, NEEDLE));
    }

    #[test]
    fn a_term_that_never_appeared_is_not_detected() {
        let doc = format!("The module loads files. {NEEDLE}.\n");
        assert!(!detects(&doc, NEEDLE));
    }

    #[test]
    fn a_term_in_the_preceding_block_is_detected() {
        let doc = format!("The module loads YAML files.\n\n{NEEDLE}.\n");
        assert!(detects(&doc, NEEDLE));
    }

    #[test]
    fn the_sentence_budget_runs_across_the_block_boundary() {
        // Two sentences back: the last two of the previous block reach the term.
        let near = format!("We ship YAML. It is small.\n\n{NEEDLE}.\n");
        assert!(detects(&near, NEEDLE));
        // Three sentences back: the first sentence of the previous block is out of range.
        let far = format!("We ship YAML. It is small. It is fast.\n\n{NEEDLE}.\n");
        assert!(!detects(&far, NEEDLE));
    }

    #[test]
    fn a_block_two_back_is_not_read() {
        let doc = format!("We ship YAML.\n\nThe module loads files.\n\n{NEEDLE}.\n");
        assert!(!detects(&doc, NEEDLE));
    }

    #[test]
    fn a_match_without_a_negated_clause_is_not_detected() {
        let doc = "The module loads YAML files. The parser reads JSON.\n";
        assert!(!detects(doc, "The parser reads JSON"));
    }

    #[test]
    fn a_repeated_contrast_checks_the_sentences_before_its_own_occurrence() {
        // The first copy follows "We ship YAML."; the second follows two sentences that
        // never name it.
        let doc = format!("We ship YAML. {NEEDLE}. It is fast. It is big. {NEEDLE}.\n");
        assert!(detects_nth(&doc, NEEDLE, 0));
        assert!(!detects_nth(&doc, NEEDLE, 1));
    }

    #[test]
    fn the_term_is_the_head_of_the_closing_phrase_of_the_negated_clause() {
        let needle = "The client speaks HTTP, not the old FTP protocol";
        // The head noun is `protocol`, not the modifier `FTP`.
        assert!(detects(
            &format!("The protocol is old. {needle}.\n"),
            needle
        ));
        assert!(!detects(&format!("We study FTP. {needle}.\n"), needle));
    }

    #[test]
    fn the_negated_clause_ends_at_a_conjunction_or_subordinator() {
        let needle = "It acknowledges the run; it does not confirm that it started";
        assert!(detects(&format!("We confirm runs. {needle}.\n"), needle));
        assert!(!detects(&format!("It started late. {needle}.\n"), needle));
    }

    #[test]
    fn a_later_clause_after_a_semicolon_does_not_supply_the_term() {
        let needle = "The loader accepts CSV; it does not accept YAML; it emits XML";
        // `XML` follows the negated clause, so an earlier `XML` is not the antecedent.
        assert!(!detects(&format!("We use XML. {needle}.\n"), needle));
        assert!(detects(&format!("We use YAML. {needle}.\n"), needle));
    }

    #[test]
    fn an_aside_after_a_dash_or_parenthesis_does_not_supply_the_term() {
        for needle in [
            "The loader accepts CSV; it does not accept YAML — it emits XML",
            "The loader accepts CSV; it does not accept YAML (it emits XML)",
        ] {
            assert!(!detects(&format!("We use XML. {needle}.\n"), needle));
            assert!(detects(&format!("We use YAML. {needle}.\n"), needle));
        }
    }

    #[test]
    fn a_plural_negated_term_matches_its_singular() {
        let needle = "The hosts list servers; it is not about servers";
        assert!(detects(&format!("One server runs. {needle}.\n"), needle));
    }

    #[test]
    fn a_quoted_negated_object_is_the_term() {
        for needle in [
            "The parser reads JSON; it does not read \"YAML\"",
            "The parser reads JSON; it does not read \u{201c}YAML\u{201d}",
        ] {
            assert!(detects(&format!("We ship YAML. {needle}.\n"), needle));
            assert!(!detects(&format!("We read files. {needle}.\n"), needle));
        }
    }

    #[test]
    fn sentences_before_the_match_in_an_mdx_string_literal_are_read() {
        // The literal projects to one opaque token, so a source span cannot place the match.
        let source = format!("{{'We ship YAML. It is small. {NEEDLE}.'}}\n");
        let document = parse_mdx(&source, Span::new(0, source.len()).unwrap()).unwrap();
        assert!(detects_in(&document, NEEDLE, 0));
        let far = format!("{{'We ship YAML. It is small. It is fast. {NEEDLE}.'}}\n");
        let document = parse_mdx(&far, Span::new(0, far.len()).unwrap()).unwrap();
        assert!(!detects_in(&document, NEEDLE, 0));
    }
}
