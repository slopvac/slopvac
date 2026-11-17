"""The revocation-list publisher and its workflow.

The script's output is also verified by the real verifier in
``crates/slopvac-plugins/tests/revoke.rs``.
"""

from __future__ import annotations

import base64
import importlib.util
import json
import os
import shutil
import subprocess
import sys
from datetime import UTC, datetime
from pathlib import Path

import pytest
import yaml

ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = ROOT / "scripts"
WORKFLOW = ROOT / ".github" / "workflows" / "revocations.yml"
sys.path.insert(0, str(SCRIPTS))
SPEC = importlib.util.spec_from_file_location(
    "revocation_list", SCRIPTS / "revocation_list.py"
)
assert SPEC is not None and SPEC.loader is not None
lists = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = lists
SPEC.loader.exec_module(lists)

NOW = datetime(2026, 11, 17, 5, 17, 42, 123456, tzinfo=UTC)
NEEDS_MINISIGN = pytest.mark.skipif(
    shutil.which("minisign") is None, reason="needs the minisign binary"
)


def test_render_is_the_canonical_empty_list() -> None:
    expected = (
        '{"digests":[],"expires":"2026-12-17T05:17:42Z","issued":"2026-11-17T05:17:42Z",'
        '"keys":[],"schema_version":1,"type":"revocations"}'
    )
    assert lists.render(NOW).decode() == expected


def test_render_honours_a_shorter_window() -> None:
    assert b'"expires":"2026-11-24T05:17:42Z"' in lists.render(NOW, days=7)


@pytest.mark.parametrize("days", [31, 365, 0, -1])
def test_render_refuses_a_window_outside_one_to_thirty_days(days: int) -> None:
    with pytest.raises(lists.SigningError, match="--days"):
        lists.render(NOW, days=days)


def _doc(**changes: object) -> dict[str, object]:
    base: dict[str, object] = json.loads(lists.render(NOW))
    return {**base, **changes}


@pytest.mark.parametrize(
    ("data", "why"),
    [
        (b'{"schema_version": 1}', "exactly the fields"),
        (json.dumps(_doc()).encode(), "not canonical"),
        (lists.canonical(_doc(type="rotation")), "version 1 revocation list"),
        (lists.canonical(_doc(schema_version=2)), "version 1 revocation list"),
        (lists.canonical(_doc(issued="2026-11-17")), "YYYY-MM-DD"),
        (lists.canonical(_doc(issued="2026-02-30T00:00:00Z")), "calendar"),
        (lists.canonical(_doc(expires="2026-11-17T05:17:42Z")), "at most 30 days"),
        (lists.canonical(_doc(expires="2026-12-18T05:17:43Z")), "at most 30 days"),
        (lists.canonical(_doc(keys=["c0e5e326b2f9b470"])), "keys"),
        (
            lists.canonical(_doc(keys=["C0E5E326B2F9B470", "A7628DAD436BB320"])),
            "sorted",
        ),
        (lists.canonical(_doc(digests=["sha256:abc"])), "digests"),
        (b"not json", "not JSON"),
    ],
    ids=[
        "missing-fields",
        "whitespace",
        "wrong-type",
        "wrong-schema",
        "date-only",
        "impossible-date",
        "zero-window",
        "window-over-30-days",
        "lowercase-key-id",
        "unsorted-keys",
        "short-digest",
        "not-json",
    ],
)
def test_check_list_refuses_what_the_cli_refuses(data: bytes, why: str) -> None:
    with pytest.raises(lists.SigningError, match=why):
        lists.check_list(data, NOW)


def test_check_list_accepts_exactly_thirty_days() -> None:
    data = lists.canonical(_doc(expires="2026-12-17T05:17:42Z"))
    lists.check_list(data, NOW)


def test_check_list_refuses_a_list_past_its_expiry() -> None:
    later = datetime(2026, 12, 17, 5, 17, 43, tzinfo=UTC)
    with pytest.raises(lists.SigningError, match="already expired"):
        lists.check_list(lists.render(NOW), later)


def test_check_list_refuses_a_list_over_one_mebibyte() -> None:
    digests = [f"sha256:{i:064x}" for i in range(17_000)]
    data = lists.canonical(_doc(digests=digests))
    assert len(data) > 1 << 20
    with pytest.raises(lists.SigningError, match="larger than"):
        lists.check_list(data, NOW)


