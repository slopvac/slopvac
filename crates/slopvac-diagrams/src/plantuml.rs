//! `PlantUML` extraction, line by line.
//!
//! Handles `@startuml` diagrams: sequence, class, state, activity, and the
//! component family. Other `@start` types (mindmap, gantt and so on) are
//! unsupported. Usecase shorthand such as `(Login)` and `:Actor:` is not read.

use crate::messages::DiagramPhrase;
use crate::sink::{Sink, clip};
use crate::{Extraction, ExtractionStatus, Fragment, Role, lines};

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
    frags: Sink,
    /// Work done, summed over the diagram: one per line read plus the bytes each label scan examines.
    work: usize,
}

impl Out {
    fn push(&mut self, off: usize, s: &str, range: (usize, usize), role: Role, id: &str) {
        if self.frags.is_full() {
            return;
        }
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
            logical: format!(
                "plantuml.{}/{}[{}]",
                self.kind.name(),
                role.as_str(),
                clip(id)
            ),
        });
    }
}

fn first_word_lower(t: &str) -> String {
    t.split(|c: char| c.is_whitespace() || c == '(' || c == ':')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// What the detection pass skips because it is not a structural statement.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Skip {
    None,
    Comment,
    Note,
    Legend,
    Text,
    Body,
    Action,
}

/// Advance the skip state over one line. Return whether the line is inside a skipped region.
fn skipped(skip: &mut Skip, l: &str) -> bool {
    let lower = l.to_ascii_lowercase();
    let ends = match *skip {
        Skip::None => return false,
        Skip::Comment => l.ends_with("'/"),
        Skip::Note => lower == "end note" || lower == "endnote",
        Skip::Legend => lower == "end legend" || lower == "endlegend",
        Skip::Text => lower.starts_with("end"),
        Skip::Body => l.starts_with('}'),
        Skip::Action => l.contains(';'),
    };
    if ends {
        *skip = Skip::None;
    }
    true
}

fn detect(lines: &[(usize, &str)]) -> Kind {
    let mut state = false;
    let mut class = false;
    let mut activity = false;
    let mut component = false;
    let mut skip = Skip::None;
    for (_, l) in lines {
        if skipped(&mut skip, l) {
            continue;
        }
        if l.starts_with("/'") {
            if !l.ends_with("'/") {
                skip = Skip::Comment;
            }
            continue;
        }
        if l.starts_with('\'') || l.starts_with('!') {
            continue;
        }
        if l.starts_with(':') {
            activity = true;
            if !l.contains(';') {
                skip = Skip::Action;
            }
            continue;
        }
        let w = first_word_lower(l);
        let rest = l[w.len().min(l.len())..].trim();
        match w.as_str() {
            "title" | "header" | "footer" | "caption" => {
                if rest.is_empty() {
                    skip = Skip::Text;
                }
                continue;
            }
            "legend" => {
                skip = Skip::Legend;
                continue;
            }
            "note" | "rnote" | "hnote" | "floating" => {
                if colon_outside_quotes(l).is_none() && !l.contains('"') {
                    skip = Skip::Note;
                }
                continue;
            }
            _ => {}
        }
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
        if BODY_DECLS.contains(&w.as_str()) && l.ends_with('{') {
            skip = Skip::Body;
        }
        let head = colon_outside_quotes(l).map_or(*l, |k| &l[..k]);
        if head.contains("[*]") {
            state = true;
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
    extract_counted(src, base).0
}

/// A diagram body: each line with its offset in the host file.
type Body<'a> = Vec<(usize, &'a str)>;

/// The diagram bodies of a source, and whether blocks of other types were left out.
struct Blocks<'a> {
    bodies: Vec<Body<'a>>,
    /// A block of an unsupported `@start` type was skipped.
    skipped: bool,
}

