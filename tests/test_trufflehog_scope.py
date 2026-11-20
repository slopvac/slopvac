"""The secret scan fetches only the commits it scans, and each scan has a range.

A full-history checkout once stalled past the job timeout before any scan began.
The fetch script is run for real against throwaway repositories.
"""

from __future__ import annotations

import os
import shutil
import subprocess
from pathlib import Path

import pytest
from test_merge_group_ci import MERGE_GROUP, PULL_REQUEST, ctx, evaluate, load

ZEROS = "0" * 40
PUSH = {
    "event_name": "push",
    "ref": "refs/heads/main",
    "event": {"before": "1" * 40, "after": "2" * 40},
}
PR_WITH_SHAS = {
    **PULL_REQUEST,
    "event": {"pull_request": {"base": {"sha": "a" * 40}, "head": {"sha": "d" * 40}}},
}

JOB = load("security.yml")["jobs"]["trufflehog"]
STEPS = {s["name"]: s for s in JOB["steps"]}
FETCH = STEPS["Fetch the commits under test"]
RANGE_OUTPUTS = {
    "base": "${{ steps.range.outputs.base }}",
    "head": "${{ steps.range.outputs.head }}",
}


def test_checkout_is_shallow_and_keeps_no_credentials():
    checkout = STEPS["Checkout"]["with"]
    assert "fetch-depth" not in checkout
    assert checkout["persist-credentials"] == "false"


def test_fetch_step_is_bounded_deepening_and_tag_free():
    script = FETCH["run"]
    assert FETCH["id"] == "range"
    assert FETCH["timeout-minutes"] == "11"
    assert JOB["timeout-minutes"] == "15"
    assert "timeout 300 git fetch --no-tags" in script
    assert "--depth=" in script
    assert "--tags" not in script.replace("--no-tags", "")
    assert "${{" not in script


@pytest.mark.parametrize(
    ("env_name", "context", "expected"),
    [
        ("PR_BASE", PR_WITH_SHAS, "a" * 40),
        ("PR_HEAD", PR_WITH_SHAS, "d" * 40),
        ("MG_BASE", MERGE_GROUP, "b" * 40),
        ("MG_HEAD", MERGE_GROUP, "c" * 40),
        ("PUSH_BEFORE", PUSH, "1" * 40),
        ("PUSH_AFTER", PUSH, "2" * 40),
    ],
)
def test_fetch_step_reads_the_range_of_each_event(env_name, context, expected):
    assert evaluate(FETCH["env"][env_name], ctx(context)) == expected


@pytest.mark.parametrize(
    "step", ["Scan pull request diff", "Scan merge queue entry", "Scan pushed commits"]
)
def test_each_range_scan_uses_the_verified_range(step):
    inputs = STEPS[step]["with"]
    assert {k: inputs[k] for k in ("base", "head")} == RANGE_OUTPUTS


def test_scheduled_and_manual_runs_keep_the_full_tree_scan():
    step = STEPS["Scan full tree"]
    assert "base" not in step["with"]
    assert "schedule" in step["if"]
    assert "workflow_dispatch" in step["if"]
    assert all(
        "schedule" not in STEPS[n].get("if", "")
        for n in ("Scan pull request diff", "Scan merge queue entry")
    )


# --- the fetch script, run against real repositories -------------------------

GIT = shutil.which("git")
pytestmark_git = pytest.mark.skipif(GIT is None, reason="git is required")

SHIMS = """#!/usr/bin/env bash
# timeout: GNU timeout is missing on macOS. NOOP_FETCH makes a fetch succeed
# without transferring anything; FAIL_FETCHES makes the first N fetches fail.
shift
if [[ "$1" == git && "$2" == fetch ]]; then
  if [[ -n "${NOOP_FETCH:-}" ]]; then exit 0; fi
  if [[ -n "${FAIL_FETCHES:-}" ]]; then
    seen=$(cat "$FETCH_COUNT" 2>/dev/null || echo 0)
    echo $((seen + 1)) > "$FETCH_COUNT"
    if (( seen < FAIL_FETCHES )); then echo "fetch stalled" >&2; exit 124; fi
  fi
fi
exec "$@"
"""
GIT_SHIM = """#!/usr/bin/env bash
if [[ -n "${FORCED_MB:-}" && "$1" == merge-base ]]; then echo "$FORCED_MB"; exit 0; fi
exec "$REAL_GIT" "$@"
"""


