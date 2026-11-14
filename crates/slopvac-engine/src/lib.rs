//! The linting engine that applies rules to documents.
//!
//! [`Engine::lint`] takes one [`Input`] and returns the report [`slopvac_model::Document`]:
//!
//! 1. `frontmatter-spans` finds the front matter, so the parser reads only the body.
//! 2. `slopvac-parse` splits the body into blocks with a projection from prose to source.
//! 3. Each active rule matches in the units its `scope` selects. A projected match maps
//!    back to a source [`slopvac_model::Span`].
//! 4. Filters drop a match: the allowlist, all-caps text, the rule's skipped blocks and
//!    inline code, context gates, exception detectors ([`Detectors`]) and inline
//!    directives.
//! 5. [`score_document`] turns the findings into category scores, a document score and a
//!    gate.
//!
//! [`Engine::lint_all`] runs many inputs with rayon and keeps their order. Output is
//! deterministic: findings sort by position and rule id.
//!
//! The settings come from the resolved configuration as plain data ([`Settings`]). Rule
//! kinds the engine does not run yet (metrics, structure checks, vocabulary and
//! capitalization) are skipped, and each document says so in an `info` notice.
//!
//! [`skip_ranges`] is the reusable part for other crates: the source ranges a rule skips.
//! [`discover`] finds the files to lint.

mod diagrams;
mod directives;
pub mod discover;
mod exceptions;
mod lint;
mod matching;
mod prepare;
mod score;
mod settings;
mod skip;
mod tagging;

pub use diagrams::{DiagramChecker, DiagramHit, DiagramInput, DiagramRule, SignalChecker};
pub use exceptions::{Detector, Detectors, DuplicateDetector, MatchSite};
pub use lint::{Engine, EngineError, Input, Kind};
pub use score::{CategoryInfo, Scoring, score_document};
pub use settings::{CategoryDials, Level, Mode, Settings, Thresholds};
pub use skip::{SkipRanges, skip_ranges};
