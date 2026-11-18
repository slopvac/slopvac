//! Before and after findings, compared per rule.

use std::collections::BTreeMap;
use std::path::Path;

use crate::{Error, RenameTable};

/// Which side of the migration a lint run covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// The 2.x project as it is.
    Before,
    /// The project with the migrated files.
    After,
}

/// The number of findings one rule produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// The rule id as the linter reports it.
    pub rule: String,
    /// How many findings.
    pub count: usize,
}

/// Runs a lint over the project and counts findings per rule.
///
/// `Before` runs against the files on disk. `After` runs against the files the plan
/// would write, so the caller passes them in `overlay` as (path, new text) pairs.
pub trait Linter {
    /// Lint `root` and return findings per rule.
    fn lint(
        &self,
        root: &Path,
        phase: Phase,
        overlay: &[(std::path::PathBuf, String)],
    ) -> Result<Vec<Finding>, Error>;
}

/// A linter that finds nothing, for runs that test the plan and not the findings.
#[derive(Debug, Default, Clone, Copy)]
pub struct NullLinter;

impl Linter for NullLinter {
    fn lint(
        &self,
        _root: &Path,
        _phase: Phase,
        _overlay: &[(std::path::PathBuf, String)],
    ) -> Result<Vec<Finding>, Error> {
        Ok(Vec::new())
    }
}

/// One rule's counts on both sides, under its 3.0 id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleDelta {
    /// The 3.0 rule id.
    pub rule: String,
    /// Findings before the migration.
    pub before: usize,
    /// Findings after the migration.
    pub after: usize,
}

impl RuleDelta {
    /// True when the count changed.
    #[must_use]
    pub fn changed(&self) -> bool {
        self.before != self.after
    }
}

/// Join before and after counts. A 2.x rule id maps to its 3.0 id through the rename
/// table; an id the table does not know keeps its name. Rows sort by rule id.
#[must_use]
pub fn compare(before: &[Finding], after: &[Finding], renames: &RenameTable) -> Vec<RuleDelta> {
    let mut rows: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for f in before {
        let id = renames.rule(&f.rule).unwrap_or_else(|| f.rule.clone());
        rows.entry(id).or_default().0 += f.count;
    }
    for f in after {
        rows.entry(f.rule.clone()).or_default().1 += f.count;
    }
    rows.into_iter()
        .map(|(rule, (before, after))| RuleDelta {
            rule,
            before,
            after,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(rule: &str, count: usize) -> Finding {
        Finding {
            rule: rule.into(),
            count,
        }
    }

    #[test]
    fn maps_2x_ids_and_joins_sides() {
        let rows = compare(
            &[f("orwell.not-un", 3), f("prose-craft.latinisms", 1)],
            &[f("core:orwell.not-un", 2), f("core:prose-craft.spacing", 4)],
            &RenameTable::bundled(),
        );
        assert_eq!(
            rows,
            vec![
                RuleDelta {
                    rule: "core:orwell.not-un".into(),
                    before: 3,
                    after: 2
                },
                RuleDelta {
                    rule: "core:prose-craft.latinisms".into(),
                    before: 1,
                    after: 0
                },
                RuleDelta {
                    rule: "core:prose-craft.spacing".into(),
                    before: 0,
                    after: 4
                },
            ]
        );
    }

    #[test]
    fn unchanged_rows_are_not_changed() {
        assert!(
            !RuleDelta {
                rule: "r".into(),
                before: 2,
                after: 2
            }
            .changed()
        );
    }
}
