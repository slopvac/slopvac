//! Style presets: loading, ordered resolution and conflict reporting.
//!
//! A style is a data-pack file that sets rule levels, category dials and terms
//! settings. `styles = [...]` in `slopvac.toml` lists them in order and the last
//! listed wins per setting. A style's `extends` entries resolve before it.
//! Styles can declare `conflicts`; when both sides are active the resolution
//! reports the winner, and strict mode turns the report into an error.

mod error;
mod patch;
mod reference;
mod resolve;
mod shape;
mod source;

pub use error::StyleError;
pub use patch::{CategoryDials, CategoryPatch, Level, SettingValue, SettingsPatch, TermsPatch};
pub use reference::reference_pages;
pub use resolve::{Conflict, Resolution, Setting, resolve};
pub use shape::Style;
pub use source::{BuiltinSource, DirSource, MapSource, StyleSource};
