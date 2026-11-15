//! Vocabulary rules: the project's word blocklist, keyed by part of speech.
//!
//! 2.x compiled the blocklist into one Vale `sequence` rule per part of speech. Each rule
//! flags a token whose text is a listed word (whole token, any case) and whose Penn tag
//! matches an unanchored tag pattern. This module does the same on `slopvac-pos`, so
//! `close` as a verb is flagged and "close to the limit" is not.
//!
//! Every `kind: vocabulary` rule would run the same sweep over the same blocklist, and a
//! `(word, pos)` blocklist answers one question. One rule owns the sweep and reports under
//! its id, as in 2.x. The others stay quiet and a notice says so.

use std::collections::HashMap;
use std::ops::Range;

use slopvac_model::{Finding, Notice, NoticeLevel};
use slopvac_parse::{Document as Parsed, parse_html};
use slopvac_pos::Tagger;
use slopvac_rules::{LoadedRule, RuleSet, Scope};
use slopvac_terms::{Blocklist, Pos};

use super::{Active, Candidate, Engine, EngineError, Kind, Run, notice};
use crate::matching::format_message;
use crate::prepare::Prepared;
use crate::skip;

/// The rules that may own the sweep, most preferred first. STE 1.1 is the clause that a
/// blocklist answers.
const OWNER_PREFERENCE: [&str; 2] = [
    "ste-words.word-outside-controlled-vocabulary",
    "ste-words.word-used-in-wrong-part-of-speech",
];

/// Penn tags per part of speech, as fragments. A tag matches when it contains one, as 2.x's
/// unanchored Vale `tag` pattern does: `RB|RBR|RBS` also matches `WRB`.
///
/// Parts of speech that are not open-class (prepositions, conjunctions, pronouns and
/// articles) have no rule in 2.x; a blocklist may hold them and they never match.
const fn penn(pos: Pos) -> Option<&'static [&'static str]> {
    match pos {
        Pos::Noun => Some(&["NN", "NNS", "NNP", "NNPS"]),
        Pos::Verb => Some(&["VB", "VBD", "VBG", "VBN", "VBP", "VBZ"]),
        Pos::Adjective => Some(&["JJ", "JJR", "JJS"]),
        Pos::Adverb => Some(&["RB", "RBR", "RBS"]),
        Pos::Preposition | Pos::Conjunction | Pos::Pronoun | Pos::Article => None,
    }
}

/// The blocklist as the sweep reads it, and what to say about the vocabulary rules.
#[derive(Debug, Default)]
pub(super) struct Vocabulary {
    /// Listed words, lower case, with the parts of speech they are refused as.
    words: HashMap<String, Vec<Pos>>,
    /// The vocabulary rule that owns the sweep, as an index into `vocabulary_rules`.
    owner: Option<usize>,
    notices: Vec<Notice>,
}

impl Vocabulary {
    pub(super) fn notices(&self) -> &[Notice] {
        &self.notices
    }
}

