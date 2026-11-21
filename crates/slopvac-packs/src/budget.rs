//! The X3 budgets (`docs/contract/threat-model.md`, section 6.7) for the packs of one run.
//!
//! One [`Budget`] follows a run from the first pack file read to the last pack registered. The
//! loader charges it while it reads, before it compiles and after it compiles, so a run stops as
//! soon as a budget is crossed and no later pack is read. [`crate::Packs::new`] charges a fresh
//! budget with the packs it is given, through the same methods, so a caller that loads packs
//! some other way meets the same limits.

use crate::{LimitKind, LoadedPack, PackError};

/// Most files one pack may list in `provides`, once globs are expanded.
pub const MAX_PACK_FILES: usize = slopvac_rules::MAX_PACK_FILES;
/// Most bytes of files one pack may supply: 32 MiB.
pub const MAX_PACK_BYTES: u64 = slopvac_rules::MAX_PACK_BYTES;
/// Most bytes of pack files that all packs of a run may supply together: 128 MiB.
pub const MAX_RUN_BYTES: u64 = 4 * MAX_PACK_BYTES;
/// Most rules that all packs of a run may declare together.
pub const MAX_RUN_RULES: usize = 20_000;
/// Most bytes of authored rule patterns that all packs of a run may compile together: 16 MiB.
///
/// PCRE2 does not report the size of compiled code, so the authored pattern text is the
/// measure. Each pattern is at most 8,192 bytes and each rule at most 4,096 tokens, so this
/// budget and [`MAX_RUN_RULES`] together bound the work of compiling.
pub const MAX_RUN_PATTERN_BYTES: u64 = 16 * 1024 * 1024;

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
    /// Authored pattern bytes for the whole run.
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
            return Err(over(LimitKind::Bytes, self.limits.run_bytes));
        }
        Ok(())
    }

    /// Count `n` rules a pack declares, before any is compiled.
    pub(crate) fn charge_rules(&mut self, n: usize) -> Result<(), PackError> {
        self.rules = self.rules.saturating_add(n);
        if self.rules > self.limits.run_rules {
            return Err(over(LimitKind::Rules, self.limits.run_rules as u64));
        }
        Ok(())
    }

    /// Count `n` bytes of authored patterns that were compiled.
    pub(crate) fn charge_pattern_bytes(&mut self, n: u64) -> Result<(), PackError> {
        self.pattern_bytes = self.pattern_bytes.saturating_add(n);
        if self.pattern_bytes > self.limits.run_pattern_bytes {
            return Err(over(LimitKind::PatternBytes, self.limits.run_pattern_bytes));
        }
        Ok(())
    }

    /// Count everything a loaded pack used.
    pub(crate) fn charge_pack(&mut self, pack: &LoadedPack) -> Result<(), PackError> {
        self.charge_bytes(pack.bytes)?;
        self.charge_rules(pack.rule_count())?;
        self.charge_pattern_bytes(pack.pattern_bytes())
    }
}

fn over(what: LimitKind, limit: u64) -> PackError {
    PackError::RunLimit { what, limit }
}
