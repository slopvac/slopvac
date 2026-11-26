//! The checks the binary registers next to the engine: front-matter validation. Term casing
//! needs none: the engine reports it as `core:terms.casing`.

use slopvac_config::frontmatter::{self, Policy};
use slopvac_model::{Severity, Span};

use crate::linter::{Check, Problem};

/// Front-matter validation: the `slopvac:` block checked as `slopvac validate` checks it. Each
/// diagnostic keeps its validate code and points at the span validate reports.
pub struct FrontMatterCheck {
    policy: Policy,
}

impl FrontMatterCheck {
    /// Validate with the project's `[frontmatter]` policy.
    #[must_use]
    pub fn new(policy: Policy) -> Self {
        Self { policy }
    }
}

impl Check for FrontMatterCheck {
    fn check(&self, path: &str, text: &str) -> Vec<Problem> {
        frontmatter::read(text, path, &self.policy)
            .diagnostics
            .into_iter()
            .filter_map(|d| {
                // A span past the buffer cannot be shown, so it is dropped.
                let span = Span::new(d.span.start, d.span.end).ok()?;
                Some(Problem {
                    span,
                    severity: Severity::Error,
                    code: d.code.as_str().to_owned(),
                    message: d.message,
                })
            })
            .collect()
    }
}
