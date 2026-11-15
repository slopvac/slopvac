//! The rule test-case runner.
//!
//! [`run_cases`] lints each rule's `cases` and, for a mechanical rule, its `examples`
//! through an [`Engine`], the way `vale test` does:
//!
//! - `want` is the finding count of the rule, or an inclusive range.
//! - `contains` lists strings that each appear in the matched text or message of some
//!   finding of the rule.
//! - `absent` lists strings that appear in no such finding.
//! - An example's `bad` must yield at least one finding and its `good` none.
//!
//! Only the findings of the rule under test count, so other rules in the engine do not
//! disturb a case. A rule the engine does not run under its settings (switched off, or
//! limited by `applies_to` to another genre or mode) is reported as skipped, not passed.
//! A mechanical rule is a lexical one: `tokens`, `pattern` or `substitution`.
//! An example of a heading-scoped rule holds the heading text, so the runner lints it as a
//! Markdown heading (`# text`), as the Python reference does. Case inputs run as written.
//!
//! [`coverage`] lints a corpus once and lists the mechanical rules that matched nothing in
//! it. [`cli`] wraps both as the `cases` command that the CLI mounts.

pub mod cli;

use std::collections::BTreeMap;
use std::fmt;

use slopvac_model::RuleId;
use slopvac_rules::{Case, LoadedRule, RuleSet, Scope, Want};

use crate::{Engine, EngineError, Input};

/// Where a checked text comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Origin {
    /// The rule's `cases[i]`, counting from 0.
    Case(usize),
    /// The `bad` text of the rule's `examples[i]`: at least one finding.
    ExampleBad(usize),
    /// The `good` text of the rule's `examples[i]`: no finding.
    ExampleGood(usize),
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Case(i) => write!(f, "cases[{i}]"),
            Self::ExampleBad(i) => write!(f, "examples[{i}].bad"),
            Self::ExampleGood(i) => write!(f, "examples[{i}].good"),
        }
    }
}

/// How one check ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Every expectation held.
    Passed,
    /// An expectation failed. Each entry says which and why.
    Failed(Vec<String>),
    /// The engine does not run the rule under its settings, so nothing was checked.
    Skipped,
}

/// The result of one case or example text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseResult {
    /// The rule under test.
    pub rule: RuleId,
    /// Where the text comes from.
    pub origin: Origin,
    /// How the check ended.
    pub outcome: Outcome,
}

/// Every result of one run, in rule then source order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CaseReport {
    /// One entry per case and example text.
    pub results: Vec<CaseResult>,
}

impl CaseReport {
    /// The results that failed.
    pub fn failures(&self) -> impl Iterator<Item = &CaseResult> {
        self.results
            .iter()
            .filter(|r| matches!(r.outcome, Outcome::Failed(_)))
    }

    fn count(&self, outcome: &Outcome) -> usize {
        self.results
            .iter()
            .filter(|r| std::mem::discriminant(&r.outcome) == std::mem::discriminant(outcome))
            .count()
    }

    /// The number of passed results.
    #[must_use]
    pub fn passed(&self) -> usize {
        self.count(&Outcome::Passed)
    }

    /// The number of skipped results.
    #[must_use]
    pub fn skipped(&self) -> usize {
        self.count(&Outcome::Skipped)
    }

    /// The ids of rules with a failed result, sorted and without repeats.
    #[must_use]
    pub fn failing_rules(&self) -> Vec<&RuleId> {
        let mut ids: Vec<&RuleId> = self.failures().map(|r| &r.rule).collect();
        ids.sort();
        ids.dedup();
        ids
    }

    /// True when no result failed.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.failures().next().is_none()
    }

    /// The report without the results of `rules`, to set aside a list of known failures.
    #[must_use]
    pub fn without(&self, rules: &[&str]) -> Self {
        Self {
            results: self
                .results
                .iter()
                .filter(|r| !rules.contains(&r.rule.as_str()))
                .cloned()
                .collect(),
        }
    }
}

impl fmt::Display for CaseReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for r in self.failures() {
            if let Outcome::Failed(reasons) = &r.outcome {
                for reason in reasons {
                    writeln!(f, "{} {}: {reason}", r.rule, r.origin)?;
                }
            }
        }
        write!(
            f,
            "{} passed, {} failed, {} skipped",
            self.passed(),
            self.failures().count(),
            self.skipped()
        )
    }
}

/// What the rule under test found in one text.
#[derive(Default)]
struct Seen {
    count: usize,
    /// The matched text and the message of each finding.
    texts: Vec<String>,
}

impl Seen {
    fn has(&self, needle: &str) -> bool {
        self.texts.iter().any(|t| t.contains(needle))
    }
}

fn findings_of(engine: &Engine, rule: &RuleId, text: &str) -> Result<Seen, EngineError> {
    let doc = engine.lint(&Input::markdown("case.md", text))?;
    let mut seen = Seen::default();
    for f in doc.findings.iter().filter(|f| &f.rule_id == rule) {
        seen.count += 1;
        seen.texts.push(f.matched_text.clone());
        seen.texts.push(f.message.as_str().to_owned());
    }
    Ok(seen)
}

