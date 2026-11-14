//! Genres as data: definitions, a registry with an `extends` chain, detection with
//! reasons, and resolution of the genre for one document.
//!
//! STUB(slopvac-8l4.5.12): field names follow the bead text of slopvac-8l4.2.4 until the
//! genre schema is published.

use std::collections::{BTreeMap, BTreeSet};

use globset::{Glob, GlobSetBuilder};
use serde::Deserialize;

use crate::{BlockKind, DocumentView, Error, FrontmatterCondition, Pattern};

const SEVERITIES: [&str; 4] = ["off", "suggestion", "warning", "error"];

/// One detection signal. Exactly one of `heading`, `path` or `frontmatter` is set.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DetectSignal {
    /// Pattern for any heading in the document.
    pub heading: Option<String>,
    /// Pattern for the file path.
    pub path: Option<String>,
    /// Front matter key and value pattern.
    pub frontmatter: Option<FrontmatterCondition>,
    /// Score added when the signal matches.
    pub weight: f32,
}

/// A genre definition as written in a pack.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenreSpec {
    /// Genre id, kebab-case.
    pub id: String,
    /// Parent genre. Its tolerances and contexts apply first.
    pub extends: Option<String>,
    /// An abstract genre is never detected or chosen by default.
    #[serde(default, rename = "abstract")]
    pub is_abstract: bool,
    /// One-line purpose.
    #[serde(default)]
    pub description: String,
    /// Profile name the genre implies.
    pub profile: Option<String>,
    /// Score a genre needs to be detected.
    #[serde(default = "default_threshold")]
    pub threshold: f32,
    /// Detection signals.
    #[serde(default)]
    pub detect: Vec<DetectSignal>,
    /// Severity per rule id: `off`, `suggestion`, `warning` or `error`.
    #[serde(default)]
    pub tolerances: BTreeMap<String, String>,
    /// Context tags the genre declares for the whole document, such as `security`.
    #[serde(default)]
    pub contexts: Vec<String>,
    /// Snippet for `slopvac prime write`.
    pub prime: Option<String>,
}

fn default_threshold() -> f32 {
    3.0
}

/// Parse a YAML list of genre definitions.
pub fn parse_genres(yaml: &str) -> Result<Vec<GenreSpec>, Error> {
    serde_saphyr::from_str(yaml).map_err(|e| Error::Parse(e.to_string()))
}

struct CompiledSignal {
    kind: SignalKind,
    weight: f32,
}

enum SignalKind {
    Heading(Pattern),
    Path(Pattern),
    Frontmatter(String, Option<Pattern>),
}

struct Genre {
    spec: GenreSpec,
    signals: Vec<CompiledSignal>,
}

/// Why a genre was chosen, or how well a candidate scored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reason {
    /// Kebab-case signal name, as in the lint report.
    pub signal: String,
    /// What matched.
    pub detail: String,
}

/// How the genre was chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenreSource {
    /// Front matter or a config assignment.
    Explicit,
    /// Detection signals.
    Detected,
    /// The fallback genre.
    Default,
}

/// The genre applied to one document.
#[derive(Debug, Clone, PartialEq)]
pub struct GenreResult {
    /// Genre id.
    pub id: String,
    /// How it was chosen.
    pub source: GenreSource,
    /// Signals behind the choice.
    pub reasons: Vec<Reason>,
    /// Detection score for every concrete genre, best first.
    pub scores: Vec<(String, f32)>,
}

/// A `[[genres]]` block reduced to what assignment needs.
#[derive(Debug, Clone)]
pub struct GenreAssignment {
    /// Globs relative to the config directory.
    pub files: Vec<String>,
    /// Genre id.
    pub genre: String,
}

/// Inputs to [`GenreRegistry::resolve`] besides the document.
#[derive(Debug, Clone, Default)]
pub struct ResolveOptions<'a> {
    /// `[[genres]]` blocks in file order. The last matching block wins.
    pub assignments: &'a [GenreAssignment],
    /// Path relative to the config directory, for glob matching. Defaults to the document path.
    pub relative_path: Option<&'a str>,
}

/// Validated set of genres.
pub struct GenreRegistry {
    genres: Vec<Genre>,
    default: String,
}

