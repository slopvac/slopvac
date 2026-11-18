"""Offline tests for the document-producing curation handlers.

`curate.api` and `lib.http_get` are replaced with a fake GitHub, so nothing
touches the network. Expected values are worked out by hand from the fixture.
"""

from __future__ import annotations

import base64
import json

import pytest

from evals import corpuslib as lib
from evals import curate

REPO = "o/r"
PIN = "a" * 40  # last commit before the cutoff; the repository licence is read here
OTHER = "b" * 40  # a different commit in the same repository
PIN_DATE = "2022-10-29"
BASIS = "github license API at the revision"
WORDS = " ".join(["word"] * 200)


def _commit(sha, date, message="x", parents=1):
    return {
        "sha": sha,
        "commit": {"committer": {"date": f"{date}T10:00:00Z"}, "message": message},
        "parents": [{}] * parents,
        "author": {"type": "User"},
    }


class FakeGitHub:
    """Canned GitHub API. `licences` maps a commit to its SPDX id at that commit."""

    def __init__(self, licences, *, private=False, repo_license="GPL-3.0-only"):
        self.licences = licences
        self.private = private
        self.repo_license = repo_license
        self.commits = [_commit(PIN, PIN_DATE)]
        self.path_commits = []
        self.tree = []
        self.issues = []
        self.raw: dict[str, bytes] = {}
        self.calls: list[str] = []

    def api(self, endpoint, **params):
        self.calls.append(endpoint)
        if endpoint == f"repos/{REPO}":
            return {
                "private": self.private,
                "visibility": "private" if self.private else "public",
                "fork": False,
                "license": {"spdx_id": self.repo_license},
            }
        if endpoint == f"repos/{REPO}/license":
            spdx = self.licences.get(params["ref"])
            if spdx is None:
                raise lib.FetchError("GET license failed: HTTP Error 404")
            body = base64.b64encode(b"Copyright (c) 2020 Example").decode()
            return {"license": {"spdx_id": spdx}, "content": body}
        if endpoint == f"repos/{REPO}/commits":
            if "path" in params:
                return self.path_commits
            if params.get("per_page") == 1:
                return self.commits[:1]
            return self.commits if params.get("page", 1) == 1 else []
        if endpoint.startswith(f"repos/{REPO}/git/trees/"):
            return {"tree": self.tree}
        if endpoint == "search/issues":
            return {"items": self.issues}
        raise AssertionError(f"unexpected endpoint {endpoint}")

    def http_get(self, url, token=None, retries=4):
        return self.raw[url]


def install(monkeypatch, fake):
    monkeypatch.setattr(curate, "api", fake.api)
    monkeypatch.setattr(lib, "http_get", fake.http_get)
    monkeypatch.setattr(curate.time, "sleep", lambda _s: None)


def prov(revision, date_source="commit.committer.date", basis=BASIS):
    return {
        "date_source": date_source,
        "license_revision": revision,
        "license_basis": basis,
    }


# --- files -----------------------------------------------------------------


def files_fixture(monkeypatch, licences):
    fake = FakeGitHub(licences)
    fake.tree = [{"path": "docs/a.md", "type": "blob", "size": 5000}]
    fake.raw[lib.raw_url(REPO, PIN, "docs/a.md")] = WORDS.encode()
    install(monkeypatch, fake)
    return fake


SPEC_FILES = {"repo": REPO, "genre": "readmes", "format": "markdown", "min_bytes": 1}


def test_files_record_the_repository_licence_at_the_pinned_revision(monkeypatch):
    files_fixture(monkeypatch, {PIN: "Apache-2.0"})
    [(doc, _)] = curate.curate_files(SPEC_FILES, {})
    assert (doc["license"], doc["date"]) == ("Apache-2.0", PIN_DATE)
    assert doc["provenance"] == prov(PIN)


def test_a_source_override_is_recorded_as_the_licence_basis(monkeypatch):
    files_fixture(monkeypatch, {PIN: "Apache-2.0"})
    [(doc, _)] = curate.curate_files({**SPEC_FILES, "license": "CC-BY-4.0"}, {})
    assert doc["license"] == "CC-BY-4.0"
    assert doc["provenance"]["license_basis"] == "sources.toml override"


def test_files_error_when_the_licence_lookup_at_the_pin_fails(monkeypatch):
    files_fixture(monkeypatch, {})  # no licence at the pin; HEAD must not be tried
    with pytest.raises(RuntimeError, match="no SPDX licence"):
        curate.curate_files(SPEC_FILES, {})


# --- commits ---------------------------------------------------------------


