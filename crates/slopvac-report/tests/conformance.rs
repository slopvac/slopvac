//! Full-output conformance: each format against its pinned specification, parsed back.

use std::collections::HashMap;

use jsonschema::Registry;
use scraper::{Html, Selector};
use serde_json::{Value, json};
use slopvac_model::{Gate, Report};
use slopvac_report::{Format, NoSources, RenderOptions, render, render_with};

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../schemas/v3");

const NASTY: &str = "Bad <img src=x onerror=alert(1)> & \"q\"\nsecond line\ttab | `x`";
const DOC1_PATH: &str = "docs/a b.md";
const DOC1_TEXT: &str = "alpha beta\ngamma <b>delta</b>\n";
const DOC2_PATH: &str = "ok.md";
const DOC2_TEXT: &str = "fine\n";

fn read(path: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(format!("{ROOT}/{path}")).unwrap()).unwrap()
}

fn finding(severity: &str, message: &str, span: Option<(u64, u64, [u64; 4])>) -> Value {
    let mut f = json!({
        "rule_id": "core:orwell.not-un", "pack": "core", "category": "orwell",
        "severity": severity, "message": message, "line": 1, "column": 1,
        "contexts": ["prose"], "ai_signal": "weak", "matched_text": "x",
        "provenance": {"pack": "core", "version": "3.0.0"},
    });
    if let Some((start, end, [l, c, el, ec])) = span {
        f["span"] = json!({"start": start, "end": end});
        f["line"] = json!(l);
        f["column"] = json!(c);
        f["end_line"] = json!(el);
        f["end_column"] = json!(ec);
    }
    f
}

fn report() -> Report {
    let mut base = read("conformance/lint-report/valid/multi-document.json");
    let template = base["documents"][1].clone();
    let mut doc1 = template.clone();
    doc1["path"] = json!(DOC1_PATH);
    let mut warn = finding("warning", "Replace it.", Some((6, 10, [1, 7, 1, 11])));
    warn["fix"] = json!({"edits": [{"span": {"start": 6, "end": 10}, "text": "X"}]});
    doc1["findings"] = json!([
        finding("error", NASTY, Some((0, 5, [1, 1, 1, 6]))),
        warn,
        finding("suggestion", "Consider it.", Some((11, 16, [2, 1, 2, 6]))),
        finding("warning", "Document-wide.", None),
    ]);
    doc1["gate"] =
        json!({"passed": false, "reasons": ["orwell: 3.0 per 100 words exceeds the budget of 1"]});
    doc1["unchecked"] = json!([{"code": "spell-check-skipped", "level": "warning", "message": "No dictionary.", "path": DOC1_PATH}]);
    let mut doc2 = template;
    doc2["path"] = json!(DOC2_PATH);
    doc2["findings"] = json!([finding("warning", "Mild.", Some((0, 4, [1, 1, 1, 5])))]);
    doc2["gate"] = json!({"passed": true, "reasons": []});
    doc2["unchecked"] = json!([]);
    base["documents"] = json!([doc1, doc2]);
    base["notices"] = json!([{"code": "directory-entries-skipped", "level": "info", "message": "Skipped 4 files."}]);
    serde_json::from_value(base).unwrap()
}

fn sources() -> HashMap<String, String> {
    HashMap::from([
        (DOC1_PATH.to_owned(), DOC1_TEXT.to_owned()),
        (DOC2_PATH.to_owned(), DOC2_TEXT.to_owned()),
    ])
}

fn out(format: Format) -> String {
    render(format, &report(), &sources()).unwrap()
}

