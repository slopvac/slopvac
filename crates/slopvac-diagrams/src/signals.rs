//! Deterministic diagram signals (experimental).
//!
//! These checks come from the spike `slopvac-8l4.10.1`: 37 usable human Mermaid
//! blocks, 82 draw.io files and 61 LLM Mermaid diagrams, with 60 hand-labelled
//! diagrams. They flag a label that reads as a sentence, a long note, prose that
//! points at the diagram, and prose that walks through the labels. Precision is
//! from the single-annotator sample, so every rule stays experimental.
//!
//! | rule | threshold | severity | precision (recall) on n=60 |
//! |---|---|---|---|
//! | `diagram.label-run-on` | label of 7 words or more | warning | 0.78 (0.88) |
//! | `diagram.label-title-plus-gloss` | 5 words or more, name then gloss | suggestion | untested alone |
//! | `diagram.note-long` | note of 6 words or more | warning | 1.00 (0.38) |
//! | `diagram.meta-reference` | prose that names the diagram | warning | 1.00 (0.05) |
//! | `diagram.prose-restates-labels` | 60% of 3 or more labels in 40 or more prose words | warning | 1.00 (0.67), lower bound about 0.77 |
//!
//! The human baseline for the prose rules is thin: only three human diagrams had
//! enough prose to score. `evals/corpus` has 50 committed human diagrams and no
//! stated-LLM diagram, so it can bound false positives but not recall.
//!
//! A [`Signal`] says which check fired. The rule id, severity and message of each
//! signal's rule live in the diagram rule pack ([`PACK`]); `slopvac-engine` loads the pack
//! and turns a [`SignalHit`] into a finding, so suppression and severity follow its
//! configuration.

use std::collections::HashSet;
use std::ops::Range;

use crate::{Fragment, Role};

/// A deterministic check that a rule of the diagram pack names in `match.check`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Signal {
    /// A label of seven words or more.
    LabelRunOn,
    /// A label that holds a name and a description.
    LabelTitlePlusGloss,
    /// A note of six words or more.
    NoteLong,
    /// Prose that points at the diagram.
    MetaReference,
    /// Prose that walks through the labels.
    ProseRestatesLabels,
}

impl Signal {
    /// Every signal.
    pub const ALL: [Self; 5] = [
        Self::LabelRunOn,
        Self::LabelTitlePlusGloss,
        Self::NoteLong,
        Self::MetaReference,
        Self::ProseRestatesLabels,
    ];

    /// The `match.check` name of the pack rule that runs this signal.
    #[must_use]
    pub const fn check(self) -> &'static str {
        match self {
            Self::LabelRunOn => "diagram-label-run-on",
            Self::LabelTitlePlusGloss => "diagram-label-title-plus-gloss",
            Self::NoteLong => "diagram-note-long",
            Self::MetaReference => "diagram-meta-reference",
            Self::ProseRestatesLabels => "diagram-prose-restates-labels",
        }
    }

    /// The signal a `match.check` name selects.
    #[must_use]
    pub fn from_check(check: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.check() == check)
    }
}

/// One hit of a [`Signal`]. The engine decides what it becomes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalHit {
    /// The check that fired.
    pub signal: Signal,
    /// Byte range in the host file.
    pub span: Range<usize>,
    /// What is wrong and what to do.
    pub message: String,
    /// The text the hit is about, when the span does not show it: a label decoded from a
    /// compressed page has a span over the payload.
    pub matched: Option<String>,
}

/// A paragraph of prose next to a diagram.
#[derive(Debug, Clone)]
pub struct ProseBlock<'a> {
    /// Byte range in the host file.
    pub span: Range<usize>,
    /// The paragraph text.
    pub text: &'a str,
}

/// The diagram rule pack: category files as `(file name, JSON text)`, ready for
/// `RuleSet::from_sources`. It holds a rule for every [`Signal`] and the rules that need a
/// reviewer.
pub const PACK: [(&str, &str); 1] = [("diagram.json", include_str!("../pack/diagram.json"))];

