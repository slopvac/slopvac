//! Which channel installed this binary, and the command that upgrades it.

use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

use slopvac_fs::{ReadOptions, read_capped_regular_to_string};

/// The text of a regular file of at most `cap` bytes, else `None`: a missing, special, oversized
/// or non-UTF-8 file all read as absent.
fn read_text(path: &Path, cap: u64) -> Option<String> {
    read_capped_regular_to_string(path, cap, ReadOptions::new()).ok()
}

/// Largest install receipt read; a real one is a few hundred bytes.
const MAX_RECEIPT_BYTES: u64 = 64 * 1024;
/// Largest wheel `RECORD` read; it lists the files of one wheel.
const MAX_RECORD_BYTES: u64 = 1024 * 1024;
/// Largest wheel `INSTALLER` read; it names one program.
const MAX_INSTALLER_BYTES: u64 = 4 * 1024;

/// Where to get a release when no upgrade command applies.
pub const RELEASES_URL: &str = "https://github.com/slopvac/slopvac/releases";

/// The Python tool that put a wheel install in place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WheelTool {
    /// `uv tool install slopvac`.
    Uv,
    /// `uvx slopvac`, an ephemeral environment in uv's cache.
    Uvx,
    /// `uv pip install` into an ordinary virtual environment.
    UvPip,
    /// A virtual environment or user site managed with pip.
    Pip,
}

/// How this binary was installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallMethod {
    /// The cargo-dist shell or PowerShell installer. `self update` works.
    Installer,
    /// The maturin binary wheel.
    Wheel(WheelTool),
    /// The Homebrew tap.
    Homebrew,
    /// A mise install.
    Mise,
    /// `cargo install`.
    Cargo,
    /// None of the above.
    Unknown,
}

impl InstallMethod {
    /// Stable identifier, for JSON output.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::Installer => "installer",
            Self::Wheel(WheelTool::Uv) => "uv-tool",
            Self::Wheel(WheelTool::Uvx) => "uvx",
            Self::Wheel(WheelTool::UvPip) => "uv-pip",
            Self::Wheel(WheelTool::Pip) => "pip",
            Self::Homebrew => "homebrew",
            Self::Mise => "mise",
            Self::Cargo => "cargo",
            Self::Unknown => "unknown",
        }
    }

    /// The command that upgrades this install, or `None` when no safe command exists (an unknown
    /// channel, or a cargo-style install).
    #[must_use]
    pub fn upgrade_command(self) -> Option<&'static str> {
        Some(match self {
            Self::Installer => "slopvac self update",
            Self::Wheel(WheelTool::Uv) => "uv tool upgrade slopvac",
            Self::Wheel(WheelTool::Uvx) => "uvx slopvac@latest",
            Self::Wheel(WheelTool::Pip) => "python -m pip install --upgrade slopvac",
            Self::Wheel(WheelTool::UvPip) => "uv pip install --upgrade slopvac",
            Self::Homebrew => "brew upgrade slopvac/tap/slopvac",
            Self::Mise => "mise upgrade slopvac",
            // No crate is published, so `cargo install slopvac` would fetch whoever registers the
            // name. Archive installs without a readable receipt land here too, and a `--git`
            // install builds unreleased code. Callers fall back to the signed releases page.
            Self::Cargo | Self::Unknown => return None,
        })
    }

    /// Whether the release registry for this channel is `PyPI` rather than GitHub.
    #[must_use]
    pub fn uses_pypi(self) -> bool {
        matches!(self, Self::Wheel(_))
    }
}

/// The parts of a cargo-dist install receipt that detection reads.
#[derive(Debug, Deserialize)]
struct Receipt {
    install_prefix: PathBuf,
    #[serde(default)]
    source: ReceiptSource,
}

#[derive(Debug, Default, Deserialize)]
struct ReceiptSource {
    #[serde(default)]
    app_name: String,
}

