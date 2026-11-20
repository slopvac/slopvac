//! Changed-claim candidates for review: what a revision added, changed or dropped.
//!
//! Lexical only (decision: lexical TF-IDF beat model2vec in the A/B). The steps are
//! deterministic: segment both texts into blocks (Markdown here) and sentences (`slopvac-text`), align original to revised by
//! TF-IDF cosine similarity, then diff the literals of each aligned pair (numbers,
//! code spans, flags, identifiers, names) and their negation. Local files only.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt::Write as _;
use std::path::PathBuf;

use clap::{Arg, ArgMatches, Command};
use serde::Serialize;

use crate::error::AgentsError;
use crate::messages::{AgentsPhrase, AgentsText};

/// Pairs below this similarity are not aligned.
const MIN_ALIGN: f64 = 0.35;
/// Token coverage below this makes an unaligned original sentence a dropped-fact candidate.
const COVERED: f64 = 0.6;

/// What a [`Segment`] holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Prose,
    Code,
}

#[derive(Debug, Clone)]
struct Segment {
    text: String,
    line: usize,
    kind: Kind,
}

/// A literal that a reviewer must see survive.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Literal {
    /// `number`, `code`, `flag`, `identifier` or `name`.
    pub kind: &'static str,
    /// The literal text.
    pub text: String,
}

/// A changed or added sentence of the revision.
#[derive(Debug, Clone, Serialize)]
pub struct Claim {
    /// 1-based line of the sentence in the revision.
    pub line: usize,
    /// The revised sentence.
    pub text: String,
    /// `changed` or `added`.
    pub change: &'static str,
    /// The aligned original sentence, for `changed`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original: Option<String>,
    /// Line of the original sentence.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_line: Option<usize>,
    /// TF-IDF cosine similarity of the pair, 0 for `added`.
    pub similarity: f64,
    /// Literals only the revision has.
    pub literals_added: Vec<Literal>,
    /// Literals only the original has.
    pub literals_removed: Vec<Literal>,
    /// Negation words in the original sentence, 0 for `added`.
    pub negations_original: usize,
    /// Negation words in the revised sentence.
    pub negations_revised: usize,
    /// The two counts differ.
    pub negation_changed: bool,
}

/// An original sentence with no aligned revised sentence.
#[derive(Debug, Clone, Serialize)]
pub struct Dropped {
    /// 1-based line in the original.
    pub line: usize,
    /// The original sentence.
    pub text: String,
    /// Literals that appear nowhere in the revision.
    pub lost_literals: Vec<Literal>,
    /// Share of the sentence's content words found anywhere in the revision.
    pub token_coverage: f64,
    /// True when the sentence looks lost rather than merged.
    pub fact_candidate: bool,
}

/// Counts for a comparison.
#[derive(Debug, Clone, Serialize, Default, PartialEq, Eq)]
pub struct Summary {
    /// Sentences in the original.
    pub original_sentences: usize,
    /// Sentences in the revision.
    pub revised_sentences: usize,
    /// Aligned pairs with equal text.
    pub unchanged: usize,
    /// Aligned pairs with different text.
    pub changed: usize,
    /// Revised sentences with no original.
    pub added: usize,
    /// Original sentences with no revision.
    pub dropped: usize,
}

/// The claims JSON for a reviewer.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    /// Report format version.
    pub schema_version: u8,
    /// Counts.
    pub summary: Summary,
    /// Changed and added sentences, in revision order.
    pub claims: Vec<Claim>,
    /// Unaligned original sentences, in original order.
    pub dropped: Vec<Dropped>,
}

fn fence(line: &str) -> Option<&str> {
    let t = line.trim_start();
    ["```", "~~~"].into_iter().find(|f| t.starts_with(f))
}