const RUN_ON_WORDS: usize = 7;
const GLOSS_MIN_WORDS: usize = 5;
const NOTE_WORDS: usize = 6;
const MIN_LABELS: usize = 3;
const MIN_PROSE_WORDS: usize = 40;
const MENTION_SHARE: f64 = 0.6;
const COVERAGE: f64 = 0.6;

const STOPWORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "by", "for", "from", "in", "is", "it", "its", "of",
    "on", "or", "the", "to", "with",
];

/// The label text the rules see.
fn label_text<'a>(source: &'a str, f: &'a Fragment) -> &'a str {
    f.text.as_deref().unwrap_or_else(|| &source[f.span.clone()])
}

/// The length of a line break at the start of `s`: a newline, a literal backslash-n as
/// `PlantUML` writes it, or an HTML `<br>` tag in any case, with optional spaces and a self-closing slash.
fn break_len(s: &str) -> Option<usize> {
    if s.starts_with("\\n") {
        return Some(2);
    }
    if s.starts_with('\n') {
        return Some(1);
    }
    let head = s.get(..3)?;
    if !head.eq_ignore_ascii_case("<br") {
        return None;
    }
    let rest = &s[3..];
    let trimmed = rest.trim_start();
    let trimmed = trimmed.strip_prefix('/').unwrap_or(trimmed).trim_start();
    trimmed
        .starts_with('>')
        .then(|| s.len() - trimmed.len() + 1)
}

/// The first line break in `label` as `(start, length)`.
fn find_break(label: &str) -> Option<(usize, usize)> {
    label
        .char_indices()
        .find_map(|(at, _)| break_len(&label[at..]).map(|len| (at, len)))
}

/// Split a label into words. Line breaks separate words, and tokens without a letter or digit are dropped.
fn words(label: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = label;
    loop {
        let (line, next) = match find_break(rest) {
            Some((at, len)) => (&rest[..at], Some(&rest[at + len..])),
            None => (rest, None),
        };
        out.extend(
            line.split(|c: char| c.is_whitespace() || c == '<' || c == '>')
                .filter(|w| w.chars().any(char::is_alphanumeric)),
        );
        match next {
            Some(n) => rest = n,
            None => return out,
        }
    }
}

fn word_count(label: &str) -> usize {
    words(label).len()
}

fn is_code_token(word: &str) -> bool {
    let inner = |sep: char| {
        word.split(sep).count() > 1
            && word
                .split(sep)
                .all(|part| part.chars().any(char::is_alphanumeric))
    };
    word.contains('`')
        || word.contains("()")
        || word.contains("::")
        || inner('/')
        || inner('_')
        || inner('.')
}

fn gloss_split(label: &str) -> Option<(&str, &str)> {
    let line = find_break(label).map(|(at, len)| (at, len, true));
    let other = [":", " - ", " – ", " / ", "("]
        .iter()
        .filter_map(|sep| label.find(sep).map(|at| (at, sep.len(), false)))
        .min_by_key(|&(at, ..)| at);
    let (at, len, is_break) = match (line, other) {
        (Some(l), Some(o)) => {
            if o.0 < l.0 {
                o
            } else {
                l
            }
        }
        (l, o) => l.or(o)?,
    };
    let (head, gloss) = (&label[..at], &label[at + len..]);
    // A line break between two names lists components, as in "Istio Sidecar" over
    // "Envoy Pilot". It is a gloss only when the second line is not all capitalised words.
    (!is_break || !all_capitalised(gloss)).then_some((head, gloss))
}

/// Every word starts with a capital letter, so the text reads as a name.
fn all_capitalised(text: &str) -> bool {
    let list = words(text);
    !list.is_empty()
        && list.iter().all(|w| {
            w.chars()
                .find(|c| c.is_alphanumeric())
                .is_some_and(char::is_uppercase)
        })
}

