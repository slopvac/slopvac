//! Text helpers for sentence-level taggers.

/// Drop inline code spans, turning each into one space.
pub(super) fn strip_code(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_code = false;
    for c in s.chars() {
        if c == '`' {
            if in_code {
                out.push(' ');
            }
            in_code = !in_code;
        } else if !in_code {
            out.push(c);
        }
    }
    if in_code {
        // Unbalanced backtick: keep the text after it.
        return s.replace('`', " ");
    }
    out
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
    out.trim().to_string()
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
