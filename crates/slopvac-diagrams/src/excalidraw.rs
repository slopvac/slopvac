//! Excalidraw extraction from the `.excalidraw` JSON scene.
//!
//! Text elements become fragments. A text bound to an arrow is an edge label, a
//! text bound to another shape is a node label, and a free text is a text box.
//! Frame names are groups. Deleted elements are skipped.

use std::collections::HashMap;

use crate::json::{Json, parse};
use crate::{Extraction, Fragment, Role};

pub(crate) fn extract(src: &str, base: usize) -> Extraction {
    let Some(root) = parse(src) else {
        return Extraction::default();
    };
    let Some(Json::Arr(elements)) = root.get("elements") else {
        return Extraction::default();
    };
    let types: HashMap<&str, &str> = elements
        .iter()
        .filter_map(|e| Some((e.get("id")?.as_str()?, e.get("type")?.as_str()?)))
        .collect();
    let mut fragments = Vec::new();
    for e in elements {
        if matches!(e.get("isDeleted"), Some(Json::Bool(true))) {
            continue;
        }
        let kind = e.get("type").and_then(Json::as_str);
        let (key, role) = match kind {
            Some("text") => {
                let container = e.get("containerId").and_then(Json::as_str);
                let role = match container.and_then(|id| types.get(id)) {
                    Some(&"arrow") => Role::Edge,
                    Some(_) => Role::Node,
                    None => Role::Text,
                };
                ("text", role)
            }
            Some("frame" | "magicframe") => ("name", Role::Group),
            _ => continue,
        };
        let Some(Json::Str { span, value }) = e.get(key) else {
            continue;
        };
        let id = e.get("id").and_then(Json::as_str).unwrap_or("?");
        let raw = &src[span.clone()];
        fragments.push(Fragment {
            span: base + span.start..base + span.end,
            text: (raw != value).then(|| value.clone()),
            role,
            logical: format!("excalidraw/{}[{id}]", role.as_str()),
        });
    }
    Extraction {
        diagram: Some("excalidraw".to_string()),
        fragments,
    }
}

#[cfg(test)]
mod tests {
    use crate::{Extraction, Format, Role, extract};

    const SCENE: &str = r#"{"type":"excalidraw","elements":[
{"id":"r1","type":"rectangle","isDeleted":false},
{"id":"a1","type":"arrow","isDeleted":false},
{"id":"t1","type":"text","text":"Load balancer","containerId":"r1","isDeleted":false},
{"id":"t2","type":"text","text":"calls\nover HTTP","containerId":"a1"},
{"id":"t3","type":"text","text":"Legend: blue is new","containerId":null},
{"id":"t4","type":"text","text":"Gone","isDeleted":true},
{"id":"f1","type":"frame","name":"Edge tier"}]}"#;

    fn run() -> Extraction {
        extract(Format::Excalidraw, SCENE, 10)
    }

    #[test]
    fn roles_follow_the_container() {
        let got: Vec<_> = run()
            .fragments
            .iter()
            .map(|f| (f.role, f.logical.clone()))
            .collect();
        assert_eq!(
            got,
            [
                (Role::Node, "excalidraw/node[t1]".to_string()),
                (Role::Edge, "excalidraw/edge[t2]".to_string()),
                (Role::Text, "excalidraw/text[t3]".to_string()),
                (Role::Group, "excalidraw/group[f1]".to_string()),
            ]
        );
    }

    #[test]
    fn spans_point_at_the_raw_json_bytes() {
        let ex = run();
        let first = &ex.fragments[0];
        let start = SCENE.find("Load balancer").unwrap() + 10;
        assert_eq!(first.span, start..start + "Load balancer".len());
        assert!(first.fixable());
    }

    #[test]
    fn escaped_text_is_converted_and_not_fixable() {
        let ex = run();
        let edge = &ex.fragments[1];
        assert_eq!(edge.text.as_deref(), Some("calls\nover HTTP"));
        assert!(!edge.fixable());
    }

    #[test]
    fn invalid_json_yields_nothing() {
        assert_eq!(extract(Format::Excalidraw, "{", 0), Extraction::default());
    }
}
