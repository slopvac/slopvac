//! SPIKE ONLY (regex engine study, never merged): engine routing and parity
//! instrumentation, switched by environment variables.
//!
//! * `SLOPVAC_SPIKE_ENGINE` = `interp` (default, today's policy) | `jit` (Options::jit on
//!   for every pattern, today's JIT limits and interpreter fallback) | `rust` (Rust `regex`
//!   twin wherever it compiles) | `split` (twin only for ids listed in
//!   `SLOPVAC_SPIKE_ALLOW`) | `parity` (run PCRE2 interpreter and twin, return PCRE2, log
//!   every difference) | `callout` (JIT + `PCRE2_AUTO_CALLOUT` step budget
//!   `SLOPVAC_SPIKE_BUDGET`, default 10M callouts per `pcre2_match` call).
//! * `SLOPVAC_SPIKE_DUMP` = path: at exit, append one JSON line per distinct pattern.
#![allow(missing_docs, clippy::pedantic, unreachable_pub, reason = "spike")]

use std::collections::HashMap;
use std::fmt::Write as _;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::Write as _;
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Interp,
    Jit,
    Rust,
    Split,
    Parity,
    Callout,
}

pub fn mode() -> Mode {
    static M: OnceLock<Mode> = OnceLock::new();
    *M.get_or_init(|| match std::env::var("SLOPVAC_SPIKE_ENGINE").as_deref() {
        Ok("jit") => Mode::Jit,
        Ok("rust") => Mode::Rust,
        Ok("split") => Mode::Split,
        Ok("parity") => Mode::Parity,
        Ok("callout") => Mode::Callout,
        _ => Mode::Interp,
    })
}

pub fn budget() -> u64 {
    static B: OnceLock<u64> = OnceLock::new();
    *B.get_or_init(|| {
        std::env::var("SLOPVAC_SPIKE_BUDGET")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(10_000_000)
    })
}

/// Total callouts over the process, for the overhead figure.
pub static CALLOUTS: AtomicU64 = AtomicU64::new(0);
/// Callout budget aborts over the process.
pub static BUDGET_HITS: AtomicU64 = AtomicU64::new(0);
/// Limit errors (any engine) over the process, by kind.
pub static LIMIT_HITS: AtomicU64 = AtomicU64::new(0);

