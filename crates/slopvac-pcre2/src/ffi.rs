//! The only module with `unsafe` code: raw calls into the 8-bit PCRE2 API.
//!
//! Everything here wraps one PCRE2 object in an owning type whose safe methods uphold the
//! C API's preconditions, so `lib.rs` stays entirely safe. The invariants are:
//!
//! * Each pointer held in a `NonNull` comes from a PCRE2 constructor that returned
//!   non-null, is owned by exactly one wrapper and is freed exactly once, in `Drop`.
//! * Subjects are `&str`, so they are valid UTF-8, and every start offset is checked to be
//!   a character boundary no greater than the length. That justifies `PCRE2_NO_UTF_CHECK`.
//! * A compiled `Code` is never modified after construction; `Contexts` and `MatchData`
//!   are used by one thread at a time (see their docs).
#![allow(unsafe_code)]

use std::cell::{OnceCell, RefCell};
use std::ffi::c_void;
use std::ptr::{self, NonNull};

use pcre2_sys::{
    PCRE2_CONFIG_JIT, PCRE2_CONFIG_VERSION, PCRE2_INFO_JITSIZE, PCRE2_JIT_COMPLETE, PCRE2_NO_JIT,
    PCRE2_NO_UTF_CHECK, PCRE2_NOTEMPTY_ATSTART, PCRE2_UNSET, pcre2_code_8, pcre2_code_free_8,
    pcre2_compile_8, pcre2_compile_context_8, pcre2_compile_context_create_8,
    pcre2_compile_context_free_8, pcre2_config_8, pcre2_get_error_message_8,
    pcre2_get_ovector_count_8, pcre2_get_ovector_pointer_8, pcre2_jit_compile_8, pcre2_jit_stack_8,
    pcre2_jit_stack_assign_8, pcre2_jit_stack_create_8, pcre2_jit_stack_free_8, pcre2_match_8,
    pcre2_match_context_8, pcre2_match_context_create_8, pcre2_match_context_free_8,
    pcre2_match_data_8, pcre2_match_data_create_from_pattern_8, pcre2_match_data_free_8,
    pcre2_pattern_info_8, pcre2_set_depth_limit_8, pcre2_set_heap_limit_8, pcre2_set_match_limit_8,
    pcre2_set_max_pattern_length_8, pcre2_set_max_varlookbehind_8, pcre2_set_parens_nest_limit_8,
};

use crate::Limits;

/// A subject for an empty string. PCRE2 may be handed a non-null pointer with length zero.
static EMPTY_SUBJECT: [u8; 1] = [0];

/// Initial JIT stack size. PCRE2's own default for JIT machine stacks.
const JIT_STACK_START: usize = 32 * 1024;

/// A compile failure: PCRE2's error code and the pattern offset it reports.
#[derive(Debug)]
pub(crate) struct CompileFailure {
    pub(crate) code: i32,
    pub(crate) offset: usize,
}

/// A compiled pattern, optionally JIT-compiled.
#[derive(Debug)]
pub(crate) struct Code {
    ptr: NonNull<pcre2_code_8>,
    jit: bool,
}

// SAFETY: a compiled pattern is read-only after `pcre2_compile`/`pcre2_jit_compile`
// return, and the PCRE2 documentation allows one compiled pattern (JIT included) to be
// matched from several threads at once, provided each thread uses its own match data and,
// for JIT, its own JIT stack. `Contexts` is per thread and `MatchData` per search.
unsafe impl Send for Code {}
// SAFETY: see the `Send` impl; `&Code` only permits matching, which does not mutate it.
unsafe impl Sync for Code {}

/// Bounds on what the compiler may accept, set on the compile context.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CompileBounds {
    pub(crate) max_pattern_bytes: usize,
    pub(crate) parens_nest_limit: u32,
    pub(crate) max_var_lookbehind: u32,
}

/// An owned compile context, freed on drop.
struct CompileContext(NonNull<pcre2_compile_context_8>);

