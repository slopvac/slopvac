//! The generated CLI reference names every visible flag, argument and command.

#[path = "../examples/reference.rs"]
#[allow(dead_code)]
mod reference;

use clap::Command;

/// Every visible long flag as (page, flag), found by walking the tree with no help from the
/// generator.
fn visible_flags(cmd: &Command, page: &str, found: &mut Vec<(String, String)>) {
    for arg in cmd.get_arguments() {
        if let Some(long) = arg.get_long()
            && !arg.is_hide_set()
            && !arg.is_global_set()
            && long != "help"
            && long != "version"
        {
            found.push((page.to_owned(), format!("--{long}")));
        }
    }
    for sub in cmd.get_subcommands().filter(|s| !s.is_hide_set()) {
        if sub.get_name() != "help" {
            found.push((page.to_owned(), sub.get_name().to_owned()));
            visible_flags(sub, page, found);
        }
    }
}

#[test]
fn every_visible_flag_and_subcommand_appears_on_its_commands_page() {
    let mut root = slopvac_cli::cli::command();
    root.build();
    let pages = reference::pages(&root);
    let mut checked = 0;
    for top in root
        .get_subcommands()
        .filter(|s| !s.is_hide_set() && s.get_name() != "help")
    {
        let path = format!("cli/{}.md", top.get_name());
        let text = &pages
            .iter()
            .find(|(p, _)| *p == path)
            .unwrap_or_else(|| panic!("{path} missing"))
            .1;
        let mut expected = Vec::new();
        visible_flags(top, &path, &mut expected);
        for (_, token) in expected {
            assert!(text.contains(&token), "{path} lacks {token}");
            checked += 1;
        }
    }
    assert!(checked > 20, "walked only {checked} items");
}

#[test]
fn hidden_commands_get_no_page_and_the_index_lists_global_flags_and_exit_codes() {
    let pages = reference::pages(&slopvac_cli::cli::command());
    assert!(pages.iter().all(|(p, _)| p != "cli/cli-reference.md"));
    let index = &pages
        .iter()
        .find(|(p, _)| p == "cli/index.md")
        .expect("index")
        .1;
    for needle in [
        "--offline",
        "--color",
        "--no-color",
        "| `0` |",
        "| `1` |",
        "| `2` |",
    ] {
        assert!(index.contains(needle), "index lacks {needle}");
    }
}