def _keypair(tmp_path: Path) -> tuple[Path, Path]:
    secret, public = tmp_path / "throwaway.key", tmp_path / "throwaway.pub"
    subprocess.run(
        ["minisign", "-G", "-W", "-f", "-p", str(public), "-s", str(secret)],
        check=True,
        capture_output=True,
        stdin=subprocess.DEVNULL,
    )
    return secret, public


@NEEDS_MINISIGN
def test_build_signs_and_verify_accepts_it(tmp_path: Path) -> None:
    secret, public = _keypair(tmp_path)
    out = tmp_path / "out"
    lists.build(
        out,
        public,
        {"MINISIGN_SECRET_KEY": secret.read_text(), "RUNNER_TEMP": str(tmp_path)},
        NOW,
        30,
    )
    assert sorted(p.name for p in out.iterdir()) == [
        "revocations.json",
        "revocations.json.minisig",
    ]
    lists.verify(out, public, NOW)
    # The key file is gone from the temp directory.
    assert not list(tmp_path.glob("minisign-*"))


@NEEDS_MINISIGN
def test_verify_refuses_a_list_changed_after_signing(tmp_path: Path) -> None:
    secret, public = _keypair(tmp_path)
    out = tmp_path / "out"
    env = {"MINISIGN_SECRET_KEY": secret.read_text(), "RUNNER_TEMP": str(tmp_path)}
    lists.build(out, public, env, NOW, 30)
    (out / "revocations.json").write_bytes(lists.render(NOW, days=7))
    with pytest.raises(lists.SigningError, match="signature check failed"):
        lists.verify(out, public, NOW)


@NEEDS_MINISIGN
def test_verify_refuses_a_signature_by_another_key(tmp_path: Path) -> None:
    secret, public = _keypair(tmp_path)
    out = tmp_path / "out"
    env = {"MINISIGN_SECRET_KEY": secret.read_text(), "RUNNER_TEMP": str(tmp_path)}
    lists.build(out, public, env, NOW, 30)
    with pytest.raises(lists.SigningError, match="signature check failed"):
        lists.verify(out, lists.PUBKEY, NOW)


@NEEDS_MINISIGN
def test_verify_refuses_a_legacy_signature(tmp_path: Path) -> None:
    secret, public = _keypair(tmp_path)
    out = tmp_path / "out"
    out.mkdir()
    (out / "revocations.json").write_bytes(lists.render(NOW))
    subprocess.run(
        ["minisign", "-S", "-l", "-W", "-s", str(secret)]
        + ["-m", str(out / "revocations.json")]
        + ["-x", str(out / "revocations.json.minisig")],
        check=True,
        capture_output=True,
        stdin=subprocess.DEVNULL,
    )
    signature = (out / "revocations.json.minisig").read_text().splitlines()
    assert base64.b64decode(signature[1])[:2] == b"Ed"
    with pytest.raises(lists.SigningError, match="not a prehashed"):
        lists.verify(out, public, NOW)


def _signed(tmp_path: Path, name: str, issued: datetime, days: int = 30) -> Path:
    """A list issued at ``issued``, signed by the throwaway key of ``tmp_path``."""
    key, public = tmp_path / "throwaway.key", tmp_path / "throwaway.pub"
    if not key.exists():
        _keypair(tmp_path)
    out = tmp_path / name
    env = {"MINISIGN_SECRET_KEY": key.read_text(), "RUNNER_TEMP": str(tmp_path)}
    lists.build(out, public, env, issued, days)
    return out


EARLIER = datetime(2026, 11, 10, 5, 0, 0, tzinfo=UTC)
LATER = datetime(2026, 11, 17, 5, 0, 0, tzinfo=UTC)


@NEEDS_MINISIGN
def test_check_newer_accepts_a_strictly_newer_candidate(tmp_path: Path) -> None:
    candidate = _signed(tmp_path, "candidate", LATER)
    current = _signed(tmp_path, "current", EARLIER)
    lists.check_newer(candidate, current, tmp_path / "throwaway.pub", LATER)


@NEEDS_MINISIGN
@pytest.mark.parametrize(
    ("candidate_at", "current_at"),
    [(EARLIER, LATER), (LATER, LATER)],
    ids=["older", "equal"],
)
def test_check_newer_refuses_an_older_or_equal_candidate(
    tmp_path: Path, candidate_at: datetime, current_at: datetime
) -> None:
    candidate = _signed(tmp_path, "candidate", candidate_at)
    current = _signed(tmp_path, "current", current_at)
    with pytest.raises(lists.SigningError, match="not newer"):
        lists.check_newer(candidate, current, tmp_path / "throwaway.pub", LATER)


