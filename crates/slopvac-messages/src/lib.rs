//! Message catalogs: the entry types, the one render function and the Fluent export behind every user-facing message.
//!
//! A catalog is an enum that derives [`Catalog`]. Each variant is one message with a stable
//! kebab-case code, a template with named `{parameters}` and a one-line summary. Every
//! message, whatever its crate, renders through [`render`], and [`to_ftl`] exports the same
//! entries as a Fluent file. Moving to `fluent-bundle` later therefore changes this crate
//! only: codes are valid Fluent message ids, parameters are named plain values and counts are
//! marked for plural selection.
//!
//! ```
//! use slopvac_messages::{Catalog, render};
//!
//! #[derive(Debug, Catalog)]
//! enum ReadError {
//!     /// The file could not be read.
//!     #[code("file-unreadable")]
//!     #[message("cannot read {path}: {reason}")]
//!     Unreadable {
//!         #[display_path]
//!         path: std::path::PathBuf,
//!         reason: String,
//!     },
//! }
//!
//! let error = ReadError::Unreadable { path: "a.md".into(), reason: "denied".into() };
//! assert_eq!(error.to_string(), "cannot read a.md: denied");
//! assert_eq!(render(&error), error.to_string());
//! assert_eq!(error.code(), "file-unreadable");
//! ```

mod audit;
mod catalog;
mod entry;
mod ftl;
mod template;
mod value;

pub use audit::{Violation, audit};
pub use catalog::{Catalog, render, write_message};
pub use entry::{Entry, Kind, Param, ParamKind};
pub use ftl::to_ftl;
pub use slopvac_messages_derive::Catalog;
pub use value::{Arg, ToCount, Value};
