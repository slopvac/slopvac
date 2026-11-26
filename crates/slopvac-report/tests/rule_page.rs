//! Only the rules of the built-in plugin `core` have a documentation page.

use slopvac_model::RuleId;
use slopvac_report::RenderOptions;

fn options() -> RenderOptions {
    RenderOptions {
        docs_base: Some("https://docs.example/slopvac/".to_owned()),
    }
}

#[test]
fn a_rule_of_core_has_a_page() {
    let id = RuleId::new("core:orwell.not-un").unwrap();
    assert_eq!(
        options().rule_page(&id, "orwell").as_deref(),
        Some("https://docs.example/slopvac/v3/rules/orwell/not-un/")
    );
}

#[test]
fn a_rule_of_another_plugin_has_no_page() {
    let id = RuleId::new("acme:style.casing").unwrap();
    assert_eq!(options().rule_page(&id, "style"), None);
}
