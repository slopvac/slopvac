//! Load category files into a [`RuleSet`] with every rule compiled.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::ops::Range;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use slopvac_model::{CategoryId, Density, GenreId, NonEmptyString, PackId, RuleId};
use slopvac_text::TextError;

use crate::error::{LoadError, LoadErrorKind, LoadErrors, PackBudget};
use crate::matcher::{CompileOptions, LexicalMatcher, compile};
use crate::model::{Category, LocalRuleId, Match, MatchKind, ProfileName, Rule, Scope, Weight};

/// The most bytes of one rule or category file. The largest shipped file is about 130 KiB, so
/// 2 MiB leaves more than a tenfold margin for a pack.
pub const MAX_RULE_FILE_BYTES: u64 = 2 * 1024 * 1024;

/// The longest authored pattern a rule or category file may carry, in UTF-8 bytes as written,
/// before placeholders expand. Patterns that the loader builds keep the larger
/// `slopvac-pcre2` limit.
pub const MAX_AUTHORED_PATTERN_BYTES: usize = 8192;

// Budget names in `docs/contract/policies.md` section 1.9 and their constants:
// `plugin-files` is `MAX_PACK_FILES`, `plugin-bytes` is `MAX_PACK_BYTES`, `plugin-entries` is
// `MAX_PACK_ENTRIES`, and `rule-entries` is `MAX_RULE_ENTRIES`.

/// The most rule or category files one pack directory may load, after files that are the same
/// file under several names count once. The shipped pack has 23, so 256 leaves a wide margin.
pub const MAX_PACK_FILES: usize = 256;

/// The most directory entries one pack directory may hold, whatever their names. It bounds the
/// scan before any file is sorted or resolved, and leaves room for aliases and other files
/// beside the pack: four times [`MAX_PACK_FILES`].
pub const MAX_PACK_ENTRIES: usize = 4 * MAX_PACK_FILES;

/// The most bytes read in total from the files of one pack directory. The shipped pack is about
/// 450 KiB, so 32 MiB leaves a wide margin and still holds the pack in memory at once.
pub const MAX_PACK_BYTES: u64 = 32 * 1024 * 1024;