impl CompileContext {
    fn new(bounds: CompileBounds) -> Option<Self> {
        // SAFETY: a null general context selects the default allocator.
        let raw = unsafe { pcre2_compile_context_create_8(ptr::null_mut()) };
        let context = Self(NonNull::new(raw)?);
        // SAFETY: `context.0` is a live compile context. The setters fail only for a null
        // context, so their results carry no information here.
        unsafe { pcre2_set_max_pattern_length_8(context.0.as_ptr(), bounds.max_pattern_bytes) };
        // SAFETY: as above.
        unsafe { pcre2_set_parens_nest_limit_8(context.0.as_ptr(), bounds.parens_nest_limit) };
        // SAFETY: as above.
        unsafe { pcre2_set_max_varlookbehind_8(context.0.as_ptr(), bounds.max_var_lookbehind) };
        Some(context)
    }
}

impl Drop for CompileContext {
    fn drop(&mut self) {
        // SAFETY: created by `pcre2_compile_context_create_8`, freed only here.
        unsafe { pcre2_compile_context_free_8(self.0.as_ptr()) }
    }
}

impl Code {
    /// Compile `pattern` under `bounds` with the given PCRE2 option bits. When `jit` is
    /// set, also try to JIT it; `is_jit` is true only if JIT code really exists.
    pub(crate) fn compile(
        pattern: &str,
        options: u32,
        bounds: CompileBounds,
        jit: bool,
    ) -> Result<Self, CompileFailure> {
        let Some(context) = CompileContext::new(bounds) else {
            return Err(CompileFailure {
                code: pcre2_sys::PCRE2_ERROR_NOMEMORY,
                offset: 0,
            });
        };
        let mut code = 0_i32;
        let mut offset = 0_usize;
        // SAFETY: `pattern` points to `pattern.len()` readable bytes (a dangling but
        // non-null pointer when the length is zero, which PCRE2 never reads); `code` and
        // `offset` are valid for writes; `context` is a live compile context.
        let raw = unsafe {
            pcre2_compile_8(
                pattern.as_ptr(),
                pattern.len(),
                options,
                &raw mut code,
                &raw mut offset,
                context.0.as_ptr(),
            )
        };
        let ptr = NonNull::new(raw).ok_or(CompileFailure { code, offset })?;
        let mut code = Self { ptr, jit: false };
        if jit {
            // SAFETY: `code.ptr` is a live compiled pattern that no one else references yet.
            let rc = unsafe { pcre2_jit_compile_8(code.ptr.as_ptr(), PCRE2_JIT_COMPLETE) };
            // A non-zero code means no JIT (unsupported target or out of memory). A zero
            // code does not prove JIT code exists: patterns that start with `(*NO_JIT)`
            // "succeed" and keep running on the interpreter. Ask for the JIT code size.
            code.jit = rc == 0 && code.jit_size() > 0;
        }
        Ok(code)
    }

    /// Size in bytes of the JIT code, or zero if there is none.
    fn jit_size(&self) -> usize {
        let mut size = 0_usize;
        // SAFETY: `self.ptr` is a live compiled pattern; `PCRE2_INFO_JITSIZE` writes one
        // `usize` to `where`, which is valid for it.
        let rc = unsafe {
            pcre2_pattern_info_8(
                self.ptr.as_ptr(),
                PCRE2_INFO_JITSIZE,
                (&raw mut size).cast::<c_void>(),
            )
        };
        if rc == 0 { size } else { 0 }
    }

    /// Whether JIT code exists for this pattern.
    pub(crate) fn is_jit(&self) -> bool {
        self.jit
    }
}

impl Drop for Code {
    fn drop(&mut self) {
        // SAFETY: `ptr` was returned by `pcre2_compile_8` and is freed only here. The JIT
        // code attached to it is released by the same call.
        unsafe { pcre2_code_free_8(self.ptr.as_ptr()) }
    }
}