fn fixture(name: &str) -> Value {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn assert_valid(validator: &jsonschema::Validator, instance: &Value, what: &str) {
    let errors: Vec<String> = validator
        .iter_errors(instance)
        .map(|e| format!("{e} at {}", e.instance_path()))
        .collect();
    assert!(errors.is_empty(), "{what}: {errors:?}");
}

#[test]
fn json_validates_against_the_lint_report_schema() {
    let common = read("common.schema.json");
    let id = common["$id"].as_str().unwrap().to_owned();
    let registry = Registry::new().add(id, common).unwrap().prepare().unwrap();
    let validator = jsonschema::options()
        .with_registry(&registry)
        .build(&read("lint-report.schema.json"))
        .unwrap();
    let rendered: Value = serde_json::from_str(&out(Format::Json)).unwrap();
    assert_valid(&validator, &rendered, "json");
}

#[test]
fn sarif_validates_and_maps_severities() {
    let schema = fixture("sarif-schema-2.1.0.json");
    let validator = jsonschema::draft4::new(&schema).unwrap();
    let sarif: Value = serde_json::from_str(&out(Format::Sarif)).unwrap();
    assert_valid(&validator, &sarif, "sarif");
    let results = sarif["runs"][0]["results"].as_array().unwrap();
    let levels: Vec<&str> = results
        .iter()
        .map(|r| r["level"].as_str().unwrap())
        .collect();
    assert_eq!(levels, ["error", "warning", "note", "warning", "warning"]);
    assert_eq!(
        results[0]["locations"][0]["physicalLocation"]["artifactLocation"]["uri"],
        "docs/a%20b.md"
    );
    // The document-wide finding has a line and no column.
    assert_eq!(
        results[3]["locations"][0]["physicalLocation"]["region"],
        json!({"startLine": 1})
    );
    let invocation = &sarif["runs"][0]["invocations"][0];
    assert_eq!(invocation["executionSuccessful"], false);
    let ids: Vec<&str> = invocation["toolExecutionNotifications"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["descriptor"]["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["directory-entries-skipped", "spell-check-skipped"]);
}

#[test]
fn rdjsonl_lines_validate_and_documents_wide_findings_have_no_column() {
    let validator = jsonschema::draft7::new(&fixture("reviewdog-diagnostic.schema.json")).unwrap();
    let rendered = out(Format::Rdjsonl);
    let lines: Vec<Value> = rendered
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 5);
    for (i, line) in lines.iter().enumerate() {
        assert_valid(&validator, line, &format!("rdjsonl line {i}"));
    }
    let severities: Vec<&str> = lines
        .iter()
        .map(|l| l["severity"].as_str().unwrap())
        .collect();
    assert_eq!(
        severities,
        ["ERROR", "WARNING", "INFO", "WARNING", "WARNING"]
    );
    assert_eq!(lines[3]["location"]["range"], json!({"start": {"line": 1}}));
    assert_eq!(lines[1]["suggestions"][0]["text"], "X");
}

#[test]
fn codeclimate_validates_and_keeps_descriptions_on_one_line() {
    let validator = jsonschema::draft7::new(&fixture("codeclimate-issue.schema.json")).unwrap();
    let issues: Vec<Value> = serde_json::from_str(&out(Format::CodeClimate)).unwrap();
    assert_eq!(issues.len(), 5);
    for (i, issue) in issues.iter().enumerate() {
        assert_valid(&validator, issue, &format!("codeclimate issue {i}"));
    }
    let severities: Vec<&str> = issues
        .iter()
        .map(|i| i["severity"].as_str().unwrap())
        .collect();
    assert_eq!(severities, ["major", "minor", "info", "minor", "minor"]);
    assert!(!issues[0]["description"].as_str().unwrap().contains('\n'));
}

#[test]
fn junit_failures_follow_the_gate_not_the_warnings() {
    let xml = out(Format::Junit);
    let doc = roxmltree::Document::parse(&xml).unwrap();
    let root = doc.root_element();
    assert_eq!(root.attribute("tests"), Some("7"));
    assert_eq!(root.attribute("failures"), Some("1"));
    let suites: Vec<_> = root
        .children()
        .filter(roxmltree::Node::is_element)
        .collect();
    assert_eq!(suites.len(), 2);
    assert_eq!(suites[0].attribute("name"), Some(DOC1_PATH));
    assert_eq!(
        (
            suites[0].attribute("tests"),
            suites[0].attribute("failures")
        ),
        (Some("5"), Some("1"))
    );
    assert_eq!(
        (
            suites[1].attribute("tests"),
            suites[1].attribute("failures")
        ),
        (Some("2"), Some("0"))
    );
    let failure_count = doc
        .descendants()
        .filter(|n| n.has_tag_name("failure"))
        .count();
    assert_eq!(failure_count, 1);
    let failure = doc
        .descendants()
        .find(|n| n.has_tag_name("failure"))
        .unwrap();
    assert!(
        failure
            .attribute("message")
            .unwrap()
            .contains("exceeds the budget")
    );
    // The error finding's text survives as system-out on a passing case.
    let first_out = doc
        .descendants()
        .find(|n| n.has_tag_name("system-out"))
        .unwrap();
    assert!(
        first_out
            .text()
            .unwrap()
            .contains("<img src=x onerror=alert(1)>")
    );
}

#[test]
fn junit_passes_when_every_gate_passes_even_with_warnings() {
    let mut r = report();
    r.documents[0].gate = Gate::passed();
    let xml = render(Format::Junit, &r, &sources()).unwrap();
    let doc = roxmltree::Document::parse(&xml).unwrap();
    assert_eq!(doc.root_element().attribute("failures"), Some("0"));
    assert_eq!(
        doc.descendants()
            .filter(|n| n.has_tag_name("failure"))
            .count(),
        0
    );
}

#[test]
fn checkstyle_parses_and_attribute_whitespace_round_trips() {
    let xml = out(Format::Checkstyle);
    let doc = roxmltree::Document::parse(&xml).unwrap();
    let files: Vec<_> = doc
        .descendants()
        .filter(|n| n.has_tag_name("file"))
        .collect();
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].attribute("name"), Some(DOC1_PATH));
    let errors: Vec<_> = files[0]
        .children()
        .filter(roxmltree::Node::is_element)
        .collect();
    assert_eq!(errors.len(), 4);
    assert_eq!(errors[0].attribute("message"), Some(NASTY));
    let severities: Vec<_> = errors
        .iter()
        .map(|e| e.attribute("severity").unwrap())
        .collect();
    assert_eq!(severities, ["error", "warning", "info", "warning"]);
}

