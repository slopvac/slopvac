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
//! Each rule runs in an engine of its own with every other rule switched off
//! ([`Engine::isolate`]), so an unrelated rule cannot change or abort a case.
//!
//! A check passes only after the engine executed the rule. A rule the engine cannot run
//! (switched off, limited by `applies_to` to another genre or mode, or of a kind or style
//! the engine does not implement) and a rule that is testable but has no cases or
//! checkable examples report [`Outcome::Incomplete`], never [`Outcome::Passed`]. A
//! judgement rule is documentation only: it never runs and never reports. An id that
//! names no rule is a [`CasesError`].
//! A mechanically checkable rule is any rule that is not a judgement rule. An example of a
//! heading-scoped rule holds the heading text, so the runner lints it as a
//! Markdown heading (`# text`), as the Python reference does. Case inputs run as written.
//! [`CaseReport::unchecked_notice`] names the incomplete rules in a warning notice.
//!
//! [`coverage`] lints a corpus once and lists the mechanically checkable rules that matched
//! nothing in it. [`cli`] wraps both as the `cases` command that the CLI mounts.

pub mod cli;

use std::collections::BTreeMap;
use std::fmt;

use slopvac_model::{Notice, NoticeLevel, RuleId};
use slopvac_rules::{Case, LoadedRule, MatchKind, Scope, Want};

use crate::{Engine, EngineError, Input, Level};

/// Where a checked text comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Origin {
    /// The rule's `cases[i]`, counting from 0.
    Case(usize),
    /// The rule has nothing to run: no cases and no checkable examples.
    Missing,
    /// The `bad` text of the rule's `examples[i]`: at least one finding.
    ExampleBad(usize),
    /// The `good` text of the rule's `examples[i]`: no finding.
    ExampleGood(usize),
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => f.write_str("cases"),
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
    /// Nothing was checked, and the reason says why. This is not a pass.
    Incomplete(String),
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

    /// The results that checked nothing.
    pub fn incomplete(&self) -> impl Iterator<Item = &CaseResult> {
        self.results
            .iter()
            .filter(|r| matches!(r.outcome, Outcome::Incomplete(_)))
    }

    /// The number of passed results.
    #[must_use]
    pub fn passed(&self) -> usize {
        self.results
            .iter()
            .filter(|r| r.outcome == Outcome::Passed)
            .count()
    }

    /// The ids of rules with a failed result, sorted and without repeats.
    #[must_use]
    pub fn failing_rules(&self) -> Vec<&RuleId> {
        sorted_rules(self.failures())
    }

    /// The ids of rules with an incomplete result, sorted and without repeats.
    #[must_use]
    pub fn incomplete_rules(&self) -> Vec<&RuleId> {
        sorted_rules(self.incomplete())
    }

    /// True when no result failed or came out incomplete.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.failures().next().is_none() && self.incomplete().next().is_none()
    }

    /// The warning notice for the rules the runner could not check, or `None` when it
    /// checked them all. `data.rules` lists their ids. Judgement rules never appear in it.
    #[must_use]
    pub fn unchecked_notice(&self) -> Option<Notice> {
        let rules = self.incomplete_rules();
        if rules.is_empty() {
            return None;
        }
        let data = serde_json::Map::from_iter([(
            "rules".to_owned(),
            rules
                .iter()
                .map(|id| serde_json::Value::String(id.to_string()))
                .collect(),
        )]);
        Some(
            crate::lint::notice(
                NoticeLevel::Warning,
                &crate::messages::EngineNotice::CasesRuleUnchecked { count: rules.len() },
            )
            .with_data(data),
        )
    }

    /// The exit code, by the contract ruling in `docs/contract/rules.md`: 1 when a case or
    /// example failed, else 2 when a rule went unchecked, else 0. Judgement rules never
    /// reach a result, so they never change it.
    #[must_use]
    pub fn exit_code(&self) -> i32 {
        if self.failures().next().is_some() {
            1
        } else if self.incomplete().next().is_some() {
            2
        } else {
            0
        }
    }

    /// The report without the results of `rules`, to set aside a list of known problems.
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

fn sorted_rules<'a>(results: impl Iterator<Item = &'a CaseResult>) -> Vec<&'a RuleId> {
    let mut ids: Vec<&RuleId> = results.map(|r| &r.rule).collect();
    ids.sort();
    ids.dedup();
    ids
}