@NEEDS_MINISIGN
def test_check_newer_allows_the_first_publication(tmp_path: Path) -> None:
    candidate = _signed(tmp_path, "candidate", LATER)
    lists.check_newer(
        candidate, tmp_path / "nothing-published", tmp_path / "throwaway.pub", LATER
    )


@NEEDS_MINISIGN
def test_check_newer_replaces_a_lapsed_published_list(tmp_path: Path) -> None:
    current = _signed(tmp_path, "current", EARLIER, days=1)
    candidate = _signed(tmp_path, "candidate", LATER)
    lists.check_newer(candidate, current, tmp_path / "throwaway.pub", LATER)


@NEEDS_MINISIGN
def test_check_newer_refuses_a_published_list_with_a_bad_signature(
    tmp_path: Path,
) -> None:
    candidate = _signed(tmp_path, "candidate", LATER)
    current = _signed(tmp_path, "current", EARLIER)
    (current / "revocations.json").write_bytes(lists.render(EARLIER, days=7))
    with pytest.raises(lists.SigningError, match="signature check failed"):
        lists.check_newer(candidate, current, tmp_path / "throwaway.pub", LATER)


@NEEDS_MINISIGN
def test_check_newer_refuses_a_published_directory_without_a_list(
    tmp_path: Path,
) -> None:
    candidate = _signed(tmp_path, "candidate", LATER)
    (tmp_path / "empty").mkdir()
    with pytest.raises(lists.SigningError, match="is missing"):
        lists.check_newer(
            candidate, tmp_path / "empty", tmp_path / "throwaway.pub", LATER
        )


def test_build_without_a_key_fails_before_writing(tmp_path: Path) -> None:
    with pytest.raises(lists.SigningError, match="MINISIGN_SECRET_KEY is not set"):
        lists.build(tmp_path / "out", lists.PUBKEY, {}, NOW, 30)
    assert not (tmp_path / "out").exists()


def test_the_cli_prints_the_list_and_fails_cleanly(tmp_path: Path) -> None:
    ok = subprocess.run(
        [sys.executable, str(SCRIPTS / "revocation_list.py"), "render"]
        + ["--now", "2026-11-17T05:17:42Z"],
        capture_output=True,
        text=True,
        check=False,
    )
    assert ok.returncode == 0
    assert ok.stdout.startswith('{"digests":[],"expires":"2026-12-17T05:17:42Z"')
    bad = subprocess.run(
        [sys.executable, str(SCRIPTS / "revocation_list.py"), "render", "--days", "31"],
        capture_output=True,
        text=True,
        check=False,
    )
    assert bad.returncode == 1
    assert bad.stderr.startswith("error:")


# ---- The workflow ----


@pytest.fixture(scope="module")
def workflow() -> dict:
    # PyYAML reads the bare `on` key as boolean True.
    return yaml.safe_load(WORKFLOW.read_text(encoding="utf-8"))


def _text(job: dict) -> str:
    return yaml.safe_dump(job)


def test_triggers_are_a_weekly_schedule_and_manual_dispatch_only(
    workflow: dict,
) -> None:
    triggers = workflow[True]
    assert set(triggers) == {"schedule", "workflow_dispatch"}
    assert len(triggers["schedule"]) == 1
    cron = triggers["schedule"][0]["cron"].split()
    day, month, weekday = cron[2:]
    assert (day, month) == ("*", "*")
    assert weekday.isdigit()


def test_the_workflow_grants_nothing_by_default(workflow: dict) -> None:
    assert workflow["permissions"] == {}


def test_only_build_sign_reads_the_secret(workflow: dict) -> None:
    jobs = workflow["jobs"]
    assert "MINISIGN_SECRET_KEY" in _text(jobs["build-sign"])
    assert "secrets." not in _text(jobs["push"]).replace("secrets.GITHUB_TOKEN", "")
    assert "MINISIGN_SECRET_KEY" not in _text(jobs["push"])
    assert "MINISIGN_SECRET_KEY" not in yaml.safe_dump(
        {k: v for k, v in workflow.items() if k != "jobs"}
    )


def test_the_secret_is_in_one_step_env_only(workflow: dict) -> None:
    steps = workflow["jobs"]["build-sign"]["steps"]
    holders = [s for s in steps if "MINISIGN_SECRET_KEY" in _text(s)]
    assert len(holders) == 1
    assert holders[0]["env"] == {
        "MINISIGN_SECRET_KEY": "${{ secrets.MINISIGN_SECRET_KEY }}"
    }
    assert "MINISIGN_SECRET_KEY" not in holders[0]["run"]


