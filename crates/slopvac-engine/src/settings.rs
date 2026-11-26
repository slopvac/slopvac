//! What the engine reads from the resolved configuration.
//!
//! The layered configuration stack (`slopvac-config`) resolves profiles, styles, genres and
//! overrides into one value per rule and category. The engine takes that result as plain
//! data, so it never re-derives a level from a profile. Python's `severity_for` and tier
//! logic (advisory demotion, authored floors) live in the config layer; this crate applies
//! the final answer.

use std::collections::{BTreeMap, BTreeSet};

use slopvac_context::{GenreAssignment, GenreRegistry, RuleLevel};
use slopvac_model::{
    CategoryName, ConfigSource, GenreResult, IdError, NonEmptyString, RuleId, Severity,
};
use slopvac_rules::ApplyMode;

/// The level a rule or category reports at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// The rule does not run.
    Off,
    /// The rule reports at this severity.
    On(Severity),
}

/// How the input is read. A rule's `applies_to.modes` selects on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// A prose file.
    #[default]
    Prose,
    /// The comments of a source file.
    CodeComments,
}

impl Mode {
    pub(crate) const fn apply_mode(self) -> ApplyMode {
        match self {
            Self::Prose => ApplyMode::Prose,
            Self::CodeComments => ApplyMode::CodeComments,
        }
    }
}

/// Dials for one category, from the resolved configuration.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
#[non_exhaustive]
pub struct CategoryDials {
    /// The level every rule of the category reports at, unless a rule has its own.
    pub level: Option<Level>,
    /// The weight in the document score. `None` keeps the weight in the rule data.
    pub weight: Option<f64>,
    /// The density budget. `None` keeps the budget the rule data gives for the profile.
    pub max_per_100_words: Option<f64>,
    /// The config cleared the budget with `"none"`: the category has no budget, and the
    /// budget in the rule data does not apply. Wins over `max_per_100_words`.
    pub budget_cleared: bool,
}

impl CategoryDials {
    /// The same dials with a category level.
    #[must_use]
    pub const fn with_level(mut self, level: Option<Level>) -> Self {
        self.level = level;
        self
    }

    /// The same dials with a score weight.
    #[must_use]
    pub const fn with_weight(mut self, weight: Option<f64>) -> Self {
        self.weight = weight;
        self
    }

    /// The same dials with a density budget.
    #[must_use]
    pub const fn with_max_per_100_words(mut self, budget: Option<f64>) -> Self {
        self.max_per_100_words = budget;
        self
    }

    /// The same dials with the budget cleared or not.
    #[must_use]
    pub const fn with_budget_cleared(mut self, cleared: bool) -> Self {
        self.budget_cleared = cleared;
        self
    }
}

/// The limits that fail a document's gate. An absent limit never fails.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
#[non_exhaustive]
pub struct Thresholds {
    /// Most errors a document may carry.
    pub max_errors: Option<usize>,
    /// Most warnings a document may carry.
    pub max_warnings: Option<usize>,
    /// Most Unicode dash findings a document may carry.
    pub max_unicode_dashes: Option<usize>,
    /// Most severity-weighted findings per 100 words.
    pub max_total_per_100_words: Option<f64>,
    /// Lowest acceptable score.
    pub min_score: Option<f64>,
}

impl Thresholds {
    /// The same limits with a cap on errors.
    #[must_use]
    pub const fn with_max_errors(mut self, limit: Option<usize>) -> Self {
        self.max_errors = limit;
        self
    }

    /// The same limits with a cap on warnings.
    #[must_use]
    pub const fn with_max_warnings(mut self, limit: Option<usize>) -> Self {
        self.max_warnings = limit;
        self
    }

    /// The same limits with a cap on Unicode dash findings.
    #[must_use]
    pub const fn with_max_unicode_dashes(mut self, limit: Option<usize>) -> Self {
        self.max_unicode_dashes = limit;
        self
    }

    /// The same limits with a cap on weighted findings per 100 words.
    #[must_use]
    pub const fn with_max_total_per_100_words(mut self, limit: Option<f64>) -> Self {
        self.max_total_per_100_words = limit;
        self
    }

    /// The same limits with a lowest acceptable score.
    #[must_use]
    pub const fn with_min_score(mut self, limit: Option<f64>) -> Self {
        self.min_score = limit;
        self
    }
}

