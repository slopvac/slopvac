//! The engine: rules in, a scored document out.
//!
//! One [`Engine`] holds the loaded rules, the resolved [`Settings`], the exception
//! detectors, the context taggers and any diagram checkers. [`Engine::lint`] runs them over
//! one [`Input`] and returns the `slopvac-model` [`Document`] that the report contract
//! describes. [`Engine::lint_all`] runs many inputs in parallel and keeps their order.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use frontmatter_spans::{FrontMatter, detect};
use rayon::prelude::*;
use slopvac_context::{TagIndex, TagRegistry};
use slopvac_diagrams::{Extraction, Format as DiagramFormat, extract};
use slopvac_model::{
    AiSignal, BomMode, CategoryId, ContextTag, Contexts, Document, Edit, Extent, Finding, Fix,
    KebabName, LineCol, NonEmptyString, Notice, NoticeLevel, PackId, PositionError, RuleId,
    RuleProvenance, Severity, SourceIndex, Span,
};
use slopvac_parse::{BlockKind, Document as Parsed, ParseError, parse_markdown, parse_mdx};
use slopvac_rules::{LoadedRule, Match, MatchKind, RuleSet, Scope};
use slopvac_text::{TextError, TextType};

use crate::diagrams::{DiagramChecker, DiagramHit, DiagramInput, DiagramRule};
use crate::directives::{Directives, Lines, Target, scan};
use crate::exceptions::{Detectors, MatchSite};
use crate::matching::{
    LazyReplacements, allowlisted, format_message, is_all_caps, is_single_replacement,
};
use crate::metrics::{self, Subject};
use crate::prepare::{Prepared, Unit};
use crate::rst::{RstAdapter, RstError, RstOptions};
use crate::score::{CategoryInfo, score_document};
use crate::settings::{Level, Settings};
use crate::skip::{self, SkipRanges};
use crate::tagging;

/// The id of the finding for a directive that does not hold.
const INVALID_SUPPRESSION: &str = "meta.invalid-suppression";
/// The pack that owns the findings of the engine itself.
const ENGINE_PACK: &str = "core";

/// What kind of file an [`Input`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Kind {
    /// Markdown.
    Markdown,
    /// MDX.
    Mdx,
    /// reStructuredText, read through docutils (see [`crate::rst`]).
    Rst,
    /// A diagram file. Only diagram rules read it.
    Diagram(DiagramFormat),
}

/// One file to lint.
#[derive(Debug, Clone)]
pub struct Input<'a> {
    /// The path, as shown in the report.
    pub path: &'a str,
    /// The file text.
    pub source: &'a str,
    /// The kind of file.
    pub kind: Kind,
    /// How a leading byte order mark counts in columns.
    pub bom: BomMode,
}

impl<'a> Input<'a> {
    /// A Markdown file read from disk.
    #[must_use]
    pub const fn markdown(path: &'a str, source: &'a str) -> Self {
        Self {
            path,
            source,
            kind: Kind::Markdown,
            bom: BomMode::Disk,
        }
    }

    /// An MDX file read from disk.
    #[must_use]
    pub const fn mdx(path: &'a str, source: &'a str) -> Self {
        Self {
            kind: Kind::Mdx,
            ..Self::markdown(path, source)
        }
    }

    /// An RST file read from disk.
    #[must_use]
    pub const fn rst(path: &'a str, source: &'a str) -> Self {
        Self {
            kind: Kind::Rst,
            ..Self::markdown(path, source)
        }
    }

    /// A diagram file read from disk.
    #[must_use]
    pub const fn diagram(path: &'a str, source: &'a str, format: DiagramFormat) -> Self {
        Self {
            kind: Kind::Diagram(format),
            ..Self::markdown(path, source)
        }
    }
}