/// Directory that holds `slopvac-receipt.json`, as cargo-dist's installers and axoupdater place it.
fn receipt_dir(var: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    if let Some(dir) = var("AXOUPDATER_CONFIG_PATH").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    if cfg!(windows) {
        return var("LOCALAPPDATA").map(|d| PathBuf::from(d).join("slopvac"));
    }
    var("XDG_CONFIG_HOME")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .or_else(|| var("HOME").map(|h| PathBuf::from(h).join(".config")))
        .map(|d| d.join("slopvac"))
}

/// Whether a receipt names `exe` as a binary this installer put in place.
fn receipt_owns(exe: &Path, var: &dyn Fn(&str) -> Option<String>) -> bool {
    let Some(dir) = receipt_dir(var) else {
        return false;
    };
    let Some(text) = read_text(&dir.join("slopvac-receipt.json"), MAX_RECEIPT_BYTES) else {
        return false;
    };
    let Ok(receipt) = serde_json::from_str::<Receipt>(&text) else {
        return false;
    };
    if receipt.source.app_name != "slopvac" {
        return false;
    }
    let Some(parent) = exe.parent() else {
        return false;
    };
    let prefix = strip_verbatim_prefix(&receipt.install_prefix);
    parent == prefix || parent == prefix.join("bin")
}

fn names(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect()
}

fn has_pair(names: &[String], first: &str, second: &str) -> bool {
    names.windows(2).any(|w| w[0] == first && w[1] == second)
}

/// Resolve `.` and `..` without touching the file system.
fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// Find the wheel that installed `exe` from its installation metadata.
///
/// A user (`~/.local`) or system (`/usr/local`) pip install puts the binary in a plain `bin`
/// directory. The wheel's `slopvac-*.dist-info` under `<prefix>/lib*/python*/site-packages` names
/// it in `RECORD` (paths relative to `site-packages`) and names the installer in `INSTALLER`.
fn dist_info_tool(exe: &Path) -> Option<WheelTool> {
    let prefix = exe.parent()?.parent()?;
    let exe = lexical_normalize(exe);
    for lib in ["lib", "lib64"] {
        let Ok(pythons) = std::fs::read_dir(prefix.join(lib)) else {
            continue;
        };
        for python in pythons.flatten() {
            if !python.file_name().to_string_lossy().starts_with("python") {
                continue;
            }
            let site = python.path().join("site-packages");
            let Ok(entries) = std::fs::read_dir(&site) else {
                continue;
            };
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if !(name.starts_with("slopvac-") && name.ends_with(".dist-info")) {
                    continue;
                }
                let record =
                    read_text(&entry.path().join("RECORD"), MAX_RECORD_BYTES).unwrap_or_default();
                let lists_exe = record.lines().any(|line| {
                    let first = line.split(',').next().unwrap_or_default();
                    lexical_normalize(&site.join(first)) == exe
                });
                if !lists_exe {
                    continue;
                }
                let installer = read_text(&entry.path().join("INSTALLER"), MAX_INSTALLER_BYTES)
                    .unwrap_or_default();
                // `INSTALLER` names the program that wrote the files, not how the user installed
                // the tool. Only uv's receipt at the environment root marks a `uv tool` install.
                return Some(if installer.trim() != "uv" {
                    WheelTool::Pip
                } else if prefix.join("uv-receipt.toml").is_file() {
                    WheelTool::Uv
                } else {
                    WheelTool::UvPip
                });
            }
        }
    }
    None
}

fn wheel_tool(exe: &Path) -> Option<WheelTool> {
    let n = names(exe);
    if has_pair(&n, "uv", "tools") {
        return Some(WheelTool::Uv);
    }
    if n.iter()
        .any(|c| matches!(c.as_str(), "archive-v0" | "builds-v0" | "environments-v2"))
    {
        return Some(WheelTool::Uvx);
    }
    if let Some(tool) = dist_info_tool(exe) {
        return Some(tool);
    }
    let venv = exe.parent().and_then(Path::parent);
    if venv.is_some_and(|v| v.join("pyvenv.cfg").is_file())
        || n.iter().any(|c| c == "site-packages")
    {
        return Some(WheelTool::Pip);
    }
    None
}

