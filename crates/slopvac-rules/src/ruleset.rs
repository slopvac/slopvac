//! Load category files into a [`RuleSet`] with every rule compiled.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::ops::Range;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use slopvac_model::{CategoryId, Density, GenreId, NonEmptyString, PackId, RuleId};
use slopvac_text::TextError;

use crate::error::{LoadError, LoadErrorKind, LoadErrors};
use crate::matcher::{CompileOptions, LexicalMatcher, compile};
use crate::model::{Category, LocalRuleId, MatchKind, ProfileName, Rule, Scope, Weight};

/// The most bytes of one rule or category file. The largest shipped file is about 130 KiB, so
/// 2 MiB leaves more than a tenfold margin for a pack.
pub const MAX_RULE_FILE_BYTES: u64 = 2 * 1024 * 1024;

/// A rule with its compiled matcher and where it came from.
#[derive(Debug)]
pub struct LoadedRule {
    id: RuleId,
    category: CategoryId,
    rule: Rule,
    matcher: Option<LexicalMatcher>,
    file: PathBuf,
    line: Option<u32>,
}

impl LoadedRule {
    /// The qualified id, `<pack>:<category>.<rule>`.
    #[must_use]
    pub fn id(&self) -> &RuleId {
        &self.id
    }

    /// The category this rule belongs to.
    #[must_use]
    pub fn category(&self) -> &CategoryId {
        &self.category
    }

    /// The rule definition.
    #[must_use]
    pub fn rule(&self) -> &Rule {
        &self.rule
    }

    /// The kind of the rule's `match` block.
    #[must_use]
    pub fn kind(&self) -> MatchKind {
        self.rule.match_.kind()
    }

    /// The compiled matcher of a `tokens`, `pattern` or `substitution` rule.
    #[must_use]
    pub fn matcher(&self) -> Option<&LexicalMatcher> {
        self.matcher.as_ref()
    }

    /// Byte ranges of every match of a lexical rule in `text`. Other kinds have none.
    ///
    /// # Errors
    /// A match limit was exhausted.
    pub fn find_all(&self, text: &str) -> Result<Vec<Range<usize>>, TextError> {
        self.matcher
            .as_ref()
            .map_or_else(|| Ok(Vec::new()), |m| m.find_all(text))
    }

    /// The file the rule was loaded from.
    #[must_use]
    pub fn file(&self) -> &Path {
        &self.file
    }

    /// The one-based line of the rule's `id`, when it could be found.
    #[must_use]
    pub fn line(&self) -> Option<u32> {
        self.line
    }
}

/// A category and its loaded rules.
#[derive(Debug)]
pub struct LoadedCategory {
    meta: Category,
    rules: Vec<LoadedRule>,
    file: PathBuf,
}

impl LoadedCategory {
    /// The category id.
    #[must_use]
    pub fn id(&self) -> &CategoryId {
        &self.meta.id
    }

    /// Display title.
    #[must_use]
    pub fn title(&self) -> &NonEmptyString {
        &self.meta.title
    }

    /// What the category covers.
    #[must_use]
    pub fn description(&self) -> &NonEmptyString {
        &self.meta.description
    }

    /// Contribution to the overall score.
    #[must_use]
    pub fn weight(&self) -> Weight {
        self.meta.weight
    }

    /// Density budget per profile, if any.
    #[must_use]
    pub fn max_per_100_words(&self) -> Option<&std::collections::BTreeMap<ProfileName, Density>> {
        self.meta.max_per_100_words.as_ref()
    }

    /// Genres this category suits.
    #[must_use]
    pub fn recommended_for(&self) -> &[GenreId] {
        self.meta.recommended_for.as_deref().unwrap_or_default()
    }

    /// The loaded rules, in file order.
    #[must_use]
    pub fn rules(&self) -> &[LoadedRule] {
        &self.rules
    }

    /// The file the category was loaded from.
    #[must_use]
    pub fn file(&self) -> &Path {
        &self.file
    }
}

/// Every category of one pack, loaded and compiled.
#[derive(Debug)]
pub struct RuleSet {
    pack: PackId,
    categories: Vec<LoadedCategory>,
    index: HashMap<RuleId, (usize, usize)>,
}

impl RuleSet {
    /// Load every category file (`json`, `yaml`, `yml` or `toml`) directly inside `dir`, in
    /// file-name order.
    ///
    /// # Errors
    /// Every problem found, each with its file and line: unreadable or invalid files,
    /// duplicate ids, patterns that do not compile and examples that disagree with their rule.
    pub fn load_dir(
        pack: &PackId,
        dir: &Path,
        options: &CompileOptions,
    ) -> Result<Self, LoadErrors> {
        let read_error = |path: &Path, e: &std::io::Error| LoadError {
            file: path.to_path_buf(),
            line: None,
            rule: None,
            kind: LoadErrorKind::Read(e.to_string()),
        };
        let mut files: Vec<PathBuf> = fs::read_dir(dir)
            .map_err(|e| read_error(dir, &e))?
            .map(|entry| entry.map(|e| e.path()).map_err(|e| read_error(dir, &e)))
            .collect::<Result<_, _>>()?;
        files.retain(|p| Format::of(p).is_some());
        files.sort();
        let sources = files
            .into_iter()
            .map(|p| {
                let text =
                    slopvac_fs::read_text_capped(&p, MAX_RULE_FILE_BYTES).map_err(|e| match e {
                        slopvac_fs::Error::CapExceeded { cap } => LoadError {
                            file: p.clone(),
                            line: None,
                            rule: None,
                            kind: LoadErrorKind::TooLarge(cap),
                        },
                        other => LoadError {
                            file: p.clone(),
                            line: None,
                            rule: None,
                            kind: LoadErrorKind::Read(other.to_string()),
                        },
                    })?;
                Ok((p, text))
            })
            .collect::<Result<Vec<_>, LoadError>>()?;
        Self::from_sources(pack, sources, options)
    }