/// Match data sized for one pattern. Not shared: each search owns one.
#[derive(Debug)]
pub(crate) struct MatchData {
    ptr: NonNull<pcre2_match_data_8>,
}

impl MatchData {
    /// Allocate match data with room for every capture group of `code`.
    pub(crate) fn new(code: &Code) -> Option<Self> {
        // SAFETY: `code.ptr` is a live compiled pattern; a null general context selects
        // the default allocator.
        let raw =
            unsafe { pcre2_match_data_create_from_pattern_8(code.ptr.as_ptr(), ptr::null_mut()) };
        NonNull::new(raw).map(|ptr| Self { ptr })
    }

    /// Number of ovector pairs: capture groups including group 0.
    pub(crate) fn group_count(&self) -> usize {
        // SAFETY: `self.ptr` is live match data.
        unsafe { pcre2_get_ovector_count_8(self.ptr.as_ptr()) as usize }
    }

    /// Byte offsets of group `index` after a successful match, or `None` if the group
    /// did not take part or `index` is out of range.
    pub(crate) fn group(&self, index: usize) -> Option<(usize, usize)> {
        let count = self.group_count();
        if index >= count {
            return None;
        }
        // SAFETY: `self.ptr` is live match data.
        let vector = unsafe { pcre2_get_ovector_pointer_8(self.ptr.as_ptr()) };
        // SAFETY: the ovector is an initialised, aligned array of `2 * count` `usize`
        // values that lives as long as `self`. The slice borrows `self`, and PCRE2 writes
        // to it only during a match, which needs `&mut MatchData` and so cannot overlap.
        let pairs = unsafe { std::slice::from_raw_parts(vector, 2 * count) };
        let (start, end) = (pairs[2 * index], pairs[2 * index + 1]);
        (start != PCRE2_UNSET && end != PCRE2_UNSET).then_some((start, end))
    }
}

impl Drop for MatchData {
    fn drop(&mut self) {
        // SAFETY: allocated by `pcre2_match_data_create_from_pattern_8`, freed only here.
        unsafe { pcre2_match_data_free_8(self.ptr.as_ptr()) }
    }
}

/// Which engine a match call runs on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Engine {
    /// The JIT, under the JIT match limit and JIT stack. Opt-in; see `Options::jit`.
    Jit,
    /// The interpreter, under the match, depth and heap limits. The default.
    Interpreter,
}

/// Why `exec` could not run a match.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExecError {
    /// The start offset is not a character boundary of the subject.
    BadOffset,
    /// PCRE2 could not allocate the JIT context or stack.
    OutOfMemory,
}

/// The JIT match context and its stack: built on first use, because the interpreter is
/// the default engine and most threads never need them.
#[derive(Debug)]
struct JitParts {
    context: NonNull<pcre2_match_context_8>,
    stack: NonNull<pcre2_jit_stack_8>,
}

impl JitParts {
    fn new(limits: Limits) -> Option<Self> {
        if !jit_supported() {
            return None;
        }
        // SAFETY: a null general context selects the default allocator.
        let context = NonNull::new(unsafe { pcre2_match_context_create_8(ptr::null_mut()) })?;
        set_limits(context, limits, limits.jit_match_limit);
        // SAFETY: a null general context selects the default allocator; the sizes are
        // plain numbers (PCRE2 returns null if they are inconsistent).
        let raw = unsafe {
            pcre2_jit_stack_create_8(
                JIT_STACK_START.min(limits.jit_stack_bytes),
                limits.jit_stack_bytes,
                ptr::null_mut(),
            )
        };
        let Some(stack) = NonNull::new(raw) else {
            // SAFETY: `context` was just created and is not stored anywhere else.
            unsafe { pcre2_match_context_free_8(context.as_ptr()) };
            return None;
        };
        // SAFETY: `context` is live; with a null callback the third argument is the JIT
        // stack itself, which the returned value keeps alive until after the context is
        // freed.
        unsafe {
            pcre2_jit_stack_assign_8(context.as_ptr(), None, stack.as_ptr().cast::<c_void>());
        };
        Some(Self { context, stack })
    }
}