def commits_fixture(monkeypatch, licences):
    fake = FakeGitHub(licences)
    message = "Subject\n\n" + "b" * 400
    fake.commits = [_commit(PIN, PIN_DATE), _commit(OTHER, "2021-03-04", message)]
    install(monkeypatch, fake)
    return fake


def test_a_commit_takes_the_licence_and_date_of_its_own_commit(monkeypatch):
    commits_fixture(monkeypatch, {PIN: "Apache-2.0", OTHER: "MIT"})
    [(doc, _)] = curate.curate_commits({"repo": REPO, "count": 5}, {})
    assert (doc["license"], doc["date"]) == ("MIT", "2021-03-04")
    assert doc["provenance"] == prov(OTHER)


def test_a_commit_errors_when_the_lookup_at_its_commit_fails(monkeypatch):
    commits_fixture(monkeypatch, {PIN: "Apache-2.0"})  # the pin has one, OTHER none
    with pytest.raises(RuntimeError, match="no SPDX licence"):
        curate.curate_commits({"repo": REPO, "count": 5}, {})


# --- pull requests ---------------------------------------------------------


def test_a_pull_request_is_dated_by_creation_at_the_repository_pin(monkeypatch):
    fake = FakeGitHub({PIN: "MIT"})
    fake.issues = [
        {
            "number": 7,
            "title": "Title",
            "body": "body " * 20,
            "user": {"type": "User"},
            "html_url": f"https://github.com/{REPO}/pull/7",
            "created_at": "2022-05-06T01:02:03Z",
        }
    ]
    install(monkeypatch, fake)
    [(doc, _)] = curate.curate_prs({"repo": REPO, "min_body": 10}, {})
    assert (doc["license"], doc["date"]) == ("MIT", "2022-05-06")
    assert doc["provenance"] == prov(PIN, "pull_request.created_at")


# --- stated-LLM JSON -------------------------------------------------------


def test_stated_llm_documents_record_the_latest_pin_as_licence_revision(monkeypatch):
    fake = FakeGitHub({PIN: "MIT"})
    path = "d/m.json"
    fake.raw[lib.raw_url(REPO, PIN, path)] = json.dumps(
        [{"output": "one two three four five six", "generator": "m"}]
    ).encode()
    install(monkeypatch, fake)
    spec = {"repo": REPO, "path": "d/{model}.json", "models": ["m"], "min_words": 5}
    [(doc, _)] = curate.curate_stated_llm(spec, {})
    assert (doc["license"], doc["date"]) == ("MIT", PIN_DATE)
    assert doc["provenance"] == prov(PIN)


# --- diagrams --------------------------------------------------------------

DIAGRAM = "```mermaid\n" + WORDS + "\n"


def diagram_fixture(monkeypatch, tmp_path, licences, **kwargs):
    fake = FakeGitHub(licences, **kwargs)
    fake.path_commits = [_commit(OTHER, "2021-03-04")]
    fake.raw[lib.raw_url(REPO, OTHER, "a.md")] = DIAGRAM.encode()
    candidates = tmp_path / "candidates.json"
    candidates.write_text(json.dumps([["md", REPO, "a.md"]]))
    monkeypatch.setattr(curate, "DIAGRAM_CANDIDATES", candidates)
    install(monkeypatch, fake)
    return fake


SPEC_DIAGRAMS = {"kind": "md", "count": 5}


def test_a_diagram_takes_the_licence_at_its_own_commit_not_the_repository_one(
    monkeypatch, tmp_path
):
    # Repository metadata says GPL today; the file's commit carried MIT.
    diagram_fixture(monkeypatch, tmp_path, {OTHER: "MIT"})
    repos: dict = {}
    [(doc, _)] = curate.curate_diagrams(SPEC_DIAGRAMS, repos)
    assert (doc["license"], doc["date"]) == ("MIT", "2021-03-04")
    assert doc["provenance"] == prov(OTHER)
    assert repos[REPO]["public"] is True


def test_a_diagram_with_no_licence_at_its_commit_is_skipped(monkeypatch, tmp_path):
    diagram_fixture(monkeypatch, tmp_path, {}, repo_license="MIT")
    assert curate.curate_diagrams(SPEC_DIAGRAMS, {}) == []


def test_a_diagram_from_a_private_repository_is_denied(monkeypatch, tmp_path):
    fake = diagram_fixture(monkeypatch, tmp_path, {OTHER: "MIT"}, private=True)
    repos: dict = {}
    assert curate.curate_diagrams(SPEC_DIAGRAMS, repos) == []
    assert repos == {}
    assert f"repos/{REPO}/commits" not in fake.calls
