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


@pytest.mark.parametrize("version", [b"true", b"1.0", b'"1"', b"2"])
def test_check_list_requires_the_integer_one_as_schema_version(version: bytes) -> None:
    good = lists.render(NOW)
    data = good.replace(b'"schema_version":1', b'"schema_version":' + version)
    assert data != good
    with pytest.raises(lists.SigningError, match="version 1 revocation list"):
        lists.check_list(data, NOW)


STUB_SIGNER = """\
#!/usr/bin/env python3
import json, os, stat, sys

key = sys.argv[sys.argv.index("-s") + 1]
with open(os.environ["STUB_LOG"], "w") as log:
    json.dump({"mode": stat.S_IMODE(os.stat(key).st_mode), "key": key}, log)
print("signing refused", file=sys.stderr)
sys.exit(1)
"""


def test_the_secret_key_file_is_private_while_signing_and_removed_when_signing_fails(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    signer = tmp_path / "stub-minisign"
    signer.write_text(STUB_SIGNER, encoding="utf-8")
    signer.chmod(0o755)
    log = tmp_path / "log.json"
    monkeypatch.setenv("MINISIGN", str(signer))
    monkeypatch.setenv("STUB_LOG", str(log))
    with pytest.raises(lists.SigningError, match="signing refused"):
        lists.build(
            tmp_path / "out",
            lists.PUBKEY,
            {"MINISIGN_SECRET_KEY": "secret-material", "RUNNER_TEMP": str(tmp_path)},
            NOW,
            30,
        )
    seen = json.loads(log.read_text())
    assert seen["mode"] == 0o600
    assert not Path(seen["key"]).exists()
    assert not list(tmp_path.glob("minisign-*"))


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


def _keypair(tmp_path: Path, name: str = "throwaway") -> tuple[Path, Path]:
    secret, public = tmp_path / f"{name}.key", tmp_path / f"{name}.pub"
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


def _signed(
    tmp_path: Path,
    name: str,
    issued: datetime,
    days: int = 30,
    key_name: str = "throwaway",
) -> Path:
    """A list issued at ``issued``, signed by the throwaway key ``key_name``."""
    key, public = tmp_path / f"{key_name}.key", tmp_path / f"{key_name}.pub"
    if not key.exists():
        _keypair(tmp_path, key_name)
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
    "auth": "Error response from registry: unauthorized: authentication required",
    "network": 'Error: Get "https://ghcr.io/v2/": dial tcp: lookup ghcr.io: no such host',
    "server": "Error: response status code 503: Service Unavailable",
}
if args[:2] == ["manifest", "fetch"]:
    if mode == "stderr-file":
        sys.stderr.write(open(os.environ["STUB_STDERR_FILE"], encoding="utf-8").read())
        sys.exit(1)
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
REAL_NOT_FOUND = ROOT / "tests" / "fixtures" / "oras_manifest_fetch_not_found.stderr"


def _real_not_found() -> str:
    lines = REAL_NOT_FOUND.read_text(encoding="utf-8").splitlines(keepends=True)
    return "".join(line for line in lines if not line.startswith("#"))


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

    def answer(
        mode: str, src: Path | None = None, *, stderr: str | None = None
    ) -> None:
        monkeypatch.setenv("STUB_MODE", "stderr-file" if stderr is not None else mode)
        if stderr is not None:
            text = tmp_path / "stub-stderr.txt"
            text.write_text(stderr, encoding="utf-8")
            monkeypatch.setenv("STUB_STDERR_FILE", str(text))
        if src is not None:
            monkeypatch.setenv("STUB_SRC", str(src))

    return answer


def test_an_absent_manifest_is_refused_unless_this_is_the_first_publication(
    tmp_path: Path, stub_oras
) -> None:
    stub_oras("", stderr=_real_not_found())
    with pytest.raises(lists.SigningError, match="first_publication"):
        lists.fetch_published(IMAGE, tmp_path / "published", first_publication=False)


