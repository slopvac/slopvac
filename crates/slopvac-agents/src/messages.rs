//! The messages `slopvac prime` prints that are not topic prose.
//!
//! [`message_entries`] lists them for the workspace message registry, which generates the
//! reference page and checks that no code changes.

use slopvac_messages::{Catalog, Entry};

/// What `prime <topic>` prints in place of a topic.
#[derive(Debug, Clone, PartialEq, Eq, Catalog)]
pub(crate) enum PrimeText {
    /// The topic's `[guidance]` control is `"off"` and no config file is known to set it.
    #[code("prime-topic-disabled")]
    #[text("slopvac prime {topic} is disabled by [guidance] {topic} = \"off\".")]
    Disabled { topic: String },
    /// The topic's `[guidance]` control is `"off"`, set in the named config file.
    #[code("prime-topic-disabled-in")]
    #[text("slopvac prime {topic} is disabled by [guidance] {topic} = \"off\" in {file}.")]
    DisabledIn {
        topic: String,
        #[display_path]
        file: String,
    },
}

/// The entries of the `prime` text catalog.
#[must_use]
pub fn message_entries() -> &'static [Entry] {
    PrimeText::entries()
}
