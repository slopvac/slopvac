//! Mermaid extraction, line by line per diagram type.
//!
//! Supported: flowchart/graph, sequence, class, state and ER diagrams. Other types
//! return no fragments. Known limits: a label that spans lines is not seen, and a
//! node that has no shape renders its id, which is not extracted.

use std::collections::HashMap;

use crate::{Extraction, Fragment, Role, lines};

struct Line<'a> {
    off: usize,
    text: &'a str,
}

struct Emitter {
    diagram: &'static str,
    out: Vec<Fragment>,
    /// Bytes examined by line searches, summed over the diagram.
    work: usize,
}

impl Emitter {
    /// Emit the trimmed, unquoted text of `s[a..b]`. `off` is the offset of `s` in the host file.
    fn push(&mut self, off: usize, s: &str, a: usize, b: usize, role: Role, id: &str) {
        let (a, b) = unwrap_label(s, a, b);
        if a >= b {
            return;
        }
        self.out.push(Fragment {
            span: off + a..off + b,
            text: None,
            role,
            logical: format!("mermaid.{}/{}[{}]", self.diagram, role.as_str(), id),
        });
    }
}

impl Emitter {
    /// Like `push`, for a quoted property value that may hold backslash escapes.
    fn push_unescaped(&mut self, off: usize, s: &str, a: usize, b: usize, role: Role, id: &str) {
        let before = self.out.len();
        self.push(off, s, a, b, role, id);
        if self.out.len() > before {
            let last = self.out.last_mut().expect("just pushed");
            let raw = &s[last.span.start - off..last.span.end - off];
            if raw.contains('\\') {
                last.text = Some(raw.replace("\\\"", "\"").replace("\\\\", "\\"));
            }
        }
    }
}

fn trim_range(s: &str, a: usize, b: usize) -> (usize, usize) {
    let t = &s[a..b];
    let lead = t.len() - t.trim_start().len();
    let t = t.trim();
    (a + lead, a + lead + t.len())
}

fn unwrap_label(s: &str, a: usize, b: usize) -> (usize, usize) {
    let (mut a, mut b) = trim_range(s, a, b);
    for q in ["\"", "`"] {
        let t = &s[a..b];
        if t.len() >= 2 && t.starts_with(q) && t.ends_with(q) {
            (a, b) = trim_range(s, a + 1, b - 1);
        }
    }
    (a, b)
}

pub(crate) fn extract(src: &str, base: usize) -> Extraction {
    extract_counted(src, base).0
}

/// Extract and also return the bytes examined by line searches.
fn extract_counted(src: &str, base: usize) -> (Extraction, usize) {
    let lines: Vec<Line<'_>> = lines(src)
        .into_iter()
        .map(|l| Line {
            off: base + l.start,
            text: l.text,
        })
        .collect();
    let mut iter = lines.iter().peekable();
    let mut front = Vec::new();
    if iter.peek().is_some_and(|l| l.text.trim_end() == "---") {
        iter.next();
        for l in iter.by_ref() {
            if l.text.trim_end() == "---" {
                break;
            }
            front.push(l);
        }
    }
    let mut header = None;
    for l in iter.by_ref() {
        let t = l.text.trim();
        if t.is_empty() || t.starts_with("%%") {
            continue;
        }
        let lead = l.text.len() - l.text.trim_start().len();
        header = Some((l.off + lead, t));
        break;
    }
    let diagram = match header.map(|(_, t)| first_word(t).split(';').next().unwrap_or("")) {
        Some("graph" | "flowchart" | "flowchart-elk") => "flowchart",
        Some("sequenceDiagram") => "sequence",
        Some("classDiagram" | "classDiagram-v2") => "class",
        Some("stateDiagram" | "stateDiagram-v2") => "state",
        Some("erDiagram") => "er",
        _ => return (Extraction::default(), 0),
    };
    let mut em = Emitter {
        diagram,
        out: Vec::new(),
        work: 0,
    };
    for l in front {
        if let Some(rest) = l.text.strip_prefix("title:") {
            em.push(
                l.off,
                l.text,
                "title:".len(),
                "title:".len() + rest.len(),
                Role::Title,
                "title",
            );
        }
    }
    let mut state = State::default();
    if let Some((off, t)) = header
        && let Some(k) = t.find(';')
    {
        let tail = &t[k + 1..];
        let lead = tail.len() - tail.trim_start().len();
        statement(&mut em, &mut state, off + k + 1 + lead, tail.trim());
    }
    for l in iter {
        let lead = l.text.len() - l.text.trim_start().len();
        statement(&mut em, &mut state, l.off + lead, l.text.trim());
    }
    (
        Extraction {
            diagram: Some(diagram.to_string()),
            fragments: em.out,
        },
        em.work,
    )
}

