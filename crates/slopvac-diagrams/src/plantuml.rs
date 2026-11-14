//! `PlantUML` extraction, line by line.
//!
//! Handles `@startuml` diagrams: sequence, class, state, activity, and the
//! component family. Other `@start` types (mindmap, gantt and so on) return no
//! fragments. Usecase shorthand such as `(Login)` and `:Actor:` is not read.

use crate::{Extraction, Fragment, Role, lines};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Sequence,
    Class,
    State,
    Activity,
    Component,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Sequence => "sequence",
            Kind::Class => "class",
            Kind::State => "state",
            Kind::Activity => "activity",
            Kind::Component => "component",
        }
    }
}

#[derive(PartialEq, Eq)]
enum Block {
    None,
    Comment,
    Note,
    Legend,
    Title,
    Header,
    Footer,
    Members(String),
    Action,
}

struct Out {
    kind: Kind,
    frags: Vec<Fragment>,
}

impl Out {
    fn push(&mut self, off: usize, s: &str, range: (usize, usize), role: Role, id: &str) {
        let t = &s[range.0..range.1];
        let lead = t.len() - t.trim_start().len();
        let mut a = range.0 + lead;
        let mut b = a + t.trim().len();
        if b - a >= 2 && s[a..b].starts_with('"') && s[a..b].ends_with('"') {
            a += 1;
            b -= 1;
        }
        if a >= b {
            return;
        }
        self.frags.push(Fragment {
            span: off + a..off + b,
            text: None,
            role,
            logical: format!("plantuml.{}/{}[{}]", self.kind.name(), role.as_str(), id),
        });
    }
}