class Lab:
    def __init__(self, root: Path) -> None:
        self.root = root
        self.env = {
            **os.environ,
            "GIT_AUTHOR_NAME": "t",
            "GIT_AUTHOR_EMAIL": "t@example.com",
            "GIT_COMMITTER_NAME": "t",
            "GIT_COMMITTER_EMAIL": "t@example.com",
            "GIT_CONFIG_GLOBAL": os.devnull,
            "GIT_CONFIG_SYSTEM": os.devnull,
        }
        self.work = root / "work"
        self.origin = root / "origin.git"
        self.git("init", "-q", "-b", "main", str(self.work), cwd=root)
        self.git("init", "-q", "--bare", str(self.origin), cwd=root)
        self.git("remote", "add", "origin", self.origin.as_uri(), cwd=self.work)
        self.shims = root / "shims"
        self.shims.mkdir()
        (self.shims / "timeout").write_text(SHIMS)
        (self.shims / "git").write_text(GIT_SHIM)
        for shim in self.shims.iterdir():
            shim.chmod(0o755)

    def git(self, *args: str, cwd: Path | None = None) -> str:
        done = subprocess.run(
            [GIT, *args],
            cwd=cwd or self.work,
            env=self.env,
            capture_output=True,
            text=True,
            check=True,
        )
        return done.stdout.strip()

    def commit(self, count: int = 1) -> list[str]:
        """Append empty commits to the current branch in one process."""
        ref = self.git("symbolic-ref", "HEAD")
        tip = subprocess.run(
            [GIT, "rev-parse", "-q", "--verify", ref],
            cwd=self.work,
            env=self.env,
            capture_output=True,
            text=True,
        ).stdout.strip()
        self.serial = getattr(self, "serial", 0)
        stream = []
        for i in range(count):
            self.serial += 1
            stream.append(
                f"commit {ref}\nmark :{i + 1}\ncommitter t <t@example.com> "
                f"{1_700_000_000 + self.serial} +0000\ndata <<EOM\nc{self.serial}\nEOM\n"
            )
            if i:
                stream.append(f"from :{i}\n")
            elif tip:
                stream.append(f"from {tip}\n")
        marks = self.root / "marks"
        subprocess.run(
            [GIT, "fast-import", "--quiet", f"--export-marks={marks}"],
            cwd=self.work,
            env=self.env,
            input="\n".join(stream),
            text=True,
            check=True,
        )
        pairs = sorted(line.split() for line in marks.read_text().splitlines())
        return [sha for _, sha in sorted(pairs, key=lambda p: int(p[0][1:]))]

    def publish(self) -> None:
        self.git("push", "-q", "--force", "origin", "--all")

    def checkout_clone(self, sha: str, depth: int = 1) -> Path:
        """What actions/checkout leaves behind: a shallow clone at one commit."""
        clone = self.root / f"clone-{sha[:8]}-{depth}"
        self.git("init", "-q", str(clone), cwd=self.root)
        self.git("remote", "add", "origin", self.origin.as_uri(), cwd=clone)
        self.git("fetch", "-q", f"--depth={depth}", "origin", sha, cwd=clone)
        self.git("checkout", "-q", sha, cwd=clone)
        return clone

    def run(self, clone: Path, event: str, base: str, head: str, **extra: str):
        out = self.root / "github_output"
        out.write_text("")
        pairs = {
            "pull_request": ("PR_BASE", "PR_HEAD"),
            "merge_group": ("MG_BASE", "MG_HEAD"),
            "push": ("PUSH_BEFORE", "PUSH_AFTER"),
        }.get(event, ("UNUSED_BASE", "UNUSED_HEAD"))
        env = {
            **self.env,
            "PATH": f"{self.shims}:{self.env['PATH']}",
            "REAL_GIT": GIT,
            "GITHUB_OUTPUT": str(out),
            "FETCH_COUNT": str(self.root / "fetch-count"),
            "EVENT_NAME": event,
            pairs[0]: base,
            pairs[1]: head,
            **extra,
        }
        done = subprocess.run(
            ["bash", "-c", FETCH["run"]],
            cwd=clone,
            env=env,
            capture_output=True,
            text=True,
        )
        outputs = dict(line.split("=", 1) for line in out.read_text().split())
        return done, outputs

    def range_is_complete(self, clone: Path, base: str, head: str) -> list[str]:
        shas = self.git("rev-list", f"{base}..{head}", cwd=clone).split()
        shallow = clone / ".git" / "shallow"
        cut = set(shallow.read_text().split()) if shallow.exists() else set()
        assert not cut & set(shas)
        return shas