/// Dispatch one trimmed statement line to its diagram type.
fn statement(em: &mut Emitter, state: &mut State, off: usize, t: &str) {
    if t.is_empty() || t.starts_with("%%") || accessibility(em, state, off, t) {
        return;
    }
    match em.diagram {
        "flowchart" => flowchart(em, off, t),
        "sequence" => sequence(em, off, t),
        "class" => class(em, state, off, t),
        "state" => state_diagram(em, state, off, t),
        _ => er(em, state, off, t),
    }
}

#[derive(Default)]
struct State {
    body: Option<String>,
    acc_block: bool,
    note_block: bool,
}

fn first_word(t: &str) -> &str {
    t.split(|c: char| c.is_whitespace()).next().unwrap_or("")
}

fn accessibility(em: &mut Emitter, st: &mut State, off: usize, t: &str) -> bool {
    if st.acc_block {
        if t.starts_with('}') {
            st.acc_block = false;
        } else {
            em.push(off, t, 0, t.len(), Role::Title, "accDescr");
        }
        return true;
    }
    for key in ["accTitle", "accDescr"] {
        if let Some(rest) = t.strip_prefix(key) {
            let rest_trim = rest.trim_start();
            if rest_trim.starts_with('{') {
                st.acc_block = true;
                return true;
            }
            if let Some(v) = rest_trim.strip_prefix(':') {
                let a = t.len() - v.len();
                em.push(off, t, a, t.len(), Role::Title, key);
                return true;
            }
        }
    }
    false
}

fn is_id_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

const SHAPES: &[(&str, &[&str])] = &[
    ("(((", &[")))"]),
    ("((", &["))"]),
    ("([", &["])"]),
    ("(", &[")"]),
    ("[[", &["]]"]),
    ("[(", &[")]"]),
    ("[/", &["/]", "\\]"]),
    ("[\\", &["\\]", "/]"]),
    ("[", &["]"]),
    ("{{", &["}}"]),
    ("{", &["}"]),
    (">", &["]"]),
];

/// Searches within one line. Each failed search is remembered so a line of
/// repeated unmatched openers costs one scan, not one scan per opener.
struct Search<'a> {
    t: &'a str,
    dead: HashMap<&'static str, usize>,
    work: usize,
}

impl<'a> Search<'a> {
    fn new(t: &'a str) -> Self {
        Search {
            t,
            dead: HashMap::new(),
            work: 0,
        }
    }

    /// Find the first position at or after `from` where any needle starts.
    /// Return the position and the matching needle length.
    fn find_any(
        &mut self,
        key: &'static str,
        needles: &[&str],
        from: usize,
    ) -> Option<(usize, usize)> {
        if self.dead.get(key).is_some_and(|&d| d <= from) {
            return None;
        }
        let bytes = self.t.as_bytes();
        for pos in from..bytes.len() {
            self.work += 1;
            for needle in needles {
                if bytes[pos..].starts_with(needle.as_bytes()) {
                    return Some((pos, needle.len()));
                }
            }
        }
        let entry = self.dead.entry(key).or_insert(from);
        *entry = (*entry).min(from);
        None
    }

    /// Find the `}` that closes the attribute block whose body starts at `from`,
    /// skipping quoted strings.
    fn attr_end(&mut self, from: usize) -> Option<usize> {
        if self.dead.get("@{").is_some_and(|&d| d <= from) {
            return None;
        }
        let bytes = self.t.as_bytes();
        let mut quoted = false;
        let mut pos = from;
        while pos < bytes.len() {
            self.work += 1;
            match bytes[pos] {
                b'\\' if quoted => pos += 1,
                b'"' => quoted = !quoted,
                b'}' if !quoted => return Some(pos),
                _ => {}
            }
            pos += 1;
        }
        let entry = self.dead.entry("@{").or_insert(from);
        *entry = (*entry).min(from);
        None
    }
}