#[test]
fn html_is_inert_and_accessible() {
    let page = Html::parse_document(&out(Format::Html));
    let sel = |s: &str| Selector::parse(s).unwrap();
    assert_eq!(page.select(&sel("h1")).count(), 1);
    assert_eq!(page.select(&sel("html[lang=en]")).count(), 1);
    assert_eq!(page.select(&sel("title")).count(), 1);
    assert_eq!(page.select(&sel("main")).count(), 1);
    for forbidden in [
        "script",
        "img",
        "iframe",
        "link",
        "object",
        "embed",
        "form",
        "[onerror]",
        "[src]",
    ] {
        assert_eq!(page.select(&sel(forbidden)).count(), 0, "{forbidden}");
    }
    let first_link = page.select(&sel("a")).next().unwrap();
    assert_eq!(first_link.value().attr("href"), Some("#findings"));
    assert!(page.select(&sel("#findings")).next().is_some());
    let headers = page.select(&sel("th")).collect::<Vec<_>>();
    assert_ne!(headers.len(), 0);
    assert!(
        headers
            .iter()
            .all(|h| matches!(h.value().attr("scope"), Some("col" | "row")))
    );
    let tables = page.select(&sel("table")).count();
    assert_eq!(page.select(&sel("table > caption")).count(), tables);
    // Headings never skip a level: h1 then h2 only.
    assert_eq!(page.select(&sel("h3, h4, h5, h6")).count(), 0);
    // The hostile message appears as text, not as markup.
    let text: String = page.root_element().text().collect();
    assert!(text.contains("<img src=x onerror=alert(1)>"));
    for word in [
        "FAIL",
        "PASS",
        "error",
        "warning",
        "suggestion",
        "UNCHECKED",
    ] {
        assert!(text.contains(word), "{word}");
    }
    let styles = page
        .select(&sel("style"))
        .next()
        .unwrap()
        .text()
        .collect::<String>();
    assert!(!styles.contains("http"));
    assert!(styles.contains("prefers-color-scheme") && styles.contains("prefers-reduced-motion"));
}

#[test]
fn rule_links_use_the_docs_base() {
    let options = RenderOptions {
        docs_base: Some("https://example.test/slopvac/".into()),
    };
    let want = "https://example.test/slopvac/v3/rules/orwell/not-un/";
    assert_eq!(
        options.rule_url("orwell", "orwell.not-un").as_deref(),
        Some("https://example.test/slopvac/v3/rules/orwell/not-un/")
    );
    assert_eq!(
        options
            .rule_url("prose-inflation", "prose-inflation.slop-lexicon")
            .as_deref(),
        Some("https://example.test/slopvac/v3/rules/prose-inflation/slop-lexicon/")
    );
    let sarif: Value =
        serde_json::from_str(&render_with(Format::Sarif, &report(), &sources(), &options).unwrap())
            .unwrap();
    assert_eq!(
        sarif["runs"][0]["tool"]["driver"]["rules"][0]["helpUri"],
        want
    );
    let rd = render_with(Format::Rdjsonl, &report(), &sources(), &options).unwrap();
    let first: Value = serde_json::from_str(rd.lines().next().unwrap()).unwrap();
    assert_eq!(first["code"]["url"], want);
    let none = RenderOptions { docs_base: None };
    let plain: Value =
        serde_json::from_str(&render_with(Format::Sarif, &report(), &sources(), &none).unwrap())
            .unwrap();
    assert!(
        plain["runs"][0]["tool"]["driver"]["rules"][0]
            .get("helpUri")
            .is_none()
    );
}