/// The most tokens, or substitutions, one rule may list. The largest shipped list has 112, so
/// 4096 leaves a wide margin. A rule over it is refused before any matcher is compiled.
pub const MAX_RULE_ENTRIES: usize = 4096;

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
    /// The qualified id, `<plugin>:<category>.<rule>`.
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
        let distinct = pack_files(dir)?;
        let mut bytes = 0_u64;
        // Read one file at a time, so the texts of a large pack are never all in memory.
        let sources = distinct.into_iter().map(move |p| {
            let read_error = |message: String| LoadError {
                file: p.clone(),
                line: None,
                rule: None,
                kind: LoadErrorKind::Read(message),
            };
            // The reader refuses a symlink, and a symlinked pack file is supported, so
            // resolve the link first. A FIFO or device then fails as not regular, without
            // blocking.
            let resolved = fs::canonicalize(&p).map_err(|e| read_error(e.to_string()))?;
            let text = slopvac_fs::read_capped_regular_to_string(
                &resolved,
                MAX_RULE_FILE_BYTES,
                slopvac_fs::ReadOptions::new(),
            )
            .map_err(|e| match e.input_kind() {
                Some(slopvac_fs::InputKind::TooLarge) => LoadError {
                    file: p.clone(),
                    line: None,
                    rule: None,
                    kind: LoadErrorKind::TooLarge(MAX_RULE_FILE_BYTES),
                },
                _ => read_error(e.to_string()),
            })?;
            bytes += text.len() as u64;
            if bytes > MAX_PACK_BYTES {
                return Err(over_budget(dir, PackBudget::Bytes, MAX_PACK_BYTES));
            }
            Ok((p, text))
        });
        Self::build(pack, sources, options)
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
        Self::build(pack, sources.into_iter().map(Ok), options)
    }

    /// Compile sources one at a time. A source that fails to arrive stops the load at once.
    fn build(
        pack: &PackId,
        sources: impl IntoIterator<Item = Result<(PathBuf, String), LoadError>>,
        options: &CompileOptions,
    ) -> Result<Self, LoadErrors> {
        let mut errors = Vec::new();
        let mut categories: Vec<LoadedCategory> = Vec::new();
        let mut seen: HashSet<CategoryId> = HashSet::new();
        for source in sources {
            let (file, text) = source?;
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

/// The rule and category files of `dir`, each file once, in file-name order.
///
/// The work is bounded before it starts: the scan stops at [`MAX_PACK_ENTRIES`] directory
/// entries, only the bounded set is sorted, and the scan stops at the first distinct file past
/// [`MAX_PACK_FILES`]. A file reached by several names is read once, under the first name in
/// file-name order.
fn pack_files(dir: &Path) -> Result<Vec<PathBuf>, LoadError> {
    let read_error = |path: &Path, e: &std::io::Error| LoadError {
        file: path.to_path_buf(),
        line: None,
        rule: None,
        kind: LoadErrorKind::Read(e.to_string()),
    };
    let too_many_files = || over_budget(dir, PackBudget::Files, MAX_PACK_FILES as u64);
    let mut files: Vec<PathBuf> = Vec::new();
    for (seen, entry) in fs::read_dir(dir)
        .map_err(|e| read_error(dir, &e))?
        .enumerate()
    {
        if seen >= MAX_PACK_ENTRIES {
            return Err(over_budget(
                dir,
                PackBudget::Entries,
                MAX_PACK_ENTRIES as u64,
            ));
        }
        let path = entry.map_err(|e| read_error(dir, &e))?.path();
        if Format::of(&path).is_some() {
            files.push(path);
        }
    }
    files.sort();
    let mut identities: HashSet<PathBuf> = HashSet::new();
    let mut distinct = Vec::new();
    for p in files {
        let id = fs::canonicalize(&p).map_err(|e| read_error(&p, &e))?;
        if identities.insert(id) {
            if distinct.len() >= MAX_PACK_FILES {
                return Err(too_many_files());
            }
            distinct.push(p);
        }
    }
    Ok(distinct)
}

/// The refusal for a pack directory (or rule file) that passes `budget`. It names the directory relative to the
/// working directory, or `<external>/<name>` outside it, never an absolute path.
fn over_budget(dir: &Path, budget: PackBudget, limit: u64) -> LoadError {
    let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let path = slopvac_fs::display_under_root(dir, &root);
    LoadError {
        file: PathBuf::from(&path),
        line: None,
        rule: None,
        kind: LoadErrorKind::BudgetExceeded {
            budget,
            limit,
            path,
        },
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
        if entry_count(&rule.match_) > MAX_RULE_ENTRIES {
            let mut e = over_budget(file, PackBudget::RuleEntries, MAX_RULE_ENTRIES as u64);
            e.line = line;
            e.rule = Some(format!("{category}.{local}"));
            errors.push(e);
            continue;
        }
        if let Some(len) = longest_over_cap(&rule.match_) {
            // The loader keeps no position for a value inside a rule, so the error carries
            // the line where the rule starts.
            let e = error(LoadErrorKind::PatternTooLong {
                len,
                limit: MAX_AUTHORED_PATTERN_BYTES,
            });
            errors.push(e);
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
            .expect("plugin, category and rule ids are valid slugs");
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

/// The authored patterns of a match: the `pattern` of a pattern match and the keys of a
/// substitution map. Tokens are literals, and the loader builds their pattern.
fn authored_patterns(m: &Match) -> Vec<&str> {
    match m {
        Match::Pattern(p) => vec![p.pattern.as_str()],
        Match::Substitution(s) => s
            .substitutions
            .entries()
            .iter()
            .map(|(p, _)| p.as_str())
            .collect(),
        _ => Vec::new(),
    }
}

/// The length of the longest authored pattern, when it is over the cap.
fn longest_over_cap(m: &Match) -> Option<usize> {
    authored_patterns(m)
        .into_iter()
        .map(str::len)
        .filter(|len| *len > MAX_AUTHORED_PATTERN_BYTES)
        .max()
}

/// How many tokens or substitutions a rule lists, the part of a match that grows with the file.
fn entry_count(m: &Match) -> usize {
    match m {
        Match::Tokens(t) => t.tokens.len(),
        Match::Substitution(s) => s.substitutions.entries().len(),
        _ => 0,
    }
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
