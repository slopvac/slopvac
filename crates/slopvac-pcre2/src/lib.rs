//! A safe wrapper over the vendored PCRE2 with slopvac's build and limits policy
//! (bead slopvac-8l4.3.20).
//!
//! The `pcre2` crate gives no control over the match, depth or heap limits, and the JIT
//! match limit does not bound CPU time, so slopvac calls PCRE2 through `pcre2-sys`
//! directly. This crate owns that FFI and keeps `unsafe` in one module (`ffi`).
//!
//! # Policy
//!
//! * The linked PCRE2 must be at least 10.43 (bounded variable-length lookbehind). The
//!   workspace pins `pcre2-sys` and `.cargo/config.toml` forces its vendored build;
//!   [`ensure_supported_version`] fails at compile time of the first pattern if a system
//!   library was linked anyway.
//! * Patterns compile in UTF mode with `\C` refused ([`CompileError::BackslashC`]), optionally with Unicode properties ([`Options`]), under
//!   a compile context that caps the pattern at [`MAX_PATTERN_BYTES`], the parenthesis
//!   nesting at [`PARENS_NEST_LIMIT`] and variable-length lookbehind at
//!   [`MAX_VAR_LOOKBEHIND`]. PCRE2's compiler memory is bounded by those caps; there is no
//!   custom allocator.
//! * **Every match runs on the interpreter** under [`Limits::MATCH`], [`Limits::DEPTH`] and
//!   [`Limits::HEAP_KIB`]. The match limit counts each backtracking step, so the work of one
//!   call is bounded. Hitting a limit is a [`MatchError`], never a panic and never "no
//!   match".
//! * The JIT is opt-in ([`Options::jit`]) and for trusted patterns only. Its match limit
//!   does not count every step of every loop (overlapping greedy repetitions can backtrack
//!   quadratically without touching it), so it does not bound CPU time. When on, a JIT
//!   attempt runs under [`Limits::JIT_MATCH`] and a [`Limits::JIT_STACK`] byte stack, uses
//!   the JIT only if PCRE2 really produced JIT code (`PCRE2_INFO_JITSIZE > 0`), and redoes
//!   the call on the interpreter when the JIT gives up.
//! * [`Regex::find_all`] follows Python's `re.finditer`: an empty match may follow a
//!   non-empty one, but not another empty match at the same position.

mod ffi;
mod version;

use std::ops::Range;
use std::sync::Arc;

pub use version::{VersionError, ensure_supported_version, linked_version};

use pcre2_sys::{
    PCRE2_CASELESS, PCRE2_ERROR_BACKSLASH_C_CALLER_DISABLED, PCRE2_ERROR_DEPTHLIMIT,
    PCRE2_ERROR_HEAP_FAILED, PCRE2_ERROR_HEAPLIMIT, PCRE2_ERROR_JIT_STACKLIMIT,
    PCRE2_ERROR_MATCHLIMIT, PCRE2_ERROR_NOMATCH, PCRE2_ERROR_NOMEMORY,
    PCRE2_ERROR_PARENTHESES_NEST_TOO_DEEP, PCRE2_NEVER_BACKSLASH_C, PCRE2_UCP, PCRE2_UTF,
};

use ffi::{Code, Contexts, Engine, ExecError, MatchData};

/// Resource limits for one [`Regex`].
///
/// The interpreter limits bound the work and memory of a single `pcre2_match` call. The
/// JIT limits bound the JIT attempt that precedes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Interpreter match limit: calls of the internal `match()` function.
    pub match_limit: u32,
    /// Interpreter backtracking depth limit.
    pub depth_limit: u32,
    /// Interpreter heap limit in KiB (PCRE2's default is about 20 GB).
    pub heap_limit_kib: u32,
    /// JIT match limit, used only when the JIT is on. It counts differently from the
    /// interpreter's and does not bound CPU time, so reaching it only triggers the
    /// interpreter retry.
    pub jit_match_limit: u32,
    /// Maximum JIT stack size in bytes.
    pub jit_stack_bytes: usize,
}

impl Limits {
    /// Interpreter match limit of the policy.
    pub const MATCH: u32 = 10_000_000;
    /// Interpreter depth limit of the policy.
    pub const DEPTH: u32 = 10_000_000;
    /// Interpreter heap limit of the policy: 256 MiB.
    pub const HEAP_KIB: u32 = 256 * 1024;
    /// JIT match limit of the policy.
    pub const JIT_MATCH: u32 = 100_000;
    /// JIT stack size of the policy: 8 MiB.
    pub const JIT_STACK: usize = 8 * 1024 * 1024;

