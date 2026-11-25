//! The X3 budgets (`docs/contract/threat-model.md`, section 6.7) for the packs of one run.
//!
//! One [`Budget`] follows a run from the first pack file read to the last pack registered. The
//! loader charges it while it reads, before it compiles and after it compiles each pattern, so a
//! run stops as soon as a budget is crossed and no later pack is read and no later pattern
//! compiles. [`crate::Packs::new`] charges a fresh budget with the packs it is given, through the
//! same methods, so a caller that loads packs some other way meets the same limits. A refusal is
//! [`PackError::Budget`], which reports `rules-budget-exceeded` with the name of the budget.

use slopvac_rules::{BudgetHit, CompileBudget, PackBudget};

use crate::{LoadedPack, PackError};

/// Most files one pack may list in `provides`, once globs are expanded (`plugin-files`).
pub const MAX_PACK_FILES: usize = slopvac_rules::MAX_PACK_FILES;
/// Most bytes of files one pack may supply, manifest included: 32 MiB (`plugin-bytes`).
pub const MAX_PACK_BYTES: u64 = slopvac_rules::MAX_PACK_BYTES;
/// Most bytes of pack files that all packs of a run may supply together, manifests included:
/// 128 MiB (`plugin-run-bytes`).
pub const MAX_RUN_BYTES: u64 = 4 * MAX_PACK_BYTES;
/// Most rules that all packs of a run may declare together (`plugin-run-rules`).
pub const MAX_RUN_RULES: usize = 20_000;
/// Most bytes of compiled rule patterns that all packs of a run may hold together
/// (`plugin-run-pattern-bytes`): the size PCRE2 reports for each pattern (`PCRE2_INFO_SIZE`).
///
/// Each pattern is at most 8,192 bytes and each rule at most 4,096 tokens. The built-in core
/// uses a small part of the budget (the `the_core_pack_stays_far_below_the_pattern_budget`
/// test holds it under a tenth).
pub const MAX_RUN_PATTERN_BYTES: u64 = 64 * 1024 * 1024;

/// The limits of a run. [`Limits::default`] holds the values above; a test passes smaller ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Limits {
    /// Files per pack.
    pub pack_files: usize,
    /// Bytes per pack.
    pub pack_bytes: u64,
    /// Bytes for the whole run.
    pub run_bytes: u64,
    /// Rules for the whole run.
    pub run_rules: usize,
    /// Compiled pattern bytes for the whole run.
    pub run_pattern_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            pack_files: MAX_PACK_FILES,
            pack_bytes: MAX_PACK_BYTES,
            run_bytes: MAX_RUN_BYTES,
            run_rules: MAX_RUN_RULES,
            run_pattern_bytes: MAX_RUN_PATTERN_BYTES,
        }
    }
}

/// What a run has used so far.
#[derive(Debug, Clone)]
pub struct Budget {
    limits: Limits,
    bytes: u64,
    rules: usize,
    pattern_bytes: u64,
}

impl Default for Budget {
    fn default() -> Self {
        Self::new(Limits::default())
    }
}

impl Budget {
    /// An unused budget under `limits`.
    #[must_use]
    pub const fn new(limits: Limits) -> Self {
        Self {
            limits,
            bytes: 0,
            rules: 0,
            pattern_bytes: 0,
        }
    }

    /// The limits.
    #[must_use]
    pub const fn limits(&self) -> &Limits {
        &self.limits
    }

    /// Count `n` bytes read from a pack file.
    pub(crate) fn charge_bytes(&mut self, n: u64) -> Result<(), PackError> {
        self.bytes = self.bytes.saturating_add(n);
        if self.bytes > self.limits.run_bytes {
            return Err(refusal(PackBudget::RunBytes, self.limits.run_bytes));
        }
        Ok(())
    }

    /// Count `n` rules a pack declares, before any is compiled.
    pub(crate) fn charge_rules(&mut self, n: usize) -> Result<(), PackError> {
        self.rules = self.rules.saturating_add(n);
        if self.rules > self.limits.run_rules {
            return Err(refusal(PackBudget::RunRules, self.limits.run_rules as u64));
        }
        Ok(())
    }

    /// The compiled pattern bytes counted so far.
    pub(crate) const fn pattern_used(&self) -> u64 {
        self.pattern_bytes
    }

    /// Count `n` bytes of compiled patterns that a pack already holds.
    fn charge_patterns(&mut self, n: u64) -> Result<(), PackError> {
        self.pattern_bytes = self.pattern_bytes.saturating_add(n);
        self.check_patterns().map_err(PackError::from)
    }

    fn check_patterns(&self) -> Result<(), BudgetHit> {
        if self.pattern_bytes > self.limits.run_pattern_bytes {
            return Err(BudgetHit {
                budget: PackBudget::RunPatternBytes,
                limit: self.limits.run_pattern_bytes,
            });
        }
        Ok(())
    }

    /// Count everything a loaded pack used.
    pub(crate) fn charge_pack(&mut self, pack: &LoadedPack) -> Result<(), PackError> {
        self.charge_bytes(pack.bytes)?;
        self.charge_rules(pack.rule_count())?;
        self.charge_patterns(pack.pattern_bytes)
    }
}

/// A pattern reserves its source bytes before it compiles, so a budget with nothing left
/// compiles nothing. When it has compiled, the reservation gives way to the compiled size.
impl CompileBudget for Budget {
    fn reserve(&mut self, source_bytes: usize) -> Result<(), BudgetHit> {
        self.pattern_bytes = self.pattern_bytes.saturating_add(source_bytes as u64);
        self.check_patterns()
    }

    fn settle(&mut self, source_bytes: usize, compiled_bytes: usize) -> Result<(), BudgetHit> {
        self.pattern_bytes = self
            .pattern_bytes
            .saturating_sub(source_bytes as u64)
            .saturating_add(compiled_bytes as u64);
        self.check_patterns()
    }
}

pub(crate) fn refusal(budget: PackBudget, limit: u64) -> PackError {
    PackError::Budget {
        budget,
        limit,
        plugin: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Source, core_source};

    #[test]
    fn the_core_pack_stays_far_below_the_pattern_budget() {
        let mut budget = Budget::default();

        LoadedPack::load_within(&core_source(), Source::Core, &mut budget).unwrap();

        assert!(budget.pattern_used() > 0);
        assert!(
            budget.pattern_used() < MAX_RUN_PATTERN_BYTES / 10,
            "{}",
            budget.pattern_used()
        );
    }
}
