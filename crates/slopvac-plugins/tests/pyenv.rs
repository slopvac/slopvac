//! Managed Python environment tests (threat model Y1 to Y7, TM-49 to TM-53).
//!
//! Lock checks run on every platform and start no process. The `unix_only`
//! tests build environments with a scripted fake uv, so they run wherever a
//! `python3` is on `PATH`, and start plugins through the host and the trust
//! gate. The `live` tests use the real uv. They skip when no uv is found,
//! fail when one is found but cannot build, and fail instead of skipping when
//! `SLOPVAC_PYENV_LIVE` is set.

mod common;

use std::collections::HashSet;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use flate2::Compression;
use flate2::write::GzEncoder;
use sha2::{Digest as _, Sha256};
use slopvac_plugins::extract::{Limits, evict, extract_targz};
use slopvac_plugins::oci::Digest;
use slopvac_plugins::pyenv::{PyEnvError, PyLock};
use slopvac_plugins::trust::VerifiedPlugin;

const PLUGIN_PY: &str = include_str!("fixtures/fake_plugin.py");

struct Scratch(PathBuf, bool);

impl Scratch {
    fn new(name: &str) -> Self {
        let p = std::env::temp_dir().join(format!("slopvac-pyenv-{name}-{}", std::process::id()));
        let _ = evict(&p);
        fs::create_dir_all(&p).unwrap();
        Self(p, true)
    }
    /// A directory that another process owns and removes.
    #[cfg(unix)]
    fn borrowed(p: PathBuf) -> Self {
        fs::create_dir_all(&p).unwrap();
        Self(p, false)
    }
    fn dir(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if self.1 {
            let _ = evict(&self.0);
        }
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .fold(String::new(), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// A wheel for a lock: package name, version, path in the tree, file bytes.
struct Wheel<'a> {
    name: &'a str,
    version: &'a str,
    path: String,
    bytes: Vec<u8>,
}

impl<'a> Wheel<'a> {
    fn new(name: &'a str, version: &'a str, bytes: Vec<u8>) -> Self {
        Self {
            name,
            version,
            path: format!(
                "wheels/{}-{version}-py3-none-any.whl",
                name.replace('-', "_")
            ),
            bytes,
        }
    }
}

/// A stored (uncompressed) ZIP archive. `declared` overrides the
/// uncompressed size the central directory states for every entry.
fn zip_with(files: &[(&str, &[u8])], declared: Option<u32>) -> Vec<u8> {
    let (mut out, mut cd) = (Vec::new(), Vec::new());
    for (name, data) in files {
        let mut crc = flate2::Crc::new();
        crc.update(data);
        let len = u32::try_from(data.len()).unwrap();
        let name_len = u16::try_from(name.len()).unwrap();
        let offset = u32::try_from(out.len()).unwrap();
        // Version 20, no flags, stored, 1980-01-01 00:00.
        let common = |v: &mut Vec<u8>, size: u32| {
            for x in [20u16, 0, 0, 0, 0x21] {
                v.extend(x.to_le_bytes());
            }
            v.extend(crc.sum().to_le_bytes());
            v.extend(len.to_le_bytes());
            v.extend(size.to_le_bytes());
            v.extend(name_len.to_le_bytes());
            v.extend(0u16.to_le_bytes());
        };
        out.extend(0x0403_4b50u32.to_le_bytes());
        common(&mut out, len);
        out.extend(name.as_bytes());
        out.extend(*data);
        cd.extend(0x0201_4b50u32.to_le_bytes());
        cd.extend(20u16.to_le_bytes());
        common(&mut cd, declared.unwrap_or(len));
        for x in [0u16, 0, 0] {
            cd.extend(x.to_le_bytes());
        }
        cd.extend(0u32.to_le_bytes());
        cd.extend(offset.to_le_bytes());
        cd.extend(name.as_bytes());
    }
    let n = u16::try_from(files.len()).unwrap();
    let cd_offset = u32::try_from(out.len()).unwrap();
    let cd_len = u32::try_from(cd.len()).unwrap();
    out.extend(cd);
    out.extend(0x0605_4b50u32.to_le_bytes());
    for x in [0u16, 0, n, n] {
        out.extend(x.to_le_bytes());
    }
    out.extend(cd_len.to_le_bytes());
    out.extend(cd_offset.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    out
}

/// A pure-Python wheel of `name` with a module whose `VALUE` is `value`,
/// plus `extra` files in its root.
#[cfg(unix)]
fn wheel<'a>(name: &'a str, version: &'a str, value: &str, extra: &[(&str, &str)]) -> Wheel<'a> {
    use base64::Engine as _;
    let module = name.replace('-', "_");
    let info = format!("{module}-{version}.dist-info");
    let mut files: Vec<(String, String)> = vec![
        (
            format!("{module}/__init__.py"),
            format!("VALUE = {value:?}\n"),
        ),
        (
            format!("{info}/METADATA"),
            format!("Metadata-Version: 2.1\nName: {name}\nVersion: {version}\n"),
        ),
        (
            format!("{info}/WHEEL"),
            "Wheel-Version: 1.0\nGenerator: test\nRoot-Is-Purelib: true\nTag: py3-none-any\n"
                .into(),
        ),
    ];
    files.extend(
        extra
            .iter()
            .map(|(n, t)| ((*n).to_owned(), (*t).to_owned())),
    );
    let mut record = String::new();
    for (n, t) in &files {
        let digest = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(t));
        let _ = writeln!(record, "{n},sha256={digest},{}", t.len());
    }
    let _ = writeln!(record, "{info}/RECORD,,");
    files.push((format!("{info}/RECORD"), record));
    let refs: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(n, t)| (n.as_str(), t.as_bytes()))
        .collect();
    Wheel::new(name, version, zip_with(&refs, None))
}

fn lock_text(wheels: &[Wheel<'_>]) -> String {
    let mut out = String::from("lock-version = \"1.0\"\ncreated-by = \"test\"\n");
    for w in wheels {
        let _ = write!(
            out,
            "\n[[packages]]\nname = \"{}\"\nversion = \"{}\"\nwheels = [{{ path = \"{}\", hashes = {{ sha256 = \"{}\" }} }}]\n",
            w.name,
            w.version,
            w.path,
            sha256_hex(&w.bytes)
        );
    }
    out
}

fn targz(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut tar = tar::Builder::new(Vec::new());
    for (name, data) in files {
        let mut h = tar::Header::new_gnu();
        h.set_size(data.len() as u64);
        h.set_mode(0o644);
        h.set_cksum();
        tar.append_data(&mut h, name, *data).unwrap();
    }
    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
    std::io::Write::write_all(&mut enc, &tar.into_inner().unwrap()).unwrap();
    enc.finish().unwrap()
}

/// Extract a Python plugin tree with `lock` and `wheels`, as sync would.
fn install(s: &Scratch, lock: &str, wheels: &[Wheel<'_>]) -> VerifiedPlugin {
    let mut files: Vec<(&str, &[u8])> = vec![
        ("plugin.py", PLUGIN_PY.as_bytes()),
        ("pylock.toml", lock.as_bytes()),
    ];
    for w in wheels {
        files.push((w.path.as_str(), w.bytes.as_slice()));
    }
    let out = extract_targz(
        targz(&files).as_slice(),
        &s.dir("cache"),
        &HashSet::new(),
        Limits::default(),
    )
    .unwrap();
    VerifiedPlugin {
        name: "fake".into(),
        version: Some("1.0.0".into()),
        source: "oci:ghcr.io/acme/fake".into(),
        root: out.root,
        tree: out.hash,
        manifest_digest: Some(Digest::of(b"fake").to_string()),
        key_fingerprint: Some("f".repeat(64)),
        key_id: None,
    }
}

const HASH: &str = "7036a9f7c5dad75b4e53b35cff5ed40a3eb1844f77ff6c19ed29f339cf113af9";

fn one_package(extra: &str) -> String {
    format!(
        "lock-version = \"1.0\"\n\n[[packages]]\nname = \"demo-pkg\"\nversion = \"1.0\"\n{extra}\n"
    )
}

#[test]
fn tm49_a_package_that_is_not_a_vendored_wheel_is_refused() {
    let wheel = format!(
        "wheels = [{{ path = \"wheels/demo_pkg-1.0-py3-none-any.whl\", hashes = {{ sha256 = \"{HASH}\" }} }}]"
    );
    let cases = [
        (
            "sdist".to_owned(),
            format!(
                "sdist = {{ path = \"demo-pkg-1.0.tar.gz\", hashes = {{ sha256 = \"{HASH}\" }} }}"
            ),
        ),
        // An sdist next to the wheels is refused too: no build backend may be reachable.
        (
            "sdist".to_owned(),
            format!(
                "{wheel}\nsdist = {{ path = \"demo-pkg-1.0.tar.gz\", hashes = {{ sha256 = \"{HASH}\" }} }}"
            ),
        ),
        (
            "vcs".to_owned(),
            "vcs = { type = \"git\", url = \"https://example.invalid/r\", commit-id = \"0\" }"
                .into(),
        ),
        (
            "directory".to_owned(),
            "directory = { path = \"src\" }".into(),
        ),
        (
            "archive".to_owned(),
            format!(
                "archive = {{ url = \"https://example.invalid/a.zip\", hashes = {{ sha256 = \"{HASH}\" }} }}"
            ),
        ),
        ("no wheels".to_owned(), String::new()),
        ("no wheels".to_owned(), "wheels = []".into()),
    ];
    for (kind, body) in cases {
        let err = PyLock::parse(&one_package(&body)).unwrap_err();
        match err {
            PyEnvError::SourceOnly {
                package,
                source_kind,
            } => {
                assert_eq!(package, "demo-pkg");
                assert_eq!(source_kind, kind, "{body}");
            }
            other => panic!("{body}: {other:?}"),
        }
    }
}

#[test]
fn a_wheel_that_is_not_a_file_in_the_tree_is_refused() {
    let cases = [
        // Only a URL: the build would have to download it.
        format!(
            "wheels = [{{ url = \"https://example.invalid/demo_pkg-1.0-py3-none-any.whl\", hashes = {{ sha256 = \"{HASH}\" }} }}]"
        ),
        format!(
            "wheels = [{{ path = \"../demo_pkg-1.0-py3-none-any.whl\", hashes = {{ sha256 = \"{HASH}\" }} }}]"
        ),
        format!(
            "wheels = [{{ path = \"/tmp/demo_pkg-1.0-py3-none-any.whl\", hashes = {{ sha256 = \"{HASH}\" }} }}]"
        ),
        format!(
            "wheels = [{{ path = \"wheels\\\\demo_pkg-1.0-py3-none-any.whl\", hashes = {{ sha256 = \"{HASH}\" }} }}]"
        ),
        format!(
            "wheels = [{{ path = \"wheels/demo-pkg-1.0.tar.gz\", hashes = {{ sha256 = \"{HASH}\" }} }}]"
        ),
        // A wheel of another package or version.
        format!(
            "wheels = [{{ path = \"wheels/other_pkg-1.0-py3-none-any.whl\", hashes = {{ sha256 = \"{HASH}\" }} }}]"
        ),
        format!(
            "wheels = [{{ path = \"wheels/demo_pkg-2.0-py3-none-any.whl\", hashes = {{ sha256 = \"{HASH}\" }} }}]"
        ),
        // No usable hash.
        "wheels = [{ path = \"wheels/demo_pkg-1.0-py3-none-any.whl\", hashes = { md5 = \"0\" } }]"
            .to_owned(),
        "wheels = [{ path = \"wheels/demo_pkg-1.0-py3-none-any.whl\", hashes = { sha256 = \"ABC\" } }]"
            .to_owned(),
    ];
    for body in cases {
        let err = PyLock::parse(&one_package(&body)).unwrap_err();
        assert!(
            matches!(&err, PyEnvError::Wheel { package, .. } if package == "demo-pkg"),
            "{body}: {err:?}"
        );
    }
}

/// A lock over one wheel of `demo-pkg` 1.0 holding the entries `names`,
/// installed as a plugin tree in `s`.
fn lock_over_entries(s: &Scratch, names: &[&str]) -> (PyLock, VerifiedPlugin) {
    let files: Vec<(&str, &[u8])> = names.iter().map(|n| (*n, b"x".as_slice())).collect();
    let w = Wheel::new("demo-pkg", "1.0", zip_with(&files, None));
    let text = lock_text(std::slice::from_ref(&w));
    let vp = install(s, &text, std::slice::from_ref(&w));
    (PyLock::parse(&text).unwrap(), vp)
}

#[test]
fn tm49_tm53_a_wheel_entry_that_lands_on_the_interpreter_or_a_startup_hook_is_refused() {
    let refused = [
        "demo_pkg-1.0.data/scripts/python",
        "demo_pkg-1.0.data/scripts/python3",
        "demo_pkg-1.0.data/scripts/python3.12",
        "demo_pkg-1.0.data/scripts/python.exe",
        "demo_pkg-1.0.data/scripts/Python",
        "demo_pkg-1.0.data/scripts/activate",
        "demo_pkg-1.0.data/scripts/deactivate.bat",
        "demo_pkg-1.0.data/scripts/pyvenv.cfg",
        "demo_pkg-1.0.data/data/pyvenv.cfg",
        "demo_pkg-1.0.data/data/bin/python",
        "demo_pkg-1.0.data/data/BIN/python",
        "demo_pkg-1.0.data/data/Scripts/python.exe",
        "demo_pkg-1.0.data/data/lib/python3.12/site-packages/x.pth",
        "demo_pkg-1.0.data/data/lib/python3.12/encodings/__init__.py",
        "evil.pth",
        "demo_pkg/nested.PTH",
        "demo_pkg-1.0.data/purelib/evil.pth",
        "demo_pkg-1.0.data/platlib/evil.pth",
        "sitecustomize.py",
        "usercustomize/__init__.py",
        "demo_pkg-1.0.data/platlib/sitecustomize.cpython-312-x86_64-linux-gnu.so",
        "demo_pkg-1.0.data/unknown/x",
        "../escape.py",
        "demo_pkg/./x.py",
        "/abs.py",
        "demo_pkg\\x.py",
    ];
    for (i, name) in refused.iter().enumerate() {
        let s = Scratch::new(&format!("entry-{i}"));
        let (lock, vp) = lock_over_entries(&s, &["demo_pkg/__init__.py", name]);
        let err = lock.verify_wheels(&vp.root).unwrap_err();
        assert!(
            matches!(&err, PyEnvError::Wheel { why, .. } if why.contains(&name.escape_debug().to_string())),
            "{name}: {err:?}"
        );
    }
}

#[test]
fn ordinary_wheel_entries_are_accepted() {
    let s = Scratch::new("entries-ok");
    let (lock, vp) = lock_over_entries(
        &s,
        &[
            "demo_pkg/__init__.py",
            "demo_pkg/sitecustomize.py",
            "demo_pkg/notes.pth.txt",
            "demo_pkg-1.0.dist-info/METADATA",
            "demo_pkg-1.0.data/scripts/demo-cli",
            "demo_pkg-1.0.data/data/share/demo/x.json",
            "demo_pkg-1.0.data/headers/demo.h",
            "demo_pkg-1.0.data/purelib/demo_extra/__init__.py",
        ],
    );
    lock.verify_wheels(&vp.root).unwrap();
}

#[test]
fn a_marker_with_a_hyphenated_quoted_value_is_accepted() {
    let wheel = format!(
        "wheels = [{{ path = \"wheels/demo_pkg-1.0-py3-none-any.whl\", hashes = {{ sha256 = \"{HASH}\" }} }}]"
    );
    for marker in [
        "platform_release == '5.15.0-91-generic'",
        "platform_version == \\\"#1 SMP\\\" or python_version >= '3.11'",
        "sys_platform == 'linux' and (platform_machine == 'x86_64' or platform_machine == 'aarch64')",
    ] {
        let text = one_package(&format!("marker = \"{marker}\"\n{wheel}"));
        let parsed = PyLock::parse(&text);
        if marker.contains('#') {
            // `#` would start a comment in a requirements file, even in quotes.
            assert!(parsed.is_err(), "{marker}");
            continue;
        }
        let lock = parsed.unwrap_or_else(|e| panic!("{marker}: {e}"));
        assert_eq!(
            lock.requirements(),
            format!("demo-pkg==1.0 ; {marker} --hash=sha256:{HASH}\n")
        );
    }
}

#[cfg(windows)]
#[test]
fn managed_python_environments_are_refused_on_windows() {
    use slopvac_plugins::pyenv::{EnvStore, InstallNetwork, Installer};
    let s = Scratch::new("windows");
    let w = Wheel::new(
        "demo-pkg",
        "1.0",
        zip_with(&[("demo_pkg/__init__.py", b"")], None),
    );
    let vp = install(
        &s,
        &lock_text(std::slice::from_ref(&w)),
        std::slice::from_ref(&w),
    );
    let store = EnvStore::open(s.dir("envs"));
    let installer = Installer {
        uv: PathBuf::from(r"C:\\nonexistent\\uv.exe"),
        python_install_dir: PathBuf::from(r"C:\\nonexistent\\python"),
        python: "3.12".into(),
        network: InstallNetwork::Offline,
    };
    let err = store.build(&installer, &vp.root, &vp.tree).unwrap_err();
    assert!(matches!(err, PyEnvError::UnsupportedPlatform), "{err:?}");
    let err = store.load(&vp.tree, "3.12").unwrap_err();
    assert!(matches!(err, PyEnvError::UnsupportedPlatform), "{err:?}");
    assert!(!s.dir("envs").exists(), "nothing was created");
}

#[test]
fn a_lock_field_that_could_inject_an_installer_option_is_refused() {
    let wheel = format!(
        "wheels = [{{ path = \"wheels/demo_pkg-1.0-py3-none-any.whl\", hashes = {{ sha256 = \"{HASH}\" }} }}]"
    );
    let cases = [
        one_package(&format!(
            "marker = \"python_version >= '3' --index-url=http://x\"\n{wheel}"
        )),
        one_package(&format!("marker = \"python_version >= '3' # c\"\n{wheel}")),
        one_package(&format!("marker = \"x\\n--index-url http://x\"\n{wheel}")),
        one_package(&format!("marker = \"os_name == 'a' -r x\"\n{wheel}")),
        one_package(&format!("marker = \"os_name == 'a -r x'\"\n{wheel}")),
        one_package(&format!("marker = \"os_name == 'a\"\n{wheel}")),
        one_package(&format!("marker = \"os_name == 'a\\\\'\"\n{wheel}")),
        one_package(&format!("marker = \"os_name == 'a'; x\"\n{wheel}")),
        format!(
            "lock-version = \"1.0\"\n\n[[packages]]\nname = \"demo-pkg --index-url=http://x\"\nversion = \"1.0\"\n{wheel}\n"
        ),
        format!(
            "lock-version = \"1.0\"\n\n[[packages]]\nname = \"demo-pkg\"\nversion = \">=1.0\"\n{wheel}\n"
        ),
    ];
    for text in cases {
        assert!(PyLock::parse(&text).is_err(), "{text}");
    }
}

#[test]
fn a_lock_of_another_major_version_is_refused() {
    for v in ["2.0", "0.9"] {
        let err = PyLock::parse(&format!("lock-version = \"{v}\"\n")).unwrap_err();
        assert!(matches!(err, PyEnvError::Lock(_)), "{v}: {err:?}");
    }
}

#[test]
fn the_requirements_pin_each_exact_version_and_every_locked_hash() {
    let a = "a".repeat(64);
    let b = "b".repeat(64);
    let c = "c".repeat(64);
    let text = format!(
        "lock-version = \"1.0\"\n\
         [[packages]]\nname = \"demo-pkg\"\nversion = \"1.0\"\nmarker = \"python_version >= '3.11'\"\n\
         wheels = [{{ path = \"wheels/demo_pkg-1.0-py3-none-any.whl\", hashes = {{ sha256 = \"{a}\" }} }},\n\
                   {{ path = \"wheels/demo_pkg-1.0-cp312-cp312-manylinux_2_17_x86_64.whl\", hashes = {{ sha256 = \"{b}\" }} }}]\n\
         [[packages]]\nname = \"Other.Pkg\"\nversion = \"2.1\"\n\
         wheels = [{{ path = \"vendor/other_pkg-2.1-py3-none-any.whl\", hashes = {{ sha256 = \"{c}\" }} }}]\n"
    );
    let lock = PyLock::parse(&text).unwrap();
    assert_eq!(
        lock.requirements(),
        format!(
            "demo-pkg==1.0 ; python_version >= '3.11' --hash=sha256:{a} --hash=sha256:{b}\n\
             Other.Pkg==2.1 --hash=sha256:{c}\n"
        )
    );
    let root = Path::new("/plugin");
    assert_eq!(
        lock.wheel_dirs(root),
        [root.join("vendor"), root.join("wheels")]
    );
}

#[test]
fn tm50_a_swapped_wheel_is_refused_before_the_installer_runs() {
    let s = Scratch::new("swap");
    let w = Wheel::new("demo-pkg", "1.0", b"original wheel".to_vec());
    let lock = lock_text(std::slice::from_ref(&w));
    let swapped = Wheel {
        bytes: b"another wheel".to_vec(),
        ..Wheel::new("demo-pkg", "1.0", Vec::new())
    };
    let vp = install(&s, &lock, &[swapped]);
    let parsed = PyLock::parse(&lock).unwrap();
    let err = parsed.verify_wheels(&vp.root).unwrap_err();
    assert!(
        matches!(&err, PyEnvError::Wheel { why, .. } if why.contains(&sha256_hex(b"another wheel"))),
        "{err:?}"
    );
    let s2 = Scratch::new("swap-missing");
    let missing = install(&s2, &lock, &[]);
    assert!(missing.root.join("pylock.toml").is_file());
    let err = parsed.verify_wheels(&missing.root).unwrap_err();
    assert!(matches!(err, PyEnvError::Wheel { .. }), "{err:?}");
}

#[test]
fn wheels_that_would_expand_past_the_environment_bounds_are_refused_before_the_installer_runs() {
    type Expect = fn(&PyEnvError) -> bool;
    let bomb = zip_with(&[("a.py", b"x"), ("b.py", b"y")], Some(600 << 20));
    let not_zip = b"not a zip archive".to_vec();
    let mut zip64 = zip_with(&[("a.py", b"x")], None);
    // Mark the central directory offset as stored in a ZIP64 record.
    let at = zip64.len() - 6;
    zip64[at..at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    let cases: [(&str, Vec<u8>, Expect); 3] = [
        ("bomb", bomb, |e| matches!(e, PyEnvError::Refused(_))),
        ("not a zip", not_zip, |e| {
            matches!(e, PyEnvError::Wheel { .. })
        }),
        (
            "zip64",
            zip64,
            |e| matches!(e, PyEnvError::Wheel { why, .. } if why.contains("ZIP64")),
        ),
    ];
    for (what, bytes, expected) in cases {
        let s = Scratch::new(&format!("bounds-{}", what.replace(' ', "-")));
        let w = Wheel::new("demo-pkg", "1.0", bytes);
        let lock = lock_text(std::slice::from_ref(&w));
        let vp = install(&s, &lock, std::slice::from_ref(&w));
        let err = PyLock::parse(&lock)
            .unwrap()
            .verify_wheels(&vp.root)
            .unwrap_err();
        assert!(expected(&err), "{what}: {err:?}");
    }
    // One 600 MiB wheel fits; two packages of 600 MiB each do not.
    let s = Scratch::new("bounds-sum");
    let one = Wheel::new(
        "demo-pkg",
        "1.0",
        zip_with(&[("a.py", b"x")], Some(600 << 20)),
    );
    let two = Wheel::new(
        "other-pkg",
        "1.0",
        zip_with(&[("b.py", b"y")], Some(600 << 20)),
    );
    let lock = lock_text(std::slice::from_ref(&one));
    let vp = install(&s, &lock, std::slice::from_ref(&one));
    PyLock::parse(&lock)
        .unwrap()
        .verify_wheels(&vp.root)
        .unwrap();
    let both = [one, two];
    let lock = lock_text(&both);
    let s2 = Scratch::new("bounds-sum2");
    let vp = install(&s2, &lock, &both);
    let err = PyLock::parse(&lock)
        .unwrap()
        .verify_wheels(&vp.root)
        .unwrap_err();
    assert!(matches!(err, PyEnvError::Refused(_)), "{err:?}");
}

/// Environments built with a scripted fake uv. The fake lays out a venv
/// whose `bin/python` links to a real interpreter, so plugins run.
#[cfg(unix)]
mod unix_only {
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;

    use super::*;
    use crate::common::{cfg, decl, docs, python};
    use slopvac_plugins::pyenv::{EnvStore, InstallNetwork, Installer, ManagedEnv};
    use slopvac_plugins::trust::{TrustError, TrustGate, TrustStore};
    use slopvac_plugins::{Plugin, PluginError, PluginSpec, SpawnGate};

    const NOW: &str = "2026-11-15T12:00:00Z";

    /// Knobs of the fake uv.
    struct Fake<'a> {
        /// What `bin/python` links to.
        interpreter: PathBuf,
        /// Written into the environment as `payload.py`.
        payload: &'a str,
        /// Exit status of `uv python find`.
        find_exit: i32,
        /// When set, `pip install` also writes a `.pth` file that appends to this marker.
        hook: Option<PathBuf>,
        /// When set, `pip install` acts like a wheel that overwrites the
        /// interpreter (`Tamper::Python`, whose replacement touches the
        /// marker) or `pyvenv.cfg` (`Tamper::Config`).
        tamper: Option<Tamper>,
    }

    #[derive(Clone)]
    enum Tamper {
        Python(PathBuf),
        Config,
    }

    impl Fake<'_> {
        fn real() -> Self {
            Self {
                interpreter: python(),
                payload: "VALUE = 1",
                find_exit: 0,
                hook: None,
                tamper: None,
            }
        }
    }

    fn write_exec(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// Write the fake uv into `dir` and return its path. Each run appends its
    /// arguments and environment to `dir/uv.log`.
    fn fake_uv(dir: &Path, fake: &Fake<'_>) -> PathBuf {
        let uv = dir.join("uv");
        let home = fs::canonicalize(&fake.interpreter).unwrap();
        let home = home.parent().unwrap();
        write_exec(
            &uv,
            &format!(
                r#"#!/bin/sh
log='{log}'
{{ printf 'ARGS'; for a; do printf ' %s' "$a"; done; printf '\n'; env | sed 's/^/ENV /'; }} >> "$log"
ver=$('{py}' -I -S -c 'import sys; print("%d.%d" % sys.version_info[:2])')
case "$1" in
python) exit {find} ;;
venv)
  for a; do dir=$a; done
  mkdir -p "$dir/bin" "$dir/lib/python$ver/site-packages"
  ln -s '{py}' "$dir/bin/python"
  printf 'home = %s\ninclude-system-site-packages = false\n' '{home}' > "$dir/pyvenv.cfg"
  ;;
pip)
  prev=; py=; req=
  for a; do case "$prev" in --python) py=$a ;; -r) req=$a ;; esac; prev=$a; done
  site="${{py%/bin/python}}/lib/python$ver/site-packages"
  cp "$req" "$site/installed.txt"
  printf '%s\n' '{payload}' > "$site/payload.py"
  {hook}
  {tamper}
  # Like uv: --compile-bytecode starts the environment's interpreter with site
  # enabled, so a .pth file in it runs.
  case " $* " in *" --compile-bytecode "*) "$py" -c pass ;; esac
  ;;
esac
"#,
                log = dir.join("uv.log").display(),
                find = fake.find_exit,
                py = fake.interpreter.display(),
                home = home.display(),
                payload = fake.payload,
                hook = fake.hook.as_ref().map_or(String::new(), |m| format!(
                    r#"printf '%s\n' "import os; open('{}', 'a').write('x')" > "$site/hook.pth""#,
                    m.display()
                )),
                tamper = match &fake.tamper {
                    None => String::new(),
                    Some(Tamper::Python(m)) => format!(
                        "rm \"$py\"; printf '#!/bin/sh\\ntouch %s\\nexec %s \"$@\"\\n' '{}' '{}' > \"$py\"; chmod 755 \"$py\"",
                        m.display(),
                        fake.interpreter.display()
                    ),
                    Some(Tamper::Config) =>
                        r#"printf 'home = /tmp\n' >> "${py%/bin/python}/pyvenv.cfg""#.to_owned(),
                },
            ),
        );
        uv
    }

    fn installer(uv: PathBuf, network: InstallNetwork) -> Installer {
        Installer {
            uv,
            python_install_dir: PathBuf::from("/nonexistent/uv-python"),
            python: "3.12".into(),
            network,
        }
    }

    fn demo() -> Vec<Wheel<'static>> {
        vec![wheel("demo-pkg", "1.0", "one", &[])]
    }

    struct Built {
        vp: VerifiedPlugin,
        env: ManagedEnv,
    }

    fn build_with(s: &Scratch, fake: &Fake<'_>, store: &str) -> Built {
        let wheels = demo();
        let vp = install(s, &lock_text(&wheels), &wheels);
        let uv = fake_uv(&s.dir(&format!("{store}-bin")), fake);
        let env = EnvStore::open(s.dir(store))
            .build(&installer(uv, InstallNetwork::Online), &vp.root, &vp.tree)
            .unwrap();
        Built { vp, env }
    }

    fn build(s: &Scratch) -> Built {
        build_with(s, &Fake::real(), "envs")
    }

    fn spec(b: &Built, path_env: &str) -> PluginSpec {
        PluginSpec {
            name: b.vp.name.clone(),
            program: b.env.python(),
            args: b.env.args(&["plugin.py".into(), "ok".into(), "{}".into()]),
            cwd: b.vp.root.clone(),
            declaration: decl(),
            path_env: path_env.into(),
            env: vec![],
        }
    }

    fn gate(s: &Scratch, b: &Built) -> TrustGate {
        TrustGate::new(TrustStore::open(s.dir("state")), vec![], vec![b.vp.clone()])
            .with_managed_python(b.vp.name.clone(), b.env.clone())
    }

    fn trusted(s: &Scratch, b: &Built) -> (TrustGate, PluginSpec) {
        let g = gate(s, b);
        let sp = spec(b, "/usr/bin:/bin");
        g.record_trust(&b.vp, &sp, NOW).unwrap();
        g.check(&sp).unwrap();
        (g, sp)
    }

    /// `lib/pythonX.Y/site-packages` of an environment.
    fn site_dir(b: &Built) -> PathBuf {
        let lib = fs::read_dir(b.env.root().join("lib"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        lib.join("site-packages")
    }

    /// [`site_dir`], made writable again.
    fn site(b: &Built) -> PathBuf {
        let site = site_dir(b);
        fs::set_permissions(&site, fs::Permissions::from_mode(0o755)).unwrap();
        site
    }

    fn log(s: &Scratch, store: &str) -> String {
        fs::read_to_string(s.dir(&format!("{store}-bin")).join("uv.log")).unwrap_or_default()
    }

    /// In the child process of [`hostile_child`], its scratch directory.
    fn child_scratch() -> Option<Scratch> {
        std::env::var_os("SLOPVAC_PYENV_CHILD").map(|d| Scratch::borrowed(PathBuf::from(d)))
    }

    /// Run `test` again in a child test process whose environment holds
    /// hostile settings, since the host's own environment is the attack
    /// surface. The child works in `dir`.
    fn hostile_child(test: &str, dir: &Path, vars: &[(&str, &Path)]) {
        let mut cmd = Command::new(std::env::current_exe().unwrap());
        cmd.args([test, "--exact", "--nocapture", "--test-threads=1"])
            .env("SLOPVAC_PYENV_CHILD", dir)
            .env("PIP_INDEX_URL", "http://127.0.0.1:9/simple")
            .env("PIP_EXTRA_INDEX_URL", "http://127.0.0.1:9/simple")
            .env("UV_INDEX_URL", "http://127.0.0.1:9/simple")
            .env("UV_DEFAULT_INDEX", "http://127.0.0.1:9/simple")
            .env("UV_NO_BUILD", "0")
            .env("UV_PYTHON", "/usr/bin/false");
        for (k, v) in vars {
            cmd.env(k, v);
        }
        let child = cmd.output().unwrap();
        let stdout = String::from_utf8_lossy(&child.stdout);
        assert!(
            child.status.success() && stdout.contains("1 passed"),
            "child test failed:\n{stdout}\n{}",
            String::from_utf8_lossy(&child.stderr)
        );
    }

    #[test]
    fn a_managed_plugin_starts_and_works_through_the_trust_gate() {
        let s = Scratch::new("u-start");
        let b = build(&s);
        let sp = spec(&b, "/usr/bin:/bin");
        let g = gate(&s, &b);
        let err = Plugin::start(&sp, &cfg(), &g).unwrap_err().error;
        assert!(matches!(err, PluginError::Untrusted(_)), "{err:?}");
        g.record_trust(&b.vp, &sp, NOW).unwrap();
        let p = Plugin::start(&sp, &cfg(), &g).unwrap();
        assert_eq!(p.tag("main", &docs(1)).unwrap().len(), 1);
        assert!(p.shutdown().clean());
    }

    #[test]
    fn tm49_the_build_refuses_a_source_only_dependency_before_the_installer_runs() {
        let s = Scratch::new("u-tm49");
        let lock = one_package(&format!(
            "sdist = {{ path = \"demo-pkg-1.0.tar.gz\", hashes = {{ sha256 = \"{HASH}\" }} }}"
        ));
        let vp = install(&s, &lock, &[]);
        let uv = fake_uv(&s.dir("envs-bin"), &Fake::real());
        let err = EnvStore::open(s.dir("envs"))
            .build(&installer(uv, InstallNetwork::Online), &vp.root, &vp.tree)
            .unwrap_err();
        assert!(matches!(err, PyEnvError::SourceOnly { .. }), "{err:?}");
        assert_eq!(log(&s, "envs"), "", "uv never ran");
        assert_eq!(EnvStore::open(s.dir("envs")).list().unwrap(), []);
    }

    #[test]
    fn tm49_a_startup_hook_from_a_wheel_does_not_run_during_the_build() {
        let s = Scratch::new("u-tm49-hook");
        let marker = s.dir("hook-ran");
        let b = build_with(
            &s,
            &Fake {
                hook: Some(marker.clone()),
                ..Fake::real()
            },
            "envs",
        );
        assert!(site_dir(&b).join("hook.pth").is_file());
        assert!(!marker.exists(), "the hook ran before trust");
        // Positive control: once trusted, the plugin's interpreter loads it.
        let (g, sp) = trusted(&s, &b);
        assert!(!marker.exists(), "the hook ran while recording trust");
        let p = Plugin::start(&sp, &cfg(), &g).unwrap();
        assert!(p.shutdown().clean());
        assert!(marker.exists(), "the hook is not live in the environment");
    }

    #[test]
    fn tm49_reintroducing_uv_bytecode_compilation_would_run_the_hook_and_is_caught() {
        // Positive control for the fake: it runs the interpreter with site
        // enabled when given --compile-bytecode, as uv does.
        let s = Scratch::new("u-tm49-fake-compile");
        let marker = s.dir("hook-ran");
        let fake = Fake {
            hook: Some(marker.clone()),
            ..Fake::real()
        };
        let uv = fake_uv(&s.dir("bin"), &fake);
        let env = s.dir("env");
        let venv = Command::new(&uv).arg("venv").arg(&env).status().unwrap();
        assert!(venv.success());
        let req = s.dir("req.txt");
        fs::write(&req, "").unwrap();
        let pip = Command::new(&uv)
            .args(["pip", "install", "--compile-bytecode", "--python"])
            .arg(env.join("bin/python"))
            .arg("-r")
            .arg(&req)
            .status()
            .unwrap();
        assert!(pip.success());
        assert!(
            marker.exists(),
            "the fake does not simulate uv's site-enabled compile"
        );
    }

    #[test]
    fn tm49_tm53_a_wheel_that_replaces_the_interpreter_or_its_config_stops_the_build() {
        for (what, tamper) in [
            ("python", Tamper::Python(PathBuf::new())),
            ("config", Tamper::Config),
        ] {
            let s = Scratch::new(&format!("u-pin-{what}"));
            let marker = s.dir("replacement-ran");
            let tamper = match tamper {
                Tamper::Python(_) => Tamper::Python(marker.clone()),
                Tamper::Config => Tamper::Config,
            };
            let wheels = demo();
            let vp = install(&s, &lock_text(&wheels), &wheels);
            let uv = fake_uv(
                &s.dir("envs-bin"),
                &Fake {
                    tamper: Some(tamper),
                    ..Fake::real()
                },
            );
            let err = EnvStore::open(s.dir("envs"))
                .build(&installer(uv, InstallNetwork::Online), &vp.root, &vp.tree)
                .unwrap_err();
            assert!(
                matches!(&err, PyEnvError::Changed { why, .. } if why.contains("before trust")),
                "{what}: {err:?}"
            );
            assert!(!marker.exists(), "{what}: the replaced interpreter ran");
            assert_eq!(EnvStore::open(s.dir("envs")).list().unwrap(), [], "{what}");
        }
    }

    #[test]
    fn a_wheel_with_a_refused_entry_stops_the_build_before_uv_runs() {
        let s = Scratch::new("u-entry");
        let w = Wheel::new(
            "demo-pkg",
            "1.0",
            zip_with(
                &[("demo_pkg-1.0.data/scripts/python", b"#!/bin/sh\n")],
                None,
            ),
        );
        let vp = install(
            &s,
            &lock_text(std::slice::from_ref(&w)),
            std::slice::from_ref(&w),
        );
        let uv = fake_uv(&s.dir("envs-bin"), &Fake::real());
        let err = EnvStore::open(s.dir("envs"))
            .build(&installer(uv, InstallNetwork::Online), &vp.root, &vp.tree)
            .unwrap_err();
        assert!(matches!(err, PyEnvError::Wheel { .. }), "{err:?}");
        assert_eq!(log(&s, "envs"), "", "uv never ran");
    }

    #[test]
    fn a_failed_record_write_removes_the_promoted_environment() {
        let s = Scratch::new("u-record-fail");
        let wheels = demo();
        let vp = install(&s, &lock_text(&wheels), &wheels);
        let uv = fake_uv(&s.dir("envs-bin"), &Fake::real());
        let store = EnvStore::open(s.dir("envs")).failing_record_writes();
        let err = store
            .build(&installer(uv, InstallNetwork::Online), &vp.root, &vp.tree)
            .unwrap_err();
        assert!(matches!(err, PyEnvError::Io { .. }), "{err:?}");
        let key = EnvStore::key(&vp.tree, "3.12");
        assert!(
            !s.dir("envs").join(&key).exists(),
            "the promoted environment is left behind"
        );
        assert!(!s.dir("envs").join(format!("{key}.json")).exists());
        let staging: Vec<_> = fs::read_dir(s.dir("envs/.staging")).unwrap().collect();
        assert!(staging.is_empty(), "staging is left behind: {staging:?}");
        assert!(
            EnvStore::open(s.dir("envs"))
                .load(&vp.tree, "3.12")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn tm51_the_installer_runs_with_a_scrubbed_environment_and_no_configuration() {
        let Some(s) = child_scratch() else {
            let s = Scratch::new("u-tm51-build");
            let conf = s.dir("uv.toml");
            fs::write(&conf, "index-url = \"http://127.0.0.1:9/simple\"\n").unwrap();
            hostile_child(
                "unix_only::tm51_the_installer_runs_with_a_scrubbed_environment_and_no_configuration",
                &s.dir("child"),
                &[
                    ("UV_CONFIG_FILE", &conf),
                    ("PIP_CONFIG_FILE", &conf),
                    ("PYTHONPATH", &s.dir("pythonpath")),
                ],
            );
            return;
        };
        build(&s);
        let log = log(&s, "envs");
        let mut runs = log.split("ARGS ").filter(|r| !r.is_empty());
        let venv = runs.next().unwrap();
        let pip = runs.next().unwrap();
        assert!(runs.next().is_none(), "two uv runs: {log}");
        for run in [venv, pip] {
            let names: Vec<&str> = run
                .lines()
                .filter_map(|l| l.strip_prefix("ENV "))
                .map(|l| l.split('=').next().unwrap())
                .collect();
            // The shell adds PWD, SHLVL and `_` itself.
            for name in &names {
                assert!(
                    ["UV_PYTHON_INSTALL_DIR", "PWD", "OLDPWD", "SHLVL", "_"].contains(name),
                    "`{name}` reached uv: {run}"
                );
            }
            assert!(names.contains(&"UV_PYTHON_INSTALL_DIR"));
            let args = run.lines().next().unwrap();
            for flag in ["--no-config", "--managed-python"] {
                assert!(args.split(' ').any(|a| a == flag), "{flag} missing: {args}");
            }
        }
        let venv_args = venv.lines().next().unwrap();
        assert!(venv_args.starts_with("venv "), "{venv_args}");
        assert!(venv_args.contains(" --no-project "), "{venv_args}");
        let pip_args = pip.lines().next().unwrap();
        assert!(pip_args.starts_with("pip install "), "{pip_args}");
        for flag in [
            "--offline",
            "--no-index",
            "--no-build",
            "--no-deps",
            "--require-hashes",
            "--no-python-downloads",
        ] {
            assert!(
                pip_args.split(' ').any(|a| a == flag),
                "{flag} missing: {pip_args}"
            );
        }
    }

    #[test]
    fn tm51_pythonpath_and_usercustomize_have_no_effect_on_the_run() {
        if let Some(s) = child_scratch() {
            let b = build(&s);
            let g = gate(&s, &b);
            let mut sp = spec(&b, "/usr/bin:/bin");
            // HOME is not a code-loading variable, so a manifest may pass it.
            sp.env = vec![("HOME".into(), std::env::var("HOME").unwrap())];
            g.record_trust(&b.vp, &sp, NOW).unwrap();
            let p = Plugin::start(&sp, &cfg(), &g).unwrap();
            assert_eq!(p.tag("main", &docs(1)).unwrap().len(), 1);
            assert!(p.shutdown().clean());
            return;
        }
        let s = Scratch::new("u-tm51-run");
        let marker = s.dir("loaded");
        let hook = format!("open({:?}, 'a').write('x')\n", marker.display().to_string());
        let pythonpath = s.dir("pythonpath");
        fs::create_dir_all(&pythonpath).unwrap();
        fs::write(pythonpath.join("sitecustomize.py"), &hook).unwrap();
        let home = s.dir("home");
        let user_site = Command::new(python())
            .env_clear()
            .env("HOME", &home)
            .args(["-c", "import site; print(site.getusersitepackages())"])
            .output()
            .unwrap();
        let user_site = PathBuf::from(String::from_utf8(user_site.stdout).unwrap().trim());
        fs::create_dir_all(&user_site).unwrap();
        fs::write(user_site.join("usercustomize.py"), &hook).unwrap();
        // Positive control: both hooks load into a plain interpreter.
        for (k, v) in [("PYTHONPATH", &pythonpath), ("HOME", &home)] {
            let _ = fs::remove_file(&marker);
            Command::new(python())
                .env_clear()
                .env(k, v)
                .args(["-c", "pass"])
                .status()
                .unwrap();
            assert!(marker.exists(), "the {k} hook did not load in the control");
        }
        let _ = fs::remove_file(&marker);
        hostile_child(
            "unix_only::tm51_pythonpath_and_usercustomize_have_no_effect_on_the_run",
            &s.dir("child"),
            &[("PYTHONPATH", &pythonpath), ("HOME", &home)],
        );
        assert!(!marker.exists(), "a hook loaded into the managed plugin");
    }

    #[test]
    fn tm51_a_managed_interpreter_without_isolation_does_not_start() {
        let s = Scratch::new("u-tm51-flags");
        let b = build(&s);
        let g = gate(&s, &b);
        for args in [
            vec!["plugin.py".to_owned(), "ok".into()],
            vec!["-B".into(), "-I".into(), "plugin.py".into()],
            vec!["-I".into(), "plugin.py".into()],
        ] {
            let sp = PluginSpec {
                args: args.clone(),
                ..spec(&b, "/usr/bin:/bin")
            };
            assert!(
                matches!(g.state(&b.vp, &sp), Err(TrustError::Program(_))),
                "{args:?}"
            );
            let err = Plugin::start(&sp, &cfg(), &g).unwrap_err().error;
            assert!(matches!(err, PluginError::Usage(_)), "{args:?}: {err:?}");
        }
    }

    #[test]
    fn tm53_a_python_earlier_on_path_does_not_run() {
        let s = Scratch::new("u-tm53");
        let marker = s.dir("path-python-ran");
        let bin = s.dir("hostile-bin");
        for name in ["python", "python3"] {
            write_exec(
                &bin.join(name),
                &format!("#!/bin/sh\ntouch '{}'\nexit 1\n", marker.display()),
            );
        }
        let b = build(&s);
        let g = gate(&s, &b);
        let sp = spec(&b, &format!("{}:/usr/bin:/bin", bin.display()));
        g.record_trust(&b.vp, &sp, NOW).unwrap();
        let p = Plugin::start(&sp, &cfg(), &g).unwrap();
        assert_eq!(p.tag("main", &docs(1)).unwrap().len(), 1);
        assert!(p.shutdown().clean());
        assert!(!marker.exists(), "the python on PATH ran");

        // Naming any other interpreter is refused, even one on the external allowlist.
        for program in [bin.join("python"), python()] {
            let other = PluginSpec {
                program: program.clone(),
                ..sp.clone()
            };
            let g = gate(&s, &b).with_external_programs([program.clone()]);
            let err = Plugin::start(&other, &cfg(), &g).unwrap_err().error;
            assert!(
                matches!(err, PluginError::Usage(_)),
                "{}: {err:?}",
                program.display()
            );
        }
        assert!(!marker.exists(), "the python on PATH ran");
    }

    #[test]
    fn tm50_a_new_pth_file_refuses_the_spawn() {
        let s = Scratch::new("u-tm50-pth");
        let b = build(&s);
        let (g, sp) = trusted(&s, &b);
        fs::write(site(&b).join("evil.pth"), "import os\n").unwrap();
        let err = Plugin::start(&sp, &cfg(), &g).unwrap_err().error;
        assert!(matches!(err, PluginError::HashMismatch(_)), "{err:?}");
        assert!(err.to_string().contains("evil.pth"), "{err}");
    }

    #[test]
    fn tm52_a_changed_or_removed_file_refuses_the_spawn() {
        type Change = fn(&Path);
        let changes: [(&str, Change); 3] = [
            ("longer", |p| fs::write(p, "VALUE = 22\n").unwrap()),
            ("removed", |p| fs::remove_file(p).unwrap()),
            ("executable", |p| {
                fs::set_permissions(p, fs::Permissions::from_mode(0o755)).unwrap();
            }),
        ];
        for (what, change) in changes {
            let s = Scratch::new(&format!("u-tm52-{what}"));
            let b = build(&s);
            let (g, sp) = trusted(&s, &b);
            let file = site(&b).join("payload.py");
            fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
            change(&file);
            let err = g.check(&sp).unwrap_err();
            assert!(
                matches!(err, PluginError::HashMismatch(_)),
                "{what}: {err:?}"
            );
            assert!(err.to_string().contains("payload.py"), "{what}: {err}");
        }
    }

    /// The Y5 trade-off: an edit that keeps the size and restores the
    /// modification time passes the spawn check, and the full check that
    /// `plugins status`, `doctor` and `plugins trust` run refuses it.
    #[test]
    fn tm52_a_same_size_edit_with_the_old_mtime_is_caught_by_the_full_check() {
        let s = Scratch::new("u-tm52-mtime");
        let b = build(&s);
        let (g, sp) = trusted(&s, &b);
        let file = site(&b).join("payload.py");
        let before = fs::metadata(&file).unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
        fs::write(&file, "VALUE = 2\n").unwrap();
        let f = fs::File::options().write(true).open(&file).unwrap();
        f.set_modified(before.modified().unwrap()).unwrap();
        drop(f);
        fs::set_permissions(&file, before.permissions()).unwrap();
        assert_eq!(fs::metadata(&file).unwrap().len(), before.len());

        g.check(&sp).unwrap();
        let err = b.env.verify_full().unwrap_err();
        assert!(
            matches!(&err, PyEnvError::Changed { path, .. } if path.ends_with("payload.py")),
            "{err:?}"
        );
        let err = g.record_trust(&b.vp, &sp, NOW).unwrap_err();
        assert!(matches!(err, TrustError::PythonEnv(_)), "{err:?}");
    }

    #[test]
    fn the_environment_listing_and_the_interpreter_identity_are_in_the_trust_hash() {
        let s = Scratch::new("u-slots");
        let base = build_with(&s, &Fake::real(), "a");
        let hash = |b: &Built| {
            gate(&s, b)
                .state(&b.vp, &spec(b, "/usr/bin:/bin"))
                .unwrap()
                .0
        };
        let trusted_hash = hash(&base);
        gate(&s, &base)
            .record_trust(&base.vp, &spec(&base, "/usr/bin:/bin"), NOW)
            .unwrap();

        let other_content = build_with(
            &s,
            &Fake {
                payload: "VALUE = 2",
                ..Fake::real()
            },
            "b",
        );
        // One interpreter path whose file changes between two builds, so the
        // environments link to the same place and only the identity differs.
        let wrapper = s.dir("interp/python");
        let interp_build = |tag: &str, store: &str| {
            write_exec(
                &wrapper,
                &format!("#!/bin/sh\n# {tag}\nexec '{}' \"$@\"\n", python().display()),
            );
            build_with(
                &s,
                &Fake {
                    interpreter: wrapper.clone(),
                    ..Fake::real()
                },
                store,
            )
        };
        // The same store, so the bytecode names the same paths; the second build replaces the first.
        let interp_a = interp_build("a", "c");
        let interp_b = interp_build("b", "c");
        assert_eq!(base.vp.tree, other_content.vp.tree, "same plugin tree");
        assert_ne!(
            base.env.environment_digest(),
            other_content.env.environment_digest()
        );
        assert_ne!(hash(&other_content), trusted_hash);
        assert_ne!(
            interp_a.env.interpreter_identity(),
            interp_b.env.interpreter_identity()
        );
        assert_eq!(
            interp_a.env.environment_digest(),
            interp_b.env.environment_digest(),
            "only the interpreter differs"
        );
        assert_ne!(hash(&interp_a), hash(&interp_b));
        let err = gate(&s, &other_content)
            .check(&spec(&other_content, "/usr/bin:/bin"))
            .unwrap_err();
        assert!(matches!(err, PluginError::Untrusted(_)), "{err:?}");
    }

    #[test]
    fn an_interpreter_changed_after_trust_refuses_the_spawn() {
        let s = Scratch::new("u-interp");
        let w = s.dir("interp/python");
        write_exec(
            &w,
            &format!("#!/bin/sh\nexec '{}' \"$@\"\n", python().display()),
        );
        let b = build_with(
            &s,
            &Fake {
                interpreter: w.clone(),
                ..Fake::real()
            },
            "envs",
        );
        let (g, sp) = trusted(&s, &b);
        write_exec(
            &w,
            &format!(
                "#!/bin/sh\n# upgraded\nexec '{}' \"$@\"\n",
                python().display()
            ),
        );
        let err = g.check(&sp).unwrap_err();
        assert!(matches!(err, PluginError::HashMismatch(_)), "{err:?}");
    }

    #[test]
    fn the_built_environment_is_read_only() {
        let s = Scratch::new("u-ro");
        let b = build(&s);
        for p in [
            b.env.root().to_owned(),
            site_dir(&b),
            site_dir(&b).join("payload.py"),
        ] {
            let mode = fs::metadata(&p).unwrap().permissions().mode();
            assert_eq!(mode & 0o222, 0, "{} is writable", p.display());
        }
    }

    #[test]
    fn offline_with_no_managed_interpreter_fails_with_its_own_error() {
        let s = Scratch::new("u-offline");
        let wheels = demo();
        let vp = install(&s, &lock_text(&wheels), &wheels);
        let uv = fake_uv(
            &s.dir("envs-bin"),
            &Fake {
                find_exit: 2,
                ..Fake::real()
            },
        );
        let err = EnvStore::open(s.dir("envs"))
            .build(&installer(uv, InstallNetwork::Offline), &vp.root, &vp.tree)
            .unwrap_err();
        assert!(
            matches!(&err, PyEnvError::OfflinePythonMissing(r) if r == "3.12"),
            "{err:?}"
        );
        assert!(!log(&s, "envs").contains("ARGS venv"), "no venv was made");
    }

    #[test]
    fn doctor_lists_an_environment_and_removes_it() {
        let s = Scratch::new("u-list");
        let b = build(&s);
        let store = EnvStore::open(s.dir("envs"));
        let listed = store.list().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].path, b.env.root());
        assert_eq!(listed[0].tree, b.vp.tree.as_str());
        store.remove(&listed[0].key).unwrap();
        assert_eq!(store.list().unwrap(), []);
        assert!(!b.env.root().exists());
        assert!(store.load(&b.vp.tree, "3.12").unwrap().is_none());
    }

    #[test]
    fn a_rebuild_replaces_a_tampered_environment() {
        let s = Scratch::new("u-rebuild");
        let b = build(&s);
        fs::write(site(&b).join("evil.pth"), "import os\n").unwrap();
        let uv = fake_uv(&s.dir("envs-bin"), &Fake::real());
        let store = EnvStore::open(s.dir("envs"));
        let again = store
            .ensure(
                &installer(uv, InstallNetwork::Online),
                &b.vp.root,
                &b.vp.tree,
            )
            .unwrap();
        assert!(!site_dir(&b).join("evil.pth").exists());
        again.verify_full().unwrap();
    }
}

/// Real uv. See the module documentation for when these skip.
#[cfg(unix)]
mod live {
    use std::process::Command;

    use super::*;
    use crate::common::{cfg, decl, docs};
    use slopvac_plugins::pyenv::{EnvStore, InstallNetwork, Installer, ManagedEnv};
    use slopvac_plugins::trust::{TrustGate, TrustStore};
    use slopvac_plugins::{Plugin, PluginError, PluginSpec, SpawnGate};

    const NOW: &str = "2026-11-15T12:00:00Z";

    /// The uv to test with, or `None` to skip.
    fn uv() -> Option<PathBuf> {
        let found = std::env::var_os("SLOPVAC_TEST_UV")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
                    .map(|d| d.join("uv"))
                    .find(|p| p.is_file())
            });
        let Some(uv) = found else {
            assert!(
                std::env::var_os("SLOPVAC_PYENV_LIVE").is_none(),
                "SLOPVAC_PYENV_LIVE is set and no uv was found"
            );
            eprintln!(
                "SKIPPED live pyenv test: no uv on PATH. Set SLOPVAC_TEST_UV=<path> to run it, or SLOPVAC_PYENV_LIVE=1 to make a missing uv fail."
            );
            return None;
        };
        let ok = Command::new(&uv)
            .env_clear()
            .arg("--version")
            .status()
            .is_ok_and(|s| s.success());
        assert!(ok, "{} is present but `uv --version` fails", uv.display());
        Some(uv)
    }

    fn installer(uv: PathBuf) -> Installer {
        let dir = Command::new(&uv).args(["python", "dir"]).output().unwrap();
        assert!(dir.status.success(), "uv python dir failed");
        Installer {
            uv,
            python_install_dir: PathBuf::from(String::from_utf8(dir.stdout).unwrap().trim()),
            python: ">=3.11".into(),
            network: InstallNetwork::Online,
        }
    }

    struct Built {
        vp: VerifiedPlugin,
        env: ManagedEnv,
    }

    fn build(s: &Scratch, uv: &Path, wheels: &[Wheel<'_>]) -> Built {
        let vp = install(s, &lock_text(wheels), wheels);
        let env = EnvStore::open(s.dir("envs"))
            .build(&installer(uv.to_owned()), &vp.root, &vp.tree)
            .unwrap_or_else(|e| panic!("uv is present but the build failed: {e}"));
        Built { vp, env }
    }

    fn spec(b: &Built) -> PluginSpec {
        PluginSpec {
            name: b.vp.name.clone(),
            program: b.env.python(),
            args: b.env.args(&["plugin.py".into(), "ok".into(), "{}".into()]),
            cwd: b.vp.root.clone(),
            declaration: decl(),
            path_env: "/usr/bin:/bin".into(),
            env: vec![],
        }
    }

    fn gate(s: &Scratch, b: &Built) -> TrustGate {
        TrustGate::new(TrustStore::open(s.dir("state")), vec![], vec![b.vp.clone()])
            .with_managed_python(b.vp.name.clone(), b.env.clone())
    }

    fn import_value(env: &ManagedEnv, module: &str) -> String {
        let out = Command::new(env.python())
            .env_clear()
            .args(["-I", "-B", "-c"])
            .arg(format!("import {module}; print({module}.VALUE)"))
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap().trim().to_owned()
    }

    #[test]
    fn live_a_locked_wheel_environment_builds_verifies_and_runs() {
        let Some(uv) = uv() else { return };
        let s = Scratch::new("live-run");
        let b = build(&s, &uv, &[wheel("demo-pkg", "1.0", "one", &[])]);
        assert_eq!(import_value(&b.env, "demo_pkg"), "one");
        b.env.check_quick().unwrap();
        b.env.verify_full().unwrap();
        let g = gate(&s, &b);
        let sp = spec(&b);
        g.record_trust(&b.vp, &sp, NOW).unwrap();
        let p = Plugin::start(&sp, &cfg(), &g).unwrap();
        assert_eq!(p.tag("main", &docs(1)).unwrap().len(), 1);
        assert!(p.shutdown().clean());
        // Running the plugin wrote nothing into the environment.
        b.env.verify_full().unwrap();
    }

    #[test]
    fn live_tm50_each_dependency_change_changes_the_trust_hash() {
        let Some(uv) = uv() else { return };
        let s = Scratch::new("live-tm50");
        let base = build(&s, &uv, &[wheel("demo-pkg", "1.0", "one", &[])]);
        let g = gate(&s, &base);
        g.record_trust(&base.vp, &spec(&base), NOW).unwrap();
        let trusted = g.state(&base.vp, &spec(&base)).unwrap().0;
        let variants: [(&str, Vec<Wheel<'static>>); 3] = [
            ("edit one pin", vec![wheel("demo-pkg", "1.1", "one", &[])]),
            (
                "add a package",
                vec![
                    wheel("demo-pkg", "1.0", "one", &[]),
                    wheel("extra-pkg", "1.0", "x", &[]),
                ],
            ),
            ("swap a wheel", vec![wheel("demo-pkg", "1.0", "two", &[])]),
        ];
        for (what, wheels) in variants {
            let s2 = Scratch::new(&format!("live-tm50-{}", what.replace(' ', "-")));
            let changed = build(&s2, &uv, &wheels);
            let g2 = TrustGate::new(
                TrustStore::open(s.dir("state")),
                vec![],
                vec![changed.vp.clone()],
            )
            .with_managed_python(changed.vp.name.clone(), changed.env.clone());
            let (hash, _) = g2.state(&changed.vp, &spec(&changed)).unwrap();
            assert_ne!(hash, trusted, "{what}");
            let err = Plugin::start(&spec(&changed), &cfg(), &g2)
                .unwrap_err()
                .error;
            assert!(matches!(err, PluginError::Untrusted(_)), "{what}: {err:?}");
        }
    }

    #[test]
    fn live_tm49_a_startup_hook_in_a_wheel_is_refused_and_never_runs() {
        let Some(uv) = uv() else { return };
        let s = Scratch::new("live-tm49-hook");
        let marker = s.dir("hook-ran");
        let hook = format!(
            "import os; open({:?}, 'a').write('x')\n",
            marker.display().to_string()
        );
        let w = wheel("demo-pkg", "1.0", "one", &[("demo_hook.pth", &hook)]);
        let vp = install(
            &s,
            &lock_text(std::slice::from_ref(&w)),
            std::slice::from_ref(&w),
        );
        let err = EnvStore::open(s.dir("envs"))
            .build(&installer(uv), &vp.root, &vp.tree)
            .unwrap_err();
        assert!(
            matches!(&err, PyEnvError::Wheel { why, .. } if why.contains("startup hook")),
            "{err:?}"
        );
        assert!(!marker.exists(), "the hook ran");
    }

    #[test]
    fn live_tm53_a_wheel_script_named_python_never_reaches_uv() {
        let Some(uv) = uv() else { return };
        let s = Scratch::new("live-collision");
        let marker = s.dir("collision-ran");
        let script = format!("#!/bin/sh\ntouch '{}'\n", marker.display());
        let w = wheel(
            "demo-pkg",
            "1.0",
            "one",
            &[("demo_pkg-1.0.data/scripts/python", &script)],
        );
        let vp = install(
            &s,
            &lock_text(std::slice::from_ref(&w)),
            std::slice::from_ref(&w),
        );
        let err = EnvStore::open(s.dir("envs"))
            .build(&installer(uv), &vp.root, &vp.tree)
            .unwrap_err();
        assert!(matches!(err, PyEnvError::Wheel { .. }), "{err:?}");
        assert!(!marker.exists());
    }

    #[test]
    fn live_tm52_a_modified_file_in_a_real_environment_refuses_the_spawn() {
        use std::os::unix::fs::PermissionsExt;
        let Some(uv) = uv() else { return };
        let s = Scratch::new("live-tm52");
        let b = build(&s, &uv, &[wheel("demo-pkg", "1.0", "one", &[])]);
        let g = gate(&s, &b);
        let sp = spec(&b);
        g.record_trust(&b.vp, &sp, NOW).unwrap();
        let init = fs::read_dir(b.env.root().join("lib"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path()
            .join("site-packages/demo_pkg/__init__.py");
        fs::set_permissions(init.parent().unwrap(), fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(&init, fs::Permissions::from_mode(0o644)).unwrap();
        fs::write(&init, "VALUE = 'tampered'\n").unwrap();
        let err = g.check(&sp).unwrap_err();
        assert!(matches!(err, PluginError::HashMismatch(_)), "{err:?}");
    }
}
