//! The hardened `git` subprocess.
//!
//! Git runs inside a repository the user pointed us at, and that repository controls
//! files git reads: `.git/config`, `.gitattributes` and the work tree. Every call here
//! closes the ways those can make `git diff` run a program or change what it prints:
//!
//! - argv is fixed and never goes through a shell. The only caller-supplied text is the
//!   base revision. It is resolved to a full object id first, and only that id reaches
//!   `diff`; a name that starts with `-` is refused so it cannot be read as an option.
//! - `--no-ext-diff` and `--no-textconv` stop `diff.external`, `GIT_EXTERNAL_DIFF` and
//!   `diff.<driver>.textconv` from running a program.
//! - `-c core.fsmonitor=false` stops a repository-configured fsmonitor hook from running
//!   on `git diff` and `git ls-files`.
//! - `--no-pager` and `--no-color` keep output plain, `--no-optional-locks` keeps a
//!   read-only query from writing the index, and `-c core.quotepath=off`, the pinned
//!   prefixes, `--inter-hunk-context=0` and `diff.relative=false` keep the patch format
//!   independent of user configuration.
//! - The child gets no `GIT_*` variables, so `GIT_DIR`, `GIT_WORK_TREE`, `GIT_INDEX_FILE`,
//!   `GIT_CONFIG_*` and the rest cannot redirect it to a different repository or inject
//!   configuration. The user's own global config is still read: it is the user's.
//! - The `diff` call ends with `--` and names no path.
//!
//! What this does not close: a clean filter named in a repository's `.git/config` can run
//! while git compares the work tree to the index. Running git in a repository you do not
//! trust carries that risk for every git command.

use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, Stdio};

use super::DiffError;

/// Options placed before every subcommand.
const GLOBAL_ARGS: [&str; 8] = [
    "--no-pager",
    "--no-optional-locks",
    "-c",
    "core.quotepath=off",
    "-c",
    "core.fsmonitor=false",
    "-c",
    "diff.relative=false",
];

/// Options for `git diff` that pin the patch format and switch off program hooks.
pub(super) const DIFF_ARGS: [&str; 10] = [
    "diff",
    "--no-ext-diff",
    "--no-textconv",
    "--no-color",
    "--unified=0",
    "--inter-hunk-context=0",
    "--find-renames",
    "--ignore-submodules=all",
    "--src-prefix=a/",
    "--dst-prefix=b/",
];

/// Run `git` with the hardened argument set and return stdout.
///
/// `args` follow the global options; `cwd` is the directory git starts in.
pub(super) fn run(cwd: &Path, args: &[&str]) -> Result<Vec<u8>, DiffError> {
    let mut command = Command::new("git");
    command.args(GLOBAL_ARGS).args(args);
    command.current_dir(cwd);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    scrub_environment(&mut command);
    let output = command.output().map_err(DiffError::Spawn)?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        Err(DiffError::Git(stderr))
    }
}

/// Remove every `GIT_*` variable from the child's environment.
fn scrub_environment(command: &mut Command) {
    let names: Vec<OsString> = std::env::vars_os()
        .map(|(name, _)| name)
        .filter(|name| is_git_variable(name))
        .collect();
    for name in names {
        command.env_remove(name);
    }
}

fn is_git_variable(name: &std::ffi::OsStr) -> bool {
    // Windows treats variable names case-insensitively.
    name.to_string_lossy()
        .to_ascii_uppercase()
        .starts_with("GIT_")
}
