//! A scripted fake `uv` for the managed Python environment tests. It lays out
//! a venv whose `bin/python` links to a real interpreter, so plugins run.
#![allow(dead_code, unreachable_pub, missing_docs)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use super::python;

/// Knobs of the fake uv.
pub struct Fake<'a> {
    /// What `bin/python` links to.
    pub interpreter: PathBuf,
    /// Written into the environment as `payload.py`.
    pub payload: &'a str,
    /// Exit status of `uv python find`.
    pub find_exit: i32,
    /// When set, `pip install` also writes a `.pth` file that appends to this marker.
    pub hook: Option<PathBuf>,
    /// When set, `pip install` acts like a wheel that overwrites the
    /// interpreter (`Tamper::Python`, whose replacement touches the
    /// marker) or `pyvenv.cfg` (`Tamper::Config`).
    pub tamper: Option<Tamper>,
    /// Shell run at the end of `pip install`, with `$site` set: extra output.
    pub pip_extra: &'a str,
    /// `uv venv` hangs instead of creating the environment.
    pub hang: bool,
    /// Shell run at the start of `uv venv`.
    pub venv_extra: &'a str,
    /// Exit 99 when a run could download: any call without both
    /// `--offline` and `--no-python-downloads`, or with `--python-downloads`.
    pub offline_only: bool,
}

#[derive(Clone)]
pub enum Tamper {
    Python(PathBuf),
    Config,
}

impl Fake<'_> {
    pub fn real() -> Self {
        Self {
            interpreter: python(),
            payload: "VALUE = 1",
            find_exit: 0,
            hook: None,
            tamper: None,
            pip_extra: "",
            hang: false,
            venv_extra: "",
            offline_only: false,
        }
    }
}

pub fn write_exec(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// Write the fake uv into `dir` and return its path. Each run appends its
/// arguments and environment to `dir/uv.log`.
pub fn fake_uv(dir: &Path, fake: &Fake<'_>) -> PathBuf {
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
{offline}
case "$1" in
python) exit {find} ;;
venv)
  {hang}
  {venv_extra}
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
  {pip_extra}
  ;;
esac
"#,
            log = dir.join("uv.log").display(),
            offline = if fake.offline_only {
                r#"case " $* " in *" --python-downloads"*) echo "download-capable: $*" >&2; exit 99 ;; esac
case " $* " in *" --offline "*) ;; *) echo "download-capable: $*" >&2; exit 99 ;; esac
case " $* " in *" --no-python-downloads "*) ;; *) echo "download-capable: $*" >&2; exit 99 ;; esac"#
            } else {
                ""
            },
            hang = if fake.hang { "exec sleep 30" } else { "" },
            venv_extra = fake.venv_extra,
            pip_extra = fake.pip_extra,
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
