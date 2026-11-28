"""scripts/ci/gate_prime_prose.sh renders every prose branch of `prime` and lints it.

The script is the CI gate for the guidance text. Two things must hold: it renders each
conditional branch of the `write` and `review` topics, and it fails on every nonzero lint
exit, including exit 2 (the run was incomplete).
"""

from __future__ import annotations

import re
import stat
import subprocess
from pathlib import Path

import pytest

from real_binary import FIXTURES, ROOT, slopvac_binary  # noqa: F401

SCRIPT = ROOT / "scripts" / "ci" / "gate_prime_prose.sh"
CONFIG = FIXTURES / "slopvac-v3.toml"
TOPICS = ROOT / "crates" / "slopvac-agents" / "topics"

# Template variables that the script's renders vary. A condition on any other variable is a
# new branch, so the script and this set must change together.
COVERED_VARIABLES = {"review_depth", "review_type", "level", "review"}
# `prime` never sets `review_approval`: only the library API does, so no CLI render shows
# its `auto` branch.
UNREACHABLE_FROM_THE_CLI = {"review_approval"}


def condition_variables(text: str) -> set[str]:
    names: set[str] = set()
    for cond in re.findall(r"{%-?\s*(?:if|elif)\s+(.*?)\s*-?%}", text):
        names |= set(re.findall(r"[a-z_]+", re.sub(r'"[^"]*"', "", cond)))
    return names - {"and", "or", "not"}


def run_gate(binary: Path, out: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["bash", str(SCRIPT), str(binary), str(CONFIG), str(out)],
        capture_output=True,
        text=True,
        check=False,
    )


def test_every_condition_in_the_topics_is_on_a_variable_the_gate_varies():
    seen: set[str] = set()
    for topic in ("write.md.j2", "review.md.j2"):
        seen |= condition_variables((TOPICS / topic).read_text(encoding="utf-8"))
    assert seen <= COVERED_VARIABLES | UNREACHABLE_FROM_THE_CLI


def review_cell(text: str) -> tuple[str, str]:
    step = re.search(r"## Steps for (\w+)", text)
    depth = re.search(r"Depth: `(\w+)`", text)
    return (step.group(1) if step else "auto", depth.group(1) if depth else "subagent")


def test_the_gate_renders_each_conditional_branch(slopvac_binary, tmp_path):  # noqa: F811
    result = run_gate(slopvac_binary, tmp_path / "out")
    assert result.returncode == 0, result.stdout + result.stderr
    docs = [p.read_text(encoding="utf-8") for p in (tmp_path / "out").glob("*.md")]
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
        (kind, depth)
        for kind in ("auto", "mechanical", "update", "rewrite")
        for depth in ("self", "subagent", "adversary")
    }
    assert any("## Verdict" not in t for t in reviews)
    assert any("## Verdict" in t for t in reviews)


@pytest.mark.parametrize("code", [0, 1, 2])
def test_the_gate_fails_on_every_nonzero_lint_exit(tmp_path, code):
    fake = tmp_path / "slopvac"
    fake.write_text(
        '#!/bin/sh\nif [ "$1" = prime ]; then echo "# Prime: x"; exit 0; fi\n'
        f"exit {code}\n",
        encoding="utf-8",
    )
    fake.chmod(fake.stat().st_mode | stat.S_IXUSR)
    assert run_gate(fake, tmp_path / "out").returncode == code
