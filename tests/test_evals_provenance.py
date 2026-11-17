"""Provenance of the corpus: licence at the content revision, date source, public repos.

No network: `evals.curate.api` is replaced with a fake that serves canned
GitHub API answers.
"""

from __future__ import annotations

import base64
import copy

import pytest

from evals import corpuslib as lib
from evals import curate

SHA = "a" * 40
OTHER_SHA = "b" * 40
MIT_TEXT = "MIT License\n\nCopyright (c) 2020 Example Authors\n"


class FakeGitHub:
    """Serves the three endpoints that pinning uses and records every call."""

    def __init__(self, *, private=False, visibility="public", license_at_sha="MIT"):
        self.calls: list[tuple[str, dict]] = []
        self.private = private
        self.visibility = visibility
        self.license_at_sha = license_at_sha

    def __call__(self, endpoint, **params):
        self.calls.append((endpoint, params))
        if endpoint.endswith("/commits"):
            return [
                {"sha": SHA, "commit": {"committer": {"date": "2022-10-29T10:00:00Z"}}}
            ]
        if endpoint.endswith("/license"):
            if params.get("ref") != SHA or self.license_at_sha is None:
                raise lib.FetchError("GET license failed: HTTP Error 404")
            return {
                "license": {"spdx_id": self.license_at_sha},
                "content": base64.b64encode(MIT_TEXT.encode()).decode(),
            }
        return {"private": self.private, "visibility": self.visibility, "fork": False}


@pytest.fixture
def github(monkeypatch):
    fake = FakeGitHub()
    monkeypatch.setattr(curate, "api", fake)
    return fake


def test_pin_records_the_revision_the_licence_was_read_at(github):
    repos: dict = {}
    info = curate.pin_repo("o/r", repos)
    assert (info["license"], info["license_revision"]) == ("MIT", SHA)
    assert ("repos/o/r/license", {"ref": SHA}) in github.calls


def test_pin_does_not_fall_back_to_the_licence_at_head(monkeypatch):
    fake = FakeGitHub(license_at_sha=None)
    monkeypatch.setattr(curate, "api", fake)
    info = curate.pin_repo("o/r", {})
    assert info["license"] is None
    assert all("ref" in p for e, p in fake.calls if e.endswith("/license"))


def test_pin_records_a_public_check_with_its_date(github):
    info = curate.pin_repo("o/r", {})
    assert info["public"] is True
    assert lib.DATE_RE.match(info["public_checked_at"])


@pytest.mark.parametrize(
    "fake",
    [FakeGitHub(private=True), FakeGitHub(visibility="internal")],
    ids=["private", "internal"],
)
def test_pin_refuses_a_repo_that_is_not_public(fake, monkeypatch):
    monkeypatch.setattr(curate, "api", fake)
    repos: dict = {}
    with pytest.raises(lib.NotPublicError):
        curate.pin_repo("o/r", repos)
    assert repos == {}


def test_a_failed_public_check_stops_before_any_commit_lookup(monkeypatch):
    fake = FakeGitHub(private=True)
    monkeypatch.setattr(curate, "api", fake)
    with pytest.raises(lib.NotPublicError):
        curate.pin_repo("o/r", {})
    assert [e for e, _ in fake.calls if e.endswith("/commits")] == []


def test_stated_llm_pin_still_records_the_revision(github):
    info = curate.pin_repo("o/r", {}, latest=True)
    assert info["license_revision"] == SHA


# --- manifest schema -------------------------------------------------------

MANIFEST = lib.load_manifest()
HUMAN_UPSTREAM = [
    d
    for d in MANIFEST["documents"]
    if d["origin"] == "human" and d["source"]["type"] != "derived"
]


def _validate(doc_changes=None, repo_changes=None, doc=None):
    doc = copy.deepcopy(
        doc or next(d for d in HUMAN_UPSTREAM if "commit" in d["source"])
    )
    doc.update(doc_changes or {})
    repos = copy.deepcopy(MANIFEST["repos"])
    repos[doc["source"]["repo"]].update(repo_changes or {})
    return lib.validate_manifest({"schema": 1, "repos": repos, "documents": [doc]})


def test_records_without_provenance_stay_valid():
    doc = copy.deepcopy(next(d for d in HUMAN_UPSTREAM if "commit" in d["source"]))
    doc.pop("provenance", None)
    assert _validate(doc=doc) == []


def test_provenance_does_not_change_the_manifest_digest():
    from evals import runner

    stripped = copy.deepcopy(MANIFEST)
    for d in stripped["documents"]:
        d.pop("provenance", None)
    assert runner.manifest_digest(stripped) == runner.manifest_digest(MANIFEST)


def test_every_upstream_human_date_names_its_source():
    missing = [
        d["id"]
        for d in HUMAN_UPSTREAM
        if d.get("provenance", {}).get("date_source") not in lib.DATE_SOURCES
    ]
    assert missing == []


def test_pull_request_dates_are_labelled_as_creation_dates():
    sources = {
        d["provenance"]["date_source"]
        for d in HUMAN_UPSTREAM
        if d["source"]["type"] == "github-pr"
    }
    assert sources == {"pull_request.created_at"}


@pytest.mark.parametrize(
    ("provenance", "fragment"),
    [
        ({"date_source": "merged_at"}, "date_source"),
        ({"license_revision": "main"}, "license_revision"),
        ({"license_revision": OTHER_SHA}, "content revision"),
    ],
)
def test_schema_rejects_bad_provenance(provenance, fragment):
    errors = _validate({"provenance": provenance})
    assert any(fragment in e for e in errors)


def test_matching_license_revision_is_accepted():
    doc = next(d for d in HUMAN_UPSTREAM if "commit" in d["source"])
    prov = {
        "date_source": "commit.committer.date",
        "license_revision": doc["source"]["commit"],
    }
    assert _validate({"provenance": prov}, doc=doc) == []


@pytest.mark.parametrize(
    ("changes", "fragment"),
    [
        ({"public": False, "public_checked_at": "2026-11-17"}, "not public"),
        ({"public": True, "public_checked_at": "yesterday"}, "public_checked_at"),
        ({"public": True}, "public_checked_at"),
    ],
)
def test_schema_rejects_bad_public_records(changes, fragment):
    assert any(fragment in e for e in _validate(repo_changes=changes))


def test_a_recorded_public_check_is_accepted():
    assert (
        _validate(repo_changes={"public": True, "public_checked_at": "2026-11-17"})
        == []
    )