impl Engine {
    /// Elect the owner of the sweep and index `blocklist`, replacing any earlier plan.
    pub(super) fn plan_vocabulary(&mut self, blocklist: &Blocklist) -> Result<(), EngineError> {
        let mut plan = Vocabulary::default();
        let rules = &self.vocabulary_rules;
        if rules.is_empty() {
            self.vocabulary = plan;
            return Ok(());
        }
        // A word that is not alphabetic, or has fewer than three letters, cannot be tagged
        // reliably. 2.x skipped it, and so do we.
        for entry in blocklist.iter() {
            let taggable = entry.word.chars().all(char::is_alphabetic)
                && entry.word.chars().count() >= 3
                && penn(entry.pos).is_some();
            if taggable {
                plan.words
                    .entry(entry.word.clone())
                    .or_default()
                    .push(entry.pos);
            }
        }
        let count = rules.len();
        if blocklist.is_empty() {
            plan.notices.push(notice(
                "vocabulary-no-blocklist",
                NoticeLevel::Info,
                format!("{count} active vocabulary rule(s) did not run: no word blocklist is set"),
            )?);
        } else if plan.words.is_empty() {
            plan.notices.push(notice(
                "vocabulary-no-taggable-entries",
                NoticeLevel::Info,
                format!(
                    "{count} active vocabulary rule(s) did not run: the blocklist holds no \
                     noun, verb, adjective or adverb entry of three or more letters"
                ),
            )?);
        } else {
            let id_of = |i: usize| {
                let active = &rules[i];
                self.sets[active.set].categories()[active.category].rules()[active.rule]
                    .id()
                    .local()
            };
            let owner = OWNER_PREFERENCE
                .iter()
                .find_map(|wanted| (0..count).find(|&i| id_of(i) == *wanted))
                .unwrap_or(0);
            if count > 1 {
                plan.notices.push(notice(
                    "vocabulary-sweep-shared",
                    NoticeLevel::Info,
                    format!(
                        "{} other vocabulary rule(s) did not run separately: the blocklist \
                         sweep runs once, under {}",
                        count - 1,
                        id_of(owner)
                    ),
                )?);
            }
            plan.owner = Some(owner);
        }
        self.vocabulary = plan;
        Ok(())
    }
}

/// The text the tagger reads for projected prose, and how to map its offsets back.
struct TaggerText {
    text: String,
    /// Each stand-in for a code span as `(start in text, bytes it adds over the one NUL)`,
    /// ascending.
    stand_ins: Vec<(usize, usize)>,
}

impl TaggerText {
    /// The projection masks each inline code span as one NUL. Replace it with a code span
    /// the tagger drops, as it drops any code, so a hyphen or a sentence break next to code
    /// behaves as it does in the source. `code_width(at)` is the number of characters
    /// between the backticks of the span masked at byte `at`: the tagger puts a run of that
    /// many `*` in the span's place, as Vale does, so the tags around it see the same word.
    fn new(prose: &str, code_width: impl Fn(usize) -> usize) -> Self {
        let mut text = String::with_capacity(prose.len());
        let mut stand_ins = Vec::new();
        for (at, c) in prose.char_indices() {
            if c == '\0' {
                let width = code_width(at).max(1);
                stand_ins.push((text.len(), width + 1));
                text.push('`');
                text.extend(std::iter::repeat_n('x', width));
                text.push('`');
            } else if c == '`' {
                // A literal backtick (the parser decoded an escape) is prose. The tagger
                // would pair it with a stand-in and drop the words between them. The
                // replacement has the same width, so offsets still line up.
                text.push('\'');
            } else {
                text.push(c);
            }
        }
        Self { text, stand_ins }
    }

    /// The offset in the projected prose of an offset in the tagger text. A token never
    /// starts or ends inside a stand-in, so each one before `at` is exactly one NUL.
    fn to_prose(&self, at: usize) -> usize {
        let before = self
            .stand_ins
            .partition_point(|&(start, added)| start + added < at);
        at - self.stand_ins[..before]
            .iter()
            .map(|&(_, added)| added)
            .sum::<usize>()
    }
}

/// What one sweep needs from the owner rule.
struct Sweep<'a> {
    active: &'a Active,
    set: &'a RuleSet,
    loaded: &'a LoadedRule,
    exceptions: Vec<&'a str>,
    detectors: Vec<&'a str>,
}

