//! Template syntax: `{name}` placeholders, `{{` and `}}` for literal braces.

/// The placeholder names in `template`, in order of appearance, or why it is malformed.
pub(crate) fn placeholders(template: &str) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
            }
            '{' => {
                let mut body = String::new();
                loop {
                    match chars.next() {
                        Some('}') => break,
                        Some(c) => body.push(c),
                        None => return Err("unclosed `{` in the template".to_owned()),
                    }
                }
                names.push(placeholder_name(&body)?);
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
            }
            '}' => return Err("unmatched `}` in the template; write `}}` for a brace".to_owned()),
            _ => {}
        }
    }
    Ok(names)
}

fn placeholder_name(body: &str) -> Result<String, String> {
    if body.is_empty() || body.chars().all(|c| c.is_ascii_digit()) {
        return Err(format!(
            "positional placeholder `{{{body}}}`; name the parameter, as in `{{path}}`"
        ));
    }
    if body.contains([':', '!']) {
        return Err(format!(
            "placeholder `{{{body}}}` has a format spec; parameters are plain values"
        ));
    }
    let name = body.strip_prefix("r#").unwrap_or(body);
    let mut chars = name.chars();
    let starts = chars.next().is_some_and(|c| c.is_ascii_alphabetic());
    if !starts || !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(format!(
            "placeholder `{{{body}}}` is not a parameter name; use letters, digits and `_`, starting with a letter"
        ));
    }
    Ok(name.to_owned())
}

/// The template with `{{` and `}}` collapsed to single braces.
pub(crate) fn unescape(template: &str) -> String {
    template.replace("{{", "{").replace("}}", "}")
}

/// Why `code` cannot be a message code, if it cannot.
///
/// A code must be a Fluent message id (an ASCII letter, then letters, digits, `-` or `_`)
/// and lower-case kebab-case (lower-case letters and digits in groups split by one `-`).
pub(crate) fn code_problem(code: &str) -> Option<String> {
    let mut chars = code.chars();
    if !chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        || !chars.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Some(format!(
            "`{code}` is not a valid Fluent message id; use ASCII letters, digits and `-`, starting with a letter"
        ));
    }
    let kebab = code.split('-').all(|group| {
        !group.is_empty()
            && group
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
    });
    (!kebab).then(|| {
        format!(
            "`{code}` is not kebab-case; use lower-case letters and digits in groups joined by single `-`"
        )
    })
}
