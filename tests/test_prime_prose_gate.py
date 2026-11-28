"""scripts/ci/gate_prime_prose.sh renders every prose branch of `prime` and lints it.

The script is the CI gate for the guidance text. These tests pin what it must do:

* lint receives the complete set of rendered files, checked at the subprocess boundary;
* every conditional expression in the `write` and `review` topics is one the gate knows,
  so a new branch cannot slip in without a render for it;
* each branch shows in the real output;
* a nonzero lint exit, including exit 2 (incomplete), fails the script;
* the output directory is never emptied: a non-empty one is refused.
"""

from __future__ import annotations

import os
import re
import subprocess
from pathlib import Path

import pytest

from real_binary import FIXTURES, ROOT, slopvac_binary  # noqa: F401

SCRIPT = ROOT / "scripts" / "ci" / "gate_prime_prose.sh"
CONFIG = FIXTURES / "slopvac-v3.toml"
TOPICS = ROOT / "crates" / "slopvac-agents" / "topics"

TYPES = ("mechanical", "update", "rewrite")
DEPTHS = ("self", "subagent", "adversary")
# Every file the gate renders, and so every file lint must receive.
RENDERS = (
    {"write-minimal.md", "write-ask.md", "write-full.md"}
    | {f"review-{kind}-{depth}.md" for kind in (*TYPES, "auto") for depth in DEPTHS}
    | {"review-auto-subagent-minimal.md"}
)

# The conditions in each topic, each with the render that takes its branch. A new
# condition, even on a variable that is already here, changes this set and fails the test
# until the gate renders the new branch and the set lists it.
CONDITIONS = {
    "write.md.j2": {
        'level != "minimal"': "write-default and write-full",
        'level == "full"': "write-full",
        'review == "auto"': "write-full",
        'review == "ask"': "write-ask",
    },
    "review.md.j2": {
        'review_depth == "self"': "review-*-self",
        'review_depth == "adversary"': "review-*-adversary",
        'review_type == "auto"': "review-auto-*",
        'review_type == "mechanical"': "review-mechanical-*",
        'review_type == "update"': "review-update-*",
        'level != "minimal"': "review-* and review-auto-subagent-minimal",
        # `prime` never sets `review_approval`. Only the library API does, so no CLI
        # render shows its `auto` branch.
        'review_approval == "auto"': "not reachable from the CLI",
    },
}


def conditions_of(text: str) -> set[str]:
    found = re.findall(r"{%-?\s*(?:if|elif)\s+(.*?)\s*-?%}", text)
    return {" ".join(cond.split()) for cond in found}


def stand_in(tmp_path: Path, *, real: Path | None = None, lint_exit: int = 0) -> Path:
    """A `slopvac` that records the argv of `lint`.

    With `real`, `prime` and `lint` run that binary. Without it, `prime` prints one line
    and `lint` exits with `lint_exit`.
    """
    record = tmp_path / "lint-argv"
    if real is not None:
        prime = lint = f'exec "{real}" "$@"'
    else:
        # RACE_FILE lets a test create a file at the moment `prime` runs, to stand in for
        # another process that writes into the output directory during the run.
        prime = (
            'if [ -n "$RACE_FILE" ] && [ ! -e "$RACE_FILE" ]; then '
            'echo mine > "$RACE_FILE"; fi; echo "# Prime: stand-in"; exit 0'
        )
        lint = f"exit {lint_exit}"
    script = tmp_path / "slopvac"
    script.write_text(
        "#!/bin/sh\n"
        f'case "$1" in\n  prime) {prime} ;;\n'
        f'  lint) printf "%s\\n" "$@" > "{record}"; {lint} ;;\nesac\n',
        encoding="utf-8",
    )
    script.chmod(0o755)
    return script


def linted(tmp_path: Path) -> list[str]:
    """The file names `lint` received, in order."""
    argv = (tmp_path / "lint-argv").read_text(encoding="utf-8").splitlines()
    assert argv[0] == "lint"
    return [Path(arg).name for arg in argv[1:]]


def run_gate(
    binary: Path, out: Path, env: dict[str, str] | None = None
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["bash", str(SCRIPT), str(binary), str(CONFIG), str(out)],
        capture_output=True,
        text=True,
        check=False,
        env={**os.environ, **(env or {})},
    )


@pytest.mark.parametrize("topic", sorted(CONDITIONS))
def test_every_condition_in_a_topic_is_a_branch_the_gate_knows(topic):
    text = (TOPICS / topic).read_text(encoding="utf-8")
    assert conditions_of(text) == set(CONDITIONS[topic])


@pytest.mark.parametrize(
    ("text", "expected"),
    [
        ('{% if level == "full" %}x{% endif %}', {'level == "full"'}),
        # A condition that wraps over lines is one condition.
        (
            '{% if review_type == "update"\n   and review_depth == "self" %}x{% endif %}',
            {'review_type == "update" and review_depth == "self"'},
        ),
        ('{%- elif\n  level != "minimal" -%}', {'level != "minimal"'}),
    ],
)
def test_conditions_are_read_whether_or_not_they_wrap_over_lines(text, expected):
    assert conditions_of(text) == expected