/// Split the source into diagram bodies, one per `@startuml` to `@enduml` block.
///
/// A body without markers is read as one block. Lines before the first `@startuml` stay in
/// the first block. Lines between blocks are ignored. A terminator must be exactly
/// `@enduml`, and an opened block must close, so a truncated or mismatched diagram is
/// reported instead of read as far as it went. A block of another type (`@startmindmap`
/// and so on) runs to its own `@end` marker and is skipped, so it does not hide the
/// supported blocks around it. A marker inside a `/' … '/` comment starts nothing.
fn blocks(src: &str, base: usize) -> Result<Blocks<'_>, ExtractionStatus> {
    let mut done = Vec::new();
    let mut current: Option<Body<'_>> = Some(Vec::new());
    let mut explicit = false;
    let mut skipped = false;
    let mut in_comment = false;
    for source_line in lines(src) {
        let text = source_line.text;
        let lead = text.len() - text.trim_start().len();
        let t = text.trim();
        let at = base + source_line.start + lead;
        let was_comment = in_comment;
        if in_comment {
            in_comment = !t.ends_with("'/");
        } else if t.starts_with("/'") && !t.ends_with("'/") {
            in_comment = true;
        }
        if was_comment || in_comment || t.starts_with("/'") {
            if let Some(body) = current.as_mut() {
                body.push((at, t));
            }
            continue;
        }
        let opens = t.starts_with("@start");
        if opens && !t.to_ascii_lowercase().starts_with("@startuml") {
            if explicit && current.is_some() {
                return Err(ExtractionStatus::Malformed(
                    DiagramPhrase::MissingEnduml.as_static(),
                ));
            }
            explicit = true;
            skipped = true;
            current = None;
            continue;
        }
        let Some(body) = current.as_mut() else {
            if opens {
                current = Some(Vec::new());
            }
            continue;
        };
        if opens && !explicit {
            explicit = true;
            continue;
        }
        if opens {
            return Err(ExtractionStatus::Malformed(
                DiagramPhrase::MissingEnduml.as_static(),
            ));
        }
        if t.starts_with("@end") {
            if !t.eq_ignore_ascii_case("@enduml") {
                return Err(ExtractionStatus::Malformed(
                    DiagramPhrase::MismatchedTerminator.as_static(),
                ));
            }
            done.extend(current.take());
            continue;
        }
        body.push((at, t));
    }
    if explicit && current.is_some() {
        return Err(ExtractionStatus::Malformed(
            DiagramPhrase::MissingEnduml.as_static(),
        ));
    }
    done.extend(current);
    Ok(Blocks {
        bodies: done,
        skipped,
    })
}

/// Extract and also return the work done: one per line read, one per body detected, and the
/// bytes label scans examine.
fn extract_counted(src: &str, base: usize) -> (Extraction, usize) {
    let Blocks { bodies, skipped } = match blocks(src, base) {
        Ok(found) => found,
        Err(status) => return (Extraction::failed(status), 0),
    };
    if bodies.is_empty() {
        return (Extraction::failed(unsupported()), 0);
    }
    let mut out = Out {
        kind: Kind::Sequence,
        frags: Sink::default(),
        work: 0,
    };
    let mut first = None;
    for body in bodies {
        if out.frags.is_full() {
            break;
        }
        out.work += 1;
        out.kind = detect(&body);
        first.get_or_insert(out.kind);
        let mut block = Block::None;
        for (at, t) in body {
            if out.frags.is_full() {
                break;
            }
            line(&mut out, &mut block, at, t);
        }
    }
    let work = out.work;
    let (fragments, hit) = out.frags.finish();
    let status = match hit {
        Some(_) => crate::sink::status(hit),
        None if skipped => unsupported(),
        None => ExtractionStatus::Ok,
    };
    (
        Extraction {
            diagram: first.map(|kind| format!("plantuml.{}", kind.name())),
            fragments,
            status,
        },
        work,
    )
}