/// Check the labels of one diagram.
#[must_use]
pub fn check_labels(source: &str, fragments: &[Fragment]) -> Vec<SignalHit> {
    let mut out = Vec::new();
    for f in fragments {
        let label = label_text(source, f);
        let count = word_count(label);
        let finding = |signal, message: String| SignalHit {
            signal,
            span: f.span.clone(),
            message,
            matched: f.text.clone(),
        };
        let has_code = words(label).iter().any(|w| is_code_token(w));
        if has_code && matches!(f.role, Role::Node | Role::Edge | Role::Group | Role::Note) {
            continue;
        }
        match f.role {
            Role::Note => {
                if count >= NOTE_WORDS {
                    out.push(finding(Signal::NoteLong,
                        format!("This note has {count} words. Name the component or cut the note to a few words."),
                    ));
                }
            }
            Role::Node | Role::Edge | Role::Group => {
                if count >= RUN_ON_WORDS {
                    out.push(finding(Signal::LabelRunOn,
                        format!("This label has {count} words. Use a short noun phrase and move the detail to the text."),
                    ));
                } else if f.role != Role::Edge
                    && count >= GLOSS_MIN_WORDS
                    && let Some((head, gloss)) = gloss_split(label)
                {
                    let head_words = word_count(head);
                    let gloss_words = word_count(gloss);
                    let list = gloss.contains(',') || gloss_words >= 3;
                    if (1..=4).contains(&head_words) && gloss_words >= 2 && list {
                        out.push(finding(Signal::LabelTitlePlusGloss,
                            "This label holds a name and a description. Keep the name and move the description to the text.".to_string(),
                        ));
                    }
                }
            }
            Role::Message | Role::Title | Role::Text | Role::Member => {}
        }
    }
    out
}

/// A word with its byte range in a paragraph.
fn spans(text: &str) -> Vec<(Range<usize>, String)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in text
        .char_indices()
        .chain(std::iter::once((text.len(), ' ')))
    {
        let part = c.is_alphanumeric() || c == '\'';
        match (part, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                out.push((s..i, text[s..i].to_lowercase()));
                start = None;
            }
            _ => {}
        }
    }
    out
}

/// Find phrases that point at the diagram. Each match is a complete phrase that ends in a
/// diagram noun or a locating word, so "as shown in the logs" and a cut-off prefix do not match.
fn meta_references(block: &ProseBlock<'_>) -> Vec<Range<usize>> {
    const NOUNS: &[&str] = &["diagram", "flowchart", "figure", "chart", "graph"];
    const VERBS: &[&str] = &["above", "below", "shows", "illustrates", "depicts"];
    const DEMONSTRATIVES: &[&str] = &["the", "this"];
    let w = spans(block.text);
    let mut out = Vec::new();
    let mut i = 0;
    while i < w.len() {
        let at = |k: usize| w.get(i + k).map_or("", |(_, s)| s.as_str());
        let len = if DEMONSTRATIVES.contains(&at(0))
            && NOUNS.contains(&at(1))
            && VERBS.contains(&at(2))
        {
            3
        } else if at(0) == "as"
            && matches!(at(1), "shown" | "illustrated" | "depicted")
            && at(2) == "in"
            && DEMONSTRATIVES.contains(&at(3))
            && NOUNS.contains(&at(4))
        {
            5
        } else if at(0) == "in"
            && DEMONSTRATIVES.contains(&at(1))
            && NOUNS.contains(&at(2))
            && matches!(at(3), "above" | "below")
        {
            4
        } else {
            i += 1;
            continue;
        };
        out.push(block.span.start + w[i].0.start..block.span.start + w[i + len - 1].0.end);
        i += len;
    }
    out
}

fn content_tokens(text: &str) -> Vec<String> {
    spans(text)
        .into_iter()
        .map(|(_, w)| w)
        .filter(|w| !STOPWORDS.contains(&w.as_str()))
        .collect()
}