fn check_case(case: &Case, seen: &Seen) -> Vec<String> {
    let mut why = Vec::new();
    match &case.want {
        Some(Want::Count(n)) if seen.count != *n as usize => {
            why.push(format!("want {n} finding(s), got {}", seen.count));
        }
        Some(Want::Range(r))
            if r.min.is_some_and(|min| seen.count < min as usize)
                || r.max.is_some_and(|max| seen.count > max as usize) =>
        {
            let bound = |b: Option<u32>| b.map_or_else(|| "-".into(), |v| v.to_string());
            why.push(format!(
                "want {}..={} finding(s), got {}",
                bound(r.min),
                bound(r.max),
                seen.count
            ));
        }
        _ => {}
    }
    for needle in case.contains.iter().flatten() {
        if !seen.has(needle.as_str()) {
            why.push(format!("no finding contains {:?}", needle.as_str()));
        }
    }
    for needle in case.absent.iter().flatten() {
        if seen.has(needle.as_str()) {
            why.push(format!("a finding contains {:?}", needle.as_str()));
        }
    }
    why
}

fn verdict(why: Vec<String>) -> Outcome {
    if why.is_empty() {
        Outcome::Passed
    } else {
        Outcome::Failed(why)
    }
}

fn run_rule(
    engine: &Engine,
    rule: &LoadedRule,
    out: &mut Vec<CaseResult>,
) -> Result<(), EngineError> {
    let spec = rule.rule();
    let mechanical = rule.matcher().is_some();
    let examples = if mechanical { spec.examples() } else { &[] };
    if spec.cases().is_empty() && examples.is_empty() {
        return Ok(());
    }
    let runs = engine.running.contains(rule.id().as_str());
    let as_example = |text: &str| {
        if spec.scope == Scope::Heading && !text.trim_start().starts_with('#') {
            format!("# {text}")
        } else {
            text.to_owned()
        }
    };
    let mut push = |origin, seen: &dyn Fn(&Seen) -> Vec<String>, text: &str| {
        let outcome = if runs {
            findings_of(engine, rule.id(), text).map(|s| verdict(seen(&s)))
        } else {
            Ok(Outcome::Skipped)
        };
        outcome.map(|outcome| {
            out.push(CaseResult {
                rule: rule.id().clone(),
                origin,
                outcome,
            });
        })
    };
    for (i, case) in spec.cases().iter().enumerate() {
        push(Origin::Case(i), &|s| check_case(case, s), &case.input)?;
    }
    for (i, example) in examples.iter().enumerate() {
        push(
            Origin::ExampleBad(i),
            &|s| {
                if s.count == 0 {
                    vec!["the bad text has no finding".to_owned()]
                } else {
                    Vec::new()
                }
            },
            &as_example(&example.bad),
        )?;
        if let Some(good) = &example.good {
            push(
                Origin::ExampleGood(i),
                &|s| {
                    if s.count == 0 {
                        Vec::new()
                    } else {
                        vec![format!("the good text has {} finding(s)", s.count)]
                    }
                },
                &as_example(good),
            )?;
        }
    }
    Ok(())
}

/// Run the cases and mechanical examples of every rule `engine` holds, or of `only`.
///
/// A rule without cases and without checkable examples adds no result.
pub fn run_cases(engine: &Engine, only: Option<&RuleId>) -> Result<CaseReport, EngineError> {
    let mut report = CaseReport::default();
    for set in &engine.sets {
        for rule in set.rules().filter(|r| only.is_none_or(|o| o == r.id())) {
            run_rule(engine, rule, &mut report.results)?;
        }
    }
    Ok(report)
}

/// How often each mechanical rule matched in a corpus.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Coverage {
    /// The number of documents linted.
    pub documents: usize,
    /// Findings per mechanical rule that the engine runs, zero for a rule that matched nothing.
    pub findings: BTreeMap<RuleId, usize>,
}

impl Coverage {
    /// The rules that matched nothing, in id order.
    pub fn unmatched(&self) -> impl Iterator<Item = &RuleId> {
        self.findings
            .iter()
            .filter(|(_, n)| **n == 0)
            .map(|(r, _)| r)
    }
}

impl fmt::Display for Coverage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let total = self.findings.len();
        let unmatched = self.unmatched().count();
        for rule in self.unmatched() {
            writeln!(f, "{rule}: matched nothing")?;
        }
        write!(
            f,
            "{} of {total} rules matched in {} document(s); {unmatched} matched nothing",
            total - unmatched,
            self.documents,
        )
    }
}

/// Lint `corpus` and count the findings of each mechanical rule the engine runs.
pub fn coverage(engine: &Engine, corpus: &[Input<'_>]) -> Result<Coverage, EngineError> {
    let mut findings: BTreeMap<RuleId, usize> = engine
        .sets
        .iter()
        .flat_map(RuleSet::rules)
        .filter(|r| r.matcher().is_some() && engine.running.contains(r.id().as_str()))
        .map(|r| (r.id().clone(), 0))
        .collect();
    for doc in engine.lint_all(corpus) {
        for f in doc?.findings {
            if let Some(n) = findings.get_mut(&f.rule_id) {
                *n += 1;
            }
        }
    }
    Ok(Coverage {
        documents: corpus.len(),
        findings,
    })
}
