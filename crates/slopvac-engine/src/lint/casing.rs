//! Term-casing rules: the project's terms, checked for the case their owners use.
//!
//! The check is [`slopvac_terms::TermSet::check_casing`], a pure function over text. This
//! module feeds it the prose of each unit, where inline code is already masked, and drops a
//! finding in a skipped block, in text the project allows or under a directive. A finding
//! carries a fix that writes the canonical spelling.

use slopvac_model::{Edit, Finding, Fix};
use slopvac_rules::Scope;

use super::{Candidate, Run};
use crate::lint::EngineError;
use crate::matching::format_message;
use crate::skip;

impl Run<'_> {
    /// Findings of the term-casing rules.
    pub(super) fn term_casing(&self) -> Result<Vec<Finding>, EngineError> {
        let engine = self.engine;
        let mut out = Vec::new();
        if engine.casing_rules.is_empty()
            || engine.terms.is_empty()
            || matches!(self.input.kind, super::Kind::Diagram(_))
        {
            return Ok(out);
        }
        let allowed = &engine.settings.allowed_words;
        for active in &engine.casing_rules {
            let set = &engine.sets[active.set];
            let loaded = &set.categories()[active.category].rules()[active.rule];
            let rule = loaded.rule();
            let skip = skip::build(
                self.parsed,
                rule.exclude_blocks.as_deref().unwrap_or(&[]),
                self.front_matter,
                true,
            );
            let exceptions: Vec<&str> = rule.exceptions().iter().map(|e| e.name.as_str()).collect();
            let detectors: Vec<&str> = rule
                .exceptions()
                .iter()
                .filter_map(|e| e.detector.as_ref().map(slopvac_rules::Slug::as_str))
                .collect();
            for unit in self.prepared.units(Scope::Prose)? {
                for hit in engine.terms.check_casing(unit.text, &[]) {
                    let span = unit.span(&hit.span);
                    if skip.hides(span)
                        || allowed.iter().any(|w| w.eq_ignore_ascii_case(&hit.found))
                        || self.guarded_in(
                            self.tags,
                            self.parsed,
                            None,
                            rule,
                            &detectors,
                            unit.text,
                            &hit.span,
                            span,
                            &[],
                        )
                        || self.suppressed(loaded.id().as_str(), &exceptions, span)
                    {
                        continue;
                    }
                    let source = &self.input.source[span.start()..span.end()];
                    // Only text that reads the same in the source can be rewritten.
                    let fix = if source == hit.found {
                        Some(
                            Fix::new(vec![Edit {
                                span,
                                text: hit.expected.clone(),
                            }])
                            .map_err(|e| EngineError::Internal(e.to_string()))?,
                        )
                    } else {
                        None
                    };
                    out.push(self.finding(Candidate {
                        rule_id: loaded.id(),
                        pack: set.pack(),
                        category: loaded.category().clone(),
                        severity: active.severity,
                        message: format_message(rule.message.as_str(), source, &hit.expected),
                        span,
                        matched_text: source.to_owned(),
                        ai_signal: rule.ai_signal,
                        fix,
                        extra_context: None,
                        covering: unit.ranges(&hit.span),
                    })?);
                }
            }
        }
        Ok(out)
    }
}