def test_an_absent_manifest_is_allowed_for_the_first_publication(
    tmp_path: Path, stub_oras
) -> None:
    stub_oras("", stderr=_real_not_found())
    lists.fetch_published(IMAGE, tmp_path / "published", first_publication=True)
    assert not (tmp_path / "published").exists()


def _not_found_for(image: str) -> str:
    return _real_not_found().replace(IMAGE, image)


_AUTH = "Error response from registry: unauthorized: authentication required\n"
_5XX = "Error response from registry: 503 Service Unavailable\n"
_DIGEST = "ghcr.io/slopvac/slopvac-revocations@sha256:" + "0" * 64
_DOTTED = "ghcr.io/a/b:latest"


@pytest.mark.parametrize(
    ("image", "stderr"),
    [
        pytest.param(IMAGE, _not_found_for("ghcr.io/slopvac/other:latest"), id="other-ref"),
        pytest.param(IMAGE, _not_found_for(IMAGE + "2"), id="stderr-longer-tag"),
        pytest.param(IMAGE + "2", _not_found_for(IMAGE), id="requested-longer-tag"),
        pytest.param(IMAGE, _not_found_for(_DIGEST), id="stderr-digest"),
        pytest.param(_DIGEST, _not_found_for(IMAGE), id="requested-digest"),
        pytest.param(_DOTTED, _not_found_for("ghcrxio/a/b:latest"), id="dot-against-x"),
        pytest.param(
            IMAGE,
            "Error response from registry: MANIFEST_UNKNOWN: manifest unknown\n",
            id="manifest-unknown-code",
        ),
        pytest.param(
            IMAGE,
            "Error response from registry: NAME_UNKNOWN: repository not known\n",
            id="name-unknown-code",
        ),
        pytest.param(IMAGE, f"Error: {IMAGE}: not found\n", id="old-short-form"),
        pytest.param(IMAGE, _real_not_found().strip() + " (retry)\n", id="trailing-text"),
        pytest.param(IMAGE, "Error: " + _real_not_found(), id="leading-text"),
        pytest.param(IMAGE, _AUTH, id="auth"),
        pytest.param(
            IMAGE, "Error response from registry: 403 not found\n", id="403-not-found"
        ),
        pytest.param(IMAGE, _5XX, id="5xx"),
        pytest.param(IMAGE, _AUTH + _real_not_found(), id="auth-then-not-found"),
        pytest.param(IMAGE, _real_not_found() + _AUTH, id="not-found-then-auth"),
        pytest.param(IMAGE, _5XX + _real_not_found(), id="5xx-then-not-found"),
        pytest.param(IMAGE, _real_not_found() + _5XX, id="not-found-then-5xx"),
        pytest.param(IMAGE, _real_not_found() * 2, id="duplicated"),
        pytest.param(IMAGE, "\n" + _real_not_found(), id="leading-blank-line"),
        pytest.param(
            IMAGE,
            "body: " + _real_not_found().strip() + "\nmore body\n",
            id="embedded-in-error-body",
        ),
    ],
)
@pytest.mark.parametrize("first_publication", [False, True])
def test_only_orass_whole_not_found_for_the_requested_ref_is_absent(
    tmp_path: Path, stub_oras, image: str, stderr: str, first_publication: bool
) -> None:
    stub_oras("", stderr=stderr)
    with pytest.raises(lists.SigningError, match="could not resolve the manifest"):
        lists.fetch_published(
            image, tmp_path / "published", first_publication=first_publication
        )


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
def test_a_valid_published_list_is_never_replaced_by_a_first_publication(
    tmp_path: Path, stub_oras
) -> None:
    published = _signed(tmp_path, "published", EARLIER)
    candidate = _signed(tmp_path, "candidate", LATER)
    stub_oras("present", published)
    pulled = tmp_path / "pulled"
    lists.fetch_published(IMAGE, pulled, first_publication=True)
    with pytest.raises(lists.SigningError, match="already published"):
        lists.check_newer(
            candidate,
            pulled,
            tmp_path / "throwaway.pub",
            LATER,
            first_publication=True,
        )


