//! What the engine reads from the resolved configuration.
//!
//! The layered configuration stack (`slopvac-config`) resolves profiles, styles, genres and
//! overrides into one value per rule and category. The engine takes that result as plain
//! data, so it never re-derives a level from a profile. Python's `severity_for` and tier
//! logic (advisory demotion, authored floors) live in the config layer; this crate applies
//! the final answer.

use std::collections::BTreeMap;

use slopvac_context::{GenreRegistry, RuleLevel};
use slopvac_model::{
    CategoryId, ConfigSource, GenreResult, IdError, NonEmptyString, RuleId, Severity,
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
pub struct CategoryDials {
    /// The level every rule of the category reports at, unless a rule has its own.
    pub level: Option<Level>,
    /// The weight in the document score. `None` keeps the weight in the rule data.
    pub weight: Option<f64>,
    /// The density budget. `None` keeps the budget the rule data gives for the profile.
    pub max_per_100_words: Option<f64>,
}

/// The limits that fail a document's gate. An absent limit never fails.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
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

/// The settings one run applies to every document.
#[derive(Debug, Clone)]
pub struct Settings {
    /// The profile name, such as `normal`. It selects a category budget in the rule data.
    pub profile: NonEmptyString,
    /// The genre, with the evidence that chose it.
    pub genre: GenreResult,
    /// How the input is read.
    pub mode: Mode,
    /// Levels set for single rules. These win over a category level.
    pub rule_levels: BTreeMap<RuleId, Level>,
    /// Dials per category.
    pub categories: BTreeMap<CategoryId, CategoryDials>,
    /// The gate limits.
    pub thresholds: Thresholds,
    /// Where the settings came from, copied to each document.
    pub config_sources: Vec<ConfigSource>,
    /// Words that capitalization rules exempt, such as the `[locale] allow` words and project
    /// terms. Case is ignored.
    pub allowed_words: Vec<String>,
}

impl Settings {
    /// Settings for `profile` and `genre` with no overrides and no limits.
    #[must_use]
    pub fn new(profile: NonEmptyString, genre: GenreResult) -> Self {
        Self {
            profile,
            genre,
            mode: Mode::Prose,
            rule_levels: BTreeMap::new(),
            categories: BTreeMap::new(),
            thresholds: Thresholds::default(),
            config_sources: Vec::new(),
            allowed_words: Vec::new(),
        }
    }

    /// Add the rule levels that the settings' genre carries in `registry`. A level already
    /// in [`Settings::rule_levels`] wins, because a project or user layer sits above the
    /// genre. Keys without a pack belong to the `core` pack. A genre the registry does not
    /// know adds nothing. A key that is no rule id is an error.
    pub fn apply_genre_levels(&mut self, registry: &GenreRegistry) -> Result<(), IdError> {
        let Ok(levels) = registry.rules(self.genre.id.as_str()) else {
            return Ok(());
        };
        for (key, level) in levels {
            let qualified = if key.contains(':') {
                key
            } else {
                format!("core:{key}")
            };
            let id = RuleId::new(qualified)?;
            self.rule_levels.entry(id).or_insert(match level {
                RuleLevel::Off => Level::Off,
                RuleLevel::Suggestion => Level::On(Severity::Suggestion),
                RuleLevel::Warning => Level::On(Severity::Warning),
                RuleLevel::Error => Level::On(Severity::Error),
            });
        }
        Ok(())
    }
}