@pytest.fixture
def lab(tmp_path: Path) -> Lab:
    if GIT is None:
        pytest.skip("git is required")
    return Lab(tmp_path)


def test_push_of_several_commits_scans_before_to_after(lab: Lab):
    (before,) = lab.commit()
    pushed = lab.commit(3)
    lab.publish()
    clone = lab.checkout_clone(pushed[-1])
    done, outputs = lab.run(clone, "push", before, pushed[-1])
    assert done.returncode == 0, done.stderr
    assert outputs == {"base": before, "head": pushed[-1]}
    assert set(lab.range_is_complete(clone, before, pushed[-1])) == set(pushed)


def test_merge_group_of_two_pull_requests_scans_from_the_main_tip(lab: Lab):
    lab.commit(2)
    main_tip = lab.commit()[0]
    first = lab.commit(2)
    second = lab.commit(2)
    lab.publish()
    clone = lab.checkout_clone(second[-1])
    done, outputs = lab.run(clone, "merge_group", main_tip, second[-1])
    assert done.returncode == 0, done.stderr
    assert outputs == {"base": main_tip, "head": second[-1]}
    assert set(lab.range_is_complete(clone, main_tip, second[-1])) == {*first, *second}


def test_pull_request_scans_from_the_merge_base_not_the_moved_base_tip(lab: Lab):
    lab.commit(2)
    branch_point = lab.commit()[0]
    lab.git("checkout", "-q", "-b", "topic")
    topic = lab.commit(2)
    lab.git("checkout", "-q", "main")
    # The base branch moved 60 commits past the branch point: deeper than the
    # first deepening step, so the merge base needs a second fetch.
    base_tip = lab.commit(60)[-1]
    lab.publish()
    clone = lab.checkout_clone(topic[-1])
    done, outputs = lab.run(clone, "pull_request", base_tip, topic[-1])
    assert done.returncode == 0, done.stderr
    assert outputs == {"base": branch_point, "head": topic[-1]}
    assert set(lab.range_is_complete(clone, branch_point, topic[-1])) == set(topic)


def test_force_pushed_branch_with_an_unavailable_old_tip_scans_all_history(lab: Lab):
    lab.commit(2)
    lab.commit(1)
    new_tip = lab.commit(1)[0]
    lab.publish()
    clone = lab.checkout_clone(new_tip)
    gone = "f" * 40
    done, outputs = lab.run(clone, "push", gone, new_tip)
    assert done.returncode == 0, done.stderr
    assert outputs == {"base": "", "head": ""}
    assert not (clone / ".git" / "shallow").exists()


def test_a_shallow_boundary_inside_the_range_fails_the_job(lab: Lab):
    lab.commit(1)
    base = lab.commit()[0]
    pushed = lab.commit(3)
    lab.publish()
    # Two commits of history, a base fetched on its own, and a fetch that
    # transfers nothing: the base looks present but the range is cut.
    clone = lab.checkout_clone(pushed[-1], depth=2)
    lab.git("fetch", "-q", "--depth=1", "origin", base, cwd=clone)
    done, outputs = lab.run(
        clone, "push", base, pushed[-1], NOOP_FETCH="1", FORCED_MB=base
    )
    assert done.returncode == 1
    assert "incomplete" in done.stdout
    assert outputs == {}