def _tampered(tmp_path: Path) -> Path:
    """A published list whose bytes no longer match its signature."""
    published = _signed(tmp_path, "published", LATER)
    (published / "revocations.json").write_bytes(lists.render(LATER, days=7))
    return published


@NEEDS_MINISIGN
def test_an_invalid_published_list_is_not_overwritten_by_default(
    tmp_path: Path, stub_oras
) -> None:
    published = _tampered(tmp_path)
    candidate = _signed(tmp_path, "candidate", LATER)
    stub_oras("present", published)
    pulled = tmp_path / "pulled"
    lists.fetch_published(IMAGE, pulled, first_publication=False)
    with pytest.raises(lists.SigningError, match="does not verify"):
        lists.check_newer(candidate, pulled, tmp_path / "throwaway.pub", LATER)


@NEEDS_MINISIGN
def test_a_first_publication_replaces_an_invalid_published_list_with_a_warning(
    tmp_path: Path, stub_oras, capsys: pytest.CaptureFixture[str]
) -> None:
    published = _tampered(tmp_path)
    candidate = _signed(tmp_path, "candidate", LATER)
    stub_oras("present", published)
    pulled = tmp_path / "pulled"
    lists.fetch_published(IMAGE, pulled, first_publication=True)
    lists.check_newer(
        candidate, pulled, tmp_path / "throwaway.pub", LATER, first_publication=True
    )
    out = capsys.readouterr().out
    assert out.startswith("::warning::replacing an existing list that failed")
    assert "signature check failed" in out
    assert out.count("\n") == 1


@NEEDS_MINISIGN
def test_a_published_list_signed_by_a_trusted_previous_key_is_replaced_by_the_new_key(
    tmp_path: Path,
) -> None:
    published = _signed(tmp_path, "published", EARLIER, key_name="old")
    candidate = _signed(tmp_path, "candidate", LATER)
    new_pub = tmp_path / "throwaway.pub"
    (tmp_path / "slopvac-release-AAAAAAAAAAAAAAAA.pub").write_text(
        (tmp_path / "old.pub").read_text()
    )
    lists.check_newer(candidate, published, new_pub, LATER)


@NEEDS_MINISIGN
def test_a_published_list_from_an_unlisted_key_is_not_replaced(
    tmp_path: Path,
) -> None:
    published = _signed(tmp_path, "published", EARLIER, key_name="old")
    candidate = _signed(tmp_path, "candidate", LATER)
    with pytest.raises(lists.SigningError, match="does not verify"):
        lists.check_newer(candidate, published, tmp_path / "throwaway.pub", LATER)


@NEEDS_MINISIGN
def test_a_candidate_signed_by_a_previous_key_is_refused(tmp_path: Path) -> None:
    published = _signed(tmp_path, "published", EARLIER)
    candidate = _signed(tmp_path, "candidate", LATER, key_name="old")
    # The previous key is trusted for the published list, and still not for the candidate.
    (tmp_path / "slopvac-release-AAAAAAAAAAAAAAAA.pub").write_text(
        (tmp_path / "old.pub").read_text()
    )
    with pytest.raises(lists.SigningError, match="signature check failed"):
        lists.check_newer(candidate, published, tmp_path / "throwaway.pub", LATER)


def _malformed_list(kind: str) -> bytes:
    good = lists.render(LATER).decode()
    if kind == "deeply-nested":
        return good.replace(
            '"keys":[]', '"keys":' + "[" * 100_000 + "]" * 100_000
        ).encode()
    if kind == "lone-surrogate":
        return good.replace('"revocations"', '"\\ud800"').encode()
    raise AssertionError(kind)


MALFORMED = ["deeply-nested", "lone-surrogate"]