fn starts_block(line: &str) -> bool {
    let t = line.trim_start();
    let ordered = t
        .split_once(". ")
        .is_some_and(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
    t.starts_with('#')
        || t.starts_with("- ")
        || t.starts_with("* ")
        || t.starts_with("+ ")
        || t.starts_with('|')
        || t.starts_with('>')
        || ordered
}

fn segments(text: &str) -> Result<Vec<Segment>, AgentsError> {
    let mut out = Vec::new();
    let mut lines = text.lines().enumerate().peekable();
    if lines.peek().is_some_and(|(_, l)| l.trim() == "---") {
        lines.next();
        for (_, l) in lines.by_ref() {
            if l.trim() == "---" {
                break;
            }
        }
    }
    let mut para: Vec<String> = Vec::new();
    let mut para_line = 0;
    let flush =
        |para: &mut Vec<String>, line: usize, out: &mut Vec<Segment>| -> Result<(), AgentsError> {
            if !para.is_empty() {
                for s in sentences(&para.join(" "))? {
                    out.push(Segment {
                        text: s,
                        line,
                        kind: Kind::Prose,
                    });
                }
                para.clear();
            }
            Ok(())
        };
    while let Some((n, line)) = lines.next() {
        if let Some(marker) = fence(line) {
            flush(&mut para, para_line, &mut out)?;
            let mut body = vec![line.to_owned()];
            for (_, l) in lines.by_ref() {
                body.push(l.to_owned());
                if l.trim_start().starts_with(marker) {
                    break;
                }
            }
            out.push(Segment {
                text: body.join("\n"),
                line: n + 1,
                kind: Kind::Code,
            });
        } else if line.trim().is_empty() {
            flush(&mut para, para_line, &mut out)?;
        } else {
            if starts_block(line) {
                flush(&mut para, para_line, &mut out)?;
            }
            if para.is_empty() {
                para_line = n + 1;
            }
            para.push(line.trim().to_owned());
        }
    }
    flush(&mut para, para_line, &mut out)?;
    Ok(out)
}

const STOPWORDS: [&str; 34] = [
    "a", "an", "the", "and", "or", "of", "to", "in", "on", "for", "is", "are", "was", "were", "be",
    "it", "its", "this", "that", "with", "as", "by", "at", "from", "can", "will", "you", "your",
    "if", "then", "so", "do", "does", "has",
];

fn normalize(t: &str) -> String {
    let t = t.to_lowercase();
    if t.len() > 3 && t.ends_with('s') && !t.ends_with("ss") {
        t[..t.len() - 1].to_owned()
    } else {
        t
    }
}

/// Sentences of one Markdown paragraph.
fn sentences(paragraph: &str) -> Result<Vec<String>, AgentsError> {
    Ok(slopvac_text::split_sentences(paragraph)?
        .into_iter()
        .map(|s| s.text)
        .collect())
}

/// Content tokens for alignment: the engine's word tokens, lowercased, without
/// stopwords, with a plural `s` removed.
///
/// The tokenizer collapses names, numbers, identifiers and code spans into placeholders
/// that depend on the sentence around them ("Alice Cooper" collapses mid-sentence but not
/// at the start). A placeholder keeps its lexical identity here: every plain word of
/// the text, beyond the occurrences the ordinary tokens already carry, is added back, so one fact reads the
/// same in both sentences.
fn tokens(text: &str) -> Result<Vec<String>, AgentsError> {
    let raw = slopvac_text::tokens(text)?;
    let collapsed = raw.iter().any(|t| !t.chars().any(char::is_alphanumeric));
    let mut out: Vec<String> = raw
        .iter()
        .filter(|t| t.chars().any(char::is_alphanumeric))
        .map(|t| t.to_lowercase())
        .filter(|t| !STOPWORDS.contains(&t.as_str()))
        .map(|t| normalize(&t))
        .collect();
    if collapsed {
        // Occurrence counts of the words the ordinary tokens already carry.
        let mut carried: HashMap<String, usize> = HashMap::new();
        for part in raw.iter().flat_map(|t| slopvac_text::joined_words(t)) {
            *carried.entry(normalize(&part)).or_insert(0) += 1;
        }
        for word in slopvac_text::joined_words(text) {
            let word = word.to_lowercase();
            if STOPWORDS.contains(&word.as_str()) {
                continue;
            }
            let word = normalize(&word);
            match carried.get_mut(&word) {
                Some(n) if *n > 0 => *n -= 1,
                _ => out.push(word),
            }
        }
    }
    Ok(out)
}

type Vector = HashMap<String, f64>;

fn vectorize(text: &str, idf: &HashMap<String, f64>) -> Result<Vector, AgentsError> {
    let mut v = Vector::new();
    for t in tokens(text)? {
        *v.entry(t).or_insert(0.0) += 1.0;
    }
    for (t, w) in &mut v {
        *w *= idf.get(t).copied().unwrap_or(1.0);
    }
    Ok(v)
}

fn cosine(a: &Vector, b: &Vector) -> f64 {
    let dot: f64 = a.iter().filter_map(|(t, w)| b.get(t).map(|x| w * x)).sum();
    let norm = |v: &Vector| v.values().map(|w| w * w).sum::<f64>().sqrt();
    let denom = norm(a) * norm(b);
    if denom == 0.0 { 0.0 } else { dot / denom }
}

fn trim_token(t: &str) -> &str {
    t.trim_matches(|c: char| ",;:!?()[]\"'“”‘’".contains(c))
        .trim_end_matches('.')
}

fn literals(text: &str) -> BTreeSet<Literal> {
    let mut found = BTreeSet::new();
    let mut add = |kind, text: &str| {
        if !text.is_empty() {
            found.insert(Literal {
                kind,
                text: text.to_owned(),
            });
        }
    };
    for (i, part) in text.split('`').enumerate() {
        if i % 2 == 1 {
            add("code", part);
        }
    }
    let prose: String = text
        .split('`')
        .enumerate()
        .map(|(i, p)| if i % 2 == 0 { p } else { " " })
        .collect::<Vec<_>>()
        .join(" ");
    for (i, raw) in prose.split_whitespace().enumerate() {
        let t = trim_token(raw);
        let Some(first) = t.chars().next() else {
            continue;
        };
        if t.starts_with("--") && t.len() > 2 {
            add("flag", t);
        } else if first.is_ascii_digit() || signed_number(t) {
            let t = t.replacen('−', "-", 1);
            add(
                "number",
                t.trim_end_matches(['%', '-']).trim_end_matches(','),
            );
            if t.ends_with('%') {
                add("number", &t);
            }
        } else if t.contains('_')
            || t.contains("::")
            || t.contains('/')
            || t.ends_with("()")
            || t.chars()
                .zip(t.chars().skip(1))
                .any(|(a, b)| a.is_lowercase() && b.is_uppercase())
            || (t.contains('.')
                && t.split('.')
                    .all(|p| !p.is_empty() && p.chars().all(|c| c.is_alphanumeric() || c == '-')))
        {
            add("identifier", t);
        } else if i > 0 && first.is_uppercase() && t.chars().any(char::is_alphabetic) && t.len() > 1
        {
            add("name", t);
        }
    }
    found
}

/// A number with a sign: `-5`, `+3` or `−2` (U+2212).
fn signed_number(t: &str) -> bool {
    let mut chars = t.chars();
    matches!(chars.next(), Some('-' | '+' | '−'))
        && chars.next().is_some_and(|c| c.is_ascii_digit())
}

fn negation_count(text: &str) -> usize {
    const WORDS: [&str; 8] = [
        "not", "no", "never", "cannot", "without", "none", "neither", "nor",
    ];
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '\'' && c != '’')
        .filter(|w| WORDS.contains(w) || w.ends_with("n't") || w.ends_with("n’t"))
        .count()
}

