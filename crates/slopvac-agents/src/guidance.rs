//! The `[guidance]` table of `slopvac.toml`, as `prime` and `setup` read it.
//!
//! Loading goes through `slopvac-config`, so layering, `extends` and schema validation
//! behave as they do for `lint`. A bad value is a `config-*` error with exit code 2.

use std::path::Path;

use slopvac_config::{ConfigError, Discovery, LoadedConfig};
use slopvac_packs::Packs;

use crate::error::{AgentsError, ConfigFailure};
use crate::messages::PrimeText;
use crate::prime::Level;

/// `guidance.write`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Write {
    /// Point agents at `prime write` before they draft prose.
    #[default]
    Auto,
    /// Drop that step and disable the topic.
    Off,
}

/// `guidance.review`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Review {
    /// Run the review workflow before a hand-off.
    #[default]
    Auto,
    /// Ask the user before running it.
    Ask,
    /// Drop the step and disable the topic.
    Off,
}

/// `guidance.review_depth`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReviewDepth {
    /// The author reviews.
    SelfReview,
    /// One fresh-context reviewer pass.
    #[default]
    Subagent,
    /// Rounds until no accepted finding remains.
    Adversary,
}

/// `guidance.report`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Report {
    /// Point agents at `prime report` when they meet a problem.
    #[default]
    Ask,
    /// Drop that guidance and disable the topic.
    Off,
}

/// The guidance settings that `prime` and `setup` act on.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Guidance {
    /// `guidance.level`: detail of the rendered `prime` topics.
    pub level: Level,
    /// `guidance.write`.
    pub write: Write,
    /// `guidance.review`.
    pub review: Review,
    /// `guidance.review_depth`.
    pub review_depth: ReviewDepth,
    /// `guidance.report`.
    pub report: Report,
    /// Config file that set each topic's control (`write`, `review`, `report`), as shown in
    /// provenance; absent when the default holds.
    sources: Vec<(&'static str, String)>,
}

impl Guidance {
    /// Read the guidance for the project at `root`.
    ///
    /// `explicit` is a `--config` path and wins over discovery. Without it, the nearest
    /// `slopvac.toml` from `root` upward applies. With no config the schema defaults hold.
    pub fn load(root: &Path, explicit: Option<&Path>) -> Result<Self, AgentsError> {
        #[expect(
            clippy::expect_used,
            reason = "the core pack is embedded in the binary and its load is tested"
        )]
        let packs = Packs::core().expect("the embedded core pack loads");
        let discovery = Discovery::new(&packs);
        let config = match explicit {
            Some(path) => Some(discovery.explicit(path)),
            None => discovery.nearest(root).transpose(),
        };
        match config {
            None => Ok(Self::default()),
            Some(Ok(config)) => Ok(Self::from_config(&config)),
            Some(Err(e)) => Err(config_error(e)),
        }
    }

    /// Whether setup installs the skill: any guidance on. With write and review both off it
    /// installs none, as 2.x does.
    #[must_use]
    pub fn skill_wanted(&self) -> bool {
        self.write != Write::Off || self.review != Review::Off
    }

    /// The config file that set the `write`, `review` or `report` control, if any did.
    #[must_use]
    pub fn source_of(&self, key: &str) -> Option<&str> {
        self.sources
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, f)| f.as_str())
    }

    /// Whether `prime <topic>` renders: `false` only for `write`, `review` or `report` when
    /// its control is `"off"`.
    #[must_use]
    pub fn topic_enabled(&self, topic: &str) -> bool {
        match topic {
            "write" => self.write != Write::Off,
            "review" => self.review != Review::Off,
            "report" => self.report != Report::Off,
            _ => true,
        }
    }

    /// The notice `prime <topic>` prints when its control is `"off"`, or `None` when the
    /// topic is on. The config file shows as provenance does for a file outside the project:
    /// `<external>/<file name>`, never an absolute path.
    #[must_use]
    pub fn disabled_notice(&self, topic: &str) -> Option<String> {
        if self.topic_enabled(topic) {
            return None;
        }
        let topic = topic.to_owned();
        let message = match self.source_of(&topic) {
            Some(file) => PrimeText::DisabledIn {
                file: slopvac_fs::display_under_root(Path::new(file), Path::new(".")),
                topic,
            },
            None => PrimeText::Disabled { topic },
        };
        Some(slopvac_messages::render(&message))
    }

    fn from_config(config: &LoadedConfig) -> Self {
        // The last file to set a key wins, as in `project_value`; keep its file for the notice.
        let setting = |key: &str| {
            config
                .files
                .iter()
                .flat_map(|f| f.settings.iter())
                .rfind(|s| s.path == ["guidance", key])
        };
        let text = |key: &str| {
            config
                .project_value(&["guidance", key])
                .and_then(|v| v.as_str().map(str::to_owned))
        };
        let sources = ["write", "review", "report"]
            .into_iter()
            .filter_map(|k| Some((k, setting(k)?.source.file.clone()?)))
            .collect();
        Self {
            level: match text("level").as_deref() {
                Some("brief") => Level::Minimal,
                Some("full") => Level::Full,
                _ => Level::Default,
            },
            write: match text("write").as_deref() {
                Some("off") => Write::Off,
                _ => Write::Auto,
            },
            review: match text("review").as_deref() {
                Some("off") => Review::Off,
                Some("ask") => Review::Ask,
                _ => Review::Auto,
            },
            review_depth: match text("review_depth").as_deref() {
                Some("self") => ReviewDepth::SelfReview,
                Some("adversary") => ReviewDepth::Adversary,
                _ => ReviewDepth::Subagent,
            },
            report: match text("report").as_deref() {
                Some("off") => Report::Off,
                _ => Report::Ask,
            },
            sources,
        }
    }
}

fn config_error(e: ConfigError) -> AgentsError {
    AgentsError::Config {
        code: e.code().as_str(),
        source: ConfigFailure::new(e),
    }
}