@pytest.mark.parametrize("kind", MALFORMED)
def test_a_malformed_list_is_a_verification_error_not_a_crash(kind: str) -> None:
    data = _malformed_list(kind)
    assert len(data) < lists.MAX_LIST_BYTES
    with pytest.raises(lists.SigningError):
        lists.check_list(data, NOW)


def _published_with(tmp_path: Path, kind: str) -> Path:
    published = _signed(tmp_path, "published", LATER)
    (published / "revocations.json").write_bytes(_malformed_list(kind))
    return published


@NEEDS_MINISIGN
@pytest.mark.parametrize("kind", MALFORMED)
def test_a_malformed_published_list_is_refused_by_default(
    tmp_path: Path, stub_oras, kind: str
) -> None:
    published = _published_with(tmp_path, kind)
    candidate = _signed(tmp_path, "candidate", LATER)
    stub_oras("present", published)
    pulled = tmp_path / "pulled"
    lists.fetch_published(IMAGE, pulled, first_publication=False)
    with pytest.raises(lists.SigningError, match="does not verify"):
        lists.check_newer(candidate, pulled, tmp_path / "throwaway.pub", LATER)


@NEEDS_MINISIGN
@pytest.mark.parametrize("kind", MALFORMED)
def test_a_malformed_published_list_is_replaced_by_a_first_publication(
    tmp_path: Path, stub_oras, capsys: pytest.CaptureFixture[str], kind: str
) -> None:
    published = _published_with(tmp_path, kind)
    candidate = _signed(tmp_path, "candidate", LATER)
    stub_oras("present", published)
    pulled = tmp_path / "pulled"
    lists.fetch_published(IMAGE, pulled, first_publication=True)
    lists.check_newer(
        candidate, pulled, tmp_path / "throwaway.pub", LATER, first_publication=True
    )
    out = capsys.readouterr().out
    assert out.startswith("::warning::replacing an existing list that failed")
    assert out.count("\n") == 1


def _non_utf8_signature(tmp_path: Path) -> Path:
    """A published list that is canonical but whose signature is not UTF-8."""
    published = _signed(tmp_path, "published", LATER)
    (published / "revocations.json.minisig").write_bytes(b"\xff\xfe\x80 not utf-8\n")
    return published


@NEEDS_MINISIGN
def test_a_non_utf8_published_signature_is_refused_by_default(
    tmp_path: Path, stub_oras
) -> None:
    published = _non_utf8_signature(tmp_path)
    candidate = _signed(tmp_path, "candidate", LATER)
    stub_oras("present", published)
    pulled = tmp_path / "pulled"
    lists.fetch_published(IMAGE, pulled, first_publication=False)
    with pytest.raises(lists.SigningError, match="does not verify"):
        lists.check_newer(candidate, pulled, tmp_path / "throwaway.pub", LATER)


@NEEDS_MINISIGN
def test_a_non_utf8_published_signature_is_replaced_by_a_first_publication(
    tmp_path: Path, stub_oras, capsys: pytest.CaptureFixture[str]
) -> None:
    published = _non_utf8_signature(tmp_path)
    candidate = _signed(tmp_path, "candidate", LATER)
    stub_oras("present", published)
    pulled = tmp_path / "pulled"
    lists.fetch_published(IMAGE, pulled, first_publication=True)
    lists.check_newer(
        candidate, pulled, tmp_path / "throwaway.pub", LATER, first_publication=True
    )
    out = capsys.readouterr().out
    assert out.startswith("::warning::replacing an existing list that failed")
    assert "not a minisign signature" in out


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
    for command in ("fetch-published", "check-newer"):
        step = next(
            s
            for s in workflow["jobs"]["push"]["steps"]
            if f"revocation_list.py {command}" in s.get("run", "")
        )
        assert step["env"] == {"FIRST_PUBLICATION": "${{ inputs.first_publication }}"}
        assert "--first-publication" in step["run"]


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
