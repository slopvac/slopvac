//! Text helpers for sentence-level taggers.

/// Drop inline code spans, turning each into one space. A span opens with a run of N
/// backticks and closes at the next run of exactly N, as in `CommonMark`. A run with no
/// closing run of the same length stays literal text.
pub(super) fn strip_code(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let mut plain = 0;
    while i < bytes.len() {
        if bytes[i] != b'`' {
            i += 1;
            continue;
        }
        let run = bytes[i..].iter().take_while(|&&b| b == b'`').count();
        match closing_run(bytes, i + run, run) {
            Some(end) => {
                out.push_str(&s[plain..i]);
                out.push(' ');
                i = end;
                plain = end;
            }
            None => i += run,
        }
    }
    out.push_str(&s[plain..]);
    out
}

/// End offset of the first run of exactly `len` backticks at or after `from`.
fn closing_run(bytes: &[u8], from: usize, len: usize) -> Option<usize> {
    let mut j = from;
    while j < bytes.len() {
        if bytes[j] == b'`' {
            let run = bytes[j..].iter().take_while(|&&b| b == b'`').count();
            if run == len {
                return Some(j + run);
            }
            j += run;
        } else {
            j += 1;
        }
    }
    None
}

/// Reduce Markdown inline syntax to words: links to their text, no emphasis marks, no code.
pub(super) fn clean(s: &str) -> String {
    let no_code = strip_code(s);
    let mut out = String::with_capacity(no_code.len());
    let chars: Vec<char> = no_code.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == ']'
            && chars.get(i + 1) == Some(&'(')
            && let Some(close) = chars[i + 2..].iter().position(|&x| x == ')')
        {
            i += 2 + close + 1;
            continue;
        }
        if !matches!(c, '[' | '*' | '_') {
            out.push(c);
        }
        i += 1;
    }
    out.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_code_links_and_emphasis() {
        assert_eq!(
            clean("**Never** pass `--force` to [the tool](http://x)."),
            "Never pass   to the tool."
        );
        assert_eq!(strip_code("a `b` c"), "a   c");
    }
}