/// Why a document could not be linted.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum EngineError {
    /// The document did not parse.
    #[error(transparent)]
    Parse(#[from] ParseError),
    /// A pattern failed while matching, for example on a match limit.
    #[error(transparent)]
    Text(#[from] TextError),
    /// A context tagger failed.
    #[error("context tagger failed: {0}")]
    Context(String),
    /// A computed position was not valid for the file.
    #[error(transparent)]
    Position(#[from] PositionError),
    /// A diagram checker returned a range that is reversed or outside the file and the
    /// prose it was given.
    #[error("diagram checker returned a bad range: {0}")]
    BadRange(String),
    /// A value the engine built broke a model invariant. This is a bug.
    #[error("internal error: {0}")]
    Internal(String),
}

/// A rule that runs.
struct Active {
    set: usize,
    category: usize,
    rule: usize,
    severity: Severity,
    replacements: LazyReplacements,
}

/// A diagram rule that runs.
struct ActiveDiagram {
    severity: Severity,
}

/// Where a rule id points.
enum Catalog {
    Loaded(usize),
    Diagram,
}

/// The engine.
pub struct Engine {
    sets: Vec<RuleSet>,
    settings: Settings,
    detectors: Detectors,
    tagger: TagRegistry,
    checkers: Vec<Box<dyn DiagramChecker>>,
    /// Lexical rules that run.
    active: Vec<Active>,
    /// Metric rules that run.
    metrics: Vec<Active>,
    /// Diagram rules that run, by id.
    diagram_rules: HashMap<RuleId, ActiveDiagram>,
    /// Every known rule id.
    catalog: HashMap<String, Catalog>,
    /// Ids of rules that run, including the ones that produce no finding here.
    running: HashSet<String>,
    /// Notices every document carries.
    notices: Vec<Notice>,
    categories: std::collections::BTreeMap<CategoryId, CategoryInfo>,
    /// How RST files are read, and the adapter built from it. The adapter finds its
    /// interpreter on the first file.
    rst_options: RstOptions,
    rst: RstAdapter,
}

fn level_of(settings: &Settings, id: &RuleId, category: &CategoryId, shipped: Severity) -> Level {
    settings
        .rule_levels
        .get(id)
        .copied()
        .or_else(|| settings.categories.get(category).and_then(|d| d.level))
        .unwrap_or(Level::On(shipped))
}

fn notice(code: &str, level: NoticeLevel, message: String) -> Result<Notice, EngineError> {
    Ok(Notice {
        code: KebabName::new(code).map_err(|e| EngineError::Internal(e.to_string()))?,
        level,
        message: NonEmptyString::new(message).map_err(|e| EngineError::Internal(e.to_string()))?,
        path: None,
    })
}

impl Engine {
    /// An engine for `sets` under `settings`, with the built-in taggers and detectors and
    /// no diagram checker.
    ///
    /// Rules of kinds the engine does not run yet (structure checks, vocabulary and
    /// capitalization, and metrics it does not measure) are skipped, and each document says
    /// so in an `info` notice.
    pub fn new(sets: Vec<RuleSet>, settings: Settings) -> Result<Self, EngineError> {
        let tagger =
            slopvac_context::builtin_registry().map_err(|e| EngineError::Context(e.to_string()))?;
        let mut engine = Self {
            sets,
            settings,
            detectors: Detectors::builtin(),
            tagger,
            checkers: Vec::new(),
            active: Vec::new(),
            metrics: Vec::new(),
            diagram_rules: HashMap::new(),
            catalog: HashMap::new(),
            running: HashSet::new(),
            notices: Vec::new(),
            categories: std::collections::BTreeMap::new(),
            rst_options: RstOptions::from_env(),
            rst: RstAdapter::new(RstOptions::from_env()),
        };
        engine.resolve()?;
        Ok(engine)
    }

    /// Replace the exception detectors.
    #[must_use]
    pub fn with_detectors(mut self, detectors: Detectors) -> Self {
        self.detectors = detectors;
        self
    }

    /// Replace the context taggers.
    #[must_use]
    pub fn with_taggers(mut self, tagger: TagRegistry) -> Self {
        self.tagger = tagger;
        self
    }

    /// Replace how RST files are read: the interpreter, the project roots it must stay
    /// out of, and the limits. The default reads `SLOPVAC_RST_CONVERTER`.
    #[must_use]
    pub fn with_rst(mut self, options: RstOptions) -> Self {
        self.rst = RstAdapter::new(options.clone());
        self.rst_options = options;
        self
    }

    /// Tell the engine where the project is: the repository and the targets of the run.
    /// An RST interpreter is never taken from inside these directories. Until they are
    /// given, no interpreter is found on `PATH` and RST files are an incomplete check.
    #[must_use]
    pub fn with_project_roots(mut self, roots: Vec<std::path::PathBuf>) -> Self {
        self.rst_options.project_roots = roots;
        self.rst = RstAdapter::new(self.rst_options.clone());
        self
    }

    /// Add a checker for diagram rules.
    #[must_use]
    pub fn with_diagram_checker(mut self, checker: Box<dyn DiagramChecker>) -> Self {
        for DiagramRule { id, severity } in checker.rules() {
            let category = category_of(&id);
            let key = id.as_str().to_owned();
            self.catalog.insert(key.clone(), Catalog::Diagram);
            if let Level::On(severity) = level_of(&self.settings, &id, &category, severity) {
                self.running.insert(key);
                self.diagram_rules.insert(id, ActiveDiagram { severity });
            }
        }
        self.checkers.push(checker);
        self
    }

    /// Decide which rules run: level, `applies_to`, and whether the engine runs the kind.
    fn resolve(&mut self) -> Result<(), EngineError> {
        let mut unrun: std::collections::BTreeMap<&'static str, usize> =
            std::collections::BTreeMap::new();
        for (s, set) in self.sets.iter().enumerate() {
            for (c, category) in set.categories().iter().enumerate() {
                let meta = self.settings.categories.get(category.id());
                let info = CategoryInfo {
                    weight: match meta.and_then(|d| d.level) {
                        Some(Level::Off) => 0.0,
                        _ => meta
                            .and_then(|d| d.weight)
                            .unwrap_or(category.weight().get()),
                    },
                    budget: meta.and_then(|d| d.max_per_100_words).or_else(|| {
                        category
                            .max_per_100_words()
                            .and_then(|m| {
                                m.iter()
                                    .find(|(p, _)| p.as_str() == self.settings.profile.as_str())
                            })
                            .map(|(_, d)| d.get())
                    }),
                };
                self.categories.insert(category.id().clone(), info);
                for (r, loaded) in category.rules().iter().enumerate() {
                    let rule = loaded.rule();
                    let Level::On(severity) = level_of(
                        &self.settings,
                        loaded.id(),
                        loaded.category(),
                        rule.severity,
                    ) else {
                        continue;
                    };
                    if let Some(applies) = &rule.applies_to {
                        let mode = self.settings.mode.apply_mode();
                        if applies.modes.as_ref().is_some_and(|m| !m.contains(&mode))
                            || applies
                                .genres
                                .as_ref()
                                .is_some_and(|g| !g.contains(&self.settings.genre.id))
                        {
                            continue;
                        }
                    }
                    self.running.insert(loaded.id().as_str().to_owned());
                    match loaded.kind() {
                        MatchKind::Tokens | MatchKind::Pattern | MatchKind::Substitution => {
                            self.active.push(Active {
                                set: s,
                                category: c,
                                rule: r,
                                severity,
                                replacements: LazyReplacements::default(),
                            });
                        }
                        MatchKind::Judgement => {}
                        MatchKind::Metric => {
                            let supported = matches!(
                                &rule.match_,
                                Match::Metric(m) if metrics::supported(m.metric.as_str())
                            );
                            if supported {
                                self.metrics.push(Active {
                                    set: s,
                                    category: c,
                                    rule: r,
                                    severity,
                                    replacements: LazyReplacements::default(),
                                });
                            } else {
                                *unrun.entry("metric").or_default() += 1;
                            }
                        }
                        MatchKind::Structure => *unrun.entry("structure").or_default() += 1,
                        MatchKind::Vocabulary => *unrun.entry("vocabulary").or_default() += 1,
                        MatchKind::Capitalization => {
                            *unrun.entry("capitalization").or_default() += 1;
                        }
                    }
                }
            }
        }
        for (s, set) in self.sets.iter().enumerate() {
            for rule in set.rules() {
                self.catalog
                    .insert(rule.id().as_str().to_owned(), Catalog::Loaded(s));
            }
        }
        for (kind, count) in unrun {
            self.notices.push(notice(
                "rule-kind-not-implemented",
                NoticeLevel::Info,
                format!(
                    "{count} active {kind} rule(s) did not run: the engine does not run {kind} \
                     rules yet"
                ),
            )?);
        }
        Ok(())
    }

    /// The loaded rule for a qualified id.
    fn loaded(&self, id: &str) -> Option<&LoadedRule> {
        let Catalog::Loaded(set) = self.catalog.get(id)? else {
            return None;
        };
        let rule_id = RuleId::new(id).ok()?;
        self.sets[*set].get(&rule_id)
    }

    /// Lint many inputs in parallel. Results keep the order of `inputs`.
    #[must_use]
    pub fn lint_all(&self, inputs: &[Input<'_>]) -> Vec<Result<Document, EngineError>> {
        inputs.par_iter().map(|input| self.lint(input)).collect()
    }

    /// Lint one input.
    pub fn lint(&self, input: &Input<'_>) -> Result<Document, EngineError> {
        // Front matter is metadata, not prose: the parser reads only what follows it.
        let front = match input.kind {
            Kind::Diagram(_) | Kind::Rst => None,
            _ => detect(input.source),
        };
        let body_start = front.as_ref().map_or(0, |b| b.body_start);
        let front_span = front
            .as_ref()
            .map(|b| Span::new(0, b.body_start).expect("0 <= body start"))
            .filter(|s| !s.is_empty());
        let body = Span::new(body_start, input.source.len()).expect("body starts inside the file");
        let front_matter = front.and_then(|b| FrontMatter::parse_block(input.source, b).ok());
        let mut extra = Vec::new();
        let parsed = match input.kind {
            Kind::Markdown => parse_markdown(input.source, body)?,
            Kind::Mdx => parse_mdx(input.source, body)?,
            Kind::Rst => self.parse_rst(input, &mut extra)?,
            Kind::Diagram(_) => Parsed::default(),
        };
        let prepared = Prepared::new(input.source, &parsed)?;
        let tags = match input.kind {
            Kind::Diagram(_) => TagIndex::default(),
            _ => tagging::tag(
                &self.tagger,
                input.path,
                input.source,
                &parsed,
                front_matter.as_ref(),
            )
            .map_err(|e| EngineError::Context(e.to_string()))?,
        };
        let lines = Lines::new(input.source);
        let directives = match input.kind {
            Kind::Diagram(_) => Directives::default(),
            _ => scan(&parsed, &lines, front_span, |id| {
                match self.catalog.get(id) {
                    None => Target::Unknown,
                    Some(_) if !self.running.contains(id) => Target::Inactive,
                    Some(Catalog::Loaded(_)) => Target::Active(
                        self.loaded(id)
                            .map(|r| {
                                r.rule()
                                    .exceptions()
                                    .iter()
                                    .map(|e| e.name.as_str())
                                    .collect()
                            })
                            .unwrap_or_default(),
                    ),
                    Some(Catalog::Diagram) => Target::Active(Vec::new()),
                }
            }),
        };
        let subject = Subject::new(&prepared, &parsed, &lines, front_span);
        let run = Run {
            engine: self,
            input,
            parsed: &parsed,
            prepared: &prepared,
            tags: &tags,
            lines: &lines,
            directives: &directives,
            front_matter: front_span,
            subject: &subject,
            index: SourceIndex::new(input.source, input.bom),
        };
        let mut findings = run.lexical()?;
        findings.extend(run.metrics()?);
        findings.extend(run.diagrams()?);
        findings.extend(run.invalid_directives()?);
        findings.sort_by(|a, b| (a.start, a.rule_id.as_str()).cmp(&(b.start, b.rule_id.as_str())));
        Ok(self.document(input, &prepared, &tags, findings, extra))
    }

    /// Read an RST file. A file that cannot be read is an empty document with a warning
    /// notice, so the run is incomplete and the file is never skipped silently.
    fn parse_rst(&self, input: &Input<'_>, extra: &mut Vec<Notice>) -> Result<Parsed, EngineError> {
        let make = |code: &str, level, message: String| -> Result<Notice, EngineError> {
            let mut notice = notice(code, level, message)?;
            notice.path = Some(input.path.to_owned());
            Ok(notice)
        };
        match self.rst.parse(input.source) {
            Ok(parsed) => {
                for n in parsed.notices {
                    let level = if n.warning {
                        NoticeLevel::Warning
                    } else {
                        NoticeLevel::Info
                    };
                    extra.push(make(n.code, level, n.message)?);
                }
                Ok(parsed.document)
            }
            Err(error) => {
                let code = if error.is_missing() {
                    "rst-converter-missing"
                } else {
                    "rst-unchecked"
                };
                extra.push(make(
                    code,
                    NoticeLevel::Warning,
                    RstError::message(&error, &[std::path::Path::new(input.path)]),
                )?);
                Ok(Parsed::default())
            }
        }
    }

    fn document(
        &self,
        input: &Input<'_>,
        prepared: &Prepared<'_>,
        tags: &TagIndex,
        findings: Vec<Finding>,
        extra: Vec<Notice>,
    ) -> Document {
        let words = prepared.words();
        let mut unchecked = self.notices.clone();
        unchecked.extend(extra);
        let scoring = score_document(
            &findings,
            words,
            &self.categories,
            &self.settings.thresholds,
            &unchecked,
        );
        Document {
            path: NonEmptyString::new(input.path).unwrap_or_else(|_| {
                NonEmptyString::new("<stdin>").expect("a literal is not empty")
            }),
            profile: self.settings.profile.clone(),
            genre: self.settings.genre.clone(),
            contexts_seen: contexts(tags.kinds_seen()),
            config_sources: self.settings.config_sources.clone(),
            words,
            sentences: prepared.sentence_count(),
            paragraphs: prepared.paragraph_count(),
            findings,
            categories: scoring.categories,
            total_findings: scoring.total_findings,
            errors: scoring.errors,
            warnings: scoring.warnings,
            suggestions: scoring.suggestions,
            per_100_words: scoring.per_100_words,
            score: scoring.score,
            gate: scoring.gate,
            unchecked,
            ai_register: scoring.ai_register,
            prose: scoring.prose,
        }
    }
}

fn category_of(id: &RuleId) -> CategoryId {
    let local = id.local();
    CategoryId::new(local.split_once('.').map_or(local, |(c, _)| c))
        .unwrap_or_else(|_| CategoryId::new("meta").expect("meta is a category id"))
}

fn contexts(kinds: Vec<&str>) -> Contexts {
    Contexts::new(
        kinds
            .into_iter()
            .filter_map(|k| ContextTag::new(k).ok())
            .collect(),
    )
    .unwrap_or_default()
}

/// The state of one document run.
struct Run<'a> {
    engine: &'a Engine,
    input: &'a Input<'a>,
    parsed: &'a Parsed,
    prepared: &'a Prepared<'a>,
    tags: &'a TagIndex,
    lines: &'a Lines<'a>,
    directives: &'a Directives,
    front_matter: Option<Span>,
    subject: &'a Subject<'a>,
    index: SourceIndex<'a>,
}

/// A paragraph next to a fenced diagram, and where a checker sees it.
struct NearbyProse<'a> {
    /// Where the paragraph's text starts in the coordinates the checker gets.
    base: usize,
    block: &'a slopvac_parse::Block,
    projection: &'a slopvac_parse::Projection,
}

impl NearbyProse<'_> {
    fn text(&self) -> &str {
        self.projection.text()
    }
}

