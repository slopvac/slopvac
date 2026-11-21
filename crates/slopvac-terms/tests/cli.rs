//! The `terms` subcommands through `command()` and `run()`.
#![allow(clippy::unwrap_used, reason = "test code: a panic is a failed test")]

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
    assert_eq!(out, "added AcmeCloud (flags wrong case)\n");
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
    terms(&dir, &["add", "Acme"]);
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
    std::fs::write(dir.join("ignore.txt"), "# c\nfrobz\nmispel\n").unwrap();
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
    for w in ["frobz", "mispel", "alpha", "beta"] {
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

fn write_terms(dir: &Path, text: &str) {
    std::fs::create_dir_all(dir.join(".slopvac")).unwrap();
    std::fs::write(dir.join(".slopvac/terms.toml"), text).unwrap();
}

#[test]
fn case_sensitive_flag_is_serialised_and_changes_checking() {
    let dir = scratch("case-flag");
    let (_, out) = terms(&dir, &["add", "Go", "--case-sensitive"]);
    assert_eq!(out, "added Go (exact spelling only)\n");
    let text = file(&dir);
    assert!(
        text.contains("term = \"Go\"\ncase_sensitive = true"),
        "{text}"
    );
    assert!(!text.contains("ignore_case"), "{text}");
    std::fs::write(dir.join("doc.md"), "We go to Go and then go again.\n").unwrap();
    let (code, out) = terms(&dir, &["check", "doc.md"]);
    assert_eq!((code, out.as_str()), (0, ""));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn inline_array_files_support_add_remove_and_import() {
    let dir = scratch("inline");
    write_terms(
        &dir,
        "# project terms\nschema_version = 3\nterms = [\n  # the product\n  { term = \"Acme\", kind = \"product\" },\n  { term = \"Zed\" },\n]\n",
    );
    let (code, _) = terms(&dir, &["add", "Widget", "--kind", "tool"]);
    assert_eq!(code, 0);
    let text = file(&dir);
    assert!(text.starts_with("# project terms\n"), "{text}");
    assert!(text.contains("# the product"), "{text}");
    assert!(text.contains("term = \"Widget\""), "{text}");
    let (_, out) = terms(&dir, &["list"]);
    assert_eq!(
        out,
        "Acme\tproduct\tproject\nZed\t-\tproject\nWidget\ttool\tproject\n"
    );
    let (code, _) = terms(&dir, &["remove", "Zed"]);
    assert_eq!(code, 0);
    assert!(!file(&dir).contains("Zed"));
    assert!(file(&dir).contains("# the product"));
    std::fs::write(dir.join("accept.txt"), "Kubernetes\nAcme\n").unwrap();
    let (_, out) = terms(&dir, &["import", "--from", "vale", "accept.txt"]);
    assert!(
        out.starts_with("imported 1 terms (1 already present"),
        "{out}"
    );
    let (_, out) = terms(&dir, &["list"]);
    assert_eq!(
        out,
        "Acme\tproduct\tproject\nWidget\ttool\tproject\nKubernetes\t-\tproject\n"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn remove_from_a_mixed_file_leaves_the_other_terms() {
    let dir = scratch("remove-inline");
    write_terms(
        &dir,
        "schema_version = 3\nterms = [{ term = \"A\" }, { term = \"B\" }]\n",
    );
    terms(&dir, &["remove", "A"]);
    let text = file(&dir);
    assert!(text.starts_with("schema_version = 3\nterms = ["), "{text}");
    assert!(!text.contains("\"A\""), "{text}");
    let (_, out) = terms(&dir, &["list"]);
    assert_eq!(out, "B\t-\tproject\n");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn vale_comments_need_a_space_and_hash_tags_are_entries() {
    let dir = scratch("vale-comments");
    std::fs::write(dir.join("accept.txt"), "# a comment\n#\n#tag\nGitHub\n").unwrap();
    let (_, out) = terms(&dir, &["import", "--from", "vale", "accept.txt"]);
    assert_eq!(
        out,
        "imported 2 terms (0 already present, 0 patterns skipped)\n"
    );
    let text = file(&dir);
    assert!(text.contains("term = \"#tag\""), "{text}");
    assert!(!text.contains("a comment"), "{text}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn codespell_quoted_lists_parse_and_malformed_ones_change_nothing() {
    let dir = scratch("codespell-quotes");
    std::fs::write(
        dir.join("pyproject.toml"),
        "[tool.codespell]\nignore-words-list = \"alpha,beta\"\n",
    )
    .unwrap();
    let (_, out) = terms(&dir, &["import", "--from", "codespell", "pyproject.toml"]);
    assert!(out.starts_with("imported 2 terms"), "{out}");
    let before = file(&dir);
    for bad in [
        "ignore-words-list = \"alpha,beta\n",
        "ignore-words-list = ['a', 'b']\n",
    ] {
        std::fs::write(dir.join("bad.cfg"), bad).unwrap();
        let m = command()
            .try_get_matches_from(["terms", "import", "--from", "codespell", "bad.cfg"])
            .unwrap();
        assert!(run(&m, &dir, &mut Vec::new()).is_err(), "{bad}");
        assert_eq!(file(&dir), before, "{bad}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn cspell_yaml_configs_are_rejected_clearly() {
    let dir = scratch("cspell-yaml");
    std::fs::write(dir.join("cspell.yaml"), "words:\n  - alpha\n").unwrap();
    let m = command()
        .try_get_matches_from(["terms", "import", "--from", "cspell", "cspell.yaml"])
        .unwrap();
    let e = run(&m, &dir, &mut Vec::new()).unwrap_err().to_string();
    assert!(e.contains("cspell YAML configs are not supported"), "{e}");
    assert!(!dir.join(".slopvac/terms.toml").exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn double_backtick_spans_hide_terms_that_contain_a_backtick() {
    let text = "a `` x`github `` b github\n";
    let ranges = markdown_code_ranges(text);
    assert_eq!(ranges.len(), 1);
    assert_eq!(ranges[0], 2..16);
    assert_eq!(&text[2..16], "`` x`github ``");
    let dir = scratch("double-tick");
    std::fs::write(dir.join("doc.md"), text).unwrap();
    let (_, out) = terms(&dir, &["check", "doc.md"]);
    assert_eq!(out, "doc.md:1:20: github -> GitHub\n");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn code_ranges_cover_fences_and_inline_spans() {
    let text = "a `b` c\n```\nd\n```\ne\n";
    let ranges = markdown_code_ranges(text);
    assert_eq!(ranges, [2..5, 8..18]);
    assert_eq!(&text[2..5], "`b`");
}

#[test]
fn importing_zero_words_into_a_new_file_leaves_a_loadable_file() {
    let dir = scratch("import-zero");
    std::fs::write(dir.join("accept.txt"), "# only a comment\n").unwrap();
    let (_, out) = terms(&dir, &["import", "--from", "vale", "accept.txt"]);
    assert!(out.starts_with("imported 0 terms"), "{out}");
    let loaded = slopvac_terms::TermsFile::load(&dir.join(".slopvac/terms.toml")).unwrap();
    assert_eq!(loaded.terms, []);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn add_with_an_invalid_kind_fails_and_leaves_the_file_unchanged() {
    let dir = scratch("add-bad-kind");
    terms(&dir, &["add", "Acme"]);
    let before = file(&dir);
    let m = command()
        .try_get_matches_from(["terms", "add", "Widget", "--kind", "Proper Noun"])
        .unwrap();
    let e = run(&m, &dir, &mut Vec::new()).unwrap_err().to_string();
    assert!(e.contains("kebab-case"), "{e}");
    assert_eq!(file(&dir), before);
    std::fs::remove_dir_all(&dir).unwrap();
}

fn run_err(dir: &Path, cli_args: &[&str]) -> String {
    let mut argv = vec!["terms"];
    argv.extend_from_slice(cli_args);
    let m = command().try_get_matches_from(argv).unwrap();
    run(&m, dir, &mut Vec::new()).unwrap_err().to_string()
}

fn sparse(path: &Path, len: u64) {
    std::fs::File::create(path).unwrap().set_len(len).unwrap();
}

#[test]
fn suggest_refuses_a_document_one_byte_over_the_cap() {
    let dir = scratch("suggest-cap");
    sparse(&dir.join("big.md"), slopvac_fs::MAX_DOCUMENT_BYTES + 1);
    let e = run_err(&dir, &["suggest", "big.md"]);
    assert!(e.starts_with("[document-too-large] big.md "), "{e}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn import_refuses_a_word_list_one_byte_over_the_cap() {
    let dir = scratch("import-cap");
    sparse(
        &dir.join("accept.txt"),
        slopvac_terms::MAX_TERMS_FILE_BYTES + 1,
    );
    let e = run_err(&dir, &["import", "--from", "vale", "accept.txt"]);
    assert!(e.starts_with("[terms-file-too-large] accept.txt "), "{e}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn errors_about_the_project_terms_file_carry_no_absolute_path() {
    let dir = scratch("labels");
    let root = dir.display().to_string();
    std::fs::create_dir_all(dir.join(".slopvac")).unwrap();
    sparse(
        &dir.join(".slopvac/terms.toml"),
        slopvac_terms::MAX_TERMS_FILE_BYTES + 1,
    );
    for args in [&["list"][..], &["add", "Acme"], &["remove", "Acme"]] {
        let e = run_err(&dir, args);
        assert!(e.contains(".slopvac/terms.toml"), "{e}");
        assert!(!e.contains(&root), "{e}");
    }
    std::fs::write(dir.join(".slopvac/terms.toml"), "not toml = = =").unwrap();
    let e = run_err(&dir, &["list"]);
    assert!(!e.contains(&root), "{e}");
    std::fs::remove_dir_all(&dir).unwrap();
}

fn assert_no_path(e: &str, root: &Path, what: &str) {
    assert!(
        !e.contains(&root.display().to_string()),
        "{what} leaked an absolute path: {e}"
    );
}

#[test]
fn input_read_errors_carry_no_absolute_path() {
    let dir = scratch("input-labels");
    // A directory where a file is expected makes the read fail with an OS error.
    std::fs::create_dir(dir.join("doc.md")).unwrap();
    let abs = dir.join("doc.md").display().to_string();
    for args in [
        vec!["check", "doc.md"],
        vec!["check", abs.as_str()],
        vec!["suggest", "doc.md"],
        vec!["import", "--from", "vale", "doc.md"],
        vec!["import", "--from", "vale", abs.as_str()],
    ] {
        let e = run_err(&dir, &args);
        assert_no_path(&e, &dir, &args.join(" "));
        assert!(e.contains("doc.md"), "{e}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn import_parse_errors_carry_no_absolute_path() {
    let dir = scratch("import-parse");
    std::fs::write(dir.join("words.yaml"), "words: []").unwrap();
    let abs = dir.join("words.yaml").display().to_string();
    let e = run_err(&dir, &["import", "--from", "cspell", abs.as_str()]);
    assert_no_path(&e, &dir, "import");
    assert!(e.contains("words.yaml"), "{e}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[cfg(unix)]
#[test]
fn save_errors_carry_no_absolute_path() {
    use std::os::unix::fs::PermissionsExt;

    let dir = scratch("save-labels");
    std::fs::write(dir.join("doc.md"), "Acme Acme and Zorp Zorp\n").unwrap();
    std::fs::write(dir.join("accept.txt"), "Zorp\n").unwrap();
    let state = dir.join(".slopvac");
    std::fs::create_dir(&state).unwrap();
    terms(&dir, &["add", "Acme"]);
    // A read-only directory makes creating the temporary file fail.
    std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o555)).unwrap();
    let writable = std::fs::File::create(state.join("probe")).is_ok();
    if !writable {
        for args in [
            vec!["add", "Beta"],
            vec!["remove", "Acme"],
            vec!["suggest", "doc.md", "--accept", "Zorp"],
            vec!["import", "--from", "vale", "accept.txt"],
        ] {
            let e = run_err(&dir, &args);
            assert_no_path(&e, &dir, &args.join(" "));
            assert!(e.contains(".slopvac/terms.toml"), "{e}");
        }
    }
    std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
}
