"""SPIKE: build an instrumented source tree.

Usage: python3 apply.py <tree>
<tree> is a scratch COPY of the repository (never a git worktree). The script copies the
overlay files (slopvac-pcre2 lib.rs and spike.rs) into it and applies small string edits to
the other crates. Each edit must match exactly once, so a drift in main fails loudly.
"""
import pathlib
import shutil
import sys

tree = pathlib.Path(sys.argv[1])
here = pathlib.Path(__file__).resolve().parent.parent
overlay = here / "overlay"

for rel in ["crates/slopvac-pcre2/src/lib.rs", "crates/slopvac-pcre2/src/spike.rs"]:
    shutil.copyfile(overlay / rel, tree / rel)


def edit(rel, pairs):
    p = tree / rel
    s = p.read_text()
    for a, b in pairs:
        assert s.count(a) == 1, (rel, a[:80], s.count(a))
        s = s.replace(a, b)
    p.write_text(s)


# regex is already in Cargo.lock (1.13.1) through other crates.
edit("crates/slopvac-pcre2/Cargo.toml", [
    ("thiserror = { workspace = true }\n",
     "thiserror = { workspace = true }\nregex = \"1.13\"\n"),
])

edit("crates/slopvac-pcre2/src/ffi.rs", [
    ("use crate::Limits;\n", r'''use crate::Limits;

/// SPIKE: the callout result that abandons a match when the step budget is spent
/// (`PCRE2_ERROR_CALLOUT`, reserved for callout functions).
pub(crate) const BUDGET_RC: i32 = -37;

unsafe extern "C" {
    fn pcre2_set_callout_8(
        mcontext: *mut pcre2_match_context_8,
        callout: Option<unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32>,
        data: *mut c_void,
    ) -> i32;
    fn atexit(cb: extern "C" fn()) -> i32;
}

/// SPIKE: run `f` at process exit (the dump).
pub(crate) fn register_exit(f: extern "C" fn()) {
    // SAFETY: plain C library call with a valid function pointer.
    let _ = unsafe { atexit(f) };
}

struct StepCounter {
    n: u64,
    budget: u64,
}

unsafe extern "C" fn spike_callout(_block: *mut c_void, data: *mut c_void) -> i32 {
    // SAFETY: `data` is the `StepCounter` that `exec` owns for the duration of the call.
    let c = unsafe { &mut *data.cast::<StepCounter>() };
    c.n += 1;
    if c.n > c.budget { BUDGET_RC } else { 0 }
}
'''),
    ("""    let rc = unsafe {
        pcre2_match_8(""", """    let callout_on =
        engine == Engine::Jit && crate::spike::mode() == crate::spike::Mode::Callout;
    let mut counter = StepCounter {
        n: 0,
        budget: crate::spike::budget(),
    };
    if callout_on {
        // SAFETY: live context; `counter` outlives the match call, and the callout is
        // removed again right after it.
        unsafe {
            pcre2_set_callout_8(
                context.as_ptr(),
                Some(spike_callout),
                (&raw mut counter).cast::<c_void>(),
            )
        };
    }
    let rc = unsafe {
        pcre2_match_8("""),
    ("""            context.as_ptr(),
        )
    };
    Ok(rc)""", """            context.as_ptr(),
        )
    };
    if callout_on {
        // SAFETY: live context; clears the callout set above.
        unsafe { pcre2_set_callout_8(context.as_ptr(), None, ptr::null_mut()) };
        crate::spike::CALLOUTS.fetch_add(counter.n, std::sync::atomic::Ordering::Relaxed);
        if rc == BUDGET_RC {
            crate::spike::BUDGET_HITS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }
    Ok(rc)"""),
])

edit("crates/slopvac-text/src/pattern.rs", [
    ("pub(crate) fn re(pattern: &str) -> Regex {", "#[track_caller]\npub(crate) fn re(pattern: &str) -> Regex {"),
    ("pub(crate) fn re_i(pattern: &str) -> Regex {", "#[track_caller]\npub(crate) fn re_i(pattern: &str) -> Regex {"),
    ("fn try_build(pattern: &str, caseless: bool, jit: bool) -> Result<Regex, CompileError> {\n    Regex::new(\n        &compat(pattern),",
     "#[track_caller]\nfn try_build(pattern: &str, caseless: bool, jit: bool) -> Result<Regex, CompileError> {\n    Regex::new_with_source(\n        &compat(pattern),\n        pattern,"),
    ("fn build(pattern: &str, caseless: bool) -> Regex {",
     "#[track_caller]\nfn build(pattern: &str, caseless: bool) -> Regex {"),
    ("    pub fn new(source: &str, caseless: bool) -> Result<Self, PatternError> {",
     "    #[track_caller]\n    pub fn new(source: &str, caseless: bool) -> Result<Self, PatternError> {"),
    ("    pub fn with_jit(source: &str, caseless: bool, jit: bool) -> Result<Self, PatternError> {\n        try_build(source, caseless, jit)\n            .map(Self)\n            .map_err(|e| PatternError(e.to_string()))\n    }",
     "    #[track_caller]\n    pub fn with_jit(source: &str, caseless: bool, jit: bool) -> Result<Self, PatternError> {\n        match try_build(source, caseless, jit) {\n            Ok(r) => Ok(Self(r)),\n            Err(e) => Err(PatternError(e.to_string())),\n        }\n    }\n\n    /// SPIKE: label the next compiles on this thread.\n    pub fn spike_set_label(label: &str) {\n        slopvac_pcre2::spike::set_label(label);\n    }"),
])

edit("crates/slopvac-context/src/pattern.rs", [
    ("    pub fn new(source: &str) -> Result<Self, Error> {", "    #[track_caller]\n    pub fn new(source: &str) -> Result<Self, Error> {"),
    ("    pub fn new_caseless(source: &str) -> Result<Self, Error> {", "    #[track_caller]\n    pub fn new_caseless(source: &str) -> Result<Self, Error> {"),
    ("    fn build(source: &str, caseless: bool) -> Result<Self, Error> {", "    #[track_caller]\n    fn build(source: &str, caseless: bool) -> Result<Self, Error> {"),
    ("        let re =\n            Regex::new(source, Options::new().caseless(caseless)).map_err(|e| Error::Pattern {\n                pattern: source.to_string(),\n                source: e,\n            })?;",
     "        let re = match Regex::new(source, Options::new().caseless(caseless)) {\n            Ok(r) => r,\n            Err(e) => {\n                return Err(Error::Pattern {\n                    pattern: source.to_string(),\n                    source: e,\n                });\n            }\n        };"),
])

edit("crates/slopvac-rules/src/ruleset.rs", [
    ("        let matcher = match compile(&rule.match_, options) {",
     "        slopvac_text::Pattern::spike_set_label(&format!(\"rule:{category}.{local}\"));\n"
     "        let compiled = compile(&rule.match_, options);\n"
     "        slopvac_text::Pattern::spike_set_label(\"\");\n"
     "        let matcher = match compiled {"),
])

edit("crates/slopvac-rules/src/matcher.rs", [
    ("    let pattern = Pattern::new(&source, ignore_case).map_err(|e| CompileError {\n        message: e.to_string(),\n        source_text: source.clone(),\n    })?;",
     "    let pattern = match Pattern::new(&source, ignore_case) {\n        Ok(p) => p,\n        Err(e) => {\n            return Err(CompileError {\n                message: e.to_string(),\n                source_text: source.clone(),\n            });\n        }\n    };"),
])
print("applied")