def test_fetch_step_rejects_a_range_that_is_not_40_hex(lab: Lab):
    clone = lab.work
    done, outputs = lab.run(clone, "pull_request", "--upload-pack=evil", "d" * 40)
    assert done.returncode == 1
    assert "not a 40-hex commit id" in done.stdout
    assert outputs == {}


def test_a_failed_fetch_is_retried_and_the_range_is_still_complete(lab: Lab):
    (before,) = lab.commit()
    pushed = lab.commit(3)
    lab.publish()
    clone = lab.checkout_clone(pushed[-1])
    done, outputs = lab.run(clone, "push", before, pushed[-1], FAIL_FETCHES="1")
    assert done.returncode == 0, done.stderr
    assert "deepening" in done.stdout
    assert outputs == {"base": before, "head": pushed[-1]}
    assert set(lab.range_is_complete(clone, before, pushed[-1])) == set(pushed)


def test_the_job_fails_when_every_fetch_fails(lab: Lab):
    (before,) = lab.commit()
    (tip,) = lab.commit()
    lab.publish()
    clone = lab.checkout_clone(tip)
    done, outputs = lab.run(clone, "pull_request", before, tip, FAIL_FETCHES="99")
    assert done.returncode != 0
    assert outputs == {}


def test_the_job_fails_when_a_fetch_leaves_an_endpoint_missing(lab: Lab):
    (before,) = lab.commit()
    (tip,) = lab.commit()
    lab.publish()
    clone = lab.checkout_clone(tip)
    done, outputs = lab.run(clone, "pull_request", before, tip, NOOP_FETCH="1")
    assert done.returncode == 1
    assert "no merge base" in done.stdout
    assert outputs == {}


def test_a_push_that_creates_a_branch_scans_the_full_history(lab: Lab):
    shas = lab.commit(4)
    lab.publish()
    clone = lab.checkout_clone(shas[-1])
    done, outputs = lab.run(clone, "push", ZEROS, shas[-1])
    assert done.returncode == 0, done.stderr
    assert outputs == {"base": "", "head": ""}
    assert lab.git("rev-list", "--count", "HEAD", cwd=clone) == "4"


@pytest.mark.parametrize("event", ["schedule", "workflow_dispatch"])
def test_scheduled_and_manual_runs_fetch_the_full_history(lab: Lab, event: str):
    shas = lab.commit(5)
    lab.publish()
    clone = lab.checkout_clone(shas[-1])
    done, outputs = lab.run(clone, event, "", "")
    assert done.returncode == 0, done.stderr
    assert lab.git("rev-list", "--count", "HEAD", cwd=clone) == "5"
    assert outputs == {"base": "", "head": ""}


TRUFFLEHOG = os.environ.get("TRUFFLEHOG_BIN") or shutil.which("trufflehog")
SECRET = "ghp_" + "aB3dE5fG7hJ9kL1mN3pQ5rS7tU9vW1xY3zA5"


def build_merge_topology(lab: Lab, main_moves_on: int) -> dict:
    """A long topic branch that merges an updated main before its last commit.

    The merge exposes main's tip as a merge base within 50 commits of the head,
    while the topic commits below it, the secret among them, are still cut off.
    """
    lab.commit(2)
    lab.git("checkout", "-q", "-b", "topic")
    early = lab.commit(30)
    leak = lab.work / "leak.txt"
    leak.write_text(f"token = {SECRET}\n")
    lab.git("add", "leak.txt")
    lab.git("commit", "-q", "-m", "add secret")
    secret_commit = lab.git("rev-parse", "HEAD")
    late = lab.commit(30)
    lab.git("checkout", "-q", "main")
    main_tip = lab.commit(5)[-1]
    lab.git("checkout", "-q", "topic")
    lab.git("merge", "-q", "--no-ff", "-m", "merge main", "main")
    merge = lab.git("rev-parse", "HEAD")
    lab.git("rm", "-q", "leak.txt")
    lab.git("commit", "-q", "-m", "remove secret")
    head = lab.git("rev-parse", "HEAD")
    lab.git("checkout", "-q", "main")
    base_tip = lab.commit(main_moves_on)[-1] if main_moves_on else main_tip
    lab.publish()
    return {
        "main_tip": main_tip,
        "base_tip": base_tip,
        "head": head,
        "topic": {*early, secret_commit, *late},
        "merge": merge,
    }