fn round(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

fn idf_table(old: &[Segment], new: &[Segment]) -> Result<HashMap<String, f64>, AgentsError> {
    let mut df: HashMap<String, usize> = HashMap::new();
    for s in old.iter().chain(new) {
        for t in tokens(&s.text)?.into_iter().collect::<HashSet<_>>() {
            *df.entry(t).or_insert(0) += 1;
        }
    }
    let n = old.len() + new.len();
    Ok(df
        .into_iter()
        .map(|(t, c)| (t, ((ratio(n + 1, c + 1)).ln()) + 1.0))
        .collect())
}

/// Compare `original` and `revised` and list the claims a reviewer must check.
///
/// # Errors
///
/// Returns `claims-text-failed` when `slopvac-text` fails to segment or tokenize, for example
/// when a pattern exhausts PCRE2's match limit.
pub fn compare(original: &str, revised: &str) -> Result<Report, AgentsError> {
    let old = segments(original)?;
    let new = segments(revised)?;
    let idf = idf_table(&old, &new)?;
    let old_v = old
        .iter()
        .map(|s| vectorize(&s.text, &idf))
        .collect::<Result<Vec<Vector>, _>>()?;
    let new_v = new
        .iter()
        .map(|s| vectorize(&s.text, &idf))
        .collect::<Result<Vec<Vector>, _>>()?;

    let mut pairs: Vec<(f64, usize, usize)> = Vec::new();
    for (i, o) in old.iter().enumerate() {
        for (j, r) in new.iter().enumerate() {
            let sim = if o.text == r.text && o.kind == r.kind {
                2.0
            } else {
                cosine(&old_v[i], &new_v[j])
            };
            if sim >= MIN_ALIGN && o.kind == r.kind {
                pairs.push((sim, i, j));
            }
        }
    }
    pairs.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    let mut old_to_new: HashMap<usize, (usize, f64)> = HashMap::new();
    let mut new_to_old: HashMap<usize, usize> = HashMap::new();
    for (sim, i, j) in pairs {
        if !old_to_new.contains_key(&i) && !new_to_old.contains_key(&j) {
            old_to_new.insert(i, (j, sim.min(1.0)));
            new_to_old.insert(j, i);
        }
    }

    let mut summary = Summary {
        original_sentences: old.len(),
        revised_sentences: new.len(),
        ..Summary::default()
    };
    let mut claims = Vec::new();
    for (j, seg) in new.iter().enumerate() {
        let Some(&i) = new_to_old.get(&j) else {
            summary.added += 1;
            claims.push(Claim {
                line: seg.line,
                text: seg.text.clone(),
                change: "added",
                original: None,
                original_line: None,
                similarity: 0.0,
                literals_added: literals(&seg.text).into_iter().collect(),
                literals_removed: Vec::new(),
                negations_original: 0,
                negations_revised: negation_count(&seg.text),
                negation_changed: false,
            });
            continue;
        };
        if old[i].text == seg.text {
            summary.unchanged += 1;
            continue;
        }
        summary.changed += 1;
        let (before, after) = (literals(&old[i].text), literals(&seg.text));
        claims.push(Claim {
            line: seg.line,
            text: seg.text.clone(),
            change: "changed",
            original: Some(old[i].text.clone()),
            original_line: Some(old[i].line),
            similarity: round(old_to_new[&i].1),
            literals_added: after.difference(&before).cloned().collect(),
            literals_removed: before.difference(&after).cloned().collect(),
            negations_original: negation_count(&old[i].text),
            negations_revised: negation_count(&seg.text),
            negation_changed: negation_count(&old[i].text) != negation_count(&seg.text),
        });
    }

    let dropped = dropped_facts(&old, &old_to_new, revised)?;
    summary.dropped = dropped.len();
    Ok(Report {
        schema_version: 1,
        summary,
        claims,
        dropped,
    })
}

#[allow(clippy::cast_precision_loss, reason = "sentence counts are tiny")]
fn ratio(part: usize, whole: usize) -> f64 {
    part as f64 / whole as f64
}

fn dropped_facts(
    old: &[Segment],
    aligned: &HashMap<usize, (usize, f64)>,
    revised: &str,
) -> Result<Vec<Dropped>, AgentsError> {
    let revised_lower = revised.to_lowercase();
    let revised_tokens: HashSet<String> = tokens(revised)?.into_iter().collect();
    let mut dropped = Vec::new();
    for (i, seg) in old.iter().enumerate() {
        if aligned.contains_key(&i) {
            continue;
        }
        let lost: Vec<Literal> = literals(&seg.text)
            .into_iter()
            .filter(|l| !revised_lower.contains(&l.text.to_lowercase()))
            .collect();
        let own = tokens(&seg.text)?;
        let coverage = if own.is_empty() {
            1.0
        } else {
            ratio(
                own.iter().filter(|t| revised_tokens.contains(*t)).count(),
                own.len(),
            )
        };
        dropped.push(Dropped {
            line: seg.line,
            text: seg.text.clone(),
            fact_candidate: !lost.is_empty() || coverage < COVERED,
            lost_literals: lost,
            token_coverage: round(coverage),
        });
    }
    Ok(dropped)
}

/// The `claims` subcommand, for the CLI epic to mount.
#[must_use]
pub fn command() -> Command {
    Command::new("claims")
        .about(AgentsPhrase::ClaimsAbout.as_static())
        .arg(
            Arg::new("revised")
                .value_name("REVISED")
                .required(true)
                .value_parser(clap::value_parser!(PathBuf))
                .help(AgentsPhrase::Revised.as_static()),
        )
        .arg(
            Arg::new("original")
                .long("original")
                .value_name("FILE")
                .required(true)
                .value_parser(clap::value_parser!(PathBuf))
                .help(AgentsPhrase::Original.as_static()),
        )
        .arg(
            Arg::new("format")
                .long("format")
                .value_parser(["json", "text"])
                .default_value("json")
                .help(AgentsPhrase::ClaimsFormat.as_static()),
        )
}

/// One line per claim and dropped fact, for people.
#[must_use]
pub fn render_text(report: &Report) -> String {
    let mut out = String::new();
    for c in &report.claims {
        let line = AgentsText::ClaimLine {
            change: c.change.to_owned(),
            line: c.line,
            text: c.text.clone(),
        };
        let _ = writeln!(out, "{line}");
        for l in &c.literals_removed {
            let removed = AgentsText::ClaimRemoved {
                kind: l.kind.to_owned(),
                text: l.text.clone(),
            };
            let _ = writeln!(out, "  {removed}");
        }
        for l in &c.literals_added {
            let added = AgentsText::ClaimAdded {
                kind: l.kind.to_owned(),
                text: l.text.clone(),
            };
            let _ = writeln!(out, "  {added}");
        }
        if c.negation_changed {
            let _ = writeln!(out, "  {}", AgentsPhrase::NegationChanged.as_static());
        }
    }
    for d in &report.dropped {
        let line = if d.fact_candidate {
            AgentsText::ClaimDroppedFact {
                line: d.line,
                text: d.text.clone(),
            }
        } else {
            AgentsText::ClaimDroppedCovered {
                line: d.line,
                text: d.text.clone(),
            }
        };
        let _ = writeln!(out, "{line}");
    }
    out
}

/// Run the parsed `claims` command. Reads two local files and nothing else.
pub fn run(matches: &ArgMatches) -> Result<String, AgentsError> {
    let read = |id: &str| -> Result<String, AgentsError> {
        let path = matches.get_one::<PathBuf>(id).cloned().unwrap_or_default();
        std::fs::read_to_string(&path).map_err(|e| AgentsError::io(path, e))
    };
    let report = compare(&read("original")?, &read("revised")?)?;
    if matches
        .get_one::<String>("format")
        .is_some_and(|f| f == "text")
    {
        return Ok(render_text(&report));
    }
    let mut json = serde_json::to_string_pretty(&report).map_err(|e| {
        AgentsError::usage(AgentsText::ClaimsJsonFailed {
            detail: e.to_string(),
        })
    })?;
    json.push('\n');
    Ok(json)
}
