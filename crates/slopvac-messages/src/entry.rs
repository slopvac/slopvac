//! The static description of one message.

/// What a message is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Kind {
    /// A failure reported with its code.
    Error,
    /// A note to the user that is not a failure.
    Notice,
    /// Help or label text with no severity.
    Text,
}

impl Kind {
    /// The lower-case name used in the lock file and on the reference page.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Notice => "notice",
            Self::Text => "text",
        }
    }
}

/// How a parameter value is passed to the template.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ParamKind {
    /// Plain text.
    Text,
    /// A number the text may count; a plural-aware template selects on it later.
    Count,
    /// A path already shown the way the user sees it.
    Path,
}

/// One named parameter of a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Param {
    /// The name used in the template, as `{name}`.
    pub name: &'static str,
    /// How the value is passed.
    pub kind: ParamKind,
}

impl Param {
    /// Describe a parameter.
    #[must_use]
    pub const fn new(name: &'static str, kind: ParamKind) -> Self {
        Self { name, kind }
    }
}

/// One message: its stable code, template and documentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Entry {
    /// Stable kebab-case code and Fluent message id. Never reused, never changes meaning.
    pub code: &'static str,
    /// What the message is for.
    pub kind: Kind,
    /// Where the message is defined, as `Enum::Variant`.
    pub key: &'static str,
    /// Text with `{name}` placeholders; `{{` and `}}` are literal braces.
    pub template: &'static str,
    /// The parameters the template may name.
    pub params: &'static [Param],
    /// One-line description for the reference page.
    pub summary: &'static str,
}

impl Entry {
    /// Describe a message. The derive macro is the usual caller.
    #[must_use]
    pub const fn new(
        code: &'static str,
        kind: Kind,
        key: &'static str,
        template: &'static str,
        params: &'static [Param],
        summary: &'static str,
    ) -> Self {
        Self {
            code,
            kind,
            key,
            template,
            params,
            summary,
        }
    }
}