@pytest.mark.parametrize(
    "main_moves_on", [0, 3], ids=["base-is-merged-tip", "base-moved-on"]
)
def test_pull_request_with_a_merge_of_main_scans_every_topic_commit(
    lab: Lab, main_moves_on: int
):
    topo = build_merge_topology(lab, main_moves_on)
    assert len(topo["topic"]) > 50
    clone = lab.checkout_clone(topo["head"])
    done, outputs = lab.run(clone, "pull_request", topo["base_tip"], topo["head"])
    assert done.returncode == 0, done.stderr
    # The merge base advanced to the main commit the topic merged.
    assert outputs == {"base": topo["main_tip"], "head": topo["head"]}
    # Depth 50 exposed that merge base but not the old topic commits; only the
    # next deepening step completed the range.
    assert "at depth 50; deepening" in done.stdout
    assert "at depth 200" not in done.stdout
    in_range = set(lab.range_is_complete(clone, topo["main_tip"], topo["head"]))
    assert in_range == {*topo["topic"], topo["merge"], topo["head"]}
    # Main's own history is not part of the scanned range.
    assert not in_range & set(lab.git("rev-list", topo["main_tip"]).split())


@pytest.mark.skipif(TRUFFLEHOG is None, reason="no trufflehog binary (TRUFFLEHOG_BIN)")
def test_trufflehog_finds_a_secret_below_a_merge_of_main(lab: Lab):
    topo = build_merge_topology(lab, 3)
    clone = lab.checkout_clone(topo["head"])
    done, outputs = lab.run(clone, "pull_request", topo["base_tip"], topo["head"])
    assert done.returncode == 0, done.stderr
    assert SECRET in trufflehog_scan(clone, outputs["base"], outputs["head"])
    assert SECRET not in trufflehog_scan(clone, outputs["head"], outputs["head"])


def trufflehog_scan(clone: Path, base: str, head: str) -> str:
    return subprocess.run(
        [
            TRUFFLEHOG,
            "git",
            clone.as_uri(),
            "--since-commit",
            base,
            "--branch",
            head,
            "--no-verification",
            "--no-update",
            "--json",
        ],
        capture_output=True,
        text=True,
        check=True,
    ).stdout


@pytest.mark.skipif(TRUFFLEHOG is None, reason="no trufflehog binary (TRUFFLEHOG_BIN)")
def test_trufflehog_finds_a_secret_added_and_deleted_inside_the_range(lab: Lab):
    (before,) = lab.commit()
    leak = lab.work / "leak.txt"
    leak.write_text(f"token = {SECRET}\n")
    lab.git("add", "leak.txt")
    lab.git("commit", "-q", "-m", "add")
    leak.unlink()
    lab.git("commit", "-q", "-a", "-m", "remove")
    head = lab.commit()[0]
    lab.publish()
    clone = lab.checkout_clone(head)
    done, outputs = lab.run(clone, "pull_request", before, head)
    assert done.returncode == 0, done.stderr

    def scan(base: str) -> str:
        return subprocess.run(
            [
                TRUFFLEHOG,
                "git",
                clone.as_uri(),
                "--since-commit",
                base,
                "--branch",
                outputs["head"],
                "--no-verification",
                "--no-update",
                "--json",
            ],
            capture_output=True,
            text=True,
            check=True,
        ).stdout

    assert SECRET in scan(outputs["base"])
    assert SECRET not in scan(outputs["head"])
