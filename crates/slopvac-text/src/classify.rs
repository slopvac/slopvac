//! Text-type classification: procedural, descriptive or safety.

use std::sync::LazyLock;

use pcre2::bytes::Regex;
use serde::{Deserialize, Serialize};

use crate::lexicon::{
    EXTRA_FINITE_STEMS, FINITE_VERB_FORMS, IMPERATIVE_VERBS, NON_NOUN_SECOND_TOKENS, is_imperative,
};
use crate::pattern::{TextError, matches_start, re, re_i};
use crate::pyrules::{casefold, trim, upper};
use crate::tokens::{stepless_tokens, strip_step_number};

/// The procedural and descriptive split that selects the sentence word cap.
///
/// The spec gives no mechanical test, so the detector uses imperative mood plus
/// note and warning markers. `Any` means a rule does not care.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TextType {
    /// A rule applies to any text type.
    #[default]
    Any,
    /// An instruction.
    Procedural,
    /// Descriptive text.
    Descriptive,
    /// A safety label.
    Safety,
}

const WORD_END: &str = r"(?![\p{L}\p{Nd}_])";

struct Patterns {
    imperative: Regex,
    phrasal: Regex,
    negative: Regex,
    to_verb: Regex,
    remember_to: Regex,
    safety_marker: Regex,
    note_marker: Regex,
    you_should: Regex,
}

static PATTERNS: LazyLock<Patterns> = LazyLock::new(|| {
    let mut verbs: Vec<&str> = IMPERATIVE_VERBS.to_vec();
    verbs.sort_by(|a, b| b.len().cmp(&a.len()).then(a.cmp(b)));
    let alt = verbs.join("|");
    Patterns {
        imperative: re_i(&format!(
            r"^(?:please\s+)?(?:do\s+not\s+|do\s+)?(?:{alt}){WORD_END}"
        )),
        phrasal: re_i(&format!(
            concat!(
                r"^(?:please\s+)?(?:(?:{alt})\s+(?:up|down|off|on|out|in|over|back|away)|",
                r"back\s+up|shut\s+down|power\s+off|turn\s+(?:off|on)|",
                r"switch\s+(?:off|on)|log\s+(?:in|out)|sign\s+(?:in|out)|",
                r"set\s+up|spin\s+up|roll\s+back|scale\s+(?:down|up)){we}"
            ),
            alt = alt,
            we = WORD_END
        )),
        negative: re_i(&format!(
            r"^(?:please\s+)?(?:don't|never|do\s+not)\s+[A-Za-z]+{WORD_END}"
        )),
        to_verb: re_i(&format!(r"^to\s+(?:{alt}){WORD_END}")),
        remember_to: re_i(&format!(
            r"^(?:remember|make\s+sure)\s+to\s+(?:{alt}){WORD_END}"
        )),
        safety_marker: re_i(&format!(
            concat!(
                r"^\s*(?:>\s*)?(?:\*{{0,2}})?(?P<marker>WARNING|CAUTION|DANGER|NOTICE|ATTENTION|IMPORTANT){we}",
                r"(?:\*{{0,2}})?\s*[:.!]?"
            ),
            we = WORD_END
        )),
        note_marker: re_i(&format!(
            concat!(
                r"^\s*(?:>\s*)?(?:\*{{0,2}})?(?:NOTE(?:\*{{0,2}})?\s*:\s*|(?:TIP|HINT|INFO){we}",
                r"(?:\*{{0,2}})?\s*[:.!]?)"
            ),
            we = WORD_END
        )),
        you_should: re_i(r"^you\s+(?:should|must|need\s+to)\b"),
    }
});

static NUMERIC: LazyLock<Regex> = LazyLock::new(|| re(r"\A\d+(?:[.,]\d+)*\z"));

fn is_finite_verb(token: &str) -> bool {
    let word = casefold(token);
    if FINITE_VERB_FORMS.contains(&word.as_str()) {
        return true;
    }
    word.chars().count() > 1
        && word
            .strip_suffix('s')
            .is_some_and(|stem| EXTRA_FINITE_STEMS.contains(&stem) || is_imperative(stem))
}

fn imperative_start(p: &Patterns, text: &str) -> Result<bool, TextError> {
    Ok(matches_start(&p.phrasal, text)? || plain_imperative_start(p, text)?)
}

fn plain_imperative_start(p: &Patterns, text: &str) -> Result<bool, TextError> {
    Ok(matches_start(&p.imperative, text)?
        || matches_start(&p.negative, text)?
        || matches_start(&p.to_verb, text)?
        || matches_start(&p.remember_to, text)?
        || matches_start(&p.you_should, text)?)
}

/// Select the sentence cap, conservatively distinguishing instructions.
///
/// Safety labels keep `Safety` for descriptive warnings, but an instruction after the
/// label is still procedural. Notes stay descriptive. The detector is lexical: a
/// closed verb list with no parser, so rare imperatives and noun-verb homographs
/// outside the list may be classified wrongly either way.
pub fn classify_text_type(text: &str) -> Result<TextType, TextError> {
    let p = &*PATTERNS;
    let stripped = trim(text);
    if let Some(caps) = p.safety_marker.captures(stripped.as_bytes())? {
        let whole = caps.get(0).expect("group 0 exists");
        let marker = caps.name("marker").map_or(String::new(), |m| {
            upper(&String::from_utf8_lossy(m.as_bytes()))
        });
        let body =
            stripped[whole.end()..].trim_start_matches([' ', ':', '.', '!', '?', '*', '-', '+']);
        if imperative_start(p, body)? {
            return Ok(TextType::Procedural);
        }
        return Ok(if marker == "IMPORTANT" {
            TextType::Descriptive
        } else {
            TextType::Safety
        });
    }
    if matches_start(&p.note_marker, stripped)? {
        return Ok(TextType::Descriptive);
    }
    let body = strip_step_number(stripped)?.trim_start_matches([' ', '-', '*', '+']);
    let stepless = strip_step_number(body)?;
    let tokens = stepless_tokens(stepless)?;
    let verb_first = tokens.first().is_some_and(|t| is_imperative(&casefold(t)));
    if verb_first && tokens.len() >= 2 && is_finite_verb(&tokens[1]) {
        return Ok(TextType::Descriptive);
    }
    if verb_first
        && tokens.len() >= 3
        && !NON_NOUN_SECOND_TOKENS.contains(&casefold(&tokens[1]).as_str())
        && !matches_start(&NUMERIC, &tokens[1])?
        && is_finite_verb(&tokens[2])
    {
        return Ok(TextType::Descriptive);
    }
    if plain_imperative_start(p, body)? {
        return Ok(TextType::Procedural);
    }
    Ok(TextType::Descriptive)
}