fn allow() -> &'static std::collections::HashSet<String> {
    static A: OnceLock<std::collections::HashSet<String>> = OnceLock::new();
    A.get_or_init(|| {
        std::env::var("SLOPVAC_SPIKE_ALLOW")
            .ok()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|s| {
                s.lines()
                    .map(|l| l.trim().to_owned())
                    .filter(|l| !l.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    })
}

#[derive(Debug)]
pub struct Spike {
    pub id: String,
    pcre: String,
    source: String,
    caseless: bool,
    ucp: bool,
    caller: String,
    label: String,
    rust_source: String,
    rust_err: Option<String>,
    pub twin: Option<regex::Regex>,
    /// Edge guards stripped by the rewrite: leading `(?<!C)` and trailing `(?!C)` as
    /// consumed context (`C` = the guard class), see [`edge_rewrite`].
    pub guarded: bool,
    pub use_twin: bool,
    calls: AtomicU64,
    diffs: AtomicU64,
    edge_diffs: AtomicU64,
    pcre_limit: AtomicU64,
    first_diff: Mutex<Option<String>>,
}

thread_local! {
    static LABEL: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
    static IN_EDGE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Set a label for the next compiles on this thread (the rule kind), "" to clear.
pub fn set_label(label: &str) {
    LABEL.with(|l| *l.borrow_mut() = label.to_owned());
}

fn registry() -> &'static Mutex<HashMap<String, Arc<Spike>>> {
    static R: OnceLock<Mutex<HashMap<String, Arc<Spike>>>> = OnceLock::new();
    R.get_or_init(|| {
        crate::ffi::register_exit(dump);
        Mutex::new(HashMap::new())
    })
}

pub fn key(pcre: &str, caseless: bool, ucp: bool) -> String {
    let mut h = DefaultHasher::new();
    pcre.hash(&mut h);
    caseless.hash(&mut h);
    ucp.hash(&mut h);
    format!("{:016x}", h.finish())
}

/// Register (or fetch) the spike record for a compiled pattern. Returns it and whether it
/// is new (the caller runs the edge suite once for a new one in parity mode).
pub fn register(
    pcre: &str,
    source: &str,
    caseless: bool,
    ucp: bool,
    caller: &std::panic::Location<'_>,
) -> (Arc<Spike>, bool) {
    let id = key(pcre, caseless, ucp);
    let mut reg = registry().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = reg.get(&id) {
        return (Arc::clone(s), false);
    }
    let mode = mode();
    let want = matches!(mode, Mode::Rust | Mode::Split | Mode::Parity)
        || std::env::var_os("SLOPVAC_SPIKE_DUMP").is_some();
    let (rust_source, guarded) = rust_source(source, ucp);
    let (twin, rust_err) = if want {
        match regex::RegexBuilder::new(&rust_source)
            .case_insensitive(caseless)
            .size_limit(256 << 20)
            .dfa_size_limit(64 << 20)
            .build()
        {
            Ok(r) => (Some(r), None),
            Err(e) => (None, Some(e.to_string())),
        }
    } else {
        (None, None)
    };
    let use_twin = twin.is_some()
        && match mode {
            Mode::Rust => true,
            Mode::Split => allow().contains(&id),
            _ => false,
        };
    let spike = Arc::new(Spike {
        id: id.clone(),
        pcre: pcre.to_owned(),
        source: source.to_owned(),
        caseless,
        ucp,
        caller: format!("{}:{}", caller.file(), caller.line()),
        label: LABEL.with(|l| l.borrow().clone()),
        rust_source,
        rust_err,
        twin,
        guarded,
        use_twin,
        calls: AtomicU64::new(0),
        diffs: AtomicU64::new(0),
        edge_diffs: AtomicU64::new(0),
        pcre_limit: AtomicU64::new(0),
        first_diff: Mutex::new(None),
    });
    reg.insert(id, Arc::clone(&spike));
    (spike, true)
}

/// Rust-side source. UCP patterns arrive in the Python dialect before `compat()`, whose
/// `\w`/`\b` equal Rust's Unicode ones. Non-UCP (context) patterns get ASCII `\w\d\s\b`.
/// A leading `(?<![\w-])` and trailing `(?![\w-])` around the whole pattern become consumed
/// context (`(?:\A|[^\w-])(?<g>...)(?:[^\w-]|\z)`); the matcher then reports group `g`.
fn rust_source(source: &str, ucp: bool) -> (String, bool) {
    let body = if ucp {
        source.to_owned()
    } else {
        ascii_classes(source)
    };
    if let Some(inner) = edge_rewrite(&body) {
        return (inner, true);
    }
    (body, false)
}

const LEAD: &str = r"(?<![\w-])";
const TRAIL: &str = r"(?![\w-])";

fn edge_rewrite(body: &str) -> Option<String> {
    let inner = body.strip_prefix(LEAD)?.strip_suffix(TRAIL)?;
    if inner.contains("(?<") || inner.contains("(?=") || inner.contains("(?!") {
        return None;
    }
    Some(format!(r"(?:\A|[^\w-])(?P<spikeg>{inner})(?:[^\w-]|\z)"))
}

/// Replace `\w \W \d \D \s \S \b \B` outside classes with ASCII spellings, and inside
/// classes with ASCII ranges. Good enough for the context patterns (no `\Q`, no `(?x)`).
fn ascii_classes(src: &str) -> String {
    let mut out = String::new();
    let mut chars = src.chars().peekable();
    let mut in_class = false;
    let mut class_start = false;
    while let Some(c) = chars.next() {
        if c == '\\' {
            let Some(n) = chars.next() else {
                out.push('\\');
                break;
            };
            let (outside, inside) = match n {
                'w' => ("(?-i:[0-9A-Za-z_])", Some("0-9A-Za-z_")),
                'W' => ("(?-i:[^0-9A-Za-z_])", None),
                'd' => ("[0-9]", Some("0-9")),
                'D' => ("[^0-9]", None),
                's' => (r"[\t\n\x0B\x0C\r ]", Some(r"\t\n\x0B\x0C\r ")),
                'S' => (r"[^\t\n\x0B\x0C\r ]", None),
                'b' if !in_class => ("(?-u:\\b)", None),
                'B' if !in_class => ("(?-u:\\B)", None),
                _ => {
                    out.push('\\');
                    out.push(n);
                    class_start = false;
                    continue;
                }
            };
            if in_class {
                match inside {
                    Some(i) => out.push_str(i),
                    None => {
                        // A negated escape inside a class: keep as written (Unicode), the
                        // parity run reports it if it matters.
                        out.push('\\');
                        out.push(n);
                    }
                }
            } else {
                out.push_str(outside);
            }
            class_start = false;
            continue;
        }
        if in_class {
            if c == ']' && !class_start {
                in_class = false;
            }
            class_start = class_start && c == '^';
            out.push(c);
            continue;
        }
        if c == '[' {
            in_class = true;
            class_start = true;
        }
        out.push(c);
    }
    out
}

// ---- matching on the twin, Python finditer semantics ----

fn next_char(text: &str, p: usize) -> usize {
    text[p..].chars().next().map_or(p + 1, |c| p + c.len_utf8())
}

/// One search from `start`; with `not_empty`, an empty match at `start` is not allowed
/// (approximation of `PCRE2_NOTEMPTY_ATSTART`: retry from the next character).
fn search(s: &Spike, text: &str, start: usize, not_empty: bool) -> Option<Range<usize>> {
    let re = s.twin.as_ref()?;
    let one = |from: usize| -> Option<Range<usize>> {
        if s.guarded {
            // The guard context may begin one character before `from`.
            let back = if from == 0 {
                0
            } else {
                text[..from]
                    .chars()
                    .next_back()
                    .map_or(0, |c| from - c.len_utf8())
            };
            let mut at = back;
            loop {
                let caps = re.captures_at(text, at)?;
                let g = caps.name("spikeg")?.range();
                if g.start >= from {
                    return Some(g);
                }
                if at >= text.len() {
                    return None;
                }
                at = next_char(text, at);
            }
        } else {
            re.find_at(text, from).map(|m| m.range())
        }
    };
    let found = one(start)?;
    if not_empty && found.is_empty() && found.start == start {
        if start >= text.len() {
            return None;
        }
        return one(next_char(text, start));
    }
    Some(found)
}

pub fn twin_find_all(s: &Spike, text: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut pos = 0;
    let mut not_empty = false;
    while pos <= text.len() {
        let Some(f) = search(s, text, pos, not_empty) else {
            break;
        };
        pos = f.end;
        not_empty = f.is_empty();
        out.push(f);
    }
    out
}

pub fn twin_find(s: &Spike, text: &str) -> Option<Range<usize>> {
    search(s, text, 0, false)
}

pub fn twin_captures(s: &Spike, text: &str) -> Option<Vec<Option<Range<usize>>>> {
    let re = s.twin.as_ref()?;
    if s.guarded {
        // Guarded patterns are only used through find_all/is_match by the engine; give
        // group 0 only.
        return search(s, text, 0, false).map(|g| vec![Some(g)]);
    }
    let caps = re.captures(text)?;
    Some(
        (0..caps.len())
            .map(|i| caps.get(i).map(|m| m.range()))
            .collect(),
    )
}

// ---- parity bookkeeping ----

pub fn record<T: PartialEq + std::fmt::Debug>(
    s: &Spike,
    op: &str,
    text: &str,
    pcre: &Result<T, crate::MatchError>,
    rust: &T,
) {
    s.calls.fetch_add(1, Relaxed);
    match pcre {
        Err(_) => {
            s.pcre_limit.fetch_add(1, Relaxed);
        }
        Ok(p) if p == rust => {}
        Ok(p) => {
            if IN_EDGE.with(std::cell::Cell::get) {
                s.edge_diffs.fetch_add(1, Relaxed);
            } else {
                s.diffs.fetch_add(1, Relaxed);
            }
            let mut first = s.first_diff.lock().unwrap_or_else(|e| e.into_inner());
            if first.is_none() {
                let excerpt: String = text.chars().take(300).collect();
                let mut p_s = format!("{p:?}");
                p_s.truncate(300);
                let mut r_s = format!("{rust:?}");
                r_s.truncate(300);
                *first = Some(format!(
                    "{}{op}: pcre={p_s} rust={r_s} text[..300]={excerpt:?} len={}",
                    if IN_EDGE.with(std::cell::Cell::get) {
                        "EDGE "
                    } else {
                        ""
                    },
                    text.len()
                ));
            }
        }
    }
}

/// Edge strings run once per new pattern in parity mode.
pub const EDGES: &[&str] = &[
    "",
    "\n",
    "word\n",
    "word \n",
    "line one\nline two\n",
    "a\r\nb\r\n",
    "x\u{180E}y \u{85}z \u{A0}w \u{2028}v \u{3000}u",
    "x² ½ Ⅻ ३ ٣ y",
    "Kelvin \u{212A} long s \u{17F} ß SS ẞ İ ı ǅ ǈ ΣΑΣ ς",
    "naïve cafe\u{301} क्षि ज़ \u{200D}\u{200C}",
    "it's it’s re-use e.g. i.e. U.S. — – - _x_ x_y",
    "A\u{FF21}\u{1D400} emoji 👍🏽 flag 🇳🇱",
    "THE QUICK BROWN fox. Very very unique, truly — honestly!",
];

pub fn in_edge(on: bool) {
    IN_EDGE.with(|e| e.set(on));
}

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(o, "\\u{:04x}", c as u32);
            }
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

