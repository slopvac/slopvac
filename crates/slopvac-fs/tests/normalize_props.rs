//! Properties of `lexical_normalize` and `display_under_root`.

use std::path::{Component, Path};

use proptest::prelude::*;
use slopvac_fs::{display_under_root, lexical_normalize};

fn segments() -> impl Strategy<Value = String> {
    prop::collection::vec(prop_oneof!["a", "b", "\\.", "\\.\\.", "", "c d"], 0..8)
        .prop_map(|v| v.join("/"))
}

proptest! {
    #[test]
    fn idempotent(rel in segments(), abs in any::<bool>()) {
        let p = if abs { format!("/{rel}") } else { rel };
        let once = lexical_normalize(Path::new(&p));
        prop_assert_eq!(lexical_normalize(&once), once);
    }

    #[test]
    fn absolute_never_escapes_root(rel in segments()) {
        let n = lexical_normalize(Path::new(&format!("/{rel}")));
        prop_assert!(n.is_absolute());
        prop_assert!(n.components().all(|c| !matches!(c, Component::ParentDir | Component::CurDir)));
    }

    #[test]
    fn relative_only_leads_with_parents(rel in segments()) {
        prop_assume!(!rel.starts_with('/'));
        let n = lexical_normalize(Path::new(&rel));
        let mut seen_normal = false;
        for c in n.components() {
            match c {
                Component::ParentDir => prop_assert!(!seen_normal),
                Component::Normal(_) => seen_normal = true,
                Component::CurDir => prop_assert_eq!(n.components().count(), 1),
                Component::Prefix(_) | Component::RootDir => prop_assert!(false),
            }
        }
    }

    #[test]
    fn display_never_shows_an_absolute_path(rel in segments(), root in segments()) {
        let shown = display_under_root(Path::new(&format!("/{rel}")), Path::new(&format!("/{root}")));
        prop_assert!(!shown.starts_with('/'));
        prop_assert!(!shown.split('/').any(|s| s == ".."));
    }
}