fn cargo_bin(var: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    var("CARGO_HOME")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .or_else(|| var("HOME").map(|h| PathBuf::from(h).join(".cargo")))
        .map(|d| strip_verbatim_prefix(&d).join("bin"))
}

/// Drop the `\\?\` prefix that `canonicalize` adds on Windows, so a path compares with the
/// install prefix in a receipt. UNC paths keep theirs.
#[must_use]
pub fn strip_verbatim_prefix(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC\\") => PathBuf::from(rest),
        _ => path.to_path_buf(),
    }
}

/// Detect the install method of the binary at `exe`.
///
/// Pass the resolved path of the running executable. `var` reads the environment. The checks run
/// from the most specific evidence to the least: the installer receipt, then the Homebrew and mise
/// directory layouts, then wheel environments, then the cargo bin directory.
#[must_use]
pub fn detect(exe: &Path, var: &dyn Fn(&str) -> Option<String>) -> InstallMethod {
    let exe = &strip_verbatim_prefix(exe);
    if receipt_owns(exe, var) {
        return InstallMethod::Installer;
    }
    let n = names(exe);
    if n.iter().any(|c| c == "Cellar")
        || has_pair(&n, "Homebrew", "bin")
        || n.iter().any(|c| c == "linuxbrew")
    {
        return InstallMethod::Homebrew;
    }
    if has_pair(&n, "mise", "installs") {
        return InstallMethod::Mise;
    }
    if let Some(tool) = wheel_tool(exe) {
        return InstallMethod::Wheel(tool);
    }
    if cargo_bin(var).is_some_and(|bin| exe.parent() == Some(bin.as_path())) {
        return InstallMethod::Cargo;
    }
    InstallMethod::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, String)]) -> impl Fn(&str) -> Option<String> + use<> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).into(), v.clone()))
            .collect();
        move |k| map.get(k).cloned()
    }

    /// The variable that locates the per-user config directory on this platform.
    const CONFIG_HOME_VAR: &str = if cfg!(windows) {
        "LOCALAPPDATA"
    } else {
        "XDG_CONFIG_HOME"
    };

    fn write_receipt(dir: &Path, prefix: &Path, app: &str) {
        std::fs::create_dir_all(dir).unwrap();
        let json = serde_json::json!({
            "binaries": ["slopvac"],
            "install_layout": "flat",
            "install_prefix": prefix,
            "provider": {"source": "cargo-dist", "version": "0.34.0"},
            "source": {"app_name": app, "name": app, "owner": "slopvac", "release_type": "github"},
            "version": "3.0.0",
        });
        std::fs::write(dir.join("slopvac-receipt.json"), json.to_string()).unwrap();
    }

    #[test]
    fn receipt_with_matching_prefix_is_the_installer() {
        let tmp = tempfile::tempdir().unwrap();
        let prefix = tmp.path().join("home/.cargo");
        write_receipt(&tmp.path().join("cfg/slopvac"), &prefix, "slopvac");
        let cfg = tmp.path().join("cfg").display().to_string();
        let var = env(&[(CONFIG_HOME_VAR, cfg)]);
        // Both layouts: binary in the prefix, or in prefix/bin.
        assert_eq!(
            detect(&prefix.join("bin/slopvac"), &var),
            InstallMethod::Installer
        );
        assert_eq!(
            detect(&prefix.join("slopvac"), &var),
            InstallMethod::Installer
        );
    }

    #[test]
    fn receipt_for_another_location_does_not_claim_the_binary() {
        let tmp = tempfile::tempdir().unwrap();
        write_receipt(
            &tmp.path().join("cfg/slopvac"),
            &tmp.path().join("a"),
            "slopvac",
        );
        let cfg = tmp.path().join("cfg").display().to_string();
        let var = env(&[(CONFIG_HOME_VAR, cfg)]);
        assert_eq!(
            detect(&tmp.path().join("b/bin/slopvac"), &var),
            InstallMethod::Unknown
        );
    }

    #[test]
    fn receipt_for_another_app_is_ignored() {
        let tmp = tempfile::tempdir().unwrap();
        let prefix = tmp.path().join("p");
        write_receipt(&tmp.path().join("cfg/slopvac"), &prefix, "other");
        let cfg = tmp.path().join("cfg").display().to_string();
        let var = env(&[(CONFIG_HOME_VAR, cfg)]);
        assert_eq!(
            detect(&prefix.join("bin/slopvac"), &var),
            InstallMethod::Unknown
        );
    }

    #[test]
    fn axoupdater_config_path_overrides_the_default_receipt_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let prefix = tmp.path().join("p");
        write_receipt(&tmp.path().join("r"), &prefix, "slopvac");
        let var = env(&[(
            "AXOUPDATER_CONFIG_PATH",
            tmp.path().join("r").display().to_string(),
        )]);
        assert_eq!(
            detect(&prefix.join("bin/slopvac"), &var),
            InstallMethod::Installer
        );
    }

    #[test]
    fn corrupt_receipt_is_not_an_installer_install() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("cfg/slopvac");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("slopvac-receipt.json"), "{nope").unwrap();
        let var = env(&[(
            CONFIG_HOME_VAR,
            tmp.path().join("cfg").display().to_string(),
        )]);
        assert_eq!(
            detect(&tmp.path().join("bin/slopvac"), &var),
            InstallMethod::Unknown
        );
    }

    #[test]
    fn layouts_without_a_receipt() {
        let none = |_: &str| None;
        let cases = [
            (
                "/opt/homebrew/Cellar/slopvac/3.0.0/bin/slopvac",
                InstallMethod::Homebrew,
            ),
            (
                "/home/linuxbrew/.linuxbrew/Cellar/slopvac/3.0.0/bin/slopvac",
                InstallMethod::Homebrew,
            ),
            (
                "/home/u/.local/share/mise/installs/slopvac/3.0.0/bin/slopvac",
                InstallMethod::Mise,
            ),
            (
                "/home/u/.local/share/uv/tools/slopvac/bin/slopvac",
                InstallMethod::Wheel(WheelTool::Uv),
            ),
            (
                "/home/u/.cache/uv/archive-v0/abc/bin/slopvac",
                InstallMethod::Wheel(WheelTool::Uvx),
            ),
            (
                "/srv/app/.venv/lib/python3.13/site-packages/slopvac/slopvac",
                InstallMethod::Wheel(WheelTool::Pip),
            ),
            ("/usr/local/bin/slopvac", InstallMethod::Unknown),
        ];
        for (path, want) in cases {
            assert_eq!(detect(Path::new(path), &none), want, "{path}");
        }
    }

    #[test]
    fn venv_with_pyvenv_cfg_is_a_pip_install() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("pyvenv.cfg"), "home = /usr/bin\n").unwrap();
        assert_eq!(
            detect(&tmp.path().join("bin/slopvac"), &|_: &str| None),
            InstallMethod::Wheel(WheelTool::Pip)
        );
    }

    #[test]
    fn pip_user_and_system_installs_are_found_through_dist_info() {
        // The scripts directory has no venv and no `site-packages` component, so only the wheel
        // metadata ties the binary to pip. `.local` is a user install, `usr/local` a system one.
        for (prefix, installer, uv_receipt, want) in [
            (".local", "pip\n", false, WheelTool::Pip),
            ("usr/local", "pip\n", false, WheelTool::Pip),
            // `uv pip install` into an ordinary venv: uv installed it, but it is not a uv tool.
            (".venv", "uv\n", false, WheelTool::UvPip),
            // A uv tool environment carries uv's receipt at its root.
            ("tool-env", "uv\n", true, WheelTool::Uv),
        ] {
            let tmp = tempfile::tempdir().unwrap();
            let root = tmp.path().join(prefix);
            let info = root.join("lib/python3.13/site-packages/slopvac-3.0.0.dist-info");
            std::fs::create_dir_all(&info).unwrap();
            std::fs::write(info.join("INSTALLER"), installer).unwrap();
            std::fs::write(
                info.join("RECORD"),
                "slopvac-3.0.0.dist-info/METADATA,sha256=x,10\n../../../bin/slopvac,sha256=y,99\n",
            )
            .unwrap();
            std::fs::create_dir_all(root.join("bin")).unwrap();
            if uv_receipt {
                std::fs::write(root.join("uv-receipt.toml"), "[tool]\n").unwrap();
            }
            let exe = root.join("bin/slopvac");
            assert_eq!(
                detect(&exe, &|_: &str| None),
                InstallMethod::Wheel(want),
                "{prefix} {installer:?} {uv_receipt}"
            );
        }
    }

    /// A pip-style install under `root` whose RECORD lists the binary, with `record` and
    /// `installer` as the file contents (`None` leaves the name for the test to create).
    fn pip_layout(root: &Path, record: Option<&str>, installer: Option<&str>) -> PathBuf {
        let info = root.join("lib/python3.13/site-packages/slopvac-3.0.0.dist-info");
        std::fs::create_dir_all(&info).unwrap();
        if let Some(r) = record {
            std::fs::write(info.join("RECORD"), r).unwrap();
        }
        if let Some(i) = installer {
            std::fs::write(info.join("INSTALLER"), i).unwrap();
        }
        std::fs::create_dir_all(root.join("bin")).unwrap();
        root.join("bin/slopvac")
    }

    const LISTS_EXE: &str = "../../../bin/slopvac,sha256=y,99\n";

    #[test]
    fn an_oversize_record_is_not_read_so_the_install_stays_unknown() {
        let tmp = tempfile::tempdir().unwrap();
        let padding = "x,y,1\n".repeat(400_000);
        let exe = pip_layout(
            &tmp.path().join(".local"),
            Some(&format!("{padding}{LISTS_EXE}")),
            Some("pip\n"),
        );
        assert_eq!(detect(&exe, &|_: &str| None), InstallMethod::Unknown);
    }

    #[test]
    fn an_oversize_installer_file_is_not_read() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = pip_layout(
            &tmp.path().join(".venv"),
            Some(LISTS_EXE),
            Some(&format!("uv{}\n", " ".repeat(20_000))),
        );
        // Unread, INSTALLER is empty text, which is not `uv`, so the wheel is a pip install.
        assert_eq!(
            detect(&exe, &|_: &str| None),
            InstallMethod::Wheel(WheelTool::Pip)
        );
    }

    #[cfg(unix)]
    #[test]
    fn fifo_metadata_files_do_not_block_detection() {
        let mkfifo = |path: &Path| {
            assert!(
                std::process::Command::new("mkfifo")
                    .arg(path)
                    .status()
                    .unwrap()
                    .success()
            );
        };
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join(".local");
        let exe = pip_layout(&root, None, None);
        let info = root.join("lib/python3.13/site-packages/slopvac-3.0.0.dist-info");
        mkfifo(&info.join("RECORD"));
        mkfifo(&info.join("INSTALLER"));
        assert_eq!(detect(&exe, &|_: &str| None), InstallMethod::Unknown);

        let receipts = tmp.path().join("receipts");
        std::fs::create_dir_all(&receipts).unwrap();
        mkfifo(&receipts.join("slopvac-receipt.json"));
        let dir = receipts.display().to_string();
        let var = move |k: &str| (k == "AXOUPDATER_CONFIG_PATH").then(|| dir.clone());
        assert_eq!(
            detect(&root.join("bin/slopvac"), &var),
            InstallMethod::Unknown
        );
    }

    #[derive(Clone, Copy, Debug)]
    enum Special {
        Fifo,
        Directory,
        Oversized,
    }

    #[derive(Clone, Copy, Debug)]
    enum Target {
        Record,
        Installer,
    }

    /// Detect on a thread. A hang fails the test; a FIFO is opened for writing so the stuck
    /// thread can end.
    #[cfg(unix)]
    fn detect_within_timeout(exe: PathBuf, fifo: &Path) -> InstallMethod {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(detect(&exe, &|_: &str| None));
        });
        if let Ok(method) = rx.recv_timeout(std::time::Duration::from_secs(5)) {
            return method;
        }
        let _ = std::fs::OpenOptions::new().write(true).open(fifo);
        panic!("detection hung on a metadata file");
    }

    // One metadata file is special, the other is valid. A special `RECORD` hides the binary, so
    // the install is unknown. A special `INSTALLER` reads as absent, so the wheel is a pip
    // install: the oversized `INSTALLER` says `uv` once trimmed, which an unbounded read would
    // take at its word.
    #[cfg(unix)]
    #[rstest::rstest]
    #[case::record_fifo(Target::Record, Special::Fifo, InstallMethod::Unknown)]
    #[case::record_directory(Target::Record, Special::Directory, InstallMethod::Unknown)]
    #[case::record_oversized(Target::Record, Special::Oversized, InstallMethod::Unknown)]
    #[case::installer_fifo(Target::Installer, Special::Fifo, InstallMethod::Wheel(WheelTool::Pip))]
    #[case::installer_directory(
        Target::Installer,
        Special::Directory,
        InstallMethod::Wheel(WheelTool::Pip)
    )]
    #[case::installer_oversized(
        Target::Installer,
        Special::Oversized,
        InstallMethod::Wheel(WheelTool::Pip)
    )]
    fn a_special_metadata_file_is_refused_without_hanging(
        #[case] target: Target,
        #[case] special: Special,
        #[case] want: InstallMethod,
    ) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join(".local");
        let record = "../../../bin/slopvac,sha256=y,99\n";
        let (exe, bad) = match target {
            Target::Record => (pip_layout(&root, None, Some("pip\n")), "RECORD"),
            Target::Installer => (pip_layout(&root, Some(record), None), "INSTALLER"),
        };
        let path = root
            .join("lib/python3.13/site-packages/slopvac-3.0.0.dist-info")
            .join(bad);
        match special {
            Special::Fifo => assert!(
                std::process::Command::new("mkfifo")
                    .arg(&path)
                    .status()
                    .unwrap()
                    .success()
            ),
            Special::Directory => std::fs::create_dir(&path).unwrap(),
            Special::Oversized => {
                let body = match target {
                    Target::Record => format!("{record}{}", "x".repeat(2 << 20)),
                    Target::Installer => format!("uv{}", " ".repeat(8 * 1024)),
                };
                std::fs::write(&path, body).unwrap();
            }
        }
        assert_eq!(detect_within_timeout(exe, &path), want);
    }

    #[test]
    fn dist_info_that_does_not_list_the_binary_does_not_claim_it() {
        let tmp = tempfile::tempdir().unwrap();
        let info = tmp
            .path()
            .join("lib/python3.13/site-packages/slopvac-3.0.0.dist-info");
        std::fs::create_dir_all(&info).unwrap();
        std::fs::write(info.join("INSTALLER"), "pip\n").unwrap();
        std::fs::write(info.join("RECORD"), "../../../bin/other,sha256=y,99\n").unwrap();
        assert_eq!(
            detect(&tmp.path().join("bin/slopvac"), &|_: &str| None),
            InstallMethod::Unknown
        );
    }

    /// `\\?\` in front of a path, as `canonicalize` writes it on Windows.
    fn verbatim(path: &Path) -> PathBuf {
        PathBuf::from(format!(r"\\?\{}", path.display()))
    }

    #[test]
    fn a_verbatim_path_on_either_side_of_a_receipt_comparison_still_matches() {
        let tmp = tempfile::tempdir().unwrap();
        let prefix = tmp.path().join("home/.cargo");
        let exe = prefix.join("bin/slopvac");
        let cases = [
            ("verbatim receipt prefix", verbatim(&prefix), exe.clone()),
            ("verbatim exe", prefix.clone(), verbatim(&exe)),
            ("both verbatim", verbatim(&prefix), verbatim(&exe)),
        ];
        for (label, receipt_prefix, exe) in cases {
            let dir = tempfile::tempdir().unwrap();
            write_receipt(dir.path(), &receipt_prefix, "slopvac");
            let var = env(&[("AXOUPDATER_CONFIG_PATH", dir.path().display().to_string())]);
            assert_eq!(detect(&exe, &var), InstallMethod::Installer, "{label}");
        }
    }

    #[test]
    fn a_verbatim_cargo_home_still_matches() {
        let home = Path::new("/home/u/.cargo");
        let var = env(&[("CARGO_HOME", verbatim(home).display().to_string())]);
        assert_eq!(
            detect(&home.join("bin/slopvac"), &var),
            InstallMethod::Cargo
        );
    }

    #[test]
    fn cargo_bin_without_a_receipt_is_cargo() {
        let var = env(&[("CARGO_HOME", "/home/u/.cargo".into())]);
        assert_eq!(
            detect(Path::new("/home/u/.cargo/bin/slopvac"), &var),
            InstallMethod::Cargo
        );
        let var = env(&[("HOME", "/home/u".into())]);
        assert_eq!(
            detect(Path::new("/home/u/.cargo/bin/slopvac"), &var),
            InstallMethod::Cargo
        );
    }

    #[test]
    fn verbatim_prefix_is_stripped_but_unc_is_kept() {
        for (input, want) in [
            (
                r"\\?\C:\Users\u\.cargo\bin\slopvac.exe",
                r"C:\Users\u\.cargo\bin\slopvac.exe",
            ),
            (
                r"\\?\UNC\server\share\slopvac.exe",
                r"\\?\UNC\server\share\slopvac.exe",
            ),
            (r"C:\plain\slopvac.exe", r"C:\plain\slopvac.exe"),
            ("/usr/bin/slopvac", "/usr/bin/slopvac"),
        ] {
            assert_eq!(
                strip_verbatim_prefix(Path::new(input)),
                Path::new(want),
                "{input}"
            );
        }
    }

    #[test]
    fn upgrade_commands_per_method() {
        let cases = [
            (InstallMethod::Installer, Some("slopvac self update")),
            (
                InstallMethod::Wheel(WheelTool::Uv),
                Some("uv tool upgrade slopvac"),
            ),
            (
                InstallMethod::Wheel(WheelTool::Uvx),
                Some("uvx slopvac@latest"),
            ),
            (
                InstallMethod::Wheel(WheelTool::Pip),
                Some("python -m pip install --upgrade slopvac"),
            ),
            (
                InstallMethod::Wheel(WheelTool::UvPip),
                Some("uv pip install --upgrade slopvac"),
            ),
            (
                InstallMethod::Homebrew,
                Some("brew upgrade slopvac/tap/slopvac"),
            ),
            (InstallMethod::Mise, Some("mise upgrade slopvac")),
            (InstallMethod::Cargo, None),
            (InstallMethod::Unknown, None),
        ];
        for (method, want) in cases {
            assert_eq!(method.upgrade_command(), want, "{}", method.id());
        }
    }
}