/// The settings one run applies to every document.
///
/// Start from [`Settings::new`] and assign the fields you change.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Settings {
    /// The profile name, such as `normal`. It selects a category budget in the rule data.
    pub profile: NonEmptyString,
    /// The genre, with the evidence that chose it.
    pub genre: GenreResult,
    /// `[[genres]]` assignments, in file order. `Some` makes the engine choose a genre for
    /// each document (front matter, then the last matching assignment, then detection) and
    /// apply that genre's levels to its findings; [`Settings::genre`] then only describes
    /// documents that have no prose to inspect, such as diagrams. `None` applies
    /// [`Settings::genre`] to every document.
    pub genre_assignments: Option<Vec<GenreAssignment>>,
    /// How the input is read.
    pub mode: Mode,
    /// Levels set for single rules. These win over a category level.
    pub rule_levels: BTreeMap<RuleId, Level>,
    /// Rules the user turned on by name: a config `rules` entry or an id given to `--category`, not a category,
    /// preset, style or genre. In code-comment mode, one of these that cannot read comments
    /// makes the run incomplete instead of joining the info notice.
    pub named_rules: BTreeSet<RuleId>,
    /// Levels the genre chain gives single rules. They sit below `rule_levels` and the
    /// `categories` levels: explicit configuration beats a genre default.
    pub genre_levels: BTreeMap<RuleId, Level>,
    /// The profile's `severity_shift`. It moves the levels that come from the genre chain and
    /// from rule definitions along suggestion, warning, error. A level in `rule_levels` or in
    /// a category is final, and `off` stays off.
    pub severity_shift: i64,
    /// Dials per category, keyed as a report names the category: the bare id for a `core`
    /// category and `<plugin>:<category>` for another plugin's.
    pub categories: BTreeMap<CategoryName, CategoryDials>,
    /// The gate limits.
    pub thresholds: Thresholds,
    /// Where the settings came from, copied to each document.
    pub config_sources: Vec<ConfigSource>,
    /// Words that capitalization rules exempt, such as the `[locale] allow` words and project
    /// terms. Case is ignored.
    pub allowed_words: Vec<String>,
}

impl Level {
    /// The level moved `shift` steps along suggestion, warning, error, clamped. `off` stays off.
    #[must_use]
    pub fn shifted(self, shift: i64) -> Self {
        const RANKS: [Severity; 3] = [Severity::Suggestion, Severity::Warning, Severity::Error];
        let Self::On(severity) = self else {
            return self;
        };
        let at = RANKS.iter().position(|r| *r == severity).unwrap_or(0);
        let top = i64::try_from(RANKS.len() - 1).unwrap_or(0);
        let to = (i64::try_from(at).unwrap_or(0) + shift).clamp(0, top);
        Self::On(RANKS[usize::try_from(to).unwrap_or(0)])
    }
}

impl Settings {
    /// Settings for `profile` and `genre` with no overrides and no limits.
    #[must_use]
    pub fn new(profile: NonEmptyString, genre: GenreResult) -> Self {
        Self {
            profile,
            genre,
            genre_assignments: None,
            mode: Mode::Prose,
            rule_levels: BTreeMap::new(),
            named_rules: BTreeSet::new(),
            genre_levels: BTreeMap::new(),
            severity_shift: 0,
            categories: BTreeMap::new(),
            thresholds: Thresholds::default(),
            config_sources: Vec::new(),
            allowed_words: Vec::new(),
        }
    }

    /// Fill [`Settings::genre_levels`] from the levels that the settings' genre carries in
    /// `registry`. A level already there stays. Keys without a pack belong to the `core`
    /// pack. A genre the registry does not know adds nothing. A key that is no rule id is
    /// an error.
    pub fn apply_genre_levels(&mut self, registry: &GenreRegistry) -> Result<(), IdError> {
        for (id, level) in genre_levels(registry, self.genre.id.as_str())? {
            self.genre_levels.entry(id).or_insert(level);
        }
        Ok(())
    }
}

/// The levels the genre chain of `genre` gives single rules. A genre the registry does not
/// know gives none.
pub(crate) fn genre_levels(
    registry: &GenreRegistry,
    genre: &str,
) -> Result<BTreeMap<RuleId, Level>, IdError> {
    let Ok(levels) = registry.rules(genre) else {
        return Ok(BTreeMap::new());
    };
    levels
        .into_iter()
        .map(|(key, level)| {
            let qualified = if key.contains(':') {
                key
            } else {
                format!("core:{key}")
            };
            let level = match level {
                RuleLevel::Off => Level::Off,
                RuleLevel::Suggestion => Level::On(Severity::Suggestion),
                RuleLevel::Warning => Level::On(Severity::Warning),
                RuleLevel::Error => Level::On(Severity::Error),
            };
            Ok((RuleId::new(qualified)?, level))
        })
        .collect()
}
