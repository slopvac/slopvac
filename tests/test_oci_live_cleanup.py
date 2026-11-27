"""scripts/ci/oci_live_cleanup.py deletes only stale versions of the CI package.

Selection is tested on plain values. The command line runs against a stand-in `gh`, the
process boundary the script owns, which serves a fixed version list and records each call.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
from datetime import UTC, datetime, timedelta
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "ci" / "oci_live_cleanup.py"
sys.path.insert(0, str(SCRIPT.parent))

import oci_live_cleanup as cleanup  # noqa: E402

NOW = datetime(2026, 11, 27, 12, 0, tzinfo=UTC)

# Serves $GH_LIST, the raw pages `gh api --paginate` prints, for a read and records every
# call; `-X DELETE` fails for $GH_FAIL_ID.
GH_STUB = """#!/usr/bin/env python3
import os, sys
from pathlib import Path
args = sys.argv[1:]
Path(os.environ["GH_CALLS"]).open("a").write(" ".join(args) + "\\n")
if "DELETE" in args:
    fail = os.environ.get("GH_FAIL_ID")
    sys.exit(1 if fail and args[-1].endswith("/" + fail) else 0)
if os.environ.get("GH_LIST_EXIT"):
    sys.exit(int(os.environ["GH_LIST_EXIT"]))
