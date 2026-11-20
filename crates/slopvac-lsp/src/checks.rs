//! The checks the binary registers next to the engine: front-matter validation and terms.

use slopvac_config::frontmatter::{self, Policy};
use slopvac_model::{Severity, Span};
use slopvac_terms::TermSet;

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

/// The code of a term spelled in a way the terms list does not accept. It is the id of the
/// engine rule that reports the same finding.
pub const TERMS_CASING_CODE: &str = "terms:terms.casing";

/// The terms check: each term spelled differently from its list entry.
pub struct TermsCheck {
    terms: TermSet,
}

impl TermsCheck {
    /// Check against `terms`.
    #[must_use]
    pub fn new(terms: TermSet) -> Self {
        Self { terms }
    }
}

impl Check for TermsCheck {
    fn check(&self, _path: &str, text: &str) -> Vec<Problem> {
        self.terms
            .check_casing(text, &[])
            .into_iter()
            .filter_map(|f| {
                let span = Span::new(f.span.start, f.span.end).ok()?;
                Some(Problem {
                    span,
                    severity: Severity::Warning,
                    code: TERMS_CASING_CODE.to_owned(),
                    message: format!("Write `{}` as `{}`.", f.found, f.expected),
                })
            })
            .collect()
    }
}
