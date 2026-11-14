//! Diagram rules: rules that read the labels of a diagram instead of prose.
//!
//! Diagram text is short labels, not sentences, so the prose pipeline does not read it.
//! `slopvac-diagrams` extracts `Fragment`s from a diagram file or from a fenced
//! `mermaid` or `plantuml` block, and a [`DiagramChecker`] turns them into hits. The engine
//! owns what a hit becomes: its level, the `slopvac-allow` directives, the position and the
//! score.

use std::ops::Range;

use slopvac_diagrams::{DiagramFinding, Extraction, ProseBlock};
use slopvac_model::{PackId, RuleId, Severity};

/// A rule a checker reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagramRule {
    /// The qualified id, such as `core:diagram.label-run-on`. The part of the local id
    /// before the first dot is the category.
    pub id: RuleId,
    /// The level when the settings set none.
    pub severity: Severity,
}

/// One diagram handed to a checker.
#[derive(Debug, Clone, Copy)]
pub struct DiagramInput<'a> {
    /// The whole host file. Fragment spans address it.
    pub source: &'a str,
    /// What `slopvac-diagrams` extracted.
    pub extraction: &'a Extraction,
    /// The nearest paragraph before and after a fenced diagram. A diagram file has none.
    pub prose: &'a [ProseBlock<'a>],
}

/// A problem a checker found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagramHit {
    /// The rule. It is one of [`DiagramChecker::rules`].
    pub rule: RuleId,
    /// Source bytes of the label or paragraph.
    pub span: Range<usize>,
    /// What is wrong and what to do.
    pub message: String,
}

/// A group of diagram rules that run together.
pub trait DiagramChecker: Send + Sync {
    /// The rules this checker can report.
    fn rules(&self) -> Vec<DiagramRule>;
    /// Check one diagram.
    fn check(&self, input: &DiagramInput<'_>) -> Vec<DiagramHit>;
}

/// The deterministic label and prose signals of `slopvac-diagrams` (`diagram.label-run-on`,
/// `diagram.label-title-plus-gloss`, `diagram.note-long`, `diagram.meta-reference` and
/// `diagram.prose-restates-labels`) as one checker under `pack`.
///
/// They are experimental and no pack ships them yet, so the engine does not register them
/// on its own. Pass this to [`crate::Engine::with_diagram_checker`].
#[derive(Debug, Clone)]
pub struct SignalChecker {
    pack: PackId,
}

impl SignalChecker {
    /// Report the signals as rules of `pack`.
    #[must_use]
    pub const fn new(pack: PackId) -> Self {
        Self { pack }
    }

    fn id(&self, rule: &str) -> RuleId {
        RuleId::new(format!("{}:{rule}", self.pack)).expect("a diagram rule id is valid")
    }
}

const SIGNALS: [(&str, Severity); 5] = [
    ("diagram.label-run-on", Severity::Warning),
    ("diagram.label-title-plus-gloss", Severity::Suggestion),
    ("diagram.note-long", Severity::Warning),
    ("diagram.meta-reference", Severity::Warning),
    ("diagram.prose-restates-labels", Severity::Warning),
];

impl DiagramChecker for SignalChecker {
    fn rules(&self) -> Vec<DiagramRule> {
        SIGNALS
            .iter()
            .map(|&(rule, severity)| DiagramRule {
                id: self.id(rule),
                severity,
            })
            .collect()
    }

    fn check(&self, input: &DiagramInput<'_>) -> Vec<DiagramHit> {
        let fragments = &input.extraction.fragments;
        let mut found = slopvac_diagrams::check_labels(input.source, fragments);
        found.extend(slopvac_diagrams::check_prose(
            input.source,
            fragments,
            input.prose,
        ));
        found
            .into_iter()
            .map(|f: DiagramFinding| DiagramHit {
                rule: self.id(f.rule),
                span: f.span,
                message: f.message,
            })
            .collect()
    }
}
