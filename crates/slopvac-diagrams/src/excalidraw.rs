//! Excalidraw extraction from the `.excalidraw` JSON scene.
//!
//! Text elements become fragments. A text bound to an arrow is an edge label, a
//! text bound to another shape is a node label, and a free text is a text box.
//! Frame names are groups. Deleted elements are skipped. An entry that is not an object, or a
//! live text element without a string `text`, makes the scene malformed. The fragments of the
//! elements before it are kept.

use std::collections::HashMap;

use crate::json::{Json, parse};
use crate::messages::DiagramPhrase;
use crate::sink::{Sink, clip};
use crate::{Extraction, ExtractionStatus, Fragment, Role};

pub(crate) fn extract(src: &str, base: usize) -> Extraction {
    let Some(root) = parse(src) else {
        return Extraction::failed(ExtractionStatus::Malformed("invalid JSON"));
    };
    let Some(Json::Arr(elements)) = root.get("elements") else {
        return Extraction::failed(ExtractionStatus::Malformed(
            DiagramPhrase::NoElements.as_static(),
        ));
    };
    let types: HashMap<&str, &str> = elements
        .iter()
        .filter_map(|e| Some((e.get("id")?.as_str()?, e.get("type")?.as_str()?)))
        .collect();
    let mut fragments = Sink::default();
    let mut malformed = None;
    for e in elements {
        if fragments.is_full() {
            break;
        }
        if !matches!(e, Json::Obj(_)) {
            malformed = Some(DiagramPhrase::ElementNotObject.as_static());
            break;
        }
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
            if key == "text" {
                malformed = Some(DiagramPhrase::TextElementEmpty.as_static());
                break;
            }
            continue;
        };
        let id = e.get("id").and_then(Json::as_str).unwrap_or("?");
        let raw = &src[span.clone()];
        fragments.push(Fragment {
            span: base + span.start..base + span.end,
            text: (raw != value).then(|| value.clone()),
            role,
            logical: format!("excalidraw/{}[{}]", role.as_str(), clip(id)),
        });
    }
    let (fragments, hit) = fragments.finish();
    Extraction {
        diagram: Some("excalidraw".to_string()),
        fragments,
        status: malformed.map_or_else(|| crate::sink::status(hit), ExtractionStatus::Malformed),
    }
}

#[cfg(test)]
mod tests {
    use crate::{Extraction, ExtractionStatus, Format, Role, extract};

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
    fn invalid_json_is_malformed() {
        assert_eq!(
            extract(Format::Excalidraw, "{", 0),
            Extraction::failed(ExtractionStatus::Malformed("invalid JSON"))
        );
    }

    #[test]
    fn a_scene_without_elements_is_malformed() {
        assert_eq!(
            extract(Format::Excalidraw, "{}", 0).status,
            ExtractionStatus::Malformed("no elements array")
        );
    }

    #[test]
    fn a_scene_with_no_elements_is_ok_and_empty() {
        let ex = extract(Format::Excalidraw, r#"{"elements":[]}"#, 0);
        assert_eq!(ex.status, ExtractionStatus::Ok);
        assert_eq!(ex.fragments, Vec::new());
    }
}