extern "C" fn dump() {
    let Some(path) = std::env::var_os("SLOPVAC_SPIKE_DUMP") else {
        return;
    };
    let reg = registry().lock().unwrap_or_else(|e| e.into_inner());
    let mut out = String::new();
    for s in reg.values() {
        let first = s
            .first_diff
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let _ = writeln!(
            out,
            "{{\"id\":{},\"mode\":{},\"caller\":{},\"label\":{},\"caseless\":{},\"ucp\":{},\"pcre\":{},\"source\":{},\"rust_source\":{},\"rust_err\":{},\"guarded\":{},\"use_twin\":{},\"calls\":{},\"diffs\":{},\"edge_diffs\":{},\"pcre_limit\":{},\"first_diff\":{}}}",
            esc(&s.id),
            esc(&format!("{:?}", mode())),
            esc(&s.caller),
            esc(&s.label),
            s.caseless,
            s.ucp,
            esc(&s.pcre),
            esc(&s.source),
            esc(&s.rust_source),
            s.rust_err.as_deref().map_or("null".to_owned(), esc),
            s.guarded,
            s.use_twin,
            s.calls.load(Relaxed),
            s.diffs.load(Relaxed),
            s.edge_diffs.load(Relaxed),
            s.pcre_limit.load(Relaxed),
            first.as_deref().map_or("null".to_owned(), esc),
        );
    }
    let _ = writeln!(
        out,
        "{{\"summary\":true,\"mode\":{},\"callouts\":{},\"budget_hits\":{},\"limit_hits\":{}}}",
        esc(&format!("{:?}", mode())),
        CALLOUTS.load(Relaxed),
        BUDGET_HITS.load(Relaxed),
        LIMIT_HITS.load(Relaxed)
    );
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = f.write_all(out.as_bytes());
    }
}
