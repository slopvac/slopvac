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
//! enough prose to score. Severities wait for the rebuilt corpus.
//!
//! STUB(slopvac-8l4.10.6): `DiagramFinding` and `Severity` stand in for the finding
//! types of `slopvac-model`, and the checks run on their own until `slopvac-engine`
//! dispatches rules.

use std::collections::HashSet;
use std::ops::Range;

use crate::{Fragment, Role};

/// Rule severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// A likely problem.
    Warning,
    /// A possible improvement.
    Suggestion,
}

/// One diagram finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagramFinding {
    /// The qualified rule id.
    pub rule: &'static str,
    /// The severity.
    pub severity: Severity,
    /// Byte range in the host file.
    pub span: Range<usize>,
    /// What is wrong and what to do.
    pub message: String,
}

/// A paragraph of prose next to a diagram.
#[derive(Debug, Clone)]
pub struct ProseBlock<'a> {
    /// Byte range in the host file.
    pub span: Range<usize>,
    /// The paragraph text.
    pub text: &'a str,
}

/// Rules that need a reviewer. Each pairs an id with the question to answer.
pub const JUDGEMENT_RULES: [(&str, &str); 3] = [
    (
        "diagram.over-explained",
        "For each node, edge and note in this diagram, would a short noun phrase of at most 4 words carry the same meaning for the intended reader? Quote any label that bundles a name with a description, any note that explains something the component name already says, and any edge label that is a sentence. Exempt sequence-diagram message verbs and legends. Answer yes/no per quoted item.",
    ),
    (
        "diagram.prose-restates",
        "Does the paragraph before or after this diagram only walk through the nodes and edges in the diagram's own order, or enumerate the components that the diagram already names, without adding a reason, constraint, number, failure mode or decision? If yes, quote the sentences that can be deleted or replaced by one sentence of added insight.",
    ),
    (
        "diagram.note-needed",
        "Does each note or annotation on a component tell the reader something that is not obvious from the component's name and its neighbours?",
    ),
];

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

/// Split a label into words. `<br>` tags separate words, and tokens without a letter or digit are dropped.
fn words(label: &str) -> Vec<&str> {
    label
        .split(|c: char| c.is_whitespace() || c == '<' || c == '>')
        .filter(|w| w.chars().any(char::is_alphanumeric))
        .filter(|w| !w.eq_ignore_ascii_case("br"))
        .collect()
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
    ["\n", "<br", ":", " - ", " – ", " / ", "("]
        .iter()
        .filter_map(|sep| label.find(sep).map(|at| (at, sep.len())))
        .min_by_key(|&(at, _)| at)
        .map(|(at, len)| (&label[..at], &label[at + len..]))
}

/// Check the labels of one diagram.
#[must_use]
pub fn check_labels(source: &str, fragments: &[Fragment]) -> Vec<DiagramFinding> {
    let mut out = Vec::new();
    for f in fragments {
        let label = label_text(source, f);
        let count = word_count(label);
        let finding = |rule, severity, message: String| DiagramFinding {
            rule,
            severity,
            span: f.span.clone(),
            message,
        };
        let has_code = words(label).iter().any(|w| is_code_token(w));
        if has_code && matches!(f.role, Role::Node | Role::Edge | Role::Group | Role::Note) {
            continue;
        }
        match f.role {
            Role::Note => {
                if count >= NOTE_WORDS {
                    out.push(finding(
                        "diagram.note-long",
                        Severity::Warning,
                        format!("This note has {count} words. Name the component or cut the note to a few words."),
                    ));
                }
            }
            Role::Node | Role::Edge | Role::Group => {
                if count >= RUN_ON_WORDS {
                    out.push(finding(
                        "diagram.label-run-on",
                        Severity::Warning,
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
                        out.push(finding(
                            "diagram.label-title-plus-gloss",
                            Severity::Suggestion,
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
    for i in 0..w.len() {
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
            continue;
        };
        out.push(block.span.start + w[i].0.start..block.span.start + w[i + len - 1].0.end);
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
) -> Vec<DiagramFinding> {
    let mut out = Vec::new();
    for block in prose {
        for span in meta_references(block) {
            out.push(DiagramFinding {
                rule: "diagram.meta-reference",
                severity: Severity::Warning,
                span,
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
            out.push(DiagramFinding {
                rule: "diagram.prose-restates-labels",
                severity: Severity::Warning,
                span: first.span.clone(),
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

    fn rules(source: &str, parts: &[(Role, &str)]) -> Vec<&'static str> {
        let fragments: Vec<_> = parts.iter().map(|(r, l)| frag(*r, source, l)).collect();
        check_labels(source, &fragments)
            .iter()
            .map(|f| f.rule)
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
            ["diagram.label-run-on"]
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
            Vec::<&str>::new()
        );
    }

    #[test]
    fn title_plus_gloss_needs_a_separator_and_a_list() {
        let a = "Clean Data: remove duplicates, nulls";
        let b = "Raw Data Storage Temporary Holding";
        let c = "Monitoring and Logging - latency, errors, drift";
        let src = format!("{a}\n{b}\n{c}\n");
        assert_eq!(
            rules(&src, &[(Role::Node, a), (Role::Node, b), (Role::Node, c)]),
            [
                "diagram.label-title-plus-gloss",
                "diagram.label-title-plus-gloss"
            ]
        );
    }

    #[test]
    fn code_tokens_exempt_long_notes_too() {
        let note = "reads config from src/app/config.rs on every start";
        assert_eq!(rules(note, &[(Role::Note, note)]), Vec::<&str>::new());
    }

    #[test]
    fn note_long_flags_six_words() {
        let five = "keys rotate every night here";
        let six = "keys rotate every night at midnight";
        let src = format!("{five}\n{six}\n");
        assert_eq!(
            rules(&src, &[(Role::Note, five), (Role::Note, six)]),
            ["diagram.note-long"]
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
        assert_eq!(check_labels(src, &[f])[0].rule, "diagram.label-run-on");
    }

    fn prose_rules(source: &str, labels: &[&str], text: &str) -> Vec<DiagramFinding> {
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
        assert_eq!(found[0].rule, "diagram.meta-reference");
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
    fn meta_reference_variants() {
        for text in [
            "As shown in the diagram, requests fan out.",
            "See the figure below for details.",
            "This flowchart illustrates retries.",
        ] {
            assert_eq!(prose_rules("", &[], text).len(), 1, "{text}");
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
        assert_eq!(found[0].rule, "diagram.prose-restates-labels");
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

    #[test]
    fn judgement_rules_have_unique_ids() {
        let ids: HashSet<_> = JUDGEMENT_RULES.iter().map(|(id, _)| id).collect();
        assert_eq!(ids.len(), JUDGEMENT_RULES.len());
    }
}