    /// The slopvac policy, used by [`Regex::new`].
    pub const POLICY: Self = Self {
        match_limit: Self::MATCH,
        depth_limit: Self::DEPTH,
        heap_limit_kib: Self::HEAP_KIB,
        jit_match_limit: Self::JIT_MATCH,
        jit_stack_bytes: Self::JIT_STACK,
    };
}

impl Default for Limits {
    fn default() -> Self {
        Self::POLICY
    }
}

/// Maximum pattern length in bytes. Longer patterns fail with [`CompileError::TooLong`].
pub const MAX_PATTERN_BYTES: usize = 64 * 1024;
/// Maximum nesting depth of parentheses (PCRE2's default, set explicitly).
pub const PARENS_NEST_LIMIT: u32 = 250;
/// Maximum length of a variable-length lookbehind branch (PCRE2's default, set explicitly).
pub const MAX_VAR_LOOKBEHIND: u32 = 255;

/// Compile options. UTF mode is always on; the JIT is off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    caseless: bool,
    ucp: bool,
    jit: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self::new()
    }
}

impl Options {
    /// Case-sensitive, no Unicode properties for `\d`, `\w` and POSIX classes, no JIT.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            caseless: false,
            ucp: false,
            jit: false,
        }
    }

    /// Try to JIT-compile the pattern (off by default). The JIT is faster but does not
    /// bound CPU time (see the crate docs), so enable it only for patterns you wrote.
    #[must_use]
    pub const fn jit(mut self, on: bool) -> Self {
        self.jit = on;
        self
    }

    /// Match case-insensitively.
    #[must_use]
    pub const fn caseless(mut self, on: bool) -> Self {
        self.caseless = on;
        self
    }

    /// Make `\d`, `\s`, `\w` and POSIX classes use Unicode properties (`PCRE2_UCP`).
    #[must_use]
    pub const fn ucp(mut self, on: bool) -> Self {
        self.ucp = on;
        self
    }

    fn bits(self) -> u32 {
        // `\C` matches one byte, so in UTF mode a match could start or end inside a
        // character and break every caller that slices a `&str`. Never allow it.
        let mut bits = PCRE2_UTF | PCRE2_NEVER_BACKSLASH_C;
        if self.caseless {
            bits |= PCRE2_CASELESS;
        }
        if self.ucp {
            bits |= PCRE2_UCP;
        }
        bits
    }
}

/// A pattern failed to compile.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum CompileError {
    /// PCRE2 rejected the pattern.
    #[error("{message} (at pattern offset {offset})")]
    Syntax {
        /// PCRE2's error text.
        message: String,
        /// Byte offset in the pattern where PCRE2 gave up.
        offset: usize,
    },
    /// The pattern is longer than [`MAX_PATTERN_BYTES`].
    #[error("pattern is {len} bytes, over the limit of {max}")]
    TooLong {
        /// Length of the pattern in bytes.
        len: usize,
        /// The limit, [`MAX_PATTERN_BYTES`].
        max: usize,
    },
    /// Parentheses nest deeper than [`PARENS_NEST_LIMIT`].
    #[error("parentheses nest deeper than {limit} levels (at pattern offset {offset})")]
    TooDeeplyNested {
        /// The limit, [`PARENS_NEST_LIMIT`].
        limit: u32,
        /// Byte offset in the pattern where PCRE2 gave up.
        offset: usize,
    },
    /// The pattern uses `\C`, which matches a single byte and so can split a character.
    #[error("`\\C` (match one byte) is not allowed (at pattern offset {offset})")]
    BackslashC {
        /// Byte offset in the pattern where PCRE2 found it.
        offset: usize,
    },
    /// PCRE2 could not allocate memory to compile the pattern.
    #[error("PCRE2 ran out of memory compiling the pattern")]
    OutOfMemory,
    /// The linked PCRE2 is older than the policy minimum.
    #[error(transparent)]
    Version(#[from] VersionError),
}

/// A match did not finish. The document is incomplete, not clean.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum MatchError {
    /// The interpreter exceeded its match limit.
    #[error("match limit of {limit} exceeded")]
    MatchLimit {
        /// The limit that was hit.
        limit: u32,
    },
    /// The interpreter exceeded its backtracking depth limit.
    #[error("depth limit of {limit} exceeded")]
    DepthLimit {
        /// The limit that was hit.
        limit: u32,
    },
    /// The interpreter exceeded its heap limit.
    #[error("heap limit of {kib} KiB exceeded")]
    HeapLimit {
        /// The limit that was hit, in KiB.
        kib: u32,
    },
    /// PCRE2 could not allocate memory for a match.
    #[error("PCRE2 ran out of memory")]
    OutOfMemory,
    /// PCRE2 reported offsets that are not usable on the subject. Not expected for
    /// patterns without `\K` inside lookarounds.
    #[error("PCRE2 returned offsets outside the subject")]
    InvalidOffsets,
    /// Any other PCRE2 matching error.
    #[error("{message} (PCRE2 error {code})")]
    Engine {
        /// PCRE2's negative error code.
        code: i32,
        /// PCRE2's error text.
        message: String,
    },
}