/// A finding before it gets its position.
struct Candidate<'r> {
    rule_id: &'r RuleId,
    pack: &'r PackId,
    category: CategoryId,
    severity: Severity,
    message: String,
    span: Span,
    matched_text: String,
    ai_signal: AiSignal,
    fix: Option<Fix>,
    extra_context: Option<&'static str>,
}

impl Run<'_> {
    fn finding(&self, c: Candidate<'_>) -> Result<Finding, EngineError> {
        let start = self.index.line_col(c.span.start())?;
        let end = self.index.line_col(c.span.end())?;
        let mut kinds = self.tags.kinds_at(&(c.span.start()..c.span.end()));
        if let Some(extra) = c.extra_context
            && !kinds.contains(&extra)
        {
            kinds.push(extra);
        }
        Ok(Finding {
            rule_id: c.rule_id.clone(),
            pack: c.pack.clone(),
            category: c.category,
            severity: c.severity,
            message: NonEmptyString::new(c.message)
                .map_err(|e| EngineError::Internal(e.to_string()))?,
            start,
            extent: Some(Extent { span: c.span, end }),
            contexts: contexts(kinds),
            ai_signal: c.ai_signal,
            matched_text: c.matched_text,
            fix: c.fix,
            provenance: Some(RuleProvenance {
                pack: c.pack.clone(),
                version: None,
                source: None,
            }),
        })
    }

    /// Whether a directive covers a finding of `id` that starts at `span`.
    fn suppressed(&self, id: &str, exceptions: &[&str], span: Span) -> bool {
        self.directives
            .suppressed(id, exceptions, self.lines.line_of(span.start()))
    }

    fn lexical(&self) -> Result<Vec<Finding>, EngineError> {
        let engine = self.engine;
        if matches!(self.input.kind, Kind::Diagram(_)) {
            // A diagram file is not prose. Only diagram rules read it.
            return Ok(Vec::new());
        }
        let inline_code = skip::build(self.parsed, &[], None, true);
        let mut out = Vec::new();
        for active in &engine.active {
            let set = &engine.sets[active.set];
            let loaded = &set.categories()[active.category].rules()[active.rule];
            let rule = loaded.rule();
            let exclusions;
            let skip: &SkipRanges = match (&rule.exclude_blocks, rule.scope) {
                (Some(kinds), scope) => {
                    exclusions =
                        skip::build(self.parsed, kinds, self.front_matter, scope != Scope::Raw);
                    &exclusions
                }
                (None, Scope::Raw) => {
                    exclusions = SkipRanges::default();
                    &exclusions
                }
                (None, _) => &inline_code,
            };
            let exceptions: Vec<&str> = rule.exceptions().iter().map(|e| e.name.as_str()).collect();
            let detectors: Vec<&str> = rule
                .exceptions()
                .iter()
                .filter_map(|e| e.detector.as_ref().map(slopvac_rules::Slug::as_str))
                .collect();
            for unit in self.prepared.units(rule.scope)? {
                for range in loaded.find_all(unit.text)? {
                    if let Some(c) =
                        self.candidate(active, set, loaded, &unit, &range, skip, &detectors)?
                    {
                        if self.suppressed(loaded.id().as_str(), &exceptions, c.span) {
                            continue;
                        }
                        out.push(self.finding(c)?);
                    }
                }
            }
        }
        Ok(out)
    }

    /// Whether the rule's context gate or an exception detector drops a match: `range` in
    /// `text`, at `span` in the source.
    fn guarded(
        &self,
        rule: &slopvac_rules::Rule,
        detectors: &[&str],
        text: &str,
        range: &Range<usize>,
        span: Span,
    ) -> bool {
        let gate = rule.contexts.as_ref();
        let kinds = if gate.is_some() || !detectors.is_empty() {
            self.tags.kinds_at(&(span.start()..span.end()))
        } else {
            Vec::new()
        };
        if let Some(gate) = gate {
            let has = |tags: &Option<slopvac_rules::UniqueVec<ContextTag>>| {
                tags.as_ref()
                    .map(|t| t.iter().any(|t| kinds.contains(&t.as_str())))
            };
            if has(&gate.skip) == Some(true) || has(&gate.only) == Some(false) {
                return true;
            }
        }
        let site = MatchSite {
            text,
            range,
            span,
            document: self.parsed,
            tags: &kinds,
        };
        detectors
            .iter()
            .any(|d| self.engine.detectors.detects(d, &site))
    }

    /// Findings of metric rules: one per sentence or paragraph over its threshold, or one for
    /// the document. A finding has a line and column 1 and no source range.
    fn metrics(&self) -> Result<Vec<Finding>, EngineError> {
        let engine = self.engine;
        if matches!(self.input.kind, Kind::Diagram(_)) {
            return Ok(Vec::new());
        }
        let inline_code = skip::build(self.parsed, &[], None, true);
        let mut out = Vec::new();
        for active in &engine.metrics {
            let set = &engine.sets[active.set];
            let loaded = &set.categories()[active.category].rules()[active.rule];
            let rule = loaded.rule();
            let Match::Metric(metric) = &rule.match_ else {
                continue;
            };
            let exceptions: Vec<&str> = rule.exceptions().iter().map(|e| e.name.as_str()).collect();
            let detectors: Vec<&str> = rule
                .exceptions()
                .iter()
                .filter_map(|e| e.detector.as_ref().map(slopvac_rules::Slug::as_str))
                .collect();
            let exclusions;
            let skip: &SkipRanges = match &rule.exclude_blocks {
                Some(kinds) => {
                    exclusions = skip::build(self.parsed, kinds, self.front_matter, true);
                    &exclusions
                }
                None => &inline_code,
            };
            for hit in self.subject.measure(metric, rule.text_type)? {
                // The guards read the measured sentence or paragraph. A document-wide
                // measurement has no unit to guard.
                if let Some(unit) = &hit.unit
                    && (skip.hides(unit.span)
                        || self.guarded(rule, &detectors, unit.text, &unit.range, unit.span))
                {
                    continue;
                }
                let at = Span::empty(hit.at);
                if self.suppressed(loaded.id().as_str(), &exceptions, at) {
                    continue;
                }
                let line = self.index.line_col(hit.at)?.line();
                out.push(Finding {
                    rule_id: loaded.id().clone(),
                    pack: set.pack().clone(),
                    category: loaded.category().clone(),
                    severity: active.severity,
                    message: NonEmptyString::new(format_message(
                        rule.message.as_str(),
                        &hit.shown,
                        &hit.limit,
                    ))
                    .map_err(|e| EngineError::Internal(e.to_string()))?,
                    start: LineCol::new(line, 1)
                        .map_err(|e| EngineError::Internal(e.to_string()))?,
                    extent: None,
                    contexts: Contexts::default(),
                    ai_signal: rule.ai_signal,
                    matched_text: String::new(),
                    fix: None,
                    provenance: Some(RuleProvenance {
                        pack: set.pack().clone(),
                        version: None,
                        source: None,
                    }),
                });
            }
        }
        Ok(out)
    }

    /// The finding for one match, or `None` when a filter drops it.
    #[allow(clippy::too_many_arguments)]
    fn candidate<'s>(
        &self,
        active: &Active,
        set: &'s RuleSet,
        loaded: &'s LoadedRule,
        unit: &Unit<'_>,
        range: &Range<usize>,
        skip: &SkipRanges,
        detectors: &[&str],
    ) -> Result<Option<Candidate<'s>>, EngineError> {
        let rule = loaded.rule();
        if rule.text_type != TextType::Any {
            let at = unit.base() + range.start;
            let typed = unit
                .sentences
                .and_then(|s| s.iter().find(|s| s.start <= at && at < s.end));
            if typed.is_none_or(|s| s.text_type != rule.text_type) {
                return Ok(None);
            }
        }
        if allowlisted(loaded, unit.text, range) {
            return Ok(None);
        }
        let matched = &unit.text[range.clone()];
        let match_all_caps = loaded
            .matcher()
            .is_some_and(slopvac_rules::LexicalMatcher::match_all_caps);
        if !match_all_caps && is_all_caps(matched) {
            return Ok(None);
        }
        let span = unit.span(range);
        if skip.hides(span) {
            return Ok(None);
        }
        if self.guarded(rule, detectors, unit.text, range, span) {
            return Ok(None);
        }
        // A match that spans masked text (inline code) reports the source text.
        let source_text = &self.input.source[span.start()..span.end()];
        let shown = if matched.contains('\0') {
            source_text
        } else {
            matched
        };
        let candidate = match &rule.match_ {
            Match::Substitution(_) => active
                .replacements
                .get(loaded)
                .find(shown)
                .map(str::to_owned),
            _ => None,
        };
        let fix = match candidate.as_deref() {
            Some(text) if is_single_replacement(text) && source_text == shown => Some(
                Fix::new(vec![Edit {
                    span,
                    text: text.to_owned(),
                }])
                .map_err(|e| EngineError::Internal(e.to_string()))?,
            ),
            _ => None,
        };
        Ok(Some(Candidate {
            rule_id: loaded.id(),
            pack: set.pack(),
            category: loaded.category().clone(),
            severity: active.severity,
            message: format_message(
                rule.message.as_str(),
                shown,
                candidate.as_deref().unwrap_or(""),
            ),
            span,
            matched_text: shown.to_owned(),
            ai_signal: rule.ai_signal,
            fix,
            extra_context: None,
        }))
    }

    /// Findings of diagram rules, over diagram files and fenced diagram blocks.
    fn diagrams(&self) -> Result<Vec<Finding>, EngineError> {
        let engine = self.engine;
        if engine.checkers.is_empty() {
            return Ok(Vec::new());
        }
        let mut diagrams: Vec<(Extraction, Vec<NearbyProse<'_>>)> = Vec::new();
        match self.input.kind {
            Kind::Diagram(format) => {
                diagrams.push((extract(format, self.input.source, 0), Vec::new()));
            }
            _ => {
                for block in &self.parsed.blocks {
                    let BlockKind::CodeBlock {
                        lang: Some(lang),
                        body,
                        fenced: true,
                        ..
                    } = &block.kind
                    else {
                        continue;
                    };
                    let Some(format) = DiagramFormat::from_lang(lang) else {
                        continue;
                    };
                    let text = &self.input.source[body.start()..body.end()];
                    diagrams.push((
                        extract(format, text, body.start()),
                        self.nearby_prose(block.span),
                    ));
                }
            }
        }
        let mut out = Vec::new();
        for (extraction, nearby) in &diagrams {
            let prose: Vec<slopvac_diagrams::ProseBlock<'_>> = nearby
                .iter()
                .map(|n| slopvac_diagrams::ProseBlock {
                    span: n.base..n.base + n.text().len(),
                    text: n.text(),
                })
                .collect();
            let input = DiagramInput {
                source: self.input.source,
                extraction,
                prose: &prose,
            };
            for checker in &engine.checkers {
                for hit in checker.check(&input) {
                    if let Some(finding) = self.diagram_finding(&hit, nearby)? {
                        out.push(finding);
                    }
                }
            }
        }
        Ok(out)
    }

    /// The paragraphs next to a fenced block: the last one before it and the first after.
    ///
    /// Their text is projected prose, so a checker's offsets into it are not source bytes.
    /// Each paragraph gets a base past the end of the source and the paragraphs follow each
    /// other, one byte apart. [`Self::source_span`] maps a hit back through the projection.
    fn nearby_prose(&self, fence: Span) -> Vec<NearbyProse<'_>> {
        let paragraph = |b: &&slopvac_parse::Block| {
            matches!(b.kind, BlockKind::Paragraph) && b.projection.is_some()
        };
        let blocks = &self.parsed.blocks;
        let before = blocks
            .iter()
            .rfind(|b| paragraph(b) && b.span.end() <= fence.start());
        let after = blocks
            .iter()
            .filter(paragraph)
            .find(|b| b.span.start() >= fence.end());
        let mut base = self.input.source.len() + 1;
        let mut out = Vec::new();
        for block in [before, after].into_iter().flatten() {
            let Some(projection) = &block.projection else {
                continue;
            };
            out.push(NearbyProse {
                base,
                block,
                projection,
            });
            base += projection.text().len() + 1;
        }
        out
    }

    /// The source span of a checker's hit: a source range as it is, or a range in the
    /// projected prose mapped back to source bytes.
    fn source_span(
        &self,
        hit: &Range<usize>,
        nearby: &[NearbyProse<'_>],
    ) -> Result<Span, EngineError> {
        let bad = |why: &str| EngineError::BadRange(format!("{hit:?}: {why}"));
        if hit.start > hit.end {
            return Err(bad("the range is reversed"));
        }
        let span = if hit.start <= self.input.source.len() {
            if hit.end > self.input.source.len() {
                return Err(bad("the range ends past the file"));
            }
            Span::new(hit.start, hit.end).map_err(|e| bad(&e.to_string()))?
        } else {
            let prose = nearby
                .iter()
                .find(|n| n.base <= hit.start && hit.end <= n.base + n.text().len())
                .ok_or_else(|| bad("the range is outside the prose"))?;
            let (from, to) = (hit.start - prose.base, hit.end - prose.base);
            if from == 0 && to == prose.text().len() {
                prose.block.span
            } else if prose.text().is_char_boundary(from) && prose.text().is_char_boundary(to) {
                prose.projection.span_for_bytes(from, to)
            } else {
                return Err(bad("the range splits a character"));
            }
        };
        self.index.check_span(span)?;
        Ok(span)
    }

    fn diagram_finding(
        &self,
        hit: &DiagramHit,
        nearby: &[NearbyProse<'_>],
    ) -> Result<Option<Finding>, EngineError> {
        let Some(active) = self.engine.diagram_rules.get(&hit.rule) else {
            return Ok(None);
        };
        let span = self.source_span(&hit.span, nearby)?;
        if self.suppressed(hit.rule.as_str(), &[], span) {
            return Ok(None);
        }
        let pack =
            PackId::new(hit.rule.pack()).map_err(|e| EngineError::Internal(e.to_string()))?;
        let rule_id = hit.rule.clone();
        let matched = self.input.source[span.start()..span.end()].to_owned();
        self.finding(Candidate {
            rule_id: &rule_id,
            pack: &pack,
            category: category_of(&rule_id),
            severity: active.severity,
            message: hit.message.clone(),
            span,
            matched_text: matched,
            ai_signal: AiSignal::None,
            fix: None,
            extra_context: Some(slopvac_diagrams::CONTEXT),
        })
        .map(Some)
    }

    /// One `meta.invalid-suppression` error for each directive that does not hold.
    fn invalid_directives(&self) -> Result<Vec<Finding>, EngineError> {
        let pack = PackId::new(ENGINE_PACK).map_err(|e| EngineError::Internal(e.to_string()))?;
        let rule_id = RuleId::new(format!("{ENGINE_PACK}:{INVALID_SUPPRESSION}"))
            .map_err(|e| EngineError::Internal(e.to_string()))?;
        self.directives
            .invalid
            .iter()
            .map(|invalid| {
                self.finding(Candidate {
                    rule_id: &rule_id,
                    pack: &pack,
                    category: CategoryId::new("meta")
                        .map_err(|e| EngineError::Internal(e.to_string()))?,
                    severity: Severity::Error,
                    message: invalid.message.clone(),
                    span: invalid.span,
                    matched_text: self.input.source[invalid.span.start()..invalid.span.end()]
                        .to_owned(),
                    ai_signal: AiSignal::None,
                    fix: None,
                    extra_context: None,
                })
            })
            .collect()
    }
}