#[test]
fn default_docs_base_comes_from_docs_site_toml() {
    let path = format!("{}/../../docs/site.toml", env!("CARGO_MANIFEST_DIR"));
    let default = RenderOptions::default().docs_base;
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let line = text
                .lines()
                .find(|l| l.trim_start().starts_with("site_url"))
                .unwrap();
            let want = line.split('=').nth(1).unwrap().trim().trim_matches('"');
            assert_eq!(default.as_deref(), Some(want));
        }
        Err(_) => assert_eq!(default, None),
    }
}

#[test]
fn spanless_findings_need_no_source_in_sarif_and_rdjsonl() {
    let mut r = report();
    for doc in &mut r.documents {
        doc.findings.retain(|f| f.extent.is_none());
    }
    for format in [Format::Sarif, Format::Rdjsonl] {
        render(format, &r, &NoSources).unwrap();
    }
}

/// Parse `#rgb` or `#rrggbb`.
fn hex(color: &str) -> [f64; 3] {
    let h = color.trim_start_matches('#');
    let h = if h.len() == 3 {
        h.chars().flat_map(|c| [c, c]).collect::<String>()
    } else {
        h.to_owned()
    };
    let channel = |i: usize| f64::from(u8::from_str_radix(&h[i..i + 2], 16).unwrap()) / 255.0;
    [channel(0), channel(2), channel(4)]
}

fn luminance(color: &str) -> f64 {
    let [r, g, b] = hex(color).map(|c| {
        if c <= 0.039_28 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    });
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

/// WCAG 2.x contrast ratio.
fn contrast(a: &str, b: &str) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// The `--name:value` pairs of the first `:root{...}` block at or after `from`.
fn root_vars(css: &str, from: usize) -> (HashMap<String, String>, usize) {
    let start = css[from..].find(":root{").unwrap() + from + ":root{".len();
    let end = css[start..].find('}').unwrap() + start;
    let vars = css[start..end]
        .split(';')
        .filter_map(|decl| decl.split_once(':'))
        .filter(|(name, _)| name.starts_with("--"))
        .map(|(name, value)| (name.trim_start_matches("--").to_owned(), value.to_owned()))
        .collect();
    (vars, end)
}

#[test]
fn html_colours_meet_wcag_aa_in_both_schemes_and_focus_is_visible() {
    let page = Html::parse_document(&out(Format::Html));
    let css: String = page
        .select(&Selector::parse("style").unwrap())
        .next()
        .unwrap()
        .text()
        .collect();
    let (light, after_light) = root_vars(&css, 0);
    let media = css.find("prefers-color-scheme:dark").unwrap();
    assert!(media > after_light, "the dark scheme follows the light one");
    let (dark, _) = root_vars(&css, media);
    for (scheme, vars) in [("light", &light), ("dark", &dark)] {
        for text in ["fg", "muted", "pass", "fail"] {
            for ground in ["bg", "panel"] {
                let ratio = contrast(&vars[text], &vars[ground]);
                assert!(ratio >= 4.5, "{scheme}: {text} on {ground} is {ratio:.2}");
            }
        }
        for ground in ["bg", "panel"] {
            let ratio = contrast(&vars["focus"], &vars[ground]);
            assert!(
                ratio >= 3.0,
                "{scheme}: focus ring on {ground} is {ratio:.2}"
            );
        }
        // Table borders and panel outlines use --muted: UI components need 3:1.
        assert!(
            contrast(&vars["muted"], &vars["bg"]) >= 3.0,
            "{scheme}: border"
        );
    }
    // A visible focus indicator on each interactive element, drawn with the focus colour.
    for selector in ["a:focus", "summary:focus"] {
        let rule = css
            .split('}')
            .find(|r| r.contains(selector))
            .unwrap_or_else(|| panic!("{selector}"));
        assert!(
            rule.contains("outline:3px solid var(--focus)"),
            "{selector}: {rule}"
        );
    }
}

#[test]
fn contrast_helper_matches_known_ratios() {
    assert!((contrast("#000", "#fff") - 21.0).abs() < 1e-9);
    assert!((contrast("#767676", "#fff") - 4.54).abs() < 0.01);
}