/// Find the range of the `label` property inside an attribute block body.
fn attr_label(t: &str, start: usize, end: usize) -> Option<(usize, usize)> {
    let bytes = t.as_bytes();
    let mut pos = start;
    while pos < end {
        while pos < end && (bytes[pos].is_ascii_whitespace() || bytes[pos] == b',') {
            pos += 1;
        }
        let key_start = pos;
        while pos < end && bytes[pos] != b':' {
            pos += 1;
        }
        let key = t[key_start..pos.min(end)].trim().trim_matches('"');
        pos += 1;
        while pos < end && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        let (value_start, value_end);
        if bytes.get(pos) == Some(&b'"') && pos < end {
            value_start = pos + 1;
            pos += 1;
            while pos < end && bytes[pos] != b'"' {
                pos += if bytes[pos] == b'\\' { 2 } else { 1 };
            }
            value_end = pos.min(end);
            pos += 1;
        } else {
            value_start = pos;
            while pos < end && bytes[pos] != b',' {
                pos += 1;
            }
            value_end = pos.min(end);
        }
        if key == "label" {
            return Some((value_start.min(value_end), value_end));
        }
    }
    None
}

fn flowchart(em: &mut Emitter, off: usize, t: &str) {
    let word = first_word(t);
    match word {
        "style" | "classDef" | "class" | "linkStyle" | "click" | "direction" | "end"
        | "callback" | "href" => return,
        "subgraph" => {
            let rest = &t[word.len()..];
            let a = word.len() + (rest.len() - rest.trim_start().len());
            let id_end = t[a..]
                .find(|c: char| !is_id_char(c))
                .map_or(t.len(), |n| a + n);
            let bracket = t[id_end..].trim_start();
            if id_end > a && bracket.starts_with('[') {
                let open = t.len() - bracket.len();
                let id = t[a..id_end].to_string();
                if let Some(n) = t[open + 1..].rfind(']') {
                    em.push(off, t, open + 1, open + 1 + n, Role::Group, &id);
                }
            } else {
                let id = t[a..].trim().to_string();
                em.push(off, t, a, t.len(), Role::Group, &id);
            }
            return;
        }
        _ => {}
    }
    let mut search = Search::new(t);
    let b = t.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() || c == b';' || c == b'&' {
            i += 1;
        } else if let Some(n) = link(em, &mut search, off, i) {
            i = n;
        } else if t[i..].chars().next().is_some_and(is_id_char) {
            i = node(em, &mut search, off, i, Role::Node);
        } else {
            i += t[i..].chars().next().map_or(1, char::len_utf8);
        }
    }
    em.work += search.work;
}

/// Parse one node starting at `i`. Return the index after it.
fn node(em: &mut Emitter, search: &mut Search<'_>, off: usize, i: usize, role: Role) -> usize {
    let t = search.t;
    let id_end = t[i..]
        .find(|c: char| !is_id_char(c))
        .map_or(t.len(), |n| i + n);
    let id = &t[i..id_end];
    let rest = &t[id_end..];
    if rest.starts_with("@{") {
        let body = id_end + 2;
        let Some(end) = search.attr_end(body) else {
            return body;
        };
        if let Some((a, b)) = attr_label(t, body, end) {
            em.push_unescaped(off, t, a, b, role, id);
        }
        return end + 1;
    }
    for (open, closers) in SHAPES {
        if !rest.starts_with(open) {
            continue;
        }
        let p = id_end + open.len();
        let body = &t[p..];
        let lead = body.len() - body.trim_start().len();
        if body[lead..].starts_with('"') {
            let q = p + lead + 1;
            if let Some((n, _)) = search.find_any("\"", &["\""], q) {
                em.push(off, t, q, n, role, id);
                let after = n + 1;
                return search
                    .find_any(open, closers, after)
                    .map_or(t.len(), |(k, len)| k + len);
            }
        }
        return match search.find_any(open, closers, p) {
            Some((k, len)) => {
                em.push(off, t, p, k, role, id);
                k + len
            }
            None => p,
        };
    }
    id_end.max(i + 1)
}