fn first_word_lower(t: &str) -> String {
    t.split(|c: char| c.is_whitespace() || c == '(' || c == ':')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn detect(lines: &[(usize, &str)]) -> Kind {
    let mut state = false;
    let mut class = false;
    let mut activity = false;
    let mut component = false;
    for (_, l) in lines {
        let w = first_word_lower(l);
        match w.as_str() {
            "participant" | "autonumber" | "activate" | "deactivate" | "alt" | "loop" | "opt"
            | "par" | "critical" | "group" | "ref" | "return" => return Kind::Sequence,
            "state" => state = true,
            "class" | "interface" | "abstract" | "enum" | "annotation" => class = true,
            "component" | "usecase" | "rectangle" | "node" | "cloud" | "folder" | "frame"
            | "artifact" | "storage" | "card" | "agent" | "package" => component = true,
            "start" | "stop" | "fork" | "while" | "repeat" | "if" | "partition" => activity = true,
            _ => {}
        }
        if l.contains("[*]") {
            state = true;
        }
        if l.starts_with(':') && l.trim_end().ends_with(';') {
            activity = true;
        }
    }
    if state {
        Kind::State
    } else if class {
        Kind::Class
    } else if activity {
        Kind::Activity
    } else if component {
        Kind::Component
    } else {
        Kind::Sequence
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

fn is_arrow(s: &str) -> bool {
    s.contains("--") || s.contains("->") || s.contains("<-") || s.contains("..") || s.contains("-(")
}

const DECLARATIONS: &[&str] = &[
    "participant",
    "actor",
    "boundary",
    "control",
    "entity",
    "database",
    "collections",
    "queue",
    "component",
    "interface",
    "usecase",
    "rectangle",
    "node",
    "cloud",
    "folder",
    "frame",
    "package",
    "artifact",
    "storage",
    "card",
    "agent",
    "class",
    "abstract",
    "enum",
    "object",
    "state",
    "annotation",
    "label",
    "person",
    "port",
    "portin",
    "portout",
    "stack",
    "file",
];
const CONTAINERS: &[&str] = &[
    "package",
    "rectangle",
    "node",
    "cloud",
    "folder",
    "frame",
    "partition",
    "namespace",
];
const BODY_DECLS: &[&str] = &[
    "class",
    "abstract",
    "enum",
    "interface",
    "object",
    "annotation",
];
const SKIPPED: &[&str] = &[
    "skinparam",
    "hide",
    "show",
    "left",
    "right",
    "top",
    "bottom",
    "scale",
    "autonumber",
    "activate",
    "deactivate",
    "set",
    "end",
    "endif",
    "endwhile",
    "start",
    "stop",
    "fork",
    "detach",
    "kill",
    "split",
    "repeat",
    "else",
    "!",
    "end note",
    "newpage",
    "mainframe",
    "title",
    "header",
    "footer",
    "caption",
    "legend",
    "note",
    "rnote",
    "hnote",
    "floating",
];

pub(crate) fn extract(src: &str, base: usize) -> Extraction {
    let mut body = Vec::new();
    let mut started = false;
    for source_line in lines(src) {
        let text = source_line.text;
        let lead = text.len() - text.trim_start().len();
        let t = text.trim();
        let at = base + source_line.start + lead;
        if !started && t.starts_with("@start") {
            if !t.to_ascii_lowercase().starts_with("@startuml") {
                return Extraction::default();
            }
            started = true;
            continue;
        }
        if t.starts_with("@end") {
            break;
        }
        body.push((at, t));
    }
    let kind = detect(&body);
    let mut out = Out {
        kind,
        frags: Vec::new(),
    };
    let mut block = Block::None;
    for (at, t) in body {
        line(&mut out, &mut block, at, t);
    }
    Extraction {
        diagram: Some(format!("plantuml.{}", kind.name())),
        fragments: out.frags,
    }
}

fn lower_eq(t: &str, a: &str, b: &str) -> bool {
    t.eq_ignore_ascii_case(a) || t.eq_ignore_ascii_case(b)
}

/// Consume `t` when a multi-line block is open. Return whether it did.
fn in_block(out: &mut Out, block: &mut Block, at: usize, t: &str) -> bool {
    match block {
        Block::Comment => {
            if t.ends_with("'/") {
                *block = Block::None;
            }
        }
        Block::Note => {
            if lower_eq(t, "end note", "endnote") {
                *block = Block::None;
            } else {
                out.push(at, t, (0, t.len()), Role::Note, "note");
            }
        }
        Block::Legend => {
            if lower_eq(t, "end legend", "endlegend") {
                *block = Block::None;
            } else {
                out.push(at, t, (0, t.len()), Role::Text, "legend");
            }
        }
        Block::Title | Block::Header | Block::Footer => {
            if t.to_ascii_lowercase().starts_with("end") {
                *block = Block::None;
            } else {
                out.push(at, t, (0, t.len()), Role::Title, "title");
            }
        }
        Block::Members(name) => {
            if t.starts_with('}') {
                *block = Block::None;
            } else if !t.starts_with("--") && !t.starts_with("==") && !t.starts_with("..") {
                let name = name.clone();
                out.push(at, t, (0, t.len()), Role::Member, &name);
            }
        }
        Block::Action => {
            let end = t.find(';');
            out.push(at, t, (0, end.unwrap_or(t.len())), Role::Node, "action");
            if end.is_some() {
                *block = Block::None;
            }
        }
        Block::None => return false,
    }
    true
}

fn line(out: &mut Out, block: &mut Block, at: usize, t: &str) {
    if t.is_empty() || in_block(out, block, at, t) {
        return;
    }
    if t.starts_with('\'') {
        return;
    }
    if t.starts_with("/'") {
        if !t.ends_with("'/") {
            *block = Block::Comment;
        }
        return;
    }
    if t.starts_with('!') {
        return;
    }
    if t.starts_with(':') && out.kind == Kind::Activity {
        let end = t.find(';');
        out.push(at, t, (1, end.unwrap_or(t.len())), Role::Node, "action");
        if end.is_none() {
            *block = Block::Action;
        }
        return;
    }
    let word = first_word_lower(t);
    let after = word.len().min(t.len());
    match word.as_str() {
        "title" | "header" | "footer" | "caption" => {
            let rest = t[after..].trim();
            if rest.is_empty() {
                *block = match word.as_str() {
                    "header" => Block::Header,
                    "footer" => Block::Footer,
                    _ => Block::Title,
                };
            } else {
                out.push(at, t, (after, t.len()), Role::Title, &word);
            }
            return;
        }
        "legend" => {
            *block = Block::Legend;
            return;
        }
        "note" | "rnote" | "hnote" | "floating" => {
            note(out, block, at, t);
            return;
        }
        "return" => {
            out.push(at, t, (after, t.len()), Role::Message, "return");
            return;
        }
        "alt" | "loop" | "opt" | "par" | "critical" | "group" | "break" | "else" | "also"
        | "ref"
            if out.kind == Kind::Sequence =>
        {
            out.push(at, t, (after, t.len()), Role::Group, &word);
            return;
        }
        "if" | "elseif" | "while" | "switch" | "case" | "else" | "repeat" | "endif" => {
            let cond = matches!(word.as_str(), "if" | "elseif" | "while" | "switch");
            parens(out, at, t, cond);
            return;
        }
        "partition" => {
            let rest = t[after..].trim_end_matches('{').trim_end();
            out.push(at, t, (after, after + rest.len()), Role::Group, "partition");
            return;
        }
        w if DECLARATIONS.contains(&w) => {
            declaration(out, block, at, t, w);
            return;
        }
        w if SKIPPED.contains(&w) => return,
        _ => {}
    }
    if t.starts_with("==") && t.ends_with("==") && out.kind == Kind::Sequence {
        out.push(at, t, (2, t.len() - 2), Role::Group, "separator");
        return;
    }
    let Some(k) = colon_outside_quotes(t) else {
        return;
    };
    if is_arrow(&t[..k]) {
        let role = if out.kind == Kind::Sequence {
            Role::Message
        } else {
            Role::Edge
        };
        out.push(at, t, (k + 1, t.len()), role, t[..k].trim());
    } else {
        let role = match out.kind {
            Kind::State => Role::Node,
            Kind::Class => Role::Member,
            _ => return,
        };
        out.push(at, t, (k + 1, t.len()), role, t[..k].trim());
    }
}

fn note(out: &mut Out, block: &mut Block, at: usize, t: &str) {
    if let Some(k) = colon_outside_quotes(t) {
        out.push(at, t, (k + 1, t.len()), Role::Note, "note");
    } else if let Some(a) = t.find('"') {
        if let Some(n) = t[a + 1..].rfind('"') {
            out.push(at, t, (a + 1, a + 1 + n), Role::Note, "note");
        }
    } else {
        *block = Block::Note;
    }
}

fn declaration(out: &mut Out, block: &mut Block, at: usize, t: &str, word: &str) {
    let name = t[word.len()..]
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_string();
    let role = if CONTAINERS.contains(&word) {
        Role::Group
    } else {
        Role::Node
    };
    if let Some(a) = t.find('"') {
        if let Some(n) = t[a + 1..].find('"') {
            out.push(at, t, (a + 1, a + 1 + n), role, &name);
        }
    } else if let (Some(a), Some(n)) = (t.find('['), t.find(']'))
        && a < n
    {
        out.push(at, t, (a + 1, n), role, &name);
    }
    if BODY_DECLS.contains(&word) && t.ends_with('{') {
        *block = Block::Members(name);
    }
}

fn parens(out: &mut Out, at: usize, t: &str, cond: bool) {
    let mut rest = 0;
    let mut first = cond;
    while let Some(a) = t[rest..].find('(') {
        let open = rest + a;
        let Some(n) = t[open..].find(')') else { break };
        let role = if first { Role::Node } else { Role::Edge };
        out.push(at, t, (open + 1, open + n), role, "branch");
        first = false;
        rest = open + n + 1;
    }
}

#[cfg(test)]
mod tests {
    use crate::{Format, Role, extract};

    fn labels(src: &str) -> Vec<(Role, String)> {
        extract(Format::PlantUml, src, 0)
            .fragments
            .iter()
            .map(|f| (f.role, src[f.span.clone()].to_string()))
            .collect()
    }

    fn r(role: Role, s: &str) -> (Role, String) {
        (role, s.to_string())
    }

    #[test]
    fn sequence_diagram() {
        let src = "@startuml\ntitle Checkout\nparticipant \"Payment service\" as P\nA -> P : charge card\nnote right of P : retries twice\nalt card declined\n  P --> A : error\nend\n@enduml\n";
        assert_eq!(
            labels(src),
            [
                r(Role::Title, "Checkout"),
                r(Role::Node, "Payment service"),
                r(Role::Message, "charge card"),
                r(Role::Note, "retries twice"),
                r(Role::Group, "card declined"),
                r(Role::Message, "error"),
            ]
        );
    }

    #[test]
    fn multi_line_note_legend_and_title() {
        let src = "@startuml\nparticipant A\nnote over A\n  first\n  second\nend note\nlegend\n  Blue means new\nendlegend\ntitle\n  Big title\nend title\n@enduml\n";
        assert_eq!(
            labels(src),
            [
                r(Role::Note, "first"),
                r(Role::Note, "second"),
                r(Role::Text, "Blue means new"),
                r(Role::Title, "Big title"),
            ]
        );
    }

    #[test]
    fn class_diagram_members_and_relations() {
        let src = "@startuml\nclass Order {\n  +id: int\n  --\n  +total()\n}\nOrder \"1\" --> \"*\" Line : contains\nOrder : +extra\n@enduml\n";
        assert_eq!(
            labels(src),
            [
                r(Role::Member, "+id: int"),
                r(Role::Member, "+total()"),
                r(Role::Edge, "contains"),
                r(Role::Member, "+extra"),
            ]
        );
    }

    #[test]
    fn state_diagram() {
        let src = "@startuml\n[*] --> Idle\nIdle : waits for a job\nIdle --> Busy : job arrives\nstate \"Long name\" as L\n@enduml\n";
        assert_eq!(
            labels(src),
            [
                r(Role::Node, "waits for a job"),
                r(Role::Edge, "job arrives"),
                r(Role::Node, "Long name"),
            ]
        );
    }

    #[test]
    fn activity_diagram() {
        let src = "@startuml\nstart\n:Read the input;\nif (valid?) then (yes)\n  :Process\n  and store;\nelse (no)\n  :Reject;\nendif\npartition \"Back end\" {\n}\nstop\n@enduml\n";
        assert_eq!(
            labels(src),
            [
                r(Role::Node, "Read the input"),
                r(Role::Node, "valid?"),
                r(Role::Edge, "yes"),
                r(Role::Node, "Process"),
                r(Role::Node, "and store"),
                r(Role::Edge, "no"),
                r(Role::Node, "Reject"),
                r(Role::Group, "Back end"),
            ]
        );
    }

    #[test]
    fn component_declarations() {
        let src = "@startuml\ncomponent [Web app] as W\npackage \"Back end\" {\n  database \"Main store\" as DB\n}\nW --> DB : reads\n@enduml\n";
        assert_eq!(
            labels(src),
            [
                r(Role::Node, "Web app"),
                r(Role::Group, "Back end"),
                r(Role::Node, "Main store"),
                r(Role::Edge, "reads"),
            ]
        );
    }

    #[test]
    fn comments_and_directives_are_skipped() {
        let src = "@startuml\n' note : no\n/' block\nnote : no '/\nskinparam x y\n!include foo : bar\nA -> B : yes\n@enduml\n";
        assert_eq!(labels(src), [r(Role::Message, "yes")]);
    }

    #[test]
    fn other_diagram_types_yield_nothing() {
        let ex = extract(Format::PlantUml, "@startmindmap\n* root\n@endmindmap\n", 0);
        assert_eq!(ex.diagram, None);
    }

    #[test]
    fn fence_body_without_markers_still_extracts_with_base() {
        let src = "A -> B : hi\n";
        let ex = extract(Format::PlantUml, src, 50);
        assert_eq!(ex.fragments[0].span, 50 + 9..50 + 11);
    }
}

#[cfg(test)]
mod line_terminator_tests {
    use crate::{Format, Role, extract};

    #[test]
    fn lone_cr_and_crlf_keep_original_offsets() {
        let src = "@startuml\rA -> B : one\r\nB -> A : two\n@enduml\r";
        let ex = extract(Format::PlantUml, src, 0);
        let got: Vec<_> = ex
            .fragments
            .iter()
            .map(|f| (f.role, &src[f.span.clone()]))
            .collect();
        assert_eq!(got, [(Role::Message, "one"), (Role::Message, "two")]);
    }
}
