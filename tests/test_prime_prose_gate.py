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
        prime = 'echo "# Prime: stand-in"; exit 0'
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


def run_gate(binary: Path, out: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["bash", str(SCRIPT), str(binary), str(CONFIG), str(out)],
        capture_output=True,
        text=True,
        check=False,
    )


@pytest.mark.parametrize("topic", sorted(CONDITIONS))
def test_every_condition_in_a_topic_is_a_branch_the_gate_knows(topic):
    text = (TOPICS / topic).read_text(encoding="utf-8")
    assert conditions_of(text) == set(CONDITIONS[topic])


def test_the_conditions_helper_sees_a_new_condition_on_a_known_variable():
    text = (TOPICS / "review.md.j2").read_text(encoding="utf-8")
    added = text + '{% if review_type == "rewrite" %}\nx\n{% endif %}\n'
    assert conditions_of(added) - set(CONDITIONS["review.md.j2"]) == {
        'review_type == "rewrite"'
    }


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


def test_a_non_empty_output_directory_is_refused_and_left_alone(tmp_path):
    gate = stand_in(tmp_path)
    out = tmp_path / "keep"
    out.mkdir()
    (out / "mine.txt").write_text("keep", encoding="utf-8")
    assert run_gate(gate, out).returncode != 0
    assert (out / "mine.txt").read_text(encoding="utf-8") == "keep"
    assert not (tmp_path / "lint-argv").exists()


@pytest.mark.parametrize("make_dir", [False, True])
def test_a_missing_or_empty_output_directory_is_used(tmp_path, make_dir):
    gate = stand_in(tmp_path)
    out = tmp_path / "out"
    if make_dir:
        out.mkdir()
    assert run_gate(gate, out).returncode == 0
    assert (out / "write-minimal.md").is_file()
