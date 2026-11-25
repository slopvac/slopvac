use std::path::PathBuf;

/// Why generating, writing or checking the schemas failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A schema file could not be read or written.
    #[error("cannot access {path}")]
    Io {
        /// The file.
        path: PathBuf,
        /// The cause.
        #[source]
        source: std::io::Error,
    },
    /// A type's schema is not a JSON object.
    #[error("the generated schema for {file} is not an object")]
    NotAnObject {
        /// The artifact file name.
        file: &'static str,
    },
    /// Two imported files both define a `$defs` entry of this name.
    #[error("{file}: `$defs/{def}` is defined by both {first} and {second}")]
    AmbiguousOwner {
        /// The artifact being generated.
        file: &'static str,
        /// The definition name.
        def: String,
        /// One owner.
        first: &'static str,
        /// The other owner.
        second: &'static str,
    },
    /// A definition generated for one file differs from the one its owner publishes, so two
    /// Rust types share a schema name.
    #[error("{file}: `$defs/{def}` differs from the definition {owner} publishes")]
    SharedDefDiffers {
        /// The artifact being generated.
        file: &'static str,
        /// The definition name.
        def: String,
        /// The file that owns the definition.
        owner: &'static str,
    },
    /// An artifact imports a file that is not generated before it.
    #[error("{file} imports {import}, which is not generated before it")]
    UnknownImport {
        /// The artifact being generated.
        file: &'static str,
        /// The imported file.
        import: &'static str,
    },
    /// An artifact asks to inline a definition it does not generate.
    #[error("{file} inlines `$defs/{def}`, which it does not generate")]
    UnknownInline {
        /// The artifact being generated.
        file: &'static str,
        /// The definition name.
        def: String,
    },
    /// An artifact asks to keep a definition that it does not generate.
    #[error("{file} keeps `$defs/{def}`, which it does not generate")]
    UnknownKeep {
        /// The artifact being generated.
        file: &'static str,
        /// The definition name.
        def: String,
    },
    /// A reference points inside a definition that is being inlined.
    #[error("{file} references inside `$defs/{def}`, which is inlined")]
    InlinePointer {
        /// The artifact being generated.
        file: &'static str,
        /// The definition name.
        def: String,
    },
    /// Two generated files publish a definition of the same name.
    #[error("`$defs/{def}` is published by both {first} and {second}")]
    DuplicateDef {
        /// The definition name.
        def: String,
        /// One file.
        first: &'static str,
        /// The other file.
        second: &'static str,
    },
}

/// The result of a generator operation.
pub type Result<T> = std::result::Result<T, Error>;