impl GenreRegistry {
    /// Validate and compile `specs`. `default` is the fallback genre id.
    pub fn new(specs: Vec<GenreSpec>, default: &str) -> Result<Self, Error> {
        let mut seen = BTreeSet::new();
        for s in &specs {
            if !seen.insert(s.id.clone()) {
                return Err(Error::Parse(format!("genre `{}` is defined twice", s.id)));
            }
            for (rule, sev) in &s.tolerances {
                if !SEVERITIES.contains(&sev.as_str()) {
                    return Err(Error::Parse(format!(
                        "genre `{}`: tolerance for `{rule}` is `{sev}`, not one of {SEVERITIES:?}",
                        s.id
                    )));
                }
            }
        }
        for s in &specs {
            if let Some(p) = &s.extends
                && !seen.contains(p)
            {
                return Err(Error::Parse(format!(
                    "genre `{}` extends unknown genre `{p}`",
                    s.id
                )));
            }
        }
        let genres = specs
            .into_iter()
            .map(compile)
            .collect::<Result<Vec<_>, _>>()?;
        let reg = Self {
            genres,
            default: default.to_string(),
        };
        for g in &reg.genres {
            reg.chain(&g.spec.id)?;
        }
        match reg.get(default) {
            Some(g) if !g.spec.is_abstract => Ok(reg),
            _ => Err(Error::UnknownGenre(default.to_string())),
        }
    }

    fn get(&self, id: &str) -> Option<&Genre> {
        self.genres.iter().find(|g| g.spec.id == id)
    }

    /// Genre ids in definition order.
    #[must_use]
    pub fn ids(&self) -> Vec<&str> {
        self.genres.iter().map(|g| g.spec.id.as_str()).collect()
    }

    /// The definition of one genre.
    #[must_use]
    pub fn spec(&self, id: &str) -> Option<&GenreSpec> {
        self.get(id).map(|g| &g.spec)
    }

    /// The genre and its ancestors, root first.
    pub fn chain(&self, id: &str) -> Result<Vec<&GenreSpec>, Error> {
        let mut out = Vec::new();
        let mut cur: Option<&str> = Some(id);
        while let Some(c) = cur {
            let g = self
                .get(c)
                .ok_or_else(|| Error::UnknownGenre(c.to_string()))?;
            if out.iter().any(|s: &&GenreSpec| s.id == g.spec.id) {
                return Err(Error::Parse(format!("genre `{id}` has an `extends` cycle")));
            }
            out.push(&g.spec);
            cur = g.spec.extends.as_deref();
        }
        out.reverse();
        Ok(out)
    }

    /// Severity per rule for a genre, with child values over parent values.
    pub fn tolerances(&self, id: &str) -> Result<BTreeMap<String, String>, Error> {
        let mut out = BTreeMap::new();
        for s in self.chain(id)? {
            out.extend(s.tolerances.clone());
        }
        Ok(out)
    }

    /// Declared context tags for a genre, parent first, without duplicates.
    pub fn contexts(&self, id: &str) -> Result<Vec<String>, Error> {
        let mut out: Vec<String> = Vec::new();
        for s in self.chain(id)? {
            for c in &s.contexts {
                if !out.contains(c) {
                    out.push(c.clone());
                }
            }
        }
        Ok(out)
    }

    /// Profile name from the nearest genre in the chain that sets one.
    pub fn profile(&self, id: &str) -> Result<Option<String>, Error> {
        Ok(self.chain(id)?.iter().rev().find_map(|s| s.profile.clone()))
    }