    /// Load category files already read into memory. The extension of each path selects the
    /// format; the path is what errors report.
    ///
    /// # Errors
    /// As [`RuleSet::load_dir`].
    pub fn from_sources(
        pack: &PackId,
        sources: impl IntoIterator<Item = (PathBuf, String)>,
        options: &CompileOptions,
    ) -> Result<Self, LoadErrors> {
        let mut errors = Vec::new();
        let mut categories: Vec<LoadedCategory> = Vec::new();
        let mut seen: HashSet<CategoryId> = HashSet::new();
        for (file, text) in sources {
            let mut category = match parse::<Category>(&file, &text) {
                Ok(c) => c,
                Err(e) => {
                    errors.push(e);
                    continue;
                }
            };
            if !seen.insert(category.id.clone()) {
                errors.push(LoadError {
                    file,
                    line: None,
                    rule: None,
                    kind: LoadErrorKind::DuplicateCategory(category.id.to_string()),
                });
                continue;
            }
            let rules = std::mem::take(&mut category.rules);
            let loaded = load_rules(
                pack,
                &category.id,
                rules,
                &file,
                &text,
                options,
                &mut errors,
            );
            categories.push(LoadedCategory {
                meta: category,
                rules: loaded,
                file,
            });
        }
        if !errors.is_empty() {
            return Err(LoadErrors(errors));
        }
        let index = categories
            .iter()
            .enumerate()
            .flat_map(|(c, cat)| {
                cat.rules
                    .iter()
                    .enumerate()
                    .map(move |(r, rule)| (rule.id.clone(), (c, r)))
            })
            .collect();
        Ok(Self {
            pack: pack.clone(),
            categories,
            index,
        })
    }

    /// The pack these rules belong to.
    #[must_use]
    pub fn pack(&self) -> &PackId {
        &self.pack
    }

    /// The categories, in file-name order.
    #[must_use]
    pub fn categories(&self) -> &[LoadedCategory] {
        &self.categories
    }

    /// Every rule, category by category in file order.
    pub fn rules(&self) -> impl Iterator<Item = &LoadedRule> {
        self.categories.iter().flat_map(|c| c.rules.iter())
    }

    /// The number of rules.
    #[must_use]
    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// Whether there are no rules.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// The rule with this qualified id.
    #[must_use]
    pub fn get(&self, id: &RuleId) -> Option<&LoadedRule> {
        let &(c, r) = self.index.get(id)?;
        Some(&self.categories[c].rules[r])
    }

    /// Rules whose `match` block has this kind.
    pub fn rules_of_kind(&self, kind: MatchKind) -> impl Iterator<Item = &LoadedRule> {
        self.rules().filter(move |r| r.kind() == kind)
    }

    /// Rules that look at this scope.
    pub fn rules_in_scope(&self, scope: Scope) -> impl Iterator<Item = &LoadedRule> {
        self.rules().filter(move |r| r.rule.scope == scope)
    }
}

fn load_rules(
    pack: &PackId,
    category: &CategoryId,
    rules: Vec<Rule>,
    file: &Path,
    text: &str,
    options: &CompileOptions,
    errors: &mut Vec<LoadError>,
) -> Vec<LoadedRule> {
    let mut seen: HashSet<LocalRuleId> = HashSet::new();
    let mut loaded = Vec::new();
    for rule in rules {
        let local = rule.id.clone();
        let line = rule_line(text, local.as_str());
        let error = |kind| LoadError {
            file: file.to_path_buf(),
            line,
            rule: Some(format!("{category}.{local}")),
            kind,
        };
        if !seen.insert(local.clone()) {
            errors.push(error(LoadErrorKind::DuplicateRule(local.to_string())));
            continue;
        }
        let matcher = match compile(&rule.match_, options) {
            Ok(m) => m,
            Err(e) => {
                errors.push(error(LoadErrorKind::Compile(e.to_string())));
                continue;
            }
        };
        if let Some(m) = &matcher {
            let before = errors.len();
            check_examples(&rule, m, |kind| errors.push(error(kind)));
            if errors.len() > before {
                continue;
            }
        }
        let id = RuleId::new(format!("{pack}:{category}.{local}"))
            .expect("pack, category and rule ids are valid slugs");
        loaded.push(LoadedRule {
            id,
            category: category.clone(),
            rule,
            matcher,
            file: file.to_path_buf(),
            line,
        });
    }
    loaded
}