def test_build_sign_runs_in_the_revocations_environment_with_read_only_contents(
    workflow: dict,
) -> None:
    job = workflow["jobs"]["build-sign"]
    assert job["environment"] == "revocations"
    assert job["permissions"] == {"contents": "read"}


def test_push_has_package_write_no_environment_and_depends_on_build_sign(
    workflow: dict,
) -> None:
    job = workflow["jobs"]["push"]
    assert job["permissions"] == {"contents": "read", "packages": "write"}
    assert "environment" not in job
    assert job["needs"] == "build-sign"


STUB_ORAS = """\
#!/usr/bin/env python3
import os, shutil, sys

mode = os.environ["STUB_MODE"]
args = sys.argv[1:]
errors = {
    "absent": "Error: ghcr.io/slopvac/slopvac-revocations:latest: not found",
    "auth": "Error response from registry: unauthorized: authentication required",
    "network": 'Error: Get "https://ghcr.io/v2/": dial tcp: lookup ghcr.io: no such host',
    "server": "Error: response status code 503: Service Unavailable",
}
if args[:2] == ["manifest", "fetch"]:
    if mode in errors:
        print(errors[mode], file=sys.stderr)
        sys.exit(1)
    print("<html>not a manifest</html>" if mode == "malformed" else '{"layers":[{},{}]}')
elif args[0] == "pull":
    if mode == "layer404":
        print("Error: failed to copy: ghcr.io/x/y@sha256:0: response status code 404: not found", file=sys.stderr)
        sys.exit(1)
    shutil.copytree(os.environ["STUB_SRC"], args[args.index("--output") + 1])
else:
    sys.exit(2)
"""
IMAGE = "ghcr.io/slopvac/slopvac-revocations:latest"


@pytest.fixture
def stub_oras(tmp_path: Path, monkeypatch: pytest.MonkeyPatch):
    """An ``oras`` on PATH that answers as ``STUB_MODE`` says."""
    bin_dir = tmp_path / "stub-bin"
    bin_dir.mkdir()
    stub = bin_dir / "oras"
    stub.write_text(STUB_ORAS, encoding="utf-8")
    stub.chmod(0o755)
    monkeypatch.setenv("PATH", f"{bin_dir}{os.pathsep}{os.environ['PATH']}")
    monkeypatch.delenv("ORAS", raising=False)

    def answer(mode: str, src: Path | None = None) -> None:
        monkeypatch.setenv("STUB_MODE", mode)
        if src is not None:
            monkeypatch.setenv("STUB_SRC", str(src))

    return answer


def test_an_absent_manifest_is_refused_unless_this_is_the_first_publication(
    tmp_path: Path, stub_oras
) -> None:
    stub_oras("absent")
    with pytest.raises(lists.SigningError, match="first_publication"):
        lists.fetch_published(IMAGE, tmp_path / "published", first_publication=False)


def test_an_absent_manifest_is_allowed_for_the_first_publication(
    tmp_path: Path, stub_oras
) -> None:
    stub_oras("absent")
    lists.fetch_published(IMAGE, tmp_path / "published", first_publication=True)
    assert not (tmp_path / "published").exists()


@pytest.mark.parametrize(
    ("mode", "why"),
    [
        ("auth", "could not resolve the manifest"),
        ("network", "could not resolve the manifest"),
        ("server", "could not resolve the manifest"),
        ("malformed", "is not JSON"),
    ],
)
@pytest.mark.parametrize("first_publication", [False, True])
def test_any_failure_other_than_an_absent_manifest_fails_closed(
    tmp_path: Path, stub_oras, mode: str, why: str, first_publication: bool
) -> None:
    stub_oras(mode, tmp_path)
    with pytest.raises(lists.SigningError, match=why):
        lists.fetch_published(
            IMAGE, tmp_path / "published", first_publication=first_publication
        )


def test_a_layer_that_404s_under_an_existing_manifest_is_not_a_first_run(
    tmp_path: Path, stub_oras
) -> None:
    stub_oras("layer404", tmp_path)
    with pytest.raises(lists.SigningError, match="could not pull the layers"):
        lists.fetch_published(IMAGE, tmp_path / "published", first_publication=False)