impl fmt::Display for CaseReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for r in &self.results {
            match &r.outcome {
                Outcome::Failed(reasons) => {
                    for reason in reasons {
                        writeln!(
                            f,
                            "{}",
                            crate::messages::EngineDetails::CaseFailedLine {
                                rule: r.rule.to_string(),
                                origin: r.origin.to_string(),
                                reason: reason.clone()
                            }
                        )?;
                    }
                }
                Outcome::Incomplete(reason) => {
                    writeln!(
                        f,
                        "{}",
                        crate::messages::EngineDetails::CaseIncompleteLine {
                            rule: r.rule.to_string(),
                            origin: r.origin.to_string(),
                            reason: reason.clone()
                        }
                    )?;
                }
                Outcome::Passed => {}
            }
        }
        write!(
            f,
            "{}",
            crate::messages::EngineDetails::CaseSummary {
                passed: self.passed(),
                failed: self.failures().count(),
                incomplete: self.incomplete().count()
            }
        )
    }
}

/// Why a run could not start.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum CasesError {
    /// The selected id names no rule of the engine.
    #[error("{}", crate::messages::no_rule(.0))]
    UnknownRule(RuleId),
    /// The engine holds no rule with cases or checkable examples, so nothing would run.
    #[error("{}", crate::messages::EngineText::NoCases)]
    NothingToRun,
    /// The engine failed.
    #[error(transparent)]
    Engine(#[from] EngineError),
}

impl Engine {
    /// An engine over the same rule sets and settings in which `rule` is the only rule that
    /// runs. Category weights and budgets stay, and so does the blocklist. Detectors,
    /// taggers, diagram checkers and RST options return to their defaults.
    pub fn isolate(&self, rule: &RuleId) -> Result<Self, EngineError> {
        let mut settings = self.settings.clone();
        for other in self.sets.iter().flat_map(|set| set.rules()) {
            if other.id() != rule {
                settings.rule_levels.insert(other.id().clone(), Level::Off);
            }
        }
        Self::new_shared(self.sets.clone(), settings)?
            .with_blocklist(&self.blocklist)
            .map(|engine| engine.with_terms(self.terms.clone()))
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
            why.push(
                crate::messages::EngineDetails::CaseCount {
                    want: *n,
                    got: seen.count,
                }
                .to_string(),
            );
        }
        Some(Want::Range(r))
            if r.min.is_some_and(|min| seen.count < min as usize)
                || r.max.is_some_and(|max| seen.count > max as usize) =>
        {
            let bound = |b: Option<u32>| b.map_or_else(|| "-".into(), |v| v.to_string());
            why.push(
                crate::messages::EngineDetails::CaseRange {
                    min: bound(r.min),
                    max: bound(r.max),
                    got: seen.count,
                }
                .to_string(),
            );
        }
        _ => {}
    }
    for needle in case.contains.iter().flatten() {
        if !seen.has(needle.as_str()) {
            why.push(
                crate::messages::EngineDetails::CaseContains {
                    needle: format!("{:?}", needle.as_str()),
                }
                .to_string(),
            );
        }
    }
    for needle in case.absent.iter().flatten() {
        if seen.has(needle.as_str()) {
            why.push(
                crate::messages::EngineDetails::CaseAbsent {
                    needle: format!("{:?}", needle.as_str()),
                }
                .to_string(),
            );
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

/// Turns what the rule found into the reasons it failed, none when it passed.
type Judge<'a> = Box<dyn Fn(&Seen) -> Vec<String> + 'a>;

/// One text to lint and the check to apply to what the rule found in it.
struct Check<'a> {
    origin: Origin,
    text: String,
    judge: Judge<'a>,
}

fn checks_of(rule: &LoadedRule) -> Vec<Check<'_>> {
    let spec = rule.rule();
    let as_example = |text: &str| {
        if spec.scope == Scope::Heading && !text.trim_start().starts_with('#') {
            format!("# {text}")
        } else {
            text.to_owned()
        }
    };
    let mut checks: Vec<Check<'_>> = Vec::new();
    for (i, case) in spec.cases().iter().enumerate() {
        checks.push(Check {
            origin: Origin::Case(i),
            text: case.input.clone(),
            judge: Box::new(move |s| check_case(case, s)),
        });
    }
    if rule.kind() != MatchKind::Judgement {
        for (i, example) in spec.examples().iter().enumerate() {
            checks.push(Check {
                origin: Origin::ExampleBad(i),
                text: as_example(&example.bad),
                judge: Box::new(|s| {
                    if s.count == 0 {
                        vec![crate::messages::EnginePhrase::CaseBadEmpty.text()]
                    } else {
                        Vec::new()
                    }
                }),
            });
            if let Some(good) = &example.good {
                checks.push(Check {
                    origin: Origin::ExampleGood(i),
                    text: as_example(good),
                    judge: Box::new(|s| {
                        if s.count == 0 {
                            Vec::new()
                        } else {
                            vec![
                                crate::messages::EngineDetails::CaseGoodCount { count: s.count }
                                    .to_string(),
                            ]
                        }
                    }),
                });
            }
        }
    }
    checks
}