impl Drop for JitParts {
    fn drop(&mut self) {
        // SAFETY: created by `pcre2_match_context_create_8`, freed only here. The context
        // goes first: it refers to the stack, never the reverse.
        unsafe { pcre2_match_context_free_8(self.context.as_ptr()) };
        // SAFETY: created by `pcre2_jit_stack_create_8`, freed only here, after the context.
        unsafe { pcre2_jit_stack_free_8(self.stack.as_ptr()) };
    }
}

/// Apply a match limit plus the depth and heap limits to a match context. The interpreter
/// context gets the interpreter's match limit and the JIT context the JIT's, which counts
/// differently. Depth and heap go on both: they matter to the interpreter, and the JIT
/// context carries them so no context is left on PCRE2's much larger defaults.
fn set_limits(context: NonNull<pcre2_match_context_8>, limits: Limits, match_limit: u32) {
    // SAFETY: `context` is a live match context owned by the caller.
    unsafe { pcre2_set_match_limit_8(context.as_ptr(), match_limit) };
    // SAFETY: as above.
    unsafe { pcre2_set_depth_limit_8(context.as_ptr(), limits.depth_limit) };
    // SAFETY: as above.
    unsafe { pcre2_set_heap_limit_8(context.as_ptr(), limits.heap_limit_kib) };
}

/// The match contexts for one thread: the interpreter's, plus the JIT's once asked for.
///
/// A PCRE2 JIT stack must not be used by two matches at once, so a `Contexts` lives in a
/// thread-local (see [`with_contexts`]); it holds raw pointers and is neither `Send`
/// nor `Sync`.
#[derive(Debug)]
pub(crate) struct Contexts {
    limits: Limits,
    interpreter: NonNull<pcre2_match_context_8>,
    jit: OnceCell<Option<JitParts>>,
}

impl Contexts {
    /// Build the interpreter context with the limits in `limits`. `None` means out of memory.
    fn new(limits: Limits) -> Option<Self> {
        // SAFETY: a null general context selects the default allocator.
        let interpreter = NonNull::new(unsafe { pcre2_match_context_create_8(ptr::null_mut()) })?;
        set_limits(interpreter, limits, limits.match_limit);
        Some(Self {
            limits,
            interpreter,
            jit: OnceCell::new(),
        })
    }

    fn jit_context(&self) -> Option<NonNull<pcre2_match_context_8>> {
        self.jit
            .get_or_init(|| JitParts::new(self.limits))
            .as_ref()
            .map(|parts| parts.context)
    }
}

impl Drop for Contexts {
    fn drop(&mut self) {
        // SAFETY: created by `pcre2_match_context_create_8` and freed only here.
        unsafe { pcre2_match_context_free_8(self.interpreter.as_ptr()) };
    }
}

thread_local! {
    static CONTEXTS: RefCell<Option<Contexts>> = const { RefCell::new(None) };
}

/// Run `f` with this thread's contexts for `limits`, building them on first use or when
/// the limits differ from the cached ones.
///
/// `f` runs no user code and cannot re-enter, so the JIT stack is never used twice at
/// once. During thread teardown the thread-local is gone and a temporary set is used.
/// `None` means PCRE2 could not allocate the contexts.
pub(crate) fn with_contexts<R>(limits: Limits, f: impl FnOnce(&Contexts) -> R) -> Option<R> {
    let mut f = Some(f);
    let cached = CONTEXTS.try_with(|slot| {
        let mut slot = slot.try_borrow_mut().ok()?;
        if slot.as_ref().is_none_or(|c| c.limits != limits) {
            // Drop the old set first so two JIT stacks never coexist.
            *slot = None;
            *slot = Some(Contexts::new(limits)?);
        }
        let contexts = slot.as_ref()?;
        f.take().map(|f| f(contexts))
    });
    match cached {
        Ok(result) => result,
        Err(_) => f.take().zip(Contexts::new(limits)).map(|(f, c)| f(&c)),
    }
}