@NEEDS_MINISIGN
def test_a_published_list_fails_the_first_publication(
    tmp_path: Path, stub_oras
) -> None:
    published = _signed(tmp_path, "published", EARLIER)
    stub_oras("present", published)
    with pytest.raises(lists.SigningError, match="already published"):
        lists.fetch_published(IMAGE, tmp_path / "pulled", first_publication=True)


@NEEDS_MINISIGN
def test_an_older_candidate_is_refused_next_to_a_newer_published_list(
    tmp_path: Path, stub_oras
) -> None:
    published = _signed(tmp_path, "published", LATER)
    candidate = _signed(tmp_path, "candidate", EARLIER)
    stub_oras("present", published)
    pulled = tmp_path / "pulled"
    lists.fetch_published(IMAGE, pulled, first_publication=False)
    with pytest.raises(lists.SigningError, match="not newer"):
        lists.check_newer(candidate, pulled, tmp_path / "throwaway.pub", LATER)


@NEEDS_MINISIGN
def test_a_newer_candidate_replaces_an_older_published_list(
    tmp_path: Path, stub_oras
) -> None:
    published = _signed(tmp_path, "published", EARLIER)
    candidate = _signed(tmp_path, "candidate", LATER)
    stub_oras("present", published)
    pulled = tmp_path / "pulled"
    lists.fetch_published(IMAGE, pulled, first_publication=False)
    lists.check_newer(candidate, pulled, tmp_path / "throwaway.pub", LATER)


def test_push_fetches_the_published_list_before_it_checks_and_pushes(
    workflow: dict,
) -> None:
    runs = [s.get("run", "") for s in workflow["jobs"]["push"]["steps"]]
    fetch = next(i for i, r in enumerate(runs) if "fetch-published" in r)
    newer = next(i for i, r in enumerate(runs) if "revocation_list.py check-newer" in r)
    push = next(i for i, r in enumerate(runs) if "oras push" in r)
    assert fetch < newer < push


def test_first_publication_is_a_manual_boolean_defaulting_to_false(
    workflow: dict,
) -> None:
    option = workflow[True]["workflow_dispatch"]["inputs"]["first_publication"]
    assert (option["type"], option["default"]) == ("boolean", False)
    step = next(
        s
        for s in workflow["jobs"]["push"]["steps"]
        if "fetch-published" in s.get("run", "")
    )
    assert step["env"] == {"FIRST_PUBLICATION": "${{ inputs.first_publication }}"}


def test_build_sign_signs_only_from_main(workflow: dict) -> None:
    condition = workflow["jobs"]["build-sign"]["if"]
    assert "github.ref == 'refs/heads/main'" in condition
    assert workflow["jobs"]["push"]["if"] == condition
    assert workflow["jobs"]["push"]["needs"] == "build-sign"


def test_push_verifies_again_before_it_pushes(workflow: dict) -> None:
    runs = [s.get("run", "") for s in workflow["jobs"]["push"]["steps"]]
    verify = next(i for i, r in enumerate(runs) if "revocation_list.py verify" in r)
    push = next(i for i, r in enumerate(runs) if "oras push" in r)
    assert verify < push


def test_push_uses_the_exact_artifact_type_and_layer_order(workflow: dict) -> None:
    env = workflow["env"]
    assert env["IMAGE"] == "ghcr.io/slopvac/slopvac-revocations:latest"
    assert env["ARTIFACT_TYPE"] == "application/vnd.slopvac.revocations.v1"
    assert env["MEDIA_LIST"] == "application/vnd.slopvac.revocations.v1+json"
    assert env["MEDIA_SIGNATURE"] == "application/vnd.slopvac.signature.minisig.v1"
    push = next(
        s["run"]
        for s in workflow["jobs"]["push"]["steps"]
        if "oras push" in s.get("run", "")
    )
    assert push.index("revocations.json:") < push.index("revocations.json.minisig:")
    assert '--artifact-type "$ARTIFACT_TYPE"' in push


def test_every_action_is_pinned_to_a_commit(workflow: dict) -> None:
    uses = [
        s["uses"] for j in workflow["jobs"].values() for s in j["steps"] if "uses" in s
    ]
    assert uses
    for ref in uses:
        assert len(ref.split("@")[1]) == 40, ref


def test_no_expression_reaches_a_shell_directly(workflow: dict) -> None:
    for job in workflow["jobs"].values():
        for step in job["steps"]:
            assert "${{" not in step.get("run", ""), step.get("name")
