//! Template scanning shared by rendering, the Fluent export and the audit.

/// A piece of a template.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Segment<'a> {
    /// Text with `{{` and `}}` already collapsed to one brace each.
    Literal(&'a str),
    /// The name inside `{name}`.
    Placeholder(&'a str),
}

/// Split `template` into literals and placeholders. An unclosed `{` stays literal.
pub(crate) fn segments(template: &str) -> Vec<Segment<'_>> {
    let bytes = template.as_bytes();
    let mut out = Vec::new();
    let mut literal_start = 0;
    let mut i = 0;
    while i < bytes.len() {
        let doubled = bytes.get(i + 1) == Some(&bytes[i]);
        match bytes[i] {
            b'{' | b'}' if doubled => {
                push_literal(&mut out, &template[literal_start..=i]);
                i += 2;
                literal_start = i;
            }
            b'{' => {
                let Some(len) = template[i..].find('}') else {
                    break;
                };
                push_literal(&mut out, &template[literal_start..i]);
                out.push(Segment::Placeholder(&template[i + 1..i + len]));
                i += len + 1;
                literal_start = i;
            }
            _ => i += 1,
        }
    }
    push_literal(&mut out, &template[literal_start..]);
    out
}

fn push_literal<'a>(out: &mut Vec<Segment<'a>>, text: &'a str) {
    if !text.is_empty() {
        out.push(Segment::Literal(text));
    }
}