/// Run one `pcre2_match` call and return its raw result code (`> 0` matched, negative
/// is an error such as `PCRE2_ERROR_NOMATCH`).
///
/// Fails if `start` is not a character boundary of `subject`, which would break the UTF-8
/// precondition that `PCRE2_NO_UTF_CHECK` relies on.
pub(crate) fn exec(
    code: &Code,
    subject: &str,
    start: usize,
    not_empty_at_start: bool,
    data: &mut MatchData,
    contexts: &Contexts,
    engine: Engine,
) -> Result<i32, ExecError> {
    if !subject.is_char_boundary(start) {
        return Err(ExecError::BadOffset);
    }
    let mut options = PCRE2_NO_UTF_CHECK;
    if not_empty_at_start {
        options |= PCRE2_NOTEMPTY_ATSTART;
    }
    let context = match engine {
        Engine::Jit => contexts.jit_context().ok_or(ExecError::OutOfMemory)?,
        Engine::Interpreter => {
            options |= PCRE2_NO_JIT;
            contexts.interpreter
        }
    };
    let bytes = if subject.is_empty() {
        EMPTY_SUBJECT.as_ptr()
    } else {
        subject.as_ptr()
    };
    // SAFETY: `code` and `data` are live; `data` was sized for `code`. `bytes` points to
    // `subject.len()` readable bytes of valid UTF-8 and `start <= subject.len()` on a
    // character boundary (checked above), so `PCRE2_NO_UTF_CHECK` is sound. `context` is
    // a live match context owned by `contexts`, used by this thread alone.
    let rc = unsafe {
        pcre2_match_8(
            code.ptr.as_ptr(),
            bytes,
            subject.len(),
            start,
            options,
            data.ptr.as_ptr(),
            context.as_ptr(),
        )
    };
    Ok(rc)
}

/// The linked library's version string, such as `10.46 2025-08-27`.
pub(crate) fn version_string() -> String {
    // SAFETY: with a null `where`, `pcre2_config` only returns the needed buffer size
    // (in bytes, terminator included) for string-valued items such as the version.
    let needed = unsafe { pcre2_config_8(PCRE2_CONFIG_VERSION, ptr::null_mut()) };
    let Ok(needed) = usize::try_from(needed) else {
        return String::new();
    };
    let mut buffer = vec![0_u8; needed.max(1)];
    // SAFETY: `buffer` is writable for `needed` bytes, the size PCRE2 just asked for.
    unsafe { pcre2_config_8(PCRE2_CONFIG_VERSION, buffer.as_mut_ptr().cast::<c_void>()) };
    let end = buffer.iter().position(|&b| b == 0).unwrap_or(buffer.len());
    String::from_utf8_lossy(&buffer[..end]).into_owned()
}

/// Whether the linked PCRE2 has JIT support.
fn jit_supported() -> bool {
    let mut value = 0_u32;
    // SAFETY: `PCRE2_CONFIG_JIT` writes one `u32` to `where`, which is valid for it.
    unsafe { pcre2_config_8(PCRE2_CONFIG_JIT, (&raw mut value).cast::<c_void>()) };
    value == 1
}

/// PCRE2's text for an error code, or a generic text if it has none.
pub(crate) fn error_message(code: i32) -> String {
    let mut buffer = [0_u8; 256];
    // SAFETY: `buffer` is writable for its full length, which is passed as the capacity.
    let written = unsafe { pcre2_get_error_message_8(code, buffer.as_mut_ptr(), buffer.len()) };
    match usize::try_from(written) {
        Ok(n) if n <= buffer.len() => String::from_utf8_lossy(&buffer[..n]).into_owned(),
        _ => format!("PCRE2 error {code}"),
    }
}