sys.stdout.write(os.environ["GH_LIST"])
"""


def version(version_id: int, age_days: float, *tags: str) -> cleanup.Version:
    return cleanup.Version(version_id, NOW - timedelta(days=age_days), tags)


def raw(version_id, age_days, *tags, **overrides):
    """One version object as the Packages API returns it, aged from the fixed clock."""
    created = (NOW - timedelta(days=age_days)).isoformat().replace("+00:00", "Z")
    entry = {
        "id": version_id,
        "created_at": created,
        "metadata": {"package_type": "container", "container": {"tags": list(tags)}},
    }
    entry.update(overrides)
    return entry


def pages(*page_lists):
    """What `gh api --paginate` prints: one JSON array per page, back to back."""
    return "\n".join(json.dumps(page) for page in page_lists) + "\n"


def test_only_versions_older_than_the_cutoff_are_selected():
    versions = [version(1, 20, "ci-1-1-a"), version(2, 14.5), version(3, 13.5), version(4, 0)]
    selected = cleanup.select_stale(versions, NOW, 14, "ci-9-")
    assert [v.id for v in selected] == [1, 2]


def test_a_version_with_a_keep_prefix_tag_survives_even_when_old():
    versions = [version(1, 30, "ci-9-1-a", "other"), version(2, 30, "ci-8-1-a")]
    selected = cleanup.select_stale(versions, NOW, 14, "ci-9-")
    assert [v.id for v in selected] == [2]


def test_an_untagged_old_version_is_selected():
    assert [v.id for v in cleanup.select_stale([version(5, 40)], NOW, 14, "ci-9-")] == [5]


@pytest.fixture
def gh_env(tmp_path):
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    gh = bin_dir / "gh"
    gh.write_text(GH_STUB)
    gh.chmod(0o755)
    calls = tmp_path / "calls"
    env = {
        **os.environ,
        "PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}",
        "GH_CALLS": str(calls),
    }
    return env, calls


def run(env, *args):
    return subprocess.run(
        [sys.executable, "-I", str(SCRIPT), "--now", NOW.isoformat(), *args],
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )


def deletes(calls: Path) -> list[str]:
    if not calls.exists():
        return []
    return [
        entry.rsplit("/", 1)[1] for entry in calls.read_text().splitlines() if "DELETE" in entry
    ]


def test_the_command_deletes_the_stale_versions_and_keeps_the_rest(gh_env):
    env, calls = gh_env
    env["GH_LIST"] = pages([raw(1, 30), raw(2, 1), raw(3, 30, "ci-9-1-a")])
    done = run(env, "--keep-prefix", "ci-9-")
    assert done.returncode == 0, done.stderr
    assert deletes(calls) == ["1"]


def test_a_version_one_day_old_is_kept_whatever_the_wall_clock_says(gh_env):
    env, calls = gh_env
    env["GH_LIST"] = pages([raw(1, 1)])
    done = run(env, "--keep-prefix", "ci-9-")
    assert done.returncode == 0, done.stderr
    assert deletes(calls) == []


def test_every_page_of_the_listing_is_read(gh_env):
    env, calls = gh_env
    env["GH_LIST"] = pages([raw(1, 30)], [raw(2, 30)], [])
    done = run(env, "--keep-prefix", "ci-9-")
    assert done.returncode == 0, done.stderr
    assert deletes(calls) == ["1", "2"]


def test_a_dry_run_deletes_nothing(gh_env):
    env, calls = gh_env
    env["GH_LIST"] = pages([raw(1, 30)])
    done = run(env, "--keep-prefix", "ci-9-", "--dry-run")
    assert done.returncode == 0, done.stderr
    assert "would delete version 1" in done.stdout
    assert deletes(calls) == []


def test_a_failed_delete_is_reported_after_the_others_are_tried(gh_env):
    env, calls = gh_env
    env["GH_LIST"] = pages([raw(1, 30), raw(2, 30)])
    env["GH_FAIL_ID"] = "1"
    done = run(env, "--keep-prefix", "ci-9-")
    assert done.returncode == 1
    assert deletes(calls) == ["1", "2"]


def test_a_failed_listing_is_incomplete_and_deletes_nothing(gh_env):
    env, calls = gh_env
    env["GH_LIST_EXIT"] = "1"
    done = run(env, "--keep-prefix", "ci-9-")
    assert done.returncode == 2
    assert deletes(calls) == []


def without(entry, *path):
    """A copy of `entry` with the key at `path` removed."""
    copy = json.loads(json.dumps(entry))
    node = copy
    for key in path[:-1]:
        node = node[key]
    del node[path[-1]]
    return copy


# Each shape is a version that is old and unprotected on its face. A reader that read the
# missing or broken tags as "no tags" would delete it, though a protected tag may hide there.
MALFORMED = [
    pytest.param(without(raw(7, 30), "metadata"), id="no-metadata"),
    pytest.param(raw(7, 30, metadata=None), id="null-metadata"),
    pytest.param(raw(7, 30, metadata="x"), id="metadata-not-an-object"),
    pytest.param(without(raw(7, 30), "metadata", "container"), id="no-container"),
    pytest.param(
        raw(7, 30, metadata={"container": None}), id="null-container"
    ),
    pytest.param(
        raw(7, 30, metadata={"container": []}), id="container-not-an-object"
    ),
    pytest.param(
        without(raw(7, 30), "metadata", "container", "tags"), id="no-tags"
    ),
    pytest.param(
        raw(7, 30, metadata={"container": {"tags": None}}), id="null-tags"
    ),
    pytest.param(
        raw(7, 30, metadata={"container": {"tags": "ci-9-1-a"}}), id="tags-not-a-list"
    ),
    pytest.param(
        raw(7, 30, metadata={"container": {"tags": ["ok", 5]}}), id="tag-not-a-string"
    ),
    pytest.param(without(raw(7, 30), "created_at"), id="no-created-at"),
    pytest.param(without(raw(7, 30), "id"), id="no-id"),
    pytest.param(raw(True, 30), id="boolean-id"),
    pytest.param(raw(7, 30, created_at="yesterday"), id="unreadable-created-at"),
    pytest.param(raw(7, 30, created_at="2026-01-01T00:00:00"), id="created-at-without-zone"),
    pytest.param("not-an-object", id="entry-not-an-object"),
]


@pytest.mark.parametrize("entry", MALFORMED)
def test_a_malformed_version_fails_the_run_closed_and_deletes_nothing(gh_env, entry):
    env, calls = gh_env
    # A valid stale version sits in the same listing, so a delete would show.
    env["GH_LIST"] = pages([raw(1, 30), entry])
    done = run(env, "--keep-prefix", "ci-9-")
    assert done.returncode == 2
    assert deletes(calls) == []
    assert done.stderr.startswith("error: ")


@pytest.mark.parametrize(
    "text",
    [
        pytest.param("", id="empty-output"),
        pytest.param("not json\n", id="not-json"),
        pytest.param('{"id": 1}\n', id="page-is-an-object"),
        pytest.param(pages([raw(1, 30)]) + "[", id="truncated-page"),
        pytest.param(pages([raw(1, 30)], "x"), id="second-page-is-a-string"),
    ],
)
def test_a_malformed_listing_fails_the_run_closed_and_deletes_nothing(gh_env, text):
    env, calls = gh_env
    env["GH_LIST"] = text
    done = run(env, "--keep-prefix", "ci-9-")
    assert done.returncode == 2
    assert deletes(calls) == []


@pytest.mark.parametrize(
    "args",
    [
        pytest.param(["--package", "slopvac-revocations", "--keep-prefix", "ci-9-"], id="other-package"),
        pytest.param(["--days", "0", "--keep-prefix", "ci-9-"], id="zero-days"),
        pytest.param(["--keep-prefix", "../x"], id="bad-prefix"),
        pytest.param(["--now", "2026-11-27T12:00:00", "--keep-prefix", "ci-9-"], id="naive-clock"),
    ],
)
def test_invalid_arguments_stop_the_run_before_any_request(gh_env, args):
    env, calls = gh_env
    done = run(env, *args)
    assert done.returncode == 2
    assert not calls.exists()
