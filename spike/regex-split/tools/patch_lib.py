"""SPIKE: apply the routing edits to the overlay copy of slopvac-pcre2/src/lib.rs."""
import sys

p = sys.argv[1]
s = open(p).read()


def rep(a, b):
    global s
    assert s.count(a) == 1, (a[:80], s.count(a))
    s = s.replace(a, b)


rep("mod ffi;\nmod version;\n", "mod ffi;\npub mod spike;\nmod version;\n")
rep("PCRE2_NEVER_BACKSLASH_C, PCRE2_UCP, PCRE2_UTF,\n};",
    "PCRE2_NEVER_BACKSLASH_C, PCRE2_UCP, PCRE2_UTF,\n    PCRE2_AUTO_CALLOUT,\n};")
rep("        if self.ucp {\n            bits |= PCRE2_UCP;\n        }\n        bits",
    "        if self.ucp {\n            bits |= PCRE2_UCP;\n        }\n"
    "        if spike::mode() == spike::Mode::Callout {\n            bits |= PCRE2_AUTO_CALLOUT;\n        }\n        bits")
rep("    /// PCRE2 could not allocate memory for a match.\n    #[error(\"PCRE2 ran out of memory\")]\n    OutOfMemory,\n    /// PCRE2 reported",
    "    /// SPIKE: the callout step budget ran out (JIT).\n    #[error(\"step budget of {limit} callouts exceeded\")]\n"
    "    StepBudget {\n        /// The budget.\n        limit: u64,\n    },\n"
    "    /// PCRE2 could not allocate memory for a match.\n    #[error(\"PCRE2 ran out of memory\")]\n    OutOfMemory,\n    /// PCRE2 reported")
rep("            Self::MatchLimit { .. } | Self::DepthLimit { .. } | Self::HeapLimit { .. }\n",
    "            Self::MatchLimit { .. }\n                | Self::DepthLimit { .. }\n"
    "                | Self::HeapLimit { .. }\n                | Self::StepBudget { .. }\n")
rep("pub struct Regex {\n    code: Arc<Code>,\n    limits: Limits,\n}",
    "pub struct Regex {\n    code: Arc<Code>,\n    limits: Limits,\n    spike: Option<Arc<spike::Spike>>,\n}")
rep("""    /// Compile `pattern` under [`Limits::POLICY`].
    pub fn new(pattern: &str, options: Options) -> Result<Self, CompileError> {
        Self::with_limits(pattern, options, Limits::POLICY)
    }
""", """    /// Compile `pattern` under [`Limits::POLICY`].
    #[track_caller]
    pub fn new(pattern: &str, options: Options) -> Result<Self, CompileError> {
        Self::with_limits(pattern, options, Limits::POLICY)
    }

    /// SPIKE: compile `pattern` for PCRE2, with `source` (the pre-rewrite pattern) for the
    /// Rust twin.
    #[track_caller]
    pub fn new_with_source(
        pattern: &str,
        source: &str,
        options: Options,
    ) -> Result<Self, CompileError> {
        Self::build(pattern, source, options, Limits::POLICY, std::panic::Location::caller())
    }
""")
rep("""    pub fn with_limits(
        pattern: &str,
        options: Options,
        limits: Limits,
    ) -> Result<Self, CompileError> {
        ensure_supported_version()?;""", """    #[track_caller]
    pub fn with_limits(
        pattern: &str,
        options: Options,
        limits: Limits,
    ) -> Result<Self, CompileError> {
        Self::build(pattern, pattern, options, limits, std::panic::Location::caller())
    }

    fn build(
        pattern: &str,
        source: &str,
        mut options: Options,
        mut limits: Limits,
        caller: &std::panic::Location<'_>,
    ) -> Result<Self, CompileError> {
        let mode = spike::mode();
        if matches!(mode, spike::Mode::Jit | spike::Mode::Callout) {
            options.jit = true;
        }
        if mode == spike::Mode::Callout && limits == Limits::POLICY {
            limits.jit_match_limit = Limits::MATCH;
        }
        ensure_supported_version()?;""")
