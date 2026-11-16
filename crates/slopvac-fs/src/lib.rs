//! Filesystem and path primitives that every crate shares: normalise, display, redact, locate,
//! read within a cap and write atomically.
//!
//! The crate is a leaf. It knows no slopvac domain types and depends only on the standard
//! library, `etcetera`, `tempfile` and `thiserror`. It targets Unix; 3.0 has no Windows support.

mod dirs;
mod error;
mod io;
mod path;

pub use dirs::{CACHE_DIR_VAR, Lookup, cache_dir, cache_dir_with, config_dir, config_dir_with};
pub use error::{Error, Result};
pub use io::{WriteOptions, read_capped, write_atomic};
pub use path::{EXTERNAL, display_under_root, lexical_normalize};