impl MatchError {
    /// Whether the error is a resource limit, as opposed to an engine fault.
    #[must_use]
    pub fn is_limit(&self) -> bool {
        matches!(
            self,
            Self::MatchLimit { .. } | Self::DepthLimit { .. } | Self::HeapLimit { .. }
        )
    }
}

/// Capture groups of one match: group 0 is the whole match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Captures(Vec<Option<Range<usize>>>);

impl Captures {
    /// Byte range of group `index`, or `None` if it is unset or out of range.
    #[must_use]
    pub fn get(&self, index: usize) -> Option<Range<usize>> {
        self.0.get(index).cloned().flatten()
    }

    /// Number of groups including group 0.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Always false: a match has at least group 0.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// A compiled pattern. Cloning is cheap and shares the compiled code.
#[derive(Clone, Debug)]
pub struct Regex {
    code: Arc<Code>,
    limits: Limits,
}

impl Regex {
    /// Compile `pattern` under [`Limits::POLICY`].
    pub fn new(pattern: &str, options: Options) -> Result<Self, CompileError> {
        Self::with_limits(pattern, options, Limits::POLICY)
    }

    /// Compile `pattern` under custom limits. Lower limits make the limit paths testable.
    pub fn with_limits(
        pattern: &str,
        options: Options,
        limits: Limits,
    ) -> Result<Self, CompileError> {
        ensure_supported_version()?;
        if pattern.len() > MAX_PATTERN_BYTES {
            return Err(CompileError::TooLong {
                len: pattern.len(),
                max: MAX_PATTERN_BYTES,
            });
        }
        let bounds = ffi::CompileBounds {
            max_pattern_bytes: MAX_PATTERN_BYTES,
            parens_nest_limit: PARENS_NEST_LIMIT,
            max_var_lookbehind: MAX_VAR_LOOKBEHIND,
        };
        let code = Code::compile(pattern, options.bits(), bounds, options.jit)
            .map_err(|f| compile_error(&f))?;
        Ok(Self {
            code: Arc::new(code),
            limits,
        })
    }

    /// Size in bytes of the compiled pattern, as `PCRE2_INFO_SIZE` reports it. JIT code is not
    /// counted. A host that bounds the work of loading patterns adds this up.
    #[must_use]
    pub fn compiled_size(&self) -> usize {
        self.code.compiled_size()
    }

    /// Whether the pattern has JIT code (false on targets without a JIT).
    #[must_use]
    pub fn is_jit(&self) -> bool {
        self.code.is_jit()
    }

    /// Whether the pattern matches anywhere in `subject`.
    pub fn is_match(&self, subject: &str) -> Result<bool, MatchError> {
        self.run(|re, data, contexts| re.search(subject, 0, false, data, contexts))
    }

    /// The first match.
    pub fn find(&self, subject: &str) -> Result<Option<Range<usize>>, MatchError> {
        Ok(self.captures(subject)?.and_then(|c| c.get(0)))
    }

    /// The first match with all its capture groups.
    pub fn captures(&self, subject: &str) -> Result<Option<Captures>, MatchError> {
        self.run(|re, data, contexts| {
            if !re.search(subject, 0, false, data, contexts)? {
                return Ok(None);
            }
            let groups = (0..data.group_count())
                .map(|index| span(subject, data.group(index)))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Some(Captures(groups)))
        })
    }

    /// Every non-overlapping match left to right, with Python `re.finditer` semantics.
    pub fn find_all(&self, subject: &str) -> Result<Vec<Range<usize>>, MatchError> {
        self.run(|re, data, contexts| {
            let mut matches = Vec::new();
            let mut position = 0;
            let mut not_empty = false;
            while position <= subject.len() {
                if !re.search(subject, position, not_empty, data, contexts)? {
                    break;
                }
                let found = span(subject, data.group(0))?.ok_or(MatchError::InvalidOffsets)?;
                // A match starting before `position` (only possible through `\K` in a
                // lookaround) would stop the loop making progress.
                if found.start < position {
                    return Err(MatchError::InvalidOffsets);
                }
                position = found.end;
                not_empty = found.is_empty();
                matches.push(found);
            }
            Ok(matches)
        })
    }

