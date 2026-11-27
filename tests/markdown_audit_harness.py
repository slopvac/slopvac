"""Run the audit step's real `run:` script against a stub linter in a temporary repository.

The script is read from `.github/workflows/markdown-audit.yml` when the test runs, so a
later edit to the step is the code under test. `uv` and `slopvac` are stubs on `PATH`: the
`uv` stub runs the embedded Python with this interpreter, and the `slopvac` stub writes the
report that a case asks for to the `--out` path of each scan.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import textwrap
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / ".github" / "workflows" / "markdown-audit.yml"
HOSTILE_NAME = "::error::pwned::add-mask::x"

_STUB_SLOPVAC = f"""#!{sys.executable}
import json, os, sys, time
from pathlib import Path

args = sys.argv[1:]
out = Path(args[args.index("--out") + 1])
files = args[args.index("--out") + 2:]
scan = out.stem
case = json.loads(Path(os.environ["AUDIT_CASE"]).read_text())
if case.get(scan + "-sleep"):
    time.sleep(30)
documents = [
    {{
        "path": path,
        "gate": {{"passed": True, "reasons": []}},
        "findings": [],
        "errors": 0,
        "warnings": 0,
        "suggestions": 0,
        "unchecked": [],
    }}
    for path in files
    # The configured scan applies the root slopvac.toml, which excludes CHANGELOG.md.
    if not (scan == "configured" and path == "CHANGELOG.md")
]
report = {{
    "documents": documents,
    "summary": {{
        "documents": len(documents), "words": 0, "findings": 0, "errors": 0,
        "warnings": 0, "suggestions": 0, "passed": True,
    }},
}}
exec(case.get(scan, ""), {{"report": report}})
if not case.get(scan + "-noreport"):
    out.write_text(json.dumps(report))
print(case.get(scan + "-stderr", ""), end="", file=sys.stderr)
print(case.get(scan + "-stdout", ""), end="")
sys.exit(case.get(scan + "-exit", 0))
"""

_STUB_UV = (
    "#!/bin/sh\n"
    # A harness-only way to put text on stderr before the audit starts, or to fail to start.
    'if [ -n "${UV_STDERR:-}" ]; then printf \'%s\\n\' "$UV_STDERR" >&2; fi\n'
    'if [ -n "${UV_FAIL:-}" ]; then exit 1; fi\n'
    "shift 4\n"
    f'exec {sys.executable} "$@"\n'
)


def audit_script() -> str:
    doc = yaml.safe_load(WORKFLOW.read_text(encoding="utf-8"))
    steps = doc["jobs"]["inventory-and-audit"]["steps"]
    (step,) = [s for s in steps if s.get("name") == "Audit tracked Markdown"]
    return step["run"]


def _git(repo: Path, *args: str) -> None:
    subprocess.run(
        ["git", "-c", "user.email=a@b.c", "-c", "user.name=t", *args],
        cwd=repo,
        check=True,
        capture_output=True,
    )


def make_repo(root: Path, extra_files: dict | None = None, symlinks: dict | None = None) -> Path:
    repo = root / "repo"
    bin_dir = root / "bin"
    repo.mkdir()
    bin_dir.mkdir()
    files = {
        "README.md": "# Readme\n",
        "docs/a.md": "# A\n",
        f"docs/{HOSTILE_NAME}.md": "# Hostile name\n",
        "CHANGELOG.md": "# Changes\n",
        ".github/action-fixtures/f.md": "# Fixture\n",
        "packages/slopvac-lint/docs/rules.md": "# Rules\n",
        "slopvac.toml": 'schema_version = 3\nexclude = [\n  "CHANGELOG.md",\n]\n',
    }
    files.update(extra_files or {})
    for name, text in files.items():
        path = repo / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")
    # Tracked symlinks: `{name: target}`, so a case can plant an alias with a hostile name.
    for name, target in (symlinks or {}).items():
        (repo / name).symlink_to(target)
    _git(repo, "init", "-q")
    _git(repo, "add", "-A")
    _git(repo, "commit", "-q", "-m", "x")
    # Untracked, so `git ls-files` does not list it: the script's own helper modules.
    (repo / "scripts").symlink_to(ROOT / "scripts")
    for name, body in (("slopvac", _STUB_SLOPVAC), ("uv", _STUB_UV)):
        (bin_dir / name).write_text(body, encoding="utf-8")
        (bin_dir / name).chmod(0o755)
    return repo


def assert_output_contract(
    done: subprocess.CompletedProcess[str],
    root: Path,
    expect: str | None = None,
    *,
    prefix: bool = False,
    payload_free: bool = False,
) -> None:
    """Check the step's output contract, whatever the case.

    The launcher sends the audit's whole stderr to a private file under `RUNNER_TEMP`, so
    the step's own stderr is always empty. The script's fixed diagnostics go to stdout. On
    a failure the shell then prints `Audit failed (exit 1)` as the last line, with the real
    code. No stream carries a traceback or a raw workflow command. `expect` is the fixed
    line a failing case must print, as the whole line or, with `prefix`, its start.
    """
    assert done.stderr == ""
    assert (root / "runner-temp" / "markdown-audit.stderr").exists()
    for stream in (done.stdout, done.stderr):
        assert "Traceback" not in stream
        assert "::" not in stream
        assert "##[" not in stream
    if expect is None:
        assert done.returncode == 0, done.stdout
        assert "Audit failed" not in done.stdout
        return
    # `stop()` and the exception hook both end the run with 1, and the shell passes it on.
    assert done.returncode == 1
    lines = done.stdout.splitlines()
    assert lines[-1] == "Audit failed (exit 1)"
    assert any(line.startswith(expect) if prefix else line == expect for line in lines), done.stdout
    if payload_free:
        assert "pwned" not in done.stdout


def run_audit(root: Path, case: dict) -> subprocess.CompletedProcess[str]:
    repo = make_repo(root, case.get("files"), case.get("symlinks"))
    # The step calls `tempfile.mkdtemp`, so keep its files under the test directory.
    scratch = root / "tmp"
    scratch.mkdir()
    (root / "runner-temp").mkdir()
    if case.get("shadow-markdown-it"):
        # A module at the checkout root that must never be imported: the audit runs the
        # interpreter in isolated mode. Importing it would leave a marker and print.
        (repo / "markdown_it.py").write_text(
            f'from pathlib import Path\nPath({str(root / "shadow-imported")!r}).write_text("x")\n'
            'print("SHADOW-IMPORTED")\nraise RuntimeError("::error::pwned")\n'
        )
    case_file = root / "case.json"
    case_file.write_text(json.dumps(case), encoding="utf-8")
    env = {
        **os.environ,
        "PATH": f"{root / 'bin'}{os.pathsep}{os.environ['PATH']}",
        "AUDIT_CASE": str(case_file),
        "TMPDIR": str(scratch),
        "RUNNER_TEMP": str(root / "runner-temp"),
        **case.get("env", {}),
        "SLOPVAC_BIN": str(root / "bin" / "slopvac"),
        "GITHUB_STEP_SUMMARY": str(root / "summary.md"),
    }
    return subprocess.run(
        ["bash", "-c", textwrap.dedent(audit_script())],
        cwd=repo,
        env=env,
        check=False,
        capture_output=True,
        text=True,
    )
