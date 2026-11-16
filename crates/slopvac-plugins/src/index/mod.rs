//! The pack index: a public listing of packs that `slopvac packs search`
//! reads. A scheduled job calls [`build_index`] and publishes the result of
//! [`Index::to_json`].
//!
//! The index is untrusted input. It has a size cap, strict parsing and
//! terminal-safe output, and its publisher keys are display-only: nothing in
//! this module converts a [`PublisherKey`] into a verification key or a key
//! policy (threat model sections 5.2, K1 and N11).
//!
//! No network code lives here. The caller fetches the bytes.

mod build;
mod model;
mod read;
mod search;

pub use build::{Listing, build_index};
pub use model::{
    Index, IndexEntry, IndexError, MAX_API, MAX_DESCRIPTION_CHARS, MAX_ENTRIES, MAX_ID_CHARS,
    MAX_INDEX_BYTES, MAX_NAME_CHARS, MAX_SOURCE_CHARS, MAX_VERSION_NUMBER, PackKind, PublisherKey,
};
pub use search::{SearchHit, search};