fn link(em: &mut Emitter, search: &mut Search<'_>, off: usize, at: usize) -> Option<usize> {
    let t = search.t;
    let rest = &t[at..];
    for (marker, terminators) in [
        ("-- ", &["-->", "---", "--x", "--o"][..]),
        ("== ", &["==>", "==="][..]),
        ("-. ", &[".->", ".-"][..]),
    ] {
        if rest.starts_with(marker) {
            let start = at + marker.len();
            let (end, len) = search.find_any(marker, terminators, start)?;
            em.push(off, t, start, end, Role::Edge, "link");
            return Some(end + len);
        }
    }
    let bytes = t.as_bytes();
    let next = bytes.get(at + 1).copied();
    let starts = match bytes[at] {
        b'<' => next.is_some_and(|c| b"-=.~".contains(&c)),
        b'-' => next.is_some_and(|c| c == b'-' || c == b'.'),
        b'=' => next == Some(b'='),
        b'~' => next == Some(b'~'),
        _ => false,
    };
    if !starts {
        return None;
    }
    let mut end = at + usize::from(bytes[at] == b'<');
    while end < bytes.len() && b"-=.~".contains(&bytes[end]) {
        end += 1;
    }
    let tip = bytes.get(end).copied();
    let after_tip = bytes.get(end + 1).copied();
    let marker_tip = tip.is_some_and(|c| b"xo".contains(&c))
        && after_tip.is_none_or(|c| c.is_ascii_whitespace() || c == b'|');
    if tip == Some(b'>') || marker_tip {
        end += 1;
    }
    let bar = end + t[end..].len() - t[end..].trim_start_matches(' ').len();
    if bytes.get(bar) == Some(&b'|')
        && let Some((close, _)) = search.find_any("|", &["|"], bar + 1)
    {
        em.push(off, t, bar + 1, close, Role::Edge, "link");
        return Some(close + 1);
    }
    Some(end)
}

fn sequence(em: &mut Emitter, off: usize, t: &str) {
    let word = first_word(t);
    let rest_at = word.len();
    let (word, t_off) = if word == "create" {
        let r = t[rest_at..].trim_start();
        let a = t.len() - r.len();
        (first_word(r), a)
    } else {
        (word, 0)
    };
    let body = &t[t_off..];
    let after_word = t_off + word.len();
    match word {
        "participant" | "actor" => {
            if let Some(k) = body.find(" as ") {
                em.push(
                    off,
                    t,
                    t_off + k + 4,
                    t.len(),
                    Role::Node,
                    body[word.len()..k].trim(),
                );
            }
        }
        "Note" | "note" => {
            if let Some(k) = t.find(':') {
                em.push(off, t, k + 1, t.len(), Role::Note, "note");
            }
        }
        "loop" | "alt" | "else" | "opt" | "par" | "and" | "critical" | "option" | "break" => {
            em.push(off, t, after_word, t.len(), Role::Group, word);
        }
        "title" => em.push(off, t, after_word, t.len(), Role::Title, "title"),
        "autonumber" | "activate" | "deactivate" | "end" | "destroy" | "link" | "links" | "box"
        | "rect" => {}
        _ => {
            if let Some(k) = t.find(':')
                && t[..k].contains('-')
            {
                em.push(off, t, k + 1, t.len(), Role::Message, t[..k].trim());
            }
        }
    }
}

fn colon_outside_quotes(t: &str) -> Option<usize> {
    let mut quoted = false;
    for (i, c) in t.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ':' if !quoted => return Some(i),
            _ => {}
        }
    }
    None
}

fn is_relation(t: &str) -> bool {
    t.contains("--") || t.contains("..") || t.contains("<|") || t.contains("|>")
}

