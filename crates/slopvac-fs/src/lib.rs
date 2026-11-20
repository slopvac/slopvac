//! Filesystem and path primitives that every crate shares: normalise, display, redact, locate,
//! read within a cap and write atomically.
//!
//! The crate is a leaf. It knows no slopvac domain types and depends only on the standard
//! library, `cap-std`, `cap-fs-ext`, `etcetera`, `rustix` and `thiserror`. It targets Unix; 3.0
//! has no Windows support.
//!
//! # Threat model
//!
//! Repository content is hostile. The confined readers and writers refuse symlinks, special
//! files and escapes that the content of the tree can plant. A concurrent local writer is
//! actor T6 of the threat model, only partly in scope: a same-user process that can write to
//! the directories involved while an operation runs can swap the destination or its parent, or
//! replace a temporary name. A staged file is compared with its name before it replaces the
//! destination, and a mismatch fails the write. That narrows the window but does not close it
//! (residual risk R6).
//!
//! # Directories are UTF-8 only
//!
//! The per-user directory functions reject an environment variable that is not valid UTF-8 with
//! an error. Their rules follow the XDG base directory specification, but they are not an
//! equivalent of `etcetera`.

mod dirs;
mod error;
mod io;
mod path;
mod read;

pub use dirs::{
    CACHE_DIR_VAR, HomeFallback, Lookup, cache_dir, cache_dir_with, config_dir, config_dir_with,
};
pub use error::{Error, InputKind, Op, Reason, Result};
pub use io::{MAX_DOCUMENT_BYTES, WriteOptions, read_capped, read_text_capped, write_atomic};
pub use path::{
    EXTERNAL, display_physical, display_under_root, lexical_normalize, resolve_physical,
};
pub use read::{
    ReadOptions, read_capped_regular, read_capped_regular_in, read_capped_regular_to_string,
    read_capped_regular_to_string_in,
};
