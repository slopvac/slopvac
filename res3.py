p = "crates/slopvac/Cargo.toml"
s = open(p).read()
head, dev = s.split("[dev-dependencies]")
dev = dev.replace("serde_json.workspace = true\n", "", 1)
open(p, "w").write(head + "[dev-dependencies]" + dev)

p = "crates/slopvac/src/init/mod.rs"
s = open(p).read()
i = s.index("/// How a path appears in a message")
j = s.index("}\n", s.index("fn shown(path"))
# find end of fn shown (its closing brace is the first "\n}\n" after fn)
k = s.index("\n}\n", s.index("fn shown(path")) + 3
s = (
    s[:i]
    + """/// How a path appears in a message: as given when relative, redacted when absolute.
fn shown(path: &Path) -> String {
    redact::shown_path(path, &std::env::current_dir().unwrap_or_default())
}
"""
    + s[k:]
)
old = """    if m.contains_id("profile")
        && let Some((_, message)) = crate::cli::REMOVED_LINT_OPTIONS
            .iter()
            .find(|(name, _)| *name == "profile")
    {
        return Err(CliError::new("removed-option", *message));
    }"""
assert old in s
s = s.replace(
    old,
    """    if m.contains_id("profile")
        && let Some(message) = crate::cli::removed_message("profile")
    {
        return Err(CliError::new("removed-option", message));
    }""",
)
open(p, "w").write(s)

p = "crates/slopvac/src/cli.rs"
s = open(p).read()
old = """    // Removed in 3.0, and declared hidden so using it gets the pointer (see `init::run`).
    cmd.arg(Arg::new("profile").long("profile").num_args(0..=1).hide(true))"""
assert old in s
s = s.replace(
    old,
    """    // Removed in 3.0; `init::run` prints the pointer.
    with_removed(cmd, &["profile"])""",
)
open(p, "w").write(s)
