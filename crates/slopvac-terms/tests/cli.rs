//! The `terms` subcommands through `command()` and `run()`.

use std::path::{Path, PathBuf};

use slopvac_terms::cli::{command, markdown_code_ranges, run};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("slopvac-terms-cli-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn terms(dir: &Path, cli_args: &[&str]) -> (i32, String) {
    let mut argv = vec!["terms"];
    argv.extend_from_slice(cli_args);
    let m = command().try_get_matches_from(argv).unwrap();
    let mut buf = Vec::new();
    let code = run(&m, dir, &mut buf).unwrap();
    (code, String::from_utf8(buf).unwrap())
}

fn file(dir: &Path) -> String {
    std::fs::read_to_string(dir.join(".slopvac/terms.toml")).unwrap()
}

#[test]
fn add_infers_case_sensitivity_from_the_term() {
    let dir = scratch("add-infer");
    let (code, out) = terms(&dir, &["add", "dbt"]);
    assert_eq!((code, out.as_str()), (0, "added dbt (ignoring case)\n"));
    let (_, out) = terms(
        &dir,
        &[
            "add",
            "AcmeCloud",
            "--kind",
            "product",
            "--variant",
            "Acme Cloud",
        ],
    );
    assert_eq!(out, "added AcmeCloud (case-sensitive)\n");
    let text = file(&dir);
    assert!(
        text.contains("term = \"dbt\"\nignore_case = true"),
        "{text}"
    );
    assert!(text.contains("term = \"AcmeCloud\""), "{text}");
    assert!(text.contains("kind = \"product\""), "{text}");
    assert!(text.contains("variants = [\"Acme Cloud\"]"), "{text}");
    assert!(!text.contains("ignore_case = true\nkind"), "{text}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn add_flag_overrides_the_inference() {
    let dir = scratch("add-flag");
    terms(&dir, &["add", "Foo", "--ignore-case"]);
    terms(&dir, &["add", "bar", "--case-sensitive"]);
    let text = file(&dir);
    assert!(
        text.contains("term = \"Foo\"\nignore_case = true"),
        "{text}"
    );
    assert!(!text.contains("term = \"bar\"\nignore_case"), "{text}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn add_and_remove_keep_comments_and_layout() {
    let dir = scratch("comments");
    std::fs::create_dir_all(dir.join(".slopvac")).unwrap();
    std::fs::write(
        dir.join(".slopvac/terms.toml"),
        "# our product names\nschema_version = 3\n\n# main product\n[[terms]]\nterm = \"Acme\"\n",
    )
    .unwrap();
    terms(&dir, &["add", "Widget"]);
    let text = file(&dir);
    assert!(
        text.starts_with("# our product names\nschema_version = 3\n"),
        "{text}"
    );
    assert!(
        text.contains("# main product\n[[terms]]\nterm = \"Acme\""),
        "{text}"
    );
    let (code, _) = terms(&dir, &["remove", "Widget"]);
    assert_eq!(code, 0);
    let text = file(&dir);
    assert!(text.contains("# main product"), "{text}");
    assert!(!text.contains("Widget"), "{text}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn remove_reports_a_missing_term_with_exit_1() {
    let dir = scratch("remove-missing");
    terms(&dir, &["add", "Acme"]);
    let (code, out) = terms(&dir, &["remove", "Nope"]);
    assert_eq!(code, 1);
    assert!(out.starts_with("no term Nope in "), "{out}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn adding_a_duplicate_is_an_error() {
    let dir = scratch("dup");
    terms(&dir, &["add", "Acme"]);
    let m = command()
        .try_get_matches_from(["terms", "add", "Acme"])
        .unwrap();
    let e = run(&m, &dir, &mut Vec::new()).unwrap_err().to_string();
    assert!(e.contains("term `Acme` already exists"), "{e}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn list_filters_by_kind_and_can_include_builtins() {
    let dir = scratch("list");
    terms(&dir, &["add", "Acme", "--kind", "product"]);
    terms(&dir, &["add", "Zed", "--kind", "tool"]);
    let (_, out) = terms(&dir, &["list", "--kind", "product"]);
    assert_eq!(out, "Acme\tproduct\tproject\n");
    let (_, out) = terms(&dir, &["list", "--builtin", "--kind", "protocol"]);
    assert!(out.contains("TCP\tprotocol\tbuiltin\n"), "{out}");
    assert!(!out.contains("Acme"), "{out}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn search_matches_terms_and_variants_ignoring_case() {
    let dir = scratch("search");
    terms(&dir, &["add", "Acme", "--variant", "ACME Corp"]);
    let (code, out) = terms(&dir, &["search", "acme c"]);
    assert_eq!((code, out.as_str()), (0, "Acme\t-\tproject\n"));
    let (code, out) = terms(&dir, &["search", "zzzz"]);
    assert_eq!((code, out.as_str()), (1, ""));
    let (_, out) = terms(&dir, &["search", "kubern"]);
    assert_eq!(out, "Kubernetes\ttool\tbuiltin\n");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn check_reports_wrong_case_with_position_and_skips_code() {
    let dir = scratch("check");
    std::fs::write(
        dir.join("doc.md"),
        "We use Github.\nRun `github` here.\n```\ngithub\n```\nAnd acme works.\n",
    )
    .unwrap();
    terms(&dir, &["add", "Acme", "--case-sensitive"]);
    let (code, out) = terms(&dir, &["check", "doc.md"]);
    assert_eq!(code, 1);
    assert_eq!(
        out,
        "doc.md:1:8: Github -> GitHub\ndoc.md:6:5: acme -> Acme\n"
    );
    std::fs::write(dir.join("clean.md"), "GitHub is fine.\n").unwrap();
    let (code, out) = terms(&dir, &["check", "clean.md"]);
    assert_eq!((code, out.as_str()), (0, ""));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn project_terms_override_builtin_spellings() {
    let dir = scratch("override");
    terms(&dir, &["add", "GitHub", "--ignore-case"]);
    std::fs::write(dir.join("doc.md"), "We use Github.\n").unwrap();
    let (code, _) = terms(&dir, &["check", "doc.md"]);
    assert_eq!(code, 0);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn import_reads_vale_accept_and_skips_patterns() {
    let dir = scratch("import-vale");
    std::fs::write(
        dir.join("accept.txt"),
        "# comment\nKubernetes\n[Kk]ubectl\nwebhook\n\nKubernetes\n",
    )
    .unwrap();
    terms(&dir, &["add", "webhook"]);
    let (code, out) = terms(&dir, &["import", "--from", "vale", "accept.txt"]);
    assert_eq!(code, 0);
    assert_eq!(
        out,
        "imported 1 terms (1 already present, 1 patterns skipped)\nskipped pattern: [Kk]ubectl\n"
    );
    assert!(file(&dir).contains("term = \"Kubernetes\""));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn import_reads_cspell_with_comments() {
    let dir = scratch("import-cspell");
    std::fs::write(
        dir.join("cspell.json"),
        "{\n // project words\n \"version\": \"0.2\",\n \"words\": [\"frobnicate\", \"Zeta\"],\n \"ignoreWords\": [\"a//b\"] /* x */\n}\n",
    )
    .unwrap();
    let (_, out) = terms(&dir, &["import", "--from", "cspell", "cspell.json"]);
    assert_eq!(
        out,
        "imported 3 terms (0 already present, 0 patterns skipped)\n"
    );
    let text = file(&dir);
    assert!(
        text.contains("term = \"frobnicate\"\nignore_case = true"),
        "{text}"
    );
    assert!(text.contains("term = \"Zeta\""), "{text}");
    assert!(text.contains("term = \"a//b\""), "{text}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn import_reads_codespell_lists_in_both_forms() {
    let dir = scratch("import-codespell");
    std::fs::write(dir.join("ignore.txt"), "# c\nteh\nmispel\n").unwrap();
    std::fs::write(
        dir.join(".codespellrc"),
        "[codespell]\nignore-words-list = alpha,beta\nskip = x\n",
    )
    .unwrap();
    let (_, out) = terms(&dir, &["import", "--from", "codespell", "ignore.txt"]);
    assert!(out.starts_with("imported 2 terms"), "{out}");
    let (_, out) = terms(&dir, &["import", "--from", "codespell", ".codespellrc"]);
    assert!(out.starts_with("imported 2 terms"), "{out}");
    let text = file(&dir);
    for w in ["teh", "mispel", "alpha", "beta"] {
        assert!(text.contains(&format!("term = \"{w}\"")), "{w}: {text}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn files_flag_selects_the_terms_file() {
    let dir = scratch("files");
    terms(&dir, &["add", "Acme", "--files", "docs/my-terms.toml"]);
    assert!(dir.join("docs/my-terms.toml").exists());
    assert!(!dir.join(".slopvac/terms.toml").exists());
    let (_, out) = terms(&dir, &["list", "--files", "docs/my-terms.toml"]);
    assert_eq!(out, "Acme\t-\tproject\n");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn code_ranges_cover_fences_and_inline_spans() {
    let text = "a `b` c\n```\nd\n```\ne\n";
    let ranges = markdown_code_ranges(text);
    assert_eq!(ranges, [2..5, 8..18]);
    assert_eq!(&text[2..5], "`b`");
}
