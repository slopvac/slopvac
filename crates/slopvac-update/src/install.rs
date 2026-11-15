//! Which channel installed this binary, and the command that upgrades it.

use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

/// The Python tool that put a wheel install in place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WheelTool {
    /// `uv tool install slopvac`.
    Uv,
    /// `uvx slopvac`, an ephemeral environment in uv's cache.
    Uvx,
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
            Self::Wheel(WheelTool::Pip) => "pip",
            Self::Homebrew => "homebrew",
            Self::Mise => "mise",
            Self::Cargo => "cargo",
            Self::Unknown => "unknown",
        }
    }

    /// The command that upgrades this install, or `None` when the channel is unknown.
    #[must_use]
    pub fn upgrade_command(self) -> Option<&'static str> {
        Some(match self {
            Self::Installer => "slopvac self update",
            Self::Wheel(WheelTool::Uv) => "uv tool upgrade slopvac",
            Self::Wheel(WheelTool::Uvx) => "uvx slopvac@latest",
            Self::Wheel(WheelTool::Pip) => "python -m pip install --upgrade slopvac",
            Self::Homebrew => "brew upgrade srobroek/tap/slopvac",
            Self::Mise => "mise upgrade slopvac",
            Self::Cargo => "cargo install --locked --force slopvac",
            Self::Unknown => return None,
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
    let Ok(text) = std::fs::read_to_string(dir.join("slopvac-receipt.json")) else {
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
    parent == receipt.install_prefix || parent == receipt.install_prefix.join("bin")
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
        .map(|d| d.join("bin"))
}

/// Detect the install method of the binary at `exe`.
///
/// Pass the resolved path of the running executable. `var` reads the environment. The checks run
/// from the most specific evidence to the least: the installer receipt, then the Homebrew and mise
/// directory layouts, then wheel environments, then the cargo bin directory.
#[must_use]
pub fn detect(exe: &Path, var: &dyn Fn(&str) -> Option<String>) -> InstallMethod {
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

    fn write_receipt(dir: &Path, prefix: &Path, app: &str) {
        std::fs::create_dir_all(dir).unwrap();
        let json = serde_json::json!({
            "binaries": ["slopvac"],
            "install_layout": "flat",
            "install_prefix": prefix,
            "provider": {"source": "cargo-dist", "version": "0.34.0"},
            "source": {"app_name": app, "name": app, "owner": "srobroek", "release_type": "github"},
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
        let var = env(&[("XDG_CONFIG_HOME", cfg)]);
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
        let var = env(&[("XDG_CONFIG_HOME", cfg)]);
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
        let var = env(&[("XDG_CONFIG_HOME", cfg)]);
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
            "XDG_CONFIG_HOME",
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
                InstallMethod::Homebrew,
                Some("brew upgrade srobroek/tap/slopvac"),
            ),
            (InstallMethod::Mise, Some("mise upgrade slopvac")),
            (
                InstallMethod::Cargo,
                Some("cargo install --locked --force slopvac"),
            ),
            (InstallMethod::Unknown, None),
        ];
        for (method, want) in cases {
            assert_eq!(method.upgrade_command(), want, "{}", method.id());
        }
    }
}