fn run_rule(
    engine: &Engine,
    rule: &LoadedRule,
    out: &mut Vec<CaseResult>,
) -> Result<(), EngineError> {
    let mut push = |origin, outcome| {
        out.push(CaseResult {
            rule: rule.id().clone(),
            origin,
            outcome,
        });
    };
    let checks = checks_of(rule);
    if checks.is_empty() {
        // A judgement rule is documentation only: it never runs and never reports.
        if rule.kind() != MatchKind::Judgement {
            push(
                Origin::Missing,
                Outcome::Incomplete(crate::messages::EnginePhrase::CaseNoExamples.text()),
            );
        }
        return Ok(());
    }
    let isolated = engine.isolate(rule.id())?;
    let unexecuted = isolated.unexecuted(rule);
    for check in checks {
        let outcome = if let Some(reason) = unexecuted {
            Outcome::Incomplete(reason.to_owned())
        } else {
            match findings_of(&isolated, rule.id(), &check.text) {
                Ok(seen) => verdict((check.judge)(&seen)),
                Err(e) => Outcome::Failed(vec![
                    crate::messages::EngineDetails::CaseEngineFailed {
                        reason: e.to_string(),
                    }
                    .to_string(),
                ]),
            }
        };
        push(check.origin, outcome);
    }
    Ok(())
}

/// Run the cases and examples of every mechanically checkable rule `engine` holds, or of
/// `only`.
///
/// A checkable rule that has no cases and no example reports
/// [`Outcome::Incomplete`], as does every check of a rule the engine does not execute.
/// A judgement rule reports nothing, even when `only` selects it.
pub fn run_cases(engine: &Engine, only: Option<&RuleId>) -> Result<CaseReport, CasesError> {
    let mut report = CaseReport::default();
    let mut found = false;
    for rule in effective_rules(engine).filter(|r| only.is_none_or(|o| o == r.id())) {
        found = true;
        run_rule(engine, rule, &mut report.results)?;
    }
    match only {
        Some(id) if !found => Err(CasesError::UnknownRule(id.clone())),
        _ if report.results.is_empty() => Err(CasesError::NothingToRun),
        _ => Ok(report),
    }
}

/// The rules the engine resolves: when several sets define an id, only the last set's
/// definition counts, as in `Engine::resolve`.
fn effective_rules(engine: &Engine) -> impl Iterator<Item = &LoadedRule> {
    let mut owner: BTreeMap<&RuleId, usize> = BTreeMap::new();
    for (s, set) in engine.sets.iter().enumerate() {
        for rule in set.rules() {
            owner.insert(rule.id(), s);
        }
    }
    engine.sets.iter().enumerate().flat_map(move |(s, set)| {
        let owner = owner.clone();
        set.rules().filter(move |r| owner.get(r.id()) == Some(&s))
    })
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
            writeln!(
                f,
                "{}",
                crate::messages::EngineDetails::CoverageUnmatched {
                    rule: rule.to_string()
                }
            )?;
        }
        write!(
            f,
            "{}",
            crate::messages::EngineDetails::CoverageSummary {
                matched: total - unmatched,
                total,
                documents: self.documents,
                unmatched
            },
        )
    }
}

/// Lint `corpus` and count the findings of each mechanical rule the engine runs.
pub fn coverage(engine: &Engine, corpus: &[Input<'_>]) -> Result<Coverage, EngineError> {
    let mut findings: BTreeMap<RuleId, usize> = effective_rules(engine)
        .filter(|r| r.kind() != MatchKind::Judgement && engine.running.contains(r.id().as_str()))
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
