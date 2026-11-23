//! The checks the binary registers next to the engine: validation of the `slopvac:` block and
//! the inline directives. Term casing needs none: the engine reports it as
//! `terms:terms.casing`.

use slopvac_config::directives;
use slopvac_config::frontmatter::{self, Policy};
use slopvac_model::{Severity, Span};

use crate::linter::{Check, Problem};

/// Front-matter validation: the `slopvac:` block and the inline directives checked as
/// `slopvac validate` checks them. Each diagnostic keeps its validate code and points at the
/// span validate reports. A directive diagnostic carries fixed text, never a document value.
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
        let block = frontmatter::read(text, path, &self.policy).diagnostics;
        // A directive diagnostic shows its code and span with a fixed text: the message of the
        // validator can quote the value of the document, which stays out of published output
        // (policies, logging and redaction).
        let directive = directives::read(text, path, &self.policy)
            .diagnostics
            .into_iter()
            .map(|mut d| {
                d.message = d.redacted_message();
                d
            });
        block
            .into_iter()
            .chain(directive)
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