rep("""        Ok(Self {
            code: Arc::new(code),
            limits,
        })
    }""", """        let mut re = Self {
            code: Arc::new(code),
            limits,
            spike: None,
        };
        if mode != spike::Mode::Interp || std::env::var_os("SLOPVAC_SPIKE_DUMP").is_some() {
            let (record, new) =
                spike::register(pattern, source, options.caseless, options.ucp, caller);
            re.spike = Some(record);
            if new && mode == spike::Mode::Parity {
                spike::in_edge(true);
                for edge in spike::EDGES {
                    let _ = re.find_all(edge);
                }
                spike::in_edge(false);
            }
        }
        Ok(re)
    }""")
rep("""    pub fn is_match(&self, subject: &str) -> Result<bool, MatchError> {
        self.run(|re, data, contexts| re.search(subject, 0, false, data, contexts))
    }""", """    pub fn is_match(&self, subject: &str) -> Result<bool, MatchError> {
        if let Some(s) = self.spike.as_deref() {
            if s.use_twin {
                return Ok(spike::twin_find(s, subject).is_some());
            }
            if spike::mode() == spike::Mode::Parity && s.twin.is_some() {
                let p = self.pcre_is_match(subject);
                spike::record(s, "is_match", subject, &p, &spike::twin_find(s, subject).is_some());
                return p;
            }
        }
        self.pcre_is_match(subject)
    }

    fn pcre_is_match(&self, subject: &str) -> Result<bool, MatchError> {
        let r = self.run(|re, data, contexts| re.search(subject, 0, false, data, contexts));
        spike_count(&r);
        r
    }""")
rep("""    pub fn captures(&self, subject: &str) -> Result<Option<Captures>, MatchError> {
        self.run(|re, data, contexts| {""", """    pub fn captures(&self, subject: &str) -> Result<Option<Captures>, MatchError> {
        if let Some(s) = self.spike.as_deref() {
            if s.use_twin {
                return Ok(spike::twin_captures(s, subject).map(Captures));
            }
            if spike::mode() == spike::Mode::Parity && s.twin.is_some() {
                let p = self.pcre_captures(subject);
                let r = spike::twin_captures(s, subject).map(Captures);
                if s.guarded {
                    let p0 = p.clone().map(|c| c.and_then(|c| c.get(0)));
                    spike::record(s, "captures0", subject, &p0, &r.and_then(|c| c.get(0)));
                } else {
                    spike::record(s, "captures", subject, &p, &r);
                }
                return p;
            }
        }
        self.pcre_captures(subject)
    }

    fn pcre_captures(&self, subject: &str) -> Result<Option<Captures>, MatchError> {
        let r = self.run(|re, data, contexts| {""")
rep("""            Ok(Some(Captures(groups)))
        })
    }""", """            Ok(Some(Captures(groups)))
        });
        spike_count(&r);
        r
    }""")
rep("""    pub fn find_all(&self, subject: &str) -> Result<Vec<Range<usize>>, MatchError> {
        self.run(|re, data, contexts| {""", """    pub fn find_all(&self, subject: &str) -> Result<Vec<Range<usize>>, MatchError> {
        if let Some(s) = self.spike.as_deref() {
            if s.use_twin {
                return Ok(spike::twin_find_all(s, subject));
            }
            if spike::mode() == spike::Mode::Parity && s.twin.is_some() {
                let p = self.pcre_find_all(subject);
                spike::record(s, "find_all", subject, &p, &spike::twin_find_all(s, subject));
                return p;
            }
        }
        self.pcre_find_all(subject)
    }

    fn pcre_find_all(&self, subject: &str) -> Result<Vec<Range<usize>>, MatchError> {
        let r = self.run(|re, data, contexts| {""")
rep("""            Ok(matches)
        })
    }""", """            Ok(matches)
        });
        spike_count(&r);
        r
    }""")
rep("""            PCRE2_ERROR_NOMATCH => Ok(false),""", """            PCRE2_ERROR_NOMATCH => Ok(false),
            ffi::BUDGET_RC => Err(MatchError::StepBudget {
                limit: spike::budget(),
            }),""")
rep("""/// Turn PCRE2 byte offsets into a range""", """fn spike_count<T>(r: &Result<T, MatchError>) {
    if matches!(r, Err(e) if e.is_limit()) {
        spike::LIMIT_HITS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Turn PCRE2 byte offsets into a range""")
# The find() goes through captures(); keep.
open(p, "w").write(s)
print("ok")
