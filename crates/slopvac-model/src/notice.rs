//! Validated catalog messages for infallible notice construction.

use slopvac_messages::{Catalog, render};

use crate::{IdError, KebabName, NonEmptyString, ValueError};

/// A catalog message whose code and rendered text satisfy the notice contract.
///
/// The fields stay private so validation cannot be undone before constructing a notice.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct NoticeMessage {
    code: KebabName,
    message: NonEmptyString,
}

impl NoticeMessage {
    /// Pair a validated code with validated, nonempty text.
    #[must_use]
    pub fn new(code: KebabName, message: NonEmptyString) -> Self {
        Self { code, message }
    }

    /// Validate a catalog's code and render its named parameters. An empty rendering uses
    /// the entry's summary, which must then be nonempty.
    ///
    /// # Errors
    ///
    /// Returns [`NoticeError::InvalidCode`] for a code that is not kebab-case, or
    /// [`NoticeError::EmptyMessage`] when both the rendering and summary are empty.
    pub fn try_from_catalog(catalog: &dyn Catalog) -> Result<Self> {
        let code = KebabName::new(catalog.code())?;
        let rendered = render(catalog);
        let message = NonEmptyString::new(if rendered.is_empty() {
            catalog.entry().summary.to_owned()
        } else {
            rendered
        })?;
        Ok(Self::new(code, message))
    }

    pub(crate) fn into_parts(self) -> (KebabName, NonEmptyString) {
        (self.code, self.message)
    }
}

/// Why an arbitrary catalog cannot supply a valid notice message.
///
/// This is a library validation error, not a coded CLI failure. Its sources retain the
/// scalar validation error and its catalog-rendered wording.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum NoticeError {
    /// The catalog supplied an invalid code.
    #[error("{}", validation_text(.0))]
    InvalidCode(#[from] IdError),
    /// Neither the rendered message nor the summary supplies text.
    #[error("{}", validation_text(.0))]
    EmptyMessage(#[from] ValueError),
    /// The data does not have the shape that the notice code declares. The text names the
    /// code and never the data.
    #[error("{}", crate::messages::notice_data_shape(.0))]
    DataShape(String),
}

fn validation_text(error: &impl ToString) -> String {
    error.to_string()
}

/// Result of validating a catalog for notice construction.
pub type Result<T> = std::result::Result<T, NoticeError>;
