//! Text helpers for sentence-level taggers.

use std::collections::HashMap;

/// Drop inline code spans, turning each into one space. A span opens with a run of N
/// backticks and closes at the next run of exactly N, as in `CommonMark`. A run with no
/// closing run of the same length stays literal text.
pub(super) fn strip_code(s: &str) -> String {
    /// A maximal run of backticks.
    struct Run {
        start: usize,
        len: usize,
        /// The run that closes a span opened here: the next run of the same length.
        close: Option<usize>,
    }
    let bytes = s.as_bytes();
    let mut runs: Vec<Run> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            let len = bytes[i..].iter().take_while(|&&b| b == b'`').count();
            crate::work::text(len);
            runs.push(Run {
                start: i,
                len,
                close: None,
            });
            i += len;
        } else {
            crate::work::text(1);
            i += 1;
        }
    }
    // One pass from the end finds the next run of each length.
    let mut next_of_len: HashMap<usize, usize> = HashMap::new();
    for (k, run) in runs.iter_mut().enumerate().rev() {
        run.close = next_of_len.insert(run.len, k);
    }
    let mut out = String::with_capacity(s.len());
    let mut plain = 0;
    let mut k = 0;
    while let Some(run) = runs.get(k) {
        crate::work::text(1);
        match run
            .close
            .and_then(|c| runs.get(c).map(|r| (c, r.start + r.len)))
        {
            Some((c, end)) => {
                out.push_str(&s[plain..run.start]);
                out.push(' ');
                plain = end;
                k = c + 1;
            }
            None => k += 1,
        }
    }
    out.push_str(&s[plain..]);
    out
}

/// Reduce Markdown inline syntax to words: links to their text, no emphasis marks, no code.
pub(super) fn clean(s: &str) -> String {
    let no_code = strip_code(s);
    let mut out = String::with_capacity(no_code.len());
    let chars: Vec<char> = no_code.chars().collect();
    // Once a `](` finds no `)` after it, no later one can, so the search runs at most once.
    let mut closer_possible = true;
    let mut i = 0;
    while i < chars.len() {
        crate::work::text(1);
        let c = chars[i];
        if c == ']' && closer_possible && chars.get(i + 1) == Some(&'(') {
            let rest = chars.get(i + 2..).unwrap_or_default();
            let found = rest
                .iter()
                .inspect(|_| crate::work::text(1))
                .position(|&x| x == ')');
            if let Some(close) = found {
                i += 2 + close + 1;
                continue;
            }
            closer_possible = false;
        }
        if !matches!(c, '[' | '*' | '_') {
            out.push(c);
        }
        i += 1;
    }
    out.trim().to_string()
}
