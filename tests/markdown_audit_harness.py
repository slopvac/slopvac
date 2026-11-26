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
import json, os, sys
from pathlib import Path

args = sys.argv[1:]
out = Path(args[args.index("--out") + 1])
files = args[args.index("--out") + 2:]
scan = out.stem
case = json.loads(Path(os.environ["AUDIT_CASE"]).read_text())
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

_STUB_UV = f"""#!/bin/sh
shift 4
exec {sys.executable} "$@"
"""


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


def make_repo(root: Path) -> Path:
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
    for name, text in files.items():
        path = repo / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")
    _git(repo, "init", "-q")
    _git(repo, "add", "-A")
    _git(repo, "commit", "-q", "-m", "x")
    # Untracked, so `git ls-files` does not list it: the script's own helper modules.
    (repo / "scripts").symlink_to(ROOT / "scripts")
    for name, body in (("slopvac", _STUB_SLOPVAC), ("uv", _STUB_UV)):
        (bin_dir / name).write_text(body, encoding="utf-8")
        (bin_dir / name).chmod(0o755)
    return repo


def run_audit(root: Path, case: dict) -> subprocess.CompletedProcess[str]:
    repo = make_repo(root)
    # The step calls `tempfile.mkdtemp`, so keep its files under the test directory.
    scratch = root / "tmp"
    scratch.mkdir()
    case_file = root / "case.json"
    case_file.write_text(json.dumps(case), encoding="utf-8")
    env = {
        **os.environ,
        "PATH": f"{root / 'bin'}{os.pathsep}{os.environ['PATH']}",
        "AUDIT_CASE": str(case_file),
        "TMPDIR": str(scratch),
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
