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
//! Metric rules measure a sentence, a paragraph or the document and report a finding with a
//! line and no source range, as 2.x did.
//!
//! The settings come from the resolved configuration as plain data ([`Settings`]). Rule
//! kinds the engine does not run yet (structure checks and the metrics it does not measure)
//! are skipped, and each document says so in an `info` notice. A capitalization rule in a
//! style other than sentence is skipped with a `warning` notice, which makes the check
//! incomplete.
//!
//! Vocabulary rules enforce a word blocklist ([`Engine::with_blocklist`]) on the
//! part-of-speech tagger of `slopvac-pos`. One rule owns the sweep. Without a blocklist
//! they do not run, and a notice says so.
//!
//! [`fix_document`] and [`fix_file`] apply the single-replacement substitutions that a run
//! found, atomically and never over code, links or front matter. [`unified_diff`] renders a
//! dry run.
//!
//! Term-casing rules ([`terms_rule_set`]) flag a term spelled in the wrong case, such as
//! `Github`, on the terms of [`Engine::with_terms`]. Inline code and the rule's excluded
//! blocks are never checked.
//!
//! [`locale`] builds the generated spelling rule for en-US, en-GB or `und`.
//!
//! [`skip_ranges`] is the reusable part for other crates: the source ranges a rule skips.
//! [`discover`] finds the files to lint. [`diff_scope`] finds the lines a branch or the
//! work tree changed, to scope findings and fixes to them.
#![warn(clippy::indexing_slicing, clippy::string_slice)]

mod capitalization;
pub mod cases;
mod commit;
mod diagrams;
pub mod diff_scope;
mod directives;
pub mod discover;
mod exceptions;
mod file;
mod fix;
mod lint;
pub mod locale;
mod matching;
mod metrics;
mod prepare;
mod protect;
pub mod rst;
mod score;
mod settings;
mod skip;
mod tagging;
mod terms;

pub use commit::{MAX_FILE_BYTES, Refusal, Step};
pub use diagrams::{
    DIAGRAM_PACK, DiagramChecker, DiagramHit, DiagramInput, DiagramRule, SignalChecker,
    diagram_rule_set,
};
pub use exceptions::{Detector, Detectors, DuplicateDetector, MatchSite};
pub use file::{FileFix, FileResult, fix_file, fix_file_hooked};
pub use fix::{FixError, FixReport, SkipReason, Skipped, fix_document, unified_diff};
pub use lint::{Engine, EngineError, Input, Kind};
pub use score::{CategoryInfo, Scoring, score_document};
pub use settings::{CategoryDials, Level, Mode, Settings, Thresholds};
pub use skip::{SkipRanges, skip_ranges};
pub use terms::{TERMS_PACK, terms_rule_set};