fn unsupported() -> ExtractionStatus {
    ExtractionStatus::Unsupported(DiagramPhrase::PlantUmlUnsupported.as_static())
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
    out.work += 1;
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
    if t.len() >= 4 && t.starts_with("==") && t.ends_with("==") && out.kind == Kind::Sequence {
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
    let name = t[word.len()..].split_whitespace().next().unwrap_or("");
    // Clip once, so a long name is not copied into every member of its body.
    let name = clip(name).into_owned();
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
    while !out.frags.is_full()
        && let Some(a) = t[rest..].find('(')
    {
        let open = rest + a;
        let Some(n) = t[open..].find(')') else { break };
        out.work += a + n + 1;
        let role = if first { Role::Node } else { Role::Edge };
        out.push(at, t, (open + 1, open + n), role, "branch");
        first = false;
        rest = open + n + 1;
    }
}

#[cfg(test)]
mod tests {
    use super::extract_counted;
    use crate::{ExtractionStatus, Format, MAX_FRAGMENTS, Role, extract};

    #[test]
    fn reading_stops_at_the_fragment_cap_instead_of_scanning_the_tail() {
        let doc = |n: usize| format!("@startuml\n{}@enduml\n", "A -> B : m\n".repeat(n));
        let head = doc(MAX_FRAGMENTS + 1);
        let full = doc(MAX_FRAGMENTS + 1 + 50_000);
        assert_eq!(extract_counted(&full, 0).1, extract_counted(&head, 0).1);
    }

    #[test]
    fn blocks_after_the_fragment_cap_are_not_detected() {
        let full = "A -> B : m\n".repeat(MAX_FRAGMENTS + 1);
        let doc = |extra: usize| {
            format!(
                "@startuml\n{full}@enduml\n{}",
                "@startuml\nA -> B : x\n@enduml\n".repeat(extra)
            )
        };
        assert_eq!(
            extract_counted(&doc(50), 0).1,
            extract_counted(&doc(1), 0).1
        );
    }

    #[test]
    fn parenthesised_labels_on_one_line_stop_at_the_fragment_cap() {
        let doc = |n: usize| format!("@startuml\nif {}\n@enduml\n", "(x) ".repeat(n));
        let head = doc(MAX_FRAGMENTS + 1);
        let full = doc(MAX_FRAGMENTS + 1 + 50_000);
        assert_eq!(extract_counted(&full, 0).1, extract_counted(&head, 0).1);
    }

    #[test]
    fn an_opened_diagram_without_an_end_marker_is_malformed() {
        let ex = extract(Format::PlantUml, "@startuml\nA -> B : hi\n", 0);
        assert_eq!(ex.status, ExtractionStatus::Malformed("missing @enduml"));
        assert_eq!(ex.fragments, Vec::new());
    }

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
    fn other_diagram_types_are_unsupported() {
        let ex = extract(Format::PlantUml, "@startmindmap\n* root\n@endmindmap\n", 0);
        assert_eq!(ex.diagram, None);
        assert_eq!(
            ex.status,
            ExtractionStatus::Unsupported("unsupported PlantUML diagram type")
        );
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

#[cfg(test)]
mod robustness_tests {
    use crate::{Format, Role, extract};

    #[test]
    fn short_equals_runs_do_not_panic() {
        for line in ["==", "===", "= =", "====", "==x", "=", "== =="] {
            let src = format!("@startuml\nparticipant A\n{line}\n@enduml\n");
            let _ = extract(Format::PlantUml, &src, 0);
        }
    }

    #[test]
    fn four_equals_is_an_empty_separator_and_yields_no_fragment() {
        let src = "@startuml\nparticipant A\n====\n@enduml\n";
        assert_eq!(extract(Format::PlantUml, src, 0).fragments, Vec::new());
    }

    #[test]
    fn keywords_in_comments_notes_and_message_text_do_not_pick_the_type() {
        for src in [
            "@startuml\n' participant A\ncomponent [W] as W\nW --> W : x\n@enduml\n",
            "@startuml\n/' participant A\nalt x '/\ncomponent [W] as W\n@enduml\n",
            "@startuml\ncomponent [W] as W\nnote over W\n  participant B\nend note\n@enduml\n",
            "@startuml\ncomponent [W] as W\nlegend\n  activate me\nendlegend\n@enduml\n",
            "@startuml\ncomponent [W] as W\nW --> W : go [*]\n@enduml\n",
        ] {
            let ex = extract(Format::PlantUml, src, 0);
            assert_eq!(ex.diagram.as_deref(), Some("plantuml.component"), "{src}");
        }
    }

    #[test]
    fn real_structure_still_picks_the_type() {
        let ex = extract(
            Format::PlantUml,
            "@startuml\n[*] --> Idle\nIdle --> Busy : x\n@enduml\n",
            0,
        );
        assert_eq!(ex.diagram.as_deref(), Some("plantuml.state"));
        let ex = extract(
            Format::PlantUml,
            "@startuml\nparticipant A\nA -> B : hi\n@enduml\n",
            0,
        );
        assert_eq!(ex.fragments[0].role, Role::Message);
    }
}