    /// Choose the genre for a document: front matter, then a config assignment, then
    /// detection, then the default.
    pub fn resolve(
        &self,
        doc: &dyn DocumentView,
        opts: &ResolveOptions<'_>,
    ) -> Result<GenreResult, Error> {
        let candidates = self.score(doc);
        let mut scores: Vec<(String, f32)> = candidates
            .iter()
            .map(|(g, s, _)| (g.spec.id.clone(), *s))
            .collect();
        scores.sort_by(|a, b| b.1.total_cmp(&a.1));
        let explicit = |id: &str, signal: &str, detail: String| -> Result<GenreResult, Error> {
            self.get(id)
                .ok_or_else(|| Error::UnknownGenre(id.to_string()))?;
            Ok(GenreResult {
                id: id.to_string(),
                source: GenreSource::Explicit,
                reasons: vec![Reason {
                    signal: signal.to_string(),
                    detail,
                }],
                scores: scores.clone(),
            })
        };
        if let Some(id) = doc.front_matter("genre") {
            return explicit(id, "front-matter", format!("`genre: {id}`"));
        }
        let rel = opts.relative_path.unwrap_or_else(|| doc.path());
        let mut assigned = None;
        for (i, a) in opts.assignments.iter().enumerate() {
            let mut b = GlobSetBuilder::new();
            for f in &a.files {
                b.add(Glob::new(f).map_err(|e| Error::Parse(format!("glob `{f}`: {e}")))?);
            }
            let set = b.build().map_err(|e| Error::Parse(e.to_string()))?;
            if set.is_match(rel) {
                assigned = Some((i, a));
            }
        }
        if let Some((i, a)) = assigned {
            return explicit(
                &a.genre,
                "path-glob",
                format!("`[[genres]]` block {i} matches `{rel}`"),
            );
        }
        let mut best: Option<&(&Genre, f32, Vec<Reason>)> = None;
        for c in candidates
            .iter()
            .filter(|(g, s, _)| !g.spec.is_abstract && *s >= g.spec.threshold)
        {
            if best.is_none_or(|b| c.1 > b.1) {
                best = Some(c);
            }
        }
        if let Some((g, _, reasons)) = best {
            return Ok(GenreResult {
                id: g.spec.id.clone(),
                source: GenreSource::Detected,
                reasons: reasons.clone(),
                scores,
            });
        }
        Ok(GenreResult {
            id: self.default.clone(),
            source: GenreSource::Default,
            reasons: vec![Reason {
                signal: "default".to_string(),
                detail: format!("no genre reached its threshold; using `{}`", self.default),
            }],
            scores,
        })
    }

    fn score<'a>(&'a self, doc: &dyn DocumentView) -> Vec<(&'a Genre, f32, Vec<Reason>)> {
        let blocks = doc.blocks();
        let headings: Vec<&str> = blocks
            .iter()
            .filter(|b| b.kind == BlockKind::Heading)
            .map(|b| b.text)
            .collect();
        self.genres
            .iter()
            .filter(|g| !g.spec.is_abstract)
            .map(|g| {
                let mut total = 0.0;
                let mut reasons = Vec::new();
                for s in &g.signals {
                    let hit = match &s.kind {
                        SignalKind::Heading(p) => headings
                            .iter()
                            .find(|h| p.is_match(h))
                            .map(|h| ("heading", format!("heading `{h}`"))),
                        SignalKind::Path(p) => p
                            .is_match(doc.path())
                            .then(|| ("path", format!("path `{}`", doc.path()))),
                        SignalKind::Frontmatter(k, p) => doc
                            .front_matter(k)
                            .filter(|v| p.as_ref().is_none_or(|p| p.is_match(v)))
                            .map(|v| ("front-matter", format!("`{k}: {v}`"))),
                    };
                    if let Some((signal, detail)) = hit {
                        total += s.weight;
                        reasons.push(Reason {
                            signal: signal.to_string(),
                            detail: format!("{detail} (+{})", s.weight),
                        });
                    }
                }
                (g, total, reasons)
            })
            .collect()
    }
}

fn compile(spec: GenreSpec) -> Result<Genre, Error> {
    let mut signals = Vec::new();
    for d in &spec.detect {
        let set = usize::from(d.heading.is_some())
            + usize::from(d.path.is_some())
            + usize::from(d.frontmatter.is_some());
        if set != 1 {
            return Err(Error::Parse(format!(
                "genre `{}`: each detect signal needs exactly one of heading, path, frontmatter",
                spec.id
            )));
        }
        if d.weight <= 0.0 {
            return Err(Error::Parse(format!(
                "genre `{}`: signal weight must be positive",
                spec.id
            )));
        }
        let kind = if let Some(h) = &d.heading {
            SignalKind::Heading(Pattern::new_caseless(h)?)
        } else if let Some(p) = &d.path {
            SignalKind::Path(Pattern::new_caseless(p)?)
        } else {
            let f = d.frontmatter.as_ref().expect("checked above");
            SignalKind::Frontmatter(
                f.key.clone(),
                f.matches
                    .as_deref()
                    .map(Pattern::new_caseless)
                    .transpose()?,
            )
        };
        signals.push(CompiledSignal {
            kind,
            weight: d.weight,
        });
    }
    Ok(Genre { spec, signals })
}