/// Every `bad` example must match and every `good` one must not.
fn check_examples(rule: &Rule, matcher: &LexicalMatcher, mut report: impl FnMut(LoadErrorKind)) {
    for (index, example) in rule.examples().iter().enumerate() {
        match matcher.is_match(&example.bad) {
            Ok(true) => {}
            Ok(false) => report(LoadErrorKind::BadExampleDoesNotMatch {
                index,
                text: example.bad.clone(),
            }),
            Err(e) => report(LoadErrorKind::Match(e.to_string())),
        }
        if let Some(good) = &example.good {
            match matcher.is_match(good) {
                Ok(false) => {}
                Ok(true) => report(LoadErrorKind::GoodExampleMatches {
                    index,
                    text: good.clone(),
                }),
                Err(e) => report(LoadErrorKind::Match(e.to_string())),
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Format {
    Json,
    Yaml,
    Toml,
}

impl Format {
    fn of(path: &Path) -> Option<Self> {
        match path.extension()?.to_str()? {
            "json" => Some(Self::Json),
            "yaml" | "yml" => Some(Self::Yaml),
            "toml" => Some(Self::Toml),
            _ => None,
        }
    }
}

fn line_of_offset(text: &str, offset: usize) -> u32 {
    let end = offset.min(text.len());
    let lines = text.as_bytes()[..end].split(|b| *b == b'\n').count();
    u32::try_from(lines).unwrap_or(u32::MAX)
}

/// Parse a file in the format its extension names.
pub(crate) fn parse<T: DeserializeOwned>(file: &Path, text: &str) -> Result<T, LoadError> {
    let fail = |line: Option<u32>, kind: LoadErrorKind| LoadError {
        file: file.to_path_buf(),
        line,
        rule: None,
        kind,
    };
    let Some(format) = Format::of(file) else {
        return Err(fail(None, LoadErrorKind::UnsupportedFormat));
    };
    match format {
        Format::Json => serde_json::from_str(text).map_err(|e| {
            let line = u32::try_from(e.line()).ok().filter(|l| *l > 0);
            fail(line, LoadErrorKind::Parse(e.to_string()))
        }),
        Format::Toml => toml::from_str(text).map_err(|e| {
            let line = e.span().map(|s| line_of_offset(text, s.start));
            fail(line, LoadErrorKind::Parse(e.message().to_owned()))
        }),
        Format::Yaml => serde_saphyr::from_str(text).map_err(|e| {
            let line = e.location().and_then(|l| u32::try_from(l.line()).ok());
            fail(line, LoadErrorKind::Parse(e.to_string()))
        }),
    }
}

/// The one-based line of the first `id` entry whose value is `id`, after the `rules` key.
///
/// Best effort across the three formats: a minified one-line file has no useful line.
pub(crate) fn rule_line(text: &str, id: &str) -> Option<u32> {
    let mut in_rules = false;
    for (i, line) in text.lines().enumerate() {
        let t = line.trim_start().trim_start_matches("- ").trim_start();
        if !in_rules {
            in_rules = t.starts_with("\"rules\"") || t.starts_with("rules") || t == "[[rules]]";
            continue;
        }
        let Some(rest) = t
            .strip_prefix("\"id\"")
            .or_else(|| t.strip_prefix("id"))
            .map(str::trim_start)
        else {
            continue;
        };
        let Some(value) = rest.strip_prefix(':').or_else(|| rest.strip_prefix('=')) else {
            continue;
        };
        let value = value.trim().trim_end_matches(',').trim_matches(['"', '\'']);
        if value == id {
            return u32::try_from(i + 1).ok();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_rule_line_in_pretty_json() {
        let text = "{\n  \"id\": \"cat\",\n  \"rules\": [\n    {\n      \"id\": \"first\",\n      \"name\": \"n\"\n    },\n    {\n      \"id\": \"second\"\n    }\n  ]\n}";
        assert_eq!(rule_line(text, "first"), Some(5));
        assert_eq!(rule_line(text, "second"), Some(9));
        assert_eq!(rule_line(text, "cat"), None);
    }

    #[test]
    fn finds_a_rule_line_in_yaml() {
        let text =
            "schema_version: 3\nid: cat\nrules:\n  - id: first\n    name: n\n  - id: \"second\"\n";
        assert_eq!(rule_line(text, "first"), Some(4));
        assert_eq!(rule_line(text, "second"), Some(6));
    }

    #[test]
    fn finds_a_rule_line_in_toml() {
        let text = "schema_version = 3\nid = \"cat\"\n\n[[rules]]\nid = \"first\"\n\n[[rules]]\nid = 'second'\n";
        assert_eq!(rule_line(text, "first"), Some(5));
        assert_eq!(rule_line(text, "second"), Some(8));
    }

    #[test]
    fn the_line_of_a_matching_prefix_is_not_confused() {
        let text = "{\n\"rules\": [\n{\n\"x\": 1,\n\"id\": \"ab\"\n},\n{\n\"id\": \"a\"\n}]}";
        assert_eq!(rule_line(text, "a"), Some(8));
    }
}
