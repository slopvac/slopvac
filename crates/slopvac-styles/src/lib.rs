//! Style presets: loading, ordered resolution and conflict reporting.
//!
//! A style is a data-pack file that sets rule levels, category dials and terms
//! settings. Its settings are layer 2 of the config stack in `slopvac-config`, each
//! value carrying the preset and line that set it. `styles = [...]` in `slopvac.toml` lists them in order and the last
//! listed wins per setting. A style's `extends` entries resolve before it.
//! Styles can declare `conflicts`; when both sides are active the resolution
//! reports the winner, and strict mode turns the report into an error.

mod detect;
mod error;
mod messages;
mod reference;
mod resolve;
mod shape;
mod source;
mod stack;
mod types;

pub use detect::{Detection, Evidence, SkippedFile, Suggestion, detect_house_styles};
pub use error::StyleError;
pub use messages::message_entries;
pub use reference::reference_pages;
pub use resolve::{Conflict, Resolution, resolve};
pub use shape::Style;
pub use source::{BuiltinSource, DirSource, MAX_STYLE_BYTES, MapSource, StyleSource};
pub use stack::StylePresets;
pub use types::{Budget, CategoryDials, CategoryPatch, Level, Terms};
