//! Diagram rules: rules that read the labels of a diagram instead of prose.
//!
//! Diagram text is short labels, not sentences, so the prose pipeline does not read it.
//! `slopvac-diagrams` extracts `Fragment`s from a diagram file or from a fenced
//! `mermaid` or `plantuml` block, and a [`DiagramChecker`] turns them into hits. The engine
//! owns what a hit becomes: its level, the `slopvac-allow` directives, the position and the
//! score.

use std::ops::Range;
use std::path::PathBuf;

use slopvac_diagrams::{Extraction, ProseBlock, Signal, SignalHit};
use slopvac_model::{PackId, RuleId, Severity};
use slopvac_rules::{CompileOptions, LoadErrors, Match, RuleSet, StructureMatch};

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

/// The pack id to give [`diagram_rule_set`] when a caller has no other.
pub const DIAGRAM_PACK: &str = "diagrams";

/// The diagram rule pack that ships with `slopvac-diagrams` as a rule set under `pack`:
/// `diagram.label-run-on`, `diagram.label-title-plus-gloss`, `diagram.note-long`,
/// `diagram.meta-reference` and `diagram.prose-restates-labels`, plus three judgement rules.
///
/// The rules are experimental and no default pack includes them. Pass this set to
/// [`crate::Engine::new`] with the other sets. The engine runs each rule whose
/// `match.check` names a [`Signal`], and applies its level, `applies_to`, category and
/// suppression settings to it like any other rule.
///
/// # Errors
/// The shipped pack fails to load, which the tests rule out.
pub fn diagram_rule_set(pack: &PackId) -> Result<RuleSet, LoadErrors> {
    RuleSet::from_sources(
        pack,
        slopvac_diagrams::PACK
            .iter()
            .map(|&(file, text)| (PathBuf::from(file), text.to_owned())),
        &CompileOptions::default(),
    )
}

/// The signal a structure rule selects, if it selects one.
pub(crate) fn signal_of(m: &StructureMatch) -> Option<Signal> {
    Signal::from_check(m.check.as_str())
}

/// The deterministic signals of `slopvac-diagrams` as one checker, for the rules of a
/// [`RuleSet`] that name them in `match.check`.
///
/// [`crate::Engine::new`] builds one for each set that has such rules, so callers rarely
/// need this.
#[derive(Debug, Clone)]
pub struct SignalChecker {
    rules: Vec<(Signal, DiagramRule)>,
}

impl SignalChecker {
    /// A checker for the signal rules of `set`, or `None` when it has none.
    #[must_use]
    pub fn from_rule_set(set: &RuleSet) -> Option<Self> {
        let rules: Vec<_> = set
            .rules()
            .filter_map(|loaded| {
                let Match::Structure(m) = &loaded.rule().match_ else {
                    return None;
                };
                let rule = DiagramRule {
                    id: loaded.id().clone(),
                    severity: loaded.rule().severity,
                };
                Some((signal_of(m)?, rule))
            })
            .collect();
        (!rules.is_empty()).then_some(Self { rules })
    }
}

impl DiagramChecker for SignalChecker {
    fn rules(&self) -> Vec<DiagramRule> {
        self.rules.iter().map(|(_, rule)| rule.clone()).collect()
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
            .flat_map(|hit: SignalHit| {
                // Every rule that names the signal gets the hit. The engine drops it for a rule
                // that is off or does not apply, so an inactive rule never hides an active one.
                self.rules
                    .iter()
                    .filter(move |(s, _)| *s == hit.signal)
                    .map(|(_, rule)| DiagramHit {
                        rule: rule.id.clone(),
                        span: hit.span.clone(),
                        message: hit.message.clone(),
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }
}