impl Run<'_> {
    /// Findings of the vocabulary sweep: tokens that are a listed word in a refused part of
    /// speech. The sweep reads the Markdown prose and the prose of raw HTML blocks, which
    /// Vale lints too (`<td>`, `<em>`).
    pub(super) fn vocabulary(&self) -> Result<Vec<Finding>, EngineError> {
        let engine = self.engine;
        let Some(owner) = engine.vocabulary.owner else {
            return Ok(Vec::new());
        };
        if matches!(self.input.kind, Kind::Diagram(_)) {
            return Ok(Vec::new());
        }
        let active = &engine.vocabulary_rules[owner];
        let set = &engine.sets[active.set];
        let loaded = &set.categories()[active.category].rules()[active.rule];
        let rule = loaded.rule();
        let sweep = Sweep {
            active,
            set,
            loaded,
            exceptions: rule.exceptions().iter().map(|e| e.name.as_str()).collect(),
            detectors: rule
                .exceptions()
                .iter()
                .filter_map(|e| e.detector.as_ref().map(slopvac_rules::Slug::as_str))
                .collect(),
        };
        let mut out = Vec::new();
        self.sweep_document(&sweep, self.parsed, self.prepared, &mut out)?;
        for block in &self.parsed.html_blocks {
            let html = parse_html(self.input.source, *block)?;
            let prepared = Prepared::new(self.input.source, &html)?;
            self.sweep_document(&sweep, &html, &prepared, &mut out)?;
        }
        Ok(out)
    }

    fn sweep_document(
        &self,
        sweep: &Sweep<'_>,
        doc: &Parsed,
        prepared: &Prepared<'_>,
        out: &mut Vec<Finding>,
    ) -> Result<(), EngineError> {
        let engine = self.engine;
        let rule = sweep.loaded.rule();
        let skip = skip::build(
            doc,
            rule.exclude_blocks.as_deref().unwrap_or(&[]),
            self.front_matter,
            true,
        );
        let tagger = Tagger::english();
        for unit in prepared.units(Scope::Prose)? {
            let prose = TaggerText::new(unit.text, |at| {
                let span = unit.span(&(at..at + 1));
                self.input.source[span.start()..span.end()]
                    .trim_matches('`')
                    .chars()
                    .count()
            });
            for token in tagger.tag_text(&prose.text) {
                let Some(pos) = refused(&engine.vocabulary.words, token.text, token.tag) else {
                    continue;
                };
                let range: Range<usize> =
                    prose.to_prose(token.span.start)..prose.to_prose(token.span.end);
                let span = unit.span(&range);
                if skip.hides(span)
                    || !self.passes_context(
                        rule.contexts.as_ref(),
                        &sweep.detectors,
                        unit.text,
                        &range,
                        span,
                    )
                    || self.suppressed(sweep.loaded.id().as_str(), &sweep.exceptions, span)
                {
                    continue;
                }
                let matched = &self.input.source[span.start()..span.end()];
                // The preferred owner's message is written for a blocklist. The other
                // vocabulary rules word clauses a blocklist cannot answer (allowed form,
                // lemma), so when one of them owns the sweep it says what is known.
                let message = if sweep.loaded.id().local() == OWNER_PREFERENCE[0] {
                    format_message(rule.message.as_str(), matched, "")
                } else {
                    format!(
                        "\"{matched}\" is on this project's word blocklist as {} {}.",
                        article(pos),
                        pos_name(pos)
                    )
                };
                out.push(self.finding(Candidate {
                    rule_id: sweep.loaded.id(),
                    pack: sweep.set.pack(),
                    category: sweep.loaded.category().clone(),
                    severity: sweep.active.severity,
                    message,
                    span,
                    matched_text: matched.to_owned(),
                    ai_signal: rule.ai_signal,
                    fix: None,
                    extra_context: None,
                })?);
            }
        }
        Ok(())
    }
}

const fn pos_name(pos: Pos) -> &'static str {
    match pos {
        Pos::Noun => "noun",
        Pos::Verb => "verb",
        Pos::Adjective => "adjective",
        Pos::Adverb => "adverb",
        Pos::Preposition => "preposition",
        Pos::Conjunction => "conjunction",
        Pos::Pronoun => "pronoun",
        Pos::Article => "article",
    }
}

const fn article(pos: Pos) -> &'static str {
    match pos {
        Pos::Adjective | Pos::Adverb | Pos::Article => "an",
        _ => "a",
    }
}

/// The part of speech `word`, tagged `tag`, is refused as, if any.
fn refused(words: &HashMap<String, Vec<Pos>>, word: &str, tag: &str) -> Option<Pos> {
    words
        .get(&word.to_lowercase())?
        .iter()
        .copied()
        .find(|&p| penn(p).is_some_and(|fragments| fragments.iter().any(|f| tag.contains(f))))
}