def test_lint_receives_every_rendered_file_and_no_other(tmp_path):
    gate = stand_in(tmp_path)
    assert run_gate(gate, tmp_path / "out").returncode == 0
    names = linted(tmp_path)
    assert sorted(names) == sorted(RENDERS)
    assert len(names) == len(set(names))


def review_cell(text: str) -> tuple[str, str]:
    step = re.search(r"## Steps for (\w+)", text)
    depth = re.search(r"Depth: `(\w+)`", text)
    return (step.group(1) if step else "auto", depth.group(1) if depth else "subagent")


def test_the_gate_renders_each_conditional_branch(slopvac_binary, tmp_path):  # noqa: F811
    gate = stand_in(tmp_path, real=slopvac_binary)
    result = run_gate(gate, tmp_path / "out")
    assert result.returncode == 0, result.stdout + result.stderr
    assert sorted(linted(tmp_path)) == sorted(RENDERS)
    docs = [(tmp_path / "out" / name).read_text(encoding="utf-8") for name in RENDERS]
    writes = [t for t in docs if "# Prime: write" in t]
    reviews = [t for t in docs if "# Prime: review" in t]

    # write: the three levels, and the review control auto, ask and off.
    assert any("## Sentences" not in t for t in writes)
    assert any("## Sentences" in t and "## Scope of a run" not in t for t in writes)
    assert any("## Scope of a run" in t for t in writes)
    assert any("Before hand-off, run `slopvac prime review`." in t for t in writes)
    assert any("Before hand-off, ask the user whether to run" in t for t in writes)
    assert any("Before hand-off" not in t for t in writes)

    # review: every pair of type and depth, and the level without the verdict block.
    assert {review_cell(t) for t in reviews} >= {
        (kind, depth) for kind in ("auto", *TYPES) for depth in DEPTHS
    }
    assert any("## Verdict" not in t for t in reviews)
    assert any("## Verdict" in t for t in reviews)


@pytest.mark.parametrize("code", [0, 1, 2])
def test_the_gate_fails_on_every_nonzero_lint_exit(tmp_path, code):
    gate = stand_in(tmp_path, lint_exit=code)
    assert run_gate(gate, tmp_path / "out").returncode == code


REFUSED = 2  # the exit code for an output path the script will not use


def test_a_non_empty_output_directory_is_refused_and_left_alone(tmp_path):
    gate = stand_in(tmp_path)
    out = tmp_path / "keep"
    out.mkdir()
    (out / "mine.txt").write_text("keep", encoding="utf-8")
    assert run_gate(gate, out).returncode == REFUSED
    assert (out / "mine.txt").read_text(encoding="utf-8") == "keep"
    assert not (tmp_path / "lint-argv").exists()


def test_a_regular_file_as_the_output_path_is_refused_and_left_alone(tmp_path):
    gate = stand_in(tmp_path)
    out = tmp_path / "file"
    out.write_text("keep", encoding="utf-8")
    assert run_gate(gate, out).returncode == REFUSED
    assert out.read_text(encoding="utf-8") == "keep"


@pytest.mark.skipif(os.geteuid() == 0, reason="root reads any directory, so none is unreadable")
def test_an_unreadable_output_directory_is_refused_and_its_files_survive(tmp_path):
    gate = stand_in(tmp_path)
    out = tmp_path / "unreadable"
    out.mkdir()
    # A file with the name of a render: a run that got past the guard would truncate it.
    victim = out / "write-minimal.md"
    victim.write_text("keep", encoding="utf-8")
    out.chmod(0o300)  # write and search, but no read: `ls` fails, writes still work
    try:
        assert run_gate(gate, out).returncode == REFUSED
    finally:
        out.chmod(0o700)
    assert victim.read_text(encoding="utf-8") == "keep"
    assert not (tmp_path / "lint-argv").exists()


def test_a_dangling_symlink_as_the_output_path_is_refused_and_not_followed(tmp_path):
    gate = stand_in(tmp_path)
    target = tmp_path / "nowhere"
    out = tmp_path / "link"
    out.symlink_to(target)
    assert run_gate(gate, out).returncode == REFUSED
    assert not target.exists()
    assert out.is_symlink()


def test_a_symlink_to_a_directory_is_refused_and_the_directory_stays_empty(tmp_path):
    gate = stand_in(tmp_path)
    real = tmp_path / "real"
    real.mkdir()
    out = tmp_path / "link"
    out.symlink_to(real, target_is_directory=True)
    assert run_gate(gate, out).returncode == REFUSED
    assert list(real.iterdir()) == []


def test_a_file_that_appears_in_the_output_directory_during_the_run_is_not_truncated(
    tmp_path,
):
    gate = stand_in(tmp_path)
    out = tmp_path / "out"
    racer = out / "review-update-self.md"
    result = run_gate(gate, out, {"RACE_FILE": str(racer)})
    assert result.returncode == 1
    assert racer.read_text(encoding="utf-8") == "mine\n"


@pytest.mark.parametrize("make_dir", [False, True])
def test_a_missing_or_empty_output_directory_is_used(tmp_path, make_dir):
    gate = stand_in(tmp_path)
    out = tmp_path / "out"
    if make_dir:
        out.mkdir()
    assert run_gate(gate, out).returncode == 0
    assert (out / "write-minimal.md").is_file()