fn class(em: &mut Emitter, st: &mut State, off: usize, t: &str) {
    if let Some(name) = st.body.clone() {
        if t.starts_with('}') {
            st.body = None;
        } else if !t.starts_with("<<") {
            em.push(off, t, 0, t.len(), Role::Member, &name);
        }
        return;
    }
    let word = first_word(t);
    match word {
        "class" => {
            let name = t[word.len()..].trim().trim_end_matches('{').trim();
            let id = name.split(['[', ' ']).next().unwrap_or(name).to_string();
            if let Some(a) = t.find("[\"").map(|k| k + 2)
                && let Some(n) = t[a..].find('"')
            {
                em.push(off, t, a, a + n, Role::Node, &id);
            }
            if t.trim_end().ends_with('{') {
                st.body = Some(id);
            }
        }
        "note" => {
            if let Some(a) = t.find('"')
                && let Some(n) = t[a + 1..].rfind('"')
            {
                em.push(off, t, a + 1, a + 1 + n, Role::Note, "note");
            }
        }
        "direction" | "namespace" | "click" | "link" | "callback" | "style" | "cssClass"
        | "classDef" | "}" => {}
        _ => {
            let Some(k) = colon_outside_quotes(t) else {
                return;
            };
            if is_relation(&t[..k]) {
                em.push(off, t, k + 1, t.len(), Role::Edge, t[..k].trim());
            } else if !t.starts_with("<<") {
                em.push(off, t, k + 1, t.len(), Role::Member, t[..k].trim());
            }
        }
    }
}

fn state_diagram(em: &mut Emitter, st: &mut State, off: usize, t: &str) {
    let word = first_word(t);
    if st.note_block {
        if t.starts_with("end note") {
            st.note_block = false;
        } else {
            em.push(off, t, 0, t.len(), Role::Note, "note");
        }
        return;
    }
    match word {
        "state" => {
            if let Some(a) = t.find('"')
                && let Some(n) = t[a + 1..].find('"')
            {
                let rest = &t[a + 1 + n + 1..];
                let id = rest.trim().trim_start_matches("as").trim().to_string();
                em.push(off, t, a + 1, a + 1 + n, Role::Node, &id);
            }
        }
        "note" => {
            if let Some(k) = t.find(':') {
                em.push(off, t, k + 1, t.len(), Role::Note, "note");
            } else {
                st.note_block = true;
            }
        }
        "direction" | "classDef" | "class" | "style" | "scale" | "hide" | "click" | "--" | "}" => {}
        _ => {
            let Some(k) = colon_outside_quotes(t) else {
                return;
            };
            if t[..k].contains("-->") {
                em.push(off, t, k + 1, t.len(), Role::Edge, t[..k].trim());
            } else {
                em.push(off, t, k + 1, t.len(), Role::Node, t[..k].trim());
            }
        }
    }
}