    /// Allocate match data and fetch this thread's contexts, then run `f`.
    fn run<R>(
        &self,
        f: impl FnOnce(&Self, &mut MatchData, &Contexts) -> Result<R, MatchError>,
    ) -> Result<R, MatchError> {
        let mut data = MatchData::new(&self.code).ok_or(MatchError::OutOfMemory)?;
        ffi::with_contexts(self.limits, |contexts| f(self, &mut data, contexts))
            .ok_or(MatchError::OutOfMemory)?
    }

    /// One search from `start`. `Ok(true)` means group 0 and the others are in `data`.
    fn search(
        &self,
        subject: &str,
        start: usize,
        not_empty_at_start: bool,
        data: &mut MatchData,
        contexts: &Contexts,
    ) -> Result<bool, MatchError> {
        let engine = if self.code.is_jit() {
            Engine::Jit
        } else {
            Engine::Interpreter
        };
        let call = |data: &mut MatchData, engine| {
            ffi::exec(
                &self.code,
                subject,
                start,
                not_empty_at_start,
                data,
                contexts,
                engine,
            )
            .map_err(|e| match e {
                ExecError::BadOffset => MatchError::InvalidOffsets,
                ExecError::OutOfMemory => MatchError::OutOfMemory,
            })
        };
        let mut rc = call(data, engine)?;
        if engine == Engine::Jit
            && (rc == PCRE2_ERROR_JIT_STACKLIMIT || rc == PCRE2_ERROR_MATCHLIMIT)
        {
            // The JIT budget or stack ran out. Redo the call on the interpreter, which
            // has its own CPU, depth and heap bounds; only those can fail from here on.
            rc = call(data, Engine::Interpreter)?;
        }
        self.classify(rc)
    }

    fn classify(&self, rc: i32) -> Result<bool, MatchError> {
        match rc {
            rc if rc > 0 => Ok(true),
            PCRE2_ERROR_NOMATCH => Ok(false),
            PCRE2_ERROR_NOMEMORY => Err(MatchError::OutOfMemory),
            PCRE2_ERROR_MATCHLIMIT => Err(MatchError::MatchLimit {
                limit: self.limits.match_limit,
            }),
            PCRE2_ERROR_DEPTHLIMIT => Err(MatchError::DepthLimit {
                limit: self.limits.depth_limit,
            }),
            PCRE2_ERROR_HEAPLIMIT => Err(MatchError::HeapLimit {
                kib: self.limits.heap_limit_kib,
            }),
            code => Err(MatchError::Engine {
                code,
                message: ffi::error_message(code),
            }),
        }
    }
}

/// Turn PCRE2 byte offsets into a range, checking they are ordered, inside the subject and
/// both on character boundaries, so callers can slice with them without panicking.
///
/// With `\C` refused at compile time, PCRE2 consumes whole characters and every offset is
/// on a boundary. The check stays as defence in depth against a PCRE2 construct that would
/// break that, and reports [`MatchError::InvalidOffsets`] instead of letting a caller panic.
fn span(
    subject: &str,
    offsets: Option<(usize, usize)>,
) -> Result<Option<Range<usize>>, MatchError> {
    let Some((start, end)) = offsets else {
        return Ok(None);
    };
    if start <= end && subject.is_char_boundary(start) && subject.is_char_boundary(end) {
        Ok(Some(start..end))
    } else {
        Err(MatchError::InvalidOffsets)
    }
}

/// Map a PCRE2 compile failure to a typed error.
fn compile_error(failure: &ffi::CompileFailure) -> CompileError {
    let code = failure.code;
    if code == PCRE2_ERROR_NOMEMORY || code == PCRE2_ERROR_HEAP_FAILED.cast_signed() {
        CompileError::OutOfMemory
    } else if code == PCRE2_ERROR_PARENTHESES_NEST_TOO_DEEP.cast_signed() {
        CompileError::TooDeeplyNested {
            limit: PARENS_NEST_LIMIT,
            offset: failure.offset,
        }
    } else if code == PCRE2_ERROR_BACKSLASH_C_CALLER_DISABLED.cast_signed() {
        CompileError::BackslashC {
            offset: failure.offset,
        }
    } else {
        CompileError::Syntax {
            message: ffi::error_message(code),
            offset: failure.offset,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocation_failures_are_out_of_memory_not_syntax_errors() {
        for code in [PCRE2_ERROR_NOMEMORY, PCRE2_ERROR_HEAP_FAILED.cast_signed()] {
            let failure = ffi::CompileFailure { code, offset: 0 };
            assert_eq!(compile_error(&failure), CompileError::OutOfMemory);
        }
    }

    #[test]
    fn a_match_that_runs_out_of_memory_is_out_of_memory() {
        let re = Regex::new("a", Options::new()).unwrap();
        assert_eq!(
            re.classify(PCRE2_ERROR_NOMEMORY),
            Err(MatchError::OutOfMemory)
        );
    }
}