/// Check the prose next to one diagram.
///
/// `prose` holds the nearest paragraph before and after the diagram. A meta
/// reference is reported per phrase. A restatement is reported once, on the
/// first paragraph.
#[must_use]
pub fn check_prose(
    source: &str,
    fragments: &[Fragment],
    prose: &[ProseBlock<'_>],
) -> Vec<SignalHit> {
    let mut out = Vec::new();
    for block in prose {
        for span in meta_references(block) {
            out.push(SignalHit {
                signal: Signal::MetaReference,
                span,
                matched: None,
                message: "This sentence points at the diagram. Say what the system does and let the diagram show the shape.".to_string(),
            });
        }
    }
    let prose_words: usize = prose.iter().map(|b| word_count(b.text)).sum();
    let labels: Vec<Vec<String>> = fragments
        .iter()
        .filter(|f| matches!(f.role, Role::Node | Role::Group))
        .map(|f| content_tokens(label_text(source, f)))
        .collect();
    if let Some(first) = prose.first()
        && labels.len() >= MIN_LABELS
        && prose_words >= MIN_PROSE_WORDS
    {
        let known: HashSet<String> = prose.iter().flat_map(|b| content_tokens(b.text)).collect();
        let mentioned = labels
            .iter()
            .filter(|tokens| {
                !tokens.is_empty()
                    && count_f64(tokens.iter().filter(|t| known.contains(*t)).count())
                        >= MENTION_SHARE * count_f64(tokens.len())
            })
            .count();
        if count_f64(mentioned) >= COVERAGE * count_f64(labels.len()) {
            out.push(SignalHit {
                signal: Signal::ProseRestatesLabels,
                span: first.span.clone(),
                matched: None,
                message: format!(
                    "The text names {mentioned} of the {} labels in the diagram. Keep what the diagram cannot show, such as reasons, limits and failure modes.",
                    labels.len()
                ),
            });
        }
    }
    out
}

#[allow(clippy::cast_precision_loss)]
fn count_f64(n: usize) -> f64 {
    n as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frag(role: Role, source: &str, label: &str) -> Fragment {
        let start = source.find(label).unwrap();
        Fragment {
            span: start..start + label.len(),
            text: None,
            role,
            logical: String::new(),
        }
    }

    fn rules(source: &str, parts: &[(Role, &str)]) -> Vec<Signal> {
        let fragments: Vec<_> = parts.iter().map(|(r, l)| frag(*r, source, l)).collect();
        check_labels(source, &fragments)
            .iter()
            .map(|f| f.signal)
            .collect()
    }

    #[test]
    fn run_on_flags_seven_words_not_six() {
        let src = "one two three four five six\none two three four five six seven\n";
        assert_eq!(
            rules(
                src,
                &[
                    (Role::Node, "one two three four five six"),
                    (Role::Node, "one two three four five six seven"),
                ]
            ),
            [Signal::LabelRunOn]
        );
    }

    #[test]
    fn run_on_exempts_message_member_text_and_code_tokens() {
        let long = "send the request to the remote service now";
        let code = "call src/api/handler.rs for the retry logic path";
        let src = format!("{long}\n{code}\n");
        assert_eq!(
            rules(
                &src,
                &[
                    (Role::Message, long),
                    (Role::Member, long),
                    (Role::Text, long),
                    (Role::Node, code),
                ]
            ),
            Vec::<Signal>::new()
        );
    }

    // "Order Service" is 2 words and "handles checkout and refunds" is 4, so each label has
    // 6 words: below the run-on threshold of 7, and a name plus a gloss of 3 words or more.
    #[rstest::rstest]
    #[case::self_closing("Order Service<br/>handles checkout and refunds")]
    #[case::spaced_self_closing("Order Service<br />handles checkout and refunds")]
    #[case::bare("Order Service<br>handles checkout and refunds")]
    #[case::upper_case("Order Service<BR/>handles checkout and refunds")]
    #[case::inner_space("Order Service<br / >handles checkout and refunds")]
    #[case::literal_backslash_n(r"Order Service\nhandles checkout and refunds")]
    fn a_break_variant_separates_name_from_gloss_and_is_not_a_word(#[case] label: &str) {
        let src = format!("{label}\n");
        assert_eq!(
            rules(&src, &[(Role::Node, label)]),
            [Signal::LabelTitlePlusGloss]
        );
    }

    #[test]
    fn a_tag_that_only_starts_with_br_is_a_word_boundary_not_a_break() {
        // `<brand>` is not a break: words are "brand" and the five around it.
        assert_eq!(words("a <brand> b c d"), ["a", "brand", "b", "c", "d"]);
    }

    #[test]
    fn title_plus_gloss_needs_a_separator_and_a_list() {
        let a = "Clean Data: remove duplicates, nulls";
        let b = "Raw Data Storage Temporary Holding";
        let c = "Monitoring and Logging - latency, errors, drift";
        let src = format!("{a}\n{b}\n{c}\n");
        assert_eq!(
            rules(&src, &[(Role::Node, a), (Role::Node, b), (Role::Node, c)]),
            [Signal::LabelTitlePlusGloss, Signal::LabelTitlePlusGloss]
        );
    }

    #[test]
    fn a_line_break_between_component_names_is_not_a_gloss() {
        let names = "Istio Sidecar\nEnvoy / Pilot Engine";
        let gloss = "Clean Data\nremove duplicates, nulls";
        let note = "Merge\n(32 + 2048 + 2048 values)";
        let src = format!("{names}\n{gloss}\n{note}\n");
        assert_eq!(
            rules(
                &src,
                &[(Role::Node, names), (Role::Node, gloss), (Role::Node, note)]
            ),
            [Signal::LabelTitlePlusGloss, Signal::LabelTitlePlusGloss]
        );
    }

    #[test]
    fn a_decoded_label_is_the_matched_text_and_the_span_stays_on_the_source() {
        let source = "<diagram>cGF5bG9hZA==</diagram>";
        let start = source.find("cGF5").unwrap();
        let fragment = Fragment {
            span: start..start + "cGF5bG9hZA==".len(),
            text: Some("one two three four five six seven".to_string()),
            role: Role::Node,
            logical: String::new(),
        };
        let hits = check_labels(source, std::slice::from_ref(&fragment));
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].span, fragment.span);
        assert_eq!(
            hits[0].matched.as_deref(),
            Some("one two three four five six seven")
        );
    }

    #[test]
    fn code_tokens_exempt_long_notes_too() {
        let note = "reads config from src/app/config.rs on every start";
        assert_eq!(rules(note, &[(Role::Note, note)]), Vec::<Signal>::new());
    }

    #[test]
    fn note_long_flags_six_words() {
        let five = "keys rotate every night here";
        let six = "keys rotate every night at midnight";
        let src = format!("{five}\n{six}\n");
        assert_eq!(
            rules(&src, &[(Role::Note, five), (Role::Note, six)]),
            [Signal::NoteLong]
        );
    }

    #[test]
    fn converted_text_is_what_counts() {
        let src = "x";
        let f = Fragment {
            span: 0..1,
            text: Some("a b c d e f g".to_string()),
            role: Role::Node,
            logical: String::new(),
        };
        assert_eq!(check_labels(src, &[f])[0].signal, Signal::LabelRunOn);
    }

    fn prose_rules(source: &str, labels: &[&str], text: &str) -> Vec<SignalHit> {
        let fragments: Vec<_> = labels.iter().map(|l| frag(Role::Node, source, l)).collect();
        let block = ProseBlock {
            span: 100..100 + text.len(),
            text,
        };
        check_prose(source, &fragments, &[block])
    }

    #[test]
    fn meta_reference_reports_the_phrase_span() {
        let text = "Overall, the diagram above shows the flow. Next, retries.";
        let found = prose_rules("", &[], text);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].signal, Signal::MetaReference);
        let start = text.find("the diagram above").unwrap() + 100;
        assert_eq!(found[0].span, start..start + "the diagram above".len());
    }

    #[test]
    fn meta_reference_needs_a_complete_phrase_ending_in_a_diagram_noun() {
        for text in [
            "As shown in the logs, requests fan out.",
            "As shown in the",
            "As shown in the diagrams team notes.",
            "In the diagram",
            "The diagram",
            "This chart is blue.",
            "as shown in",
        ] {
            assert_eq!(prose_rules("", &[], text), Vec::new(), "{text}");
        }
        let text = "Requests fan out, as shown in the diagram.";
        let found = prose_rules("", &[], text);
        let start = text.find("as shown").unwrap() + 100;
        assert_eq!(
            found[0].span,
            start..start + "as shown in the diagram".len()
        );
    }

    #[test]
    fn two_distinct_meta_references_are_reported_once_each() {
        let text = "In the diagram above and the figure below, requests retry.";
        let found = prose_rules("", &[], text);
        let got: Vec<_> = found.iter().map(|f| (f.signal, f.span.clone())).collect();
        assert_eq!(
            got,
            [
                (Signal::MetaReference, 100..120),
                (Signal::MetaReference, 125..141),
            ]
        );
    }

    #[test]
    fn meta_reference_variants_report_rule_and_exact_span() {
        for (text, phrase) in [
            (
                "As shown in the diagram, requests fan out.",
                "As shown in the diagram",
            ),
            (
                "Requests fan out, as illustrated in the diagram.",
                "as illustrated in the diagram",
            ),
            (
                "Requests fan out, as depicted in this figure.",
                "as depicted in this figure",
            ),
            ("See the figure below for details.", "the figure below"),
            (
                "This flowchart illustrates retries.",
                "This flowchart illustrates",
            ),
            ("The chart depicts the retry path.", "The chart depicts"),
            ("Details are in the diagram above.", "in the diagram above"),
        ] {
            let found = prose_rules("", &[], text);
            assert_eq!(found.len(), 1, "{text}");
            assert_eq!(found[0].signal, Signal::MetaReference, "{text}");
            let start = text.find(phrase).unwrap() + 100;
            assert_eq!(found[0].span, start..start + phrase.len(), "{text}");
        }
        assert_eq!(
            prose_rules("", &[], "The diagrams team owns this.").len(),
            0
        );
    }

    const LABELS: &str = "Load Balancer\nAPI Gateway\nAuth Service\nUser Database\n";
    const RESTATING: &str = "Requests reach the load balancer first. The balancer forwards each request to the API gateway. The gateway asks the auth service to check the token, and then the user database returns the account record for that user. Responses travel back through the same path to the client over the network connection.";

    #[test]
    fn prose_that_names_most_labels_is_flagged_once() {
        let labels = [
            "Load Balancer",
            "API Gateway",
            "Auth Service",
            "User Database",
        ];
        let found = prose_rules(LABELS, &labels, RESTATING);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].signal, Signal::ProseRestatesLabels);
        assert_eq!(found[0].span, 100..100 + RESTATING.len());
    }

    #[test]
    fn prose_that_adds_information_passes() {
        let text = "We cap each tenant at 200 requests per second because the shared cache saturates beyond that point. When the cache fails, reads fall back to the primary at a latency cost of roughly 40 milliseconds, so alerts fire on the hit ratio instead of errors.";
        let labels = [
            "Load Balancer",
            "API Gateway",
            "Auth Service",
            "User Database",
        ];
        assert_eq!(prose_rules(LABELS, &labels, text), Vec::new());
    }

    #[test]
    fn restatement_needs_enough_labels_and_prose() {
        let labels = ["Load Balancer", "API Gateway"];
        assert_eq!(prose_rules(LABELS, &labels, RESTATING), Vec::new());
        let labels = ["Load Balancer", "API Gateway", "Auth Service"];
        assert_eq!(
            prose_rules(
                LABELS,
                &labels,
                "The load balancer, API gateway and auth service."
            ),
            Vec::new()
        );
    }
}