fn er(em: &mut Emitter, st: &mut State, off: usize, t: &str) {
    if let Some(name) = st.body.clone() {
        if t.starts_with('}') {
            st.body = None;
        } else if let Some(a) = t.find('"')
            && let Some(n) = t[a + 1..].rfind('"')
        {
            em.push(off, t, a + 1, a + 1 + n, Role::Member, &name);
        }
        return;
    }
    if t.ends_with('{') {
        let name = t.trim_end_matches('{').trim();
        let id = name.split('[').next().unwrap_or(name).trim().to_string();
        if let Some(a) = t.find("[\"").map(|k| k + 2)
            && let Some(n) = t[a..].find('"')
        {
            em.push(off, t, a, a + n, Role::Node, &id);
        }
        st.body = Some(id);
        return;
    }
    if let Some(k) = colon_outside_quotes(t)
        && is_relation(&t[..k])
    {
        em.push(off, t, k + 1, t.len(), Role::Edge, t[..k].trim());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Format, extract as run};

    fn labels(src: &str) -> Vec<(Role, String)> {
        let ex = run(Format::Mermaid, src, 0);
        ex.fragments
            .iter()
            .map(|f| (f.role, src[f.span.clone()].to_string()))
            .collect()
    }

    fn n(s: &str) -> (Role, String) {
        (Role::Node, s.to_string())
    }
    fn e(s: &str) -> (Role, String) {
        (Role::Edge, s.to_string())
    }

    #[test]
    fn flowchart_node_shapes() {
        let src = "flowchart TD\n  A[Square one] --> B(Round)\n  C{{Hex}} --> D[(Database)]\n  E((Circle)) --> F>Flag]\n  G[/Slant/] --> H[\"Quoted [x]\"]\n";
        let got: Vec<_> = labels(src).into_iter().map(|l| l.1).collect();
        assert_eq!(
            got,
            [
                "Square one",
                "Round",
                "Hex",
                "Database",
                "Circle",
                "Flag",
                "Slant",
                "Quoted [x]"
            ]
        );
    }

    #[test]
    fn flowchart_edge_label_forms() {
        let src = "graph LR\n  A -->|pipe text| B\n  B -- middle text --> C\n  C == thick text ==> D\n  D -. dotted text .-> E\n  E -->|\"quoted\"| F\n";
        let got: Vec<_> = labels(src);
        assert_eq!(
            got,
            [
                e("pipe text"),
                e("middle text"),
                e("thick text"),
                e("dotted text"),
                e("quoted")
            ]
        );
    }

    #[test]
    fn flowchart_chains_subgraphs_and_skipped_lines() {
        let src = "flowchart TB\n  subgraph one [First group]\n    A[One] --> B[Two] & C[Three]; D[Four]\n  end\n  subgraph Plain title\n  end\n  classDef x fill:#fff\n  style A fill:#f9f\n  click A \"http://x\" \"tip\"\n";
        assert_eq!(
            labels(src),
            [
                (Role::Group, "First group".to_string()),
                n("One"),
                n("Two"),
                n("Three"),
                n("Four"),
                (Role::Group, "Plain title".to_string()),
            ]
        );
    }

    #[test]
    fn frontmatter_title_and_comments() {
        let src = "---\ntitle: My flow\n---\n%% note\nflowchart LR\n  A[x]\n";
        assert_eq!(labels(src), [(Role::Title, "My flow".to_string()), n("x")]);
    }

    #[test]
    fn sequence_elements() {
        let src = "sequenceDiagram\n  title Login\n  participant A as Alice Client\n  actor B\n  A->>B: Send request\n  Note over A,B: Both agree\n  loop Every minute\n    B-->>A: ok\n  end\n";
        assert_eq!(
            labels(src),
            [
                (Role::Title, "Login".to_string()),
                n("Alice Client"),
                (Role::Message, "Send request".to_string()),
                (Role::Note, "Both agree".to_string()),
                (Role::Group, "Every minute".to_string()),
                (Role::Message, "ok".to_string()),
            ]
        );
    }

    #[test]
    fn class_diagram_elements() {
        let src = "classDiagram\n  class Animal {\n    +String name\n    +eat()\n  }\n  Animal <|-- Dog : extends\n  Dog : +bark()\n  note for Dog \"Good boy\"\n  class Cat[\"Feline\"]\n";
        assert_eq!(
            labels(src),
            [
                (Role::Member, "+String name".to_string()),
                (Role::Member, "+eat()".to_string()),
                e("extends"),
                (Role::Member, "+bark()".to_string()),
                (Role::Note, "Good boy".to_string()),
                n("Feline"),
            ]
        );
    }

    #[test]
    fn state_diagram_elements() {
        let src = "stateDiagram-v2\n  state \"Waiting for input\" as Wait\n  [*] --> Wait\n  Wait --> Done : user confirms\n  Done : All finished\n  note right of Done : Terminal\n  note left of Wait\n    First line\n  end note\n";
        assert_eq!(
            labels(src),
            [
                n("Waiting for input"),
                e("user confirms"),
                n("All finished"),
                (Role::Note, "Terminal".to_string()),
                (Role::Note, "First line".to_string()),
            ]
        );
    }

    #[test]
    fn er_diagram_elements() {
        let src = "erDiagram\n  CUSTOMER ||--o{ ORDER : places\n  CUSTOMER {\n    string name PK \"Full legal name\"\n    int age\n  }\n";
        assert_eq!(
            labels(src),
            [e("places"), (Role::Member, "Full legal name".to_string())]
        );
    }

    #[test]
    fn accessibility_text_is_a_title() {
        let src = "flowchart LR\n  accTitle: Pipeline\n  accDescr {\n    Line one\n  }\n  A[x]\n";
        assert_eq!(
            labels(src),
            [
                (Role::Title, "Pipeline".to_string()),
                (Role::Title, "Line one".to_string()),
                n("x")
            ]
        );
    }

    #[test]
    fn unsupported_type_yields_nothing() {
        let ex = run(Format::Mermaid, "gantt\n  title X\n", 0);
        assert_eq!(ex.diagram, None);
        assert_eq!(ex.fragments, Vec::new());
    }

    #[test]
    fn spans_are_offset_by_base_and_crlf_is_handled() {
        let src = "flowchart TD\r\n  A[Hello] --> B[World]\r\n";
        let ex = run(Format::Mermaid, src, 100);
        let spans: Vec<_> = ex.fragments.iter().map(|f| f.span.clone()).collect();
        let a = src.find("Hello").unwrap() + 100;
        let b = src.find("World").unwrap() + 100;
        assert_eq!(spans, [a..a + 5, b..b + 5]);
        assert!(ex.fragments.iter().all(Fragment::fixable));
    }

    #[test]
    fn logical_names_identify_the_element() {
        let ex = run(Format::Mermaid, "flowchart TD\n  A[Hello]\n", 0);
        assert_eq!(ex.fragments[0].logical, "mermaid.flowchart/node[A]");
    }

    #[test]
    fn mixed_unicode_is_not_split_inside_a_character() {
        let src = "flowchart TD\n  A[Café — naïve] --> B[日本語]\n";
        assert_eq!(labels(src), [n("Café — naïve"), n("日本語")]);
    }

    #[test]
    fn attribute_label_ignores_other_properties_and_quoted_braces() {
        for (src, want) in [
            (
                "flowchart LR\n  A@{ shape: rect, label: \"Typed node\" }\n",
                "Typed node",
            ),
            (
                "flowchart LR\n  A@{ label: \"First\", shape: rect }\n",
                "First",
            ),
            (
                "flowchart LR\n  A@{ label: \"has } brace\", shape: rect }\n",
                "has } brace",
            ),
            (
                "flowchart LR\n  A@{ shape: rect, label: Bare text, icon: x }\n",
                "Bare text",
            ),
            (
                "flowchart LR\n  A@{ \"label\": \"Quoted key\" }\n",
                "Quoted key",
            ),
        ] {
            let got = labels(src);
            assert_eq!(got, [n(want)], "{src}");
        }
    }

    #[test]
    fn attribute_label_with_escaped_quote_carries_converted_text() {
        let src = "flowchart LR\n  A@{ label: \"say \\\"hi\\\"\", shape: rect }\n";
        let ex = run(Format::Mermaid, src, 0);
        assert_eq!(ex.fragments[0].text.as_deref(), Some("say \"hi\""));
        assert!(!ex.fragments[0].fixable());
    }

    #[test]
    fn malformed_repeated_openers_examine_a_linear_number_of_bytes() {
        for unit in ["A[ ", "A@{ ", "A-- ", "A[\"", "A-->|", "A[/ ", "A(("] {
            let src = format!("flowchart LR\n{}\n", unit.repeat(5_000));
            let (_, work) = extract_counted(&src, 0);
            assert!(
                work <= 4 * src.len(),
                "{unit:?}: examined {work} bytes of {}",
                src.len()
            );
        }
    }

    #[test]
    fn lone_cr_and_mixed_terminators_keep_original_offsets() {
        let src = "flowchart TD\rA[One]\r\nB[Two]\nC[Three]\r";
        let ex = run(Format::Mermaid, src, 0);
        let got: Vec<_> = ex.fragments.iter().map(|f| &src[f.span.clone()]).collect();
        assert_eq!(got, ["One", "Two", "Three"]);
        assert_eq!(ex.fragments[0].span.start, src.find("One").unwrap());
    }

    #[test]
    fn statements_after_the_header_semicolon_are_extracted() {
        let src = "flowchart TD; A[One] --> B[Two]\n";
        let ex = run(Format::Mermaid, src, 40);
        let spans: Vec<_> = ex.fragments.iter().map(|f| f.span.clone()).collect();
        let one = src.find("One").unwrap() + 40;
        let two = src.find("Two").unwrap() + 40;
        assert_eq!(spans, [one..one + 3, two..two + 3]);
        let seq = labels("sequenceDiagram; A->>B: hi\n");
        assert_eq!(seq, [(Role::Message, "hi".to_string())]);
    }
}
