"""Tests for cla_check.py: pure decisions first, then the I/O flows against a stubbed GitHub API."""

from __future__ import annotations

import base64
import hashlib
import importlib.util
import json
import re
import sys
import threading
import unittest
import urllib.parse
from collections.abc import Callable
from dataclasses import dataclass
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any, ClassVar

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "cla_check", ROOT / "scripts" / "cla_check.py"
)
assert SPEC is not None and SPEC.loader is not None
cla = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = cla
SPEC.loader.exec_module(cla)

CONFIG = cla.Config(
    version="3",
    document="CLA.md",
    allowlist=("srobroek", "dependabot[bot]"),
    signatures_repo="o/cla-signers",
    signatures_branch="main",
    signatures_path="signatures/v1/cla.json",
    app_id=4293783,
    document_sha256="d" * 64,
)
PHRASE = "I have read the slopvac CLA version 3 and I hereby sign it."


SAME = object()  # the committer is the author


def commit(
    user_id: int | None,
    login: str = "x",
    message: str = "m",
    email: str = "a@b.c",
    sha: str = "abc1234",
    committer: Any = SAME,
    committer_email: str | None = None,
) -> dict[str, Any]:
    """A commit as the pull request commits endpoint lists it. ``committer`` is SAME, None (an email
    that no GitHub account links) or an ``(id, login)`` pair."""
    author = None if user_id is None else {"id": user_id, "login": login}
    if committer is SAME:
        committer = None if user_id is None else (user_id, login)
    linked = None if committer is None else {"id": committer[0], "login": committer[1]}
    return {
        "sha": sha,
        "author": author,
        "committer": linked,
        "commit": {
            "message": message,
            "author": {"email": email},
            "committer": {"email": committer_email or email},
        },
    }


def sigs(*entries: tuple[int, str]) -> dict[str, Any]:
    return {
        "signatures": [
            {
                "id": i,
                "login": "u",
                "cla_version": v,
                "repo": "o/r",
                "comment_id": 100 + i,
            }
            for i, v in entries
        ]
    }


class PhraseTests(unittest.TestCase):
    def test_phrase_names_the_version(self) -> None:
        self.assertEqual(cla.signing_phrase("3"), PHRASE)

    def test_exact_match_ignores_surrounding_whitespace_only(self) -> None:
        self.assertTrue(cla.is_signing_comment(f"  {PHRASE}\r\n", CONFIG))

    def test_near_misses_and_wrong_version_do_not_match(self) -> None:
        for body in [
            PHRASE.lower(),
            PHRASE.rstrip("."),
            PHRASE + " thanks",
            "LGTM\n" + PHRASE,
            PHRASE.replace("version 3", "version 2"),
            PHRASE.replace("version 3", "version 30"),
            "> " + PHRASE,
            "",
        ]:
            self.assertFalse(cla.is_signing_comment(body, CONFIG), body)

    def test_windows_line_endings_around_the_phrase_are_normalised(self) -> None:
        self.assertTrue(cla.is_signing_comment(f"\r\n{PHRASE}\r\n\r\n", CONFIG))
        self.assertFalse(cla.is_signing_comment(f"{PHRASE}\r\nthanks", CONFIG))


class AllowlistTests(unittest.TestCase):
    def test_exact_logins_match_ignoring_case(self) -> None:
        patterns = ("srobroek", "dependabot[bot]")
        self.assertTrue(cla.allowlisted("srobroek", patterns))
        self.assertTrue(cla.allowlisted("SRobroek", patterns))
        self.assertTrue(cla.allowlisted("dependabot[bot]", patterns))

    def test_there_is_no_wildcard(self) -> None:
        for login in ["srobroek2", "evil[bot]", "evilbot", "bot", "b", "dependabot"]:
            self.assertFalse(cla.allowlisted(login, ("*[bot]", "srobroek")), login)


class CoauthorTests(unittest.TestCase):
    def test_parse_trailers_case_insensitively(self) -> None:
        msg = "Fix\n\nBody\n\nCo-authored-by: A B <1+ab@users.noreply.github.com>\nco-authored-by: C <c@x.io>\n"
        self.assertEqual(
            cla.parse_coauthors(msg),
            [("A B", "1+ab@users.noreply.github.com"), ("C", "c@x.io")],
        )

    def test_noreply_with_id_resolves_by_id(self) -> None:
        self.assertEqual(
            cla.noreply_identity("12345+Octo@users.noreply.github.com"), (12345, "Octo")
        )

    def test_other_emails_do_not_resolve(self) -> None:
        for email in [
            "octo@users.noreply.github.com",
            "a@b.c",
            "x+y@users.noreply.github.com.evil.io",
        ]:
            self.assertIsNone(cla.noreply_identity(email), email)


class EvaluateTests(unittest.TestCase):
    def run_eval(
        self,
        commits: list[dict[str, Any]],
        opener: tuple[int, str],
        signed: dict[str, Any],
    ) -> cla.Verdict:
        return cla.evaluate(commits, opener, signed, CONFIG)

    def test_signed_author_passes(self) -> None:
        v = self.run_eval([commit(7, "dev")], (7, "dev"), sigs((7, "3")))
        self.assertTrue(v.ok)

    def test_unsigned_author_fails_and_is_listed_once(self) -> None:
        v = self.run_eval([commit(7, "dev"), commit(7, "dev")], (7, "dev"), sigs())
        self.assertFalse(v.ok)
        self.assertEqual(v.unsigned, [(7, "dev")])

    def test_signature_for_older_version_does_not_count(self) -> None:
        v = self.run_eval([commit(7, "dev")], (7, "dev"), sigs((7, "2")))
        self.assertEqual(v.unsigned, [(7, "dev")])

    def test_signature_matches_by_id_not_login(self) -> None:
        # id 8 renamed to the login of signer 7; only id 7 signed
        v = self.run_eval([commit(8, "dev")], (8, "dev"), sigs((7, "3")))
        self.assertEqual(v.unsigned, [(8, "dev")])
        v = self.run_eval([commit(7, "renamed")], (7, "renamed"), sigs((7, "3")))
        self.assertTrue(v.ok)

    def test_allowlisted_login_needs_no_signature(self) -> None:
        v = self.run_eval(
            [commit(1, "dependabot[bot]"), commit(2, "srobroek")],
            (2, "srobroek"),
            sigs(),
        )
        self.assertTrue(v.ok)

    def test_null_author_fails_with_link_your_email(self) -> None:
        v = self.run_eval(
            [commit(None, sha="deadbeef1", email="me@home.org")],
            (7, "dev"),
            sigs((7, "3")),
        )
        self.assertFalse(v.ok)
        self.assertEqual(v.unlinked, [("deadbeef1", "me@home.org", "author")])
        self.assertIn("link", cla.report(v, CONFIG).lower())
        self.assertIn("me@home.org", cla.report(v, CONFIG))
        self.assertIn("deadbee", cla.report(v, CONFIG))

    def test_unlinked_committer_fails_and_is_named_as_the_committer(self) -> None:
        c = commit(
            7, "dev", sha="cafe0001", committer=None, committer_email="ci@host.local"
        )
        v = self.run_eval([c], (7, "dev"), sigs((7, "3")))
        self.assertFalse(v.ok)
        self.assertEqual(v.unlinked, [("cafe0001", "ci@host.local", "committer")])
        self.assertIn("committer", cla.report(v, CONFIG))

    def test_the_same_unlinked_email_as_author_and_committer_is_reported_once(
        self,
    ) -> None:
        v = self.run_eval(
            [commit(None, sha="deadbeef1", email="me@home.org")],
            (7, "dev"),
            sigs((7, "3")),
        )
        self.assertEqual(len(v.unlinked), 1)

    def test_a_linked_committer_who_is_not_the_author_must_have_signed(self) -> None:
        c = commit(7, "dev", committer=(8, "rebaser"))
        v = self.run_eval([c], (7, "dev"), sigs((7, "3")))
        self.assertEqual(v.unsigned, [(8, "rebaser")])
        self.assertTrue(self.run_eval([c], (7, "dev"), sigs((7, "3"), (8, "3"))).ok)

    def test_github_web_flow_committer_needs_no_signature(self) -> None:
        c = commit(7, "dev", committer=(19864447, "web-flow"))
        config = cla.Config(
            **{**CONFIG.__dict__, "allowlist": (*CONFIG.allowlist, "web-flow")}
        )
        self.assertTrue(cla.evaluate([c], (7, "dev"), sigs((7, "3")), config).ok)

    def test_opener_who_is_not_an_author_must_also_have_signed(self) -> None:
        v = self.run_eval([commit(7, "dev")], (9, "opener"), sigs((7, "3")))
        self.assertEqual(v.unsigned, [(9, "opener")])
        v = self.run_eval([commit(7, "dev")], (9, "opener"), sigs((7, "3"), (9, "3")))
        self.assertTrue(v.ok)

    def test_noreply_coauthor_is_required_by_id(self) -> None:
        msg = "m\n\nCo-authored-by: Z <55+zed@users.noreply.github.com>"
        v = self.run_eval([commit(7, "dev", message=msg)], (7, "dev"), sigs((7, "3")))
        self.assertEqual(v.unsigned, [(55, None)])

    def test_unresolvable_coauthor_warns_but_does_not_fail(self) -> None:
        msg = "m\n\nCo-authored-by: Z <z@example.com>"
        v = self.run_eval([commit(7, "dev", message=msg)], (7, "dev"), sigs((7, "3")))
        self.assertTrue(v.ok)
        self.assertEqual(v.warnings, ["z@example.com"])


class SignatureDocTests(unittest.TestCase):
    record: ClassVar[dict[str, Any]] = {
        "id": 7,
        "login": "dev",
        "cla_version": "3",
        "pr": 4,
        "comment_id": 99,
        "signed_at": "t",
    }

    def test_merge_adds_then_is_idempotent(self) -> None:
        once = cla.merge_signature({"signatures": []}, self.record)
        self.assertEqual(once, {"signatures": [self.record]})
        twice = cla.merge_signature(once, {**self.record, "comment_id": 100})
        self.assertEqual(twice, once)

    def test_merge_adds_new_version_for_same_user(self) -> None:
        old = {"signatures": [{**self.record, "cla_version": "2"}]}
        self.assertEqual(len(cla.merge_signature(old, self.record)["signatures"]), 2)

    def test_merge_tolerates_empty_document(self) -> None:
        self.assertEqual(
            cla.merge_signature({}, self.record), {"signatures": [self.record]}
        )


class RoutingTests(unittest.TestCase):
    def test_routes(self) -> None:
        self.assertEqual(
            cla.route("workflow_run", {"action": "completed"}), "workflow_run"
        )
        self.assertEqual(cla.route("workflow_run", {"action": "requested"}), "ignore")
        for name in ["push", "pull_request", "merge_group"]:
            self.assertEqual(cla.route(name, {"action": "opened"}), "ignore", name)

    def test_comment_events_on_pull_requests_only(self) -> None:
        for action in ["created", "edited", "deleted"]:
            payload = {"action": action, "issue": {"pull_request": {"url": "u"}}}
            self.assertEqual(cla.route("issue_comment", payload), "comment", action)
        self.assertEqual(
            cla.route("issue_comment", {"action": "created", "issue": {}}), "ignore"
        )


class CommentActionTests(unittest.TestCase):
    def action(self, action: str, body: str, previous: str | None = None) -> str:
        payload: dict[str, Any] = {"action": action, "comment": {"body": body}}
        if previous is not None:
            payload["changes"] = {"body": {"from": previous}}
        return cla.comment_action(payload, CONFIG)

    def test_created_phrase_signs_and_other_text_is_ignored(self) -> None:
        self.assertEqual(self.action("created", PHRASE), "sign")
        self.assertEqual(self.action("created", "LGTM"), "ignore")

    def test_edit_into_the_phrase_signs(self) -> None:
        self.assertEqual(self.action("edited", PHRASE, previous="draft"), "sign")

    def test_edit_away_from_the_phrase_revokes(self) -> None:
        self.assertEqual(self.action("edited", "never mind", previous=PHRASE), "revoke")

    def test_edit_between_unrelated_texts_is_ignored(self) -> None:
        self.assertEqual(self.action("edited", "b", previous="a"), "ignore")

    def test_deleting_the_phrase_revokes_other_deletes_are_ignored(self) -> None:
        self.assertEqual(self.action("deleted", PHRASE), "revoke")
        self.assertEqual(self.action("deleted", "LGTM"), "ignore")


class RemoveSignatureTests(unittest.TestCase):
    def test_removes_only_the_record_from_that_comment_in_that_repo(self) -> None:
        doc = {
            "signatures": [
                {"id": 1, "repo": "o/r", "comment_id": 5},
                {"id": 2, "repo": "o/r", "comment_id": 6},
                {"id": 3, "repo": "other/r", "comment_id": 5},
            ]
        }
        self.assertEqual(
            cla.remove_signatures(doc, "o/r", 5),
            {
                "signatures": [
                    {"id": 2, "repo": "o/r", "comment_id": 6},
                    {"id": 3, "repo": "other/r", "comment_id": 5},
                ]
            },
        )

    def test_unknown_comment_leaves_the_document_equal(self) -> None:
        doc = sigs((7, "3"))
        self.assertEqual(cla.remove_signatures(doc, "o/r", 1), doc)


HEAD = "a" * 40
COMMENT_URL = "https://github.com/o/r/pull/4#issuecomment-99"
MQ = "b" * 40
M1 = "c" * 40
SIG_URL = "/repos/o/cla-signers/contents/signatures/v1/cla.json"


@dataclass
class Pr:
    number: int
    opener: int
    sha: str
    authors: list[int]
    head_repo: str = "fork/r"
    branch: str = "feature"


class FakeGitHub:
    """Stub transport for the REST and GraphQL calls cla_check makes, for the code repo and the
    signatures repo.

    ``comments`` maps comment id to body; by default it holds the signing comment 99 and the source
    comment of every record in the starting document. PR 4 reads its commits from ``commits``; every
    other PR derives them from its ``authors``."""

    def __init__(
        self,
        commits: list[dict[str, Any]],
        doc: dict[str, Any] | None = None,
        put_failures: list[int] | None = None,
        comments: dict[int, str] | None = None,
        prs: list[Pr] | None = None,
        comment_status: dict[int, int] | None = None,
        app_checks: dict[str, list[dict[str, Any]]] | None = None,
        graphql: dict[str, Any] | None = None,
        commits_status: int = 200,
        check_post_status: dict[str, int] | None = None,
        concurrent: list[dict[str, Any]] | None = None,
        after_post: list[Callable[[], None]] | None = None,
        after_put: list[Callable[[], None]] | None = None,
        on_put: list[Callable[[], None]] | None = None,
    ) -> None:
        self.commits = commits
        self.doc = doc
        self.put_failures = list(put_failures or [])
        self.calls: list[tuple[str, str, Any]] = []
        self.version = 1
        if comments is None:
            comments = {99: PHRASE}
            for rec in (doc or {}).get("signatures", []):
                comments.setdefault(
                    rec["comment_id"], cla.signing_phrase(rec["cla_version"])
                )
        self.comments = comments
        self.comment_status = comment_status or {}
        self.prs = prs if prs is not None else [Pr(4, 7, HEAD, [7], "o/r", "feat")]
        self.app_checks = app_checks or {}
        self.graphql = graphql
        self.commits_status = commits_status
        self.check_post_status = check_post_status or {}
        self.concurrent = list(concurrent or [])
        self.after_post = list(after_post or [])
        self.after_put = list(after_put or [])
        self.on_put = list(on_put or [])

    def puts(self) -> list[tuple[str, str, Any]]:
        return [c for c in self.calls if c[0] == "PUT"]

    def writes(self) -> list[tuple[str, str, Any]]:
        return [c for c in self.calls if c[0] != "GET"]

    def posted(self) -> list[dict[str, Any]]:
        return [
            c[2]
            for c in self.calls
            if c[0] == "POST" and c[1] == "/repos/o/r/check-runs"
        ]

    def pr_commits(self, number: int) -> list[dict[str, Any]]:
        if number == 4:
            return self.commits
        return [
            commit(a, "u") for p in self.prs if p.number == number for a in p.authors
        ]

    def pr_json(self, pr: Pr) -> dict[str, Any]:
        return {
            "number": pr.number,
            "user": {"id": pr.opener, "login": "dev" if pr.opener == 7 else "u"},
            "head": {
                "sha": pr.sha,
                "ref": pr.branch,
                "repo": {
                    "full_name": pr.head_repo,
                    "owner": {"login": pr.head_repo.split("/")[0]},
                },
            },
        }

    def __call__(self, method: str, path: str, body: Any = None) -> tuple[int, Any]:
        self.calls.append((method, path, body))
        found = re.search(r"/pulls/(\d+)/commits", path)
        if method == "GET" and found:
            if self.commits_status != 200:
                return self.commits_status, {}
            page = int(re.search(r"[?&]page=(\d+)", path).group(1))  # type: ignore[union-attr]
            items = self.pr_commits(int(found.group(1)))
            return 200, items[(page - 1) * 100 : page * 100]
        single = re.search(r"/repos/o/r/pulls/(\d+)$", path)
        if method == "GET" and single:
            for pr in self.prs:
                if pr.number == int(single.group(1)):
                    return 200, self.pr_json(pr)
            return 404, {}
        if method == "GET" and "/repos/o/r/pulls?state=open" in path:
            page = int(re.search(r"[?&]page=(\d+)", path).group(1))  # type: ignore[union-attr]
            items = self.prs
            head = re.search(r"[?&]head=([^&]+)", path)
            if head:
                owner, _, branch = urllib.parse.unquote(head.group(1)).partition(":")
                items = [
                    p
                    for p in items
                    if p.head_repo.split("/")[0] == owner and p.branch == branch
                ]
            return 200, [self.pr_json(p) for p in items[(page - 1) * 100 : page * 100]]
        comment = re.search(r"/repos/o/r/issues/comments/(\d+)$", path)
        if method == "GET" and comment:
            cid = int(comment.group(1))
            if cid in self.comment_status:
                return self.comment_status[cid], {}
            if cid not in self.comments:
                return 404, {}
            return 200, {"id": cid, "body": self.comments[cid]}
        if method == "GET" and path == f"{SIG_URL}?ref=main":
            if self.doc is None:
                return 404, {}
            raw = base64.b64encode(json.dumps(self.doc).encode()).decode()
            return 200, {"content": raw, "sha": f"blob{self.version}"}
        if method == "PUT" and path == SIG_URL:
            assert body["branch"] == "main"
            if self.on_put:
                self.on_put.pop(0)()
            if self.put_failures:
                return self.put_failures.pop(0), {"message": "conflict"}
            if self.concurrent:  # another signer writes between our GET and our PUT
                other = self.concurrent.pop(0)
                self.doc = cla.merge_signature(self.doc or {}, other)
                self.version += 1
            current = f"blob{self.version}" if self.doc is not None else None
            if body.get("sha") != current:  # compare-and-set on the blob sha
                return 409, {"message": "does not match"}
            self.doc = json.loads(base64.b64decode(body["content"]))
            self.version += 1
            if self.after_put:
                self.after_put.pop(0)()
            return 200, {}
        if method == "POST" and path == "/repos/o/r/check-runs":
            status = self.check_post_status.get(body["head_sha"], 201)
            if self.after_post:
                self.after_post.pop(0)()
            return status, {}
        checks = re.search(r"/repos/o/r/commits/([0-9a-f]+)/check-runs\?(.*)$", path)
        if method == "GET" and checks:
            assert "check_name=CLA" in checks.group(
                2
            ) and "app_id=4293783" in checks.group(2)
            return 200, {"check_runs": self.app_checks.get(checks.group(1), [])}
        if method == "POST" and path == "/graphql":
            return 200, self.graphql if self.graphql is not None else {
                "errors": [{"message": "no"}]
            }
        raise AssertionError(f"unexpected call {method} {path}")


def event(tmp: Path, name: str, payload: dict[str, Any]) -> dict[str, str]:
    path = tmp / f"{name}.json"
    path.write_text(json.dumps(payload), encoding="utf-8")
    return {
        "GITHUB_EVENT_NAME": name,
        "GITHUB_EVENT_PATH": str(path),
        "GITHUB_REPOSITORY": "o/r",
    }


def good_check(
    sha_id: int = 1, version: str = "3", conclusion: str = "success"
) -> dict[str, Any]:
    return {
        "id": sha_id,
        "name": "CLA",
        "conclusion": conclusion,
        "external_id": f"cla:{version}",
    }


def queue(*entries: tuple[int, str, int], total: int | None = None) -> dict[str, Any]:
    nodes = [
        {"position": pos, "headCommit": {"oid": oid}, "pullRequest": {"number": num}}
        for pos, oid, num in entries
    ]
    return {
        "data": {
            "repository": {
                "mergeQueue": {
                    "entries": {
                        "totalCount": len(nodes) if total is None else total,
                        "nodes": nodes,
                    }
                }
            }
        }
    }


class DocumentTests(unittest.TestCase):
    def test_the_sha256_of_the_agreement_text_is_the_file_hash(self) -> None:
        self.assertEqual(
            cla.document_sha256(b"abc"), hashlib.sha256(b"abc").hexdigest()
        )

    def test_a_document_that_differs_from_the_configured_hash_is_refused(self) -> None:
        with self.assertRaisesRegex(RuntimeError, "does not match"):
            cla.verify_document(b"changed", CONFIG)

    def test_a_document_with_the_configured_hash_is_accepted(self) -> None:
        config = cla.Config(
            **{
                **CONFIG.__dict__,
                "document_sha256": hashlib.sha256(b"text").hexdigest(),
            }
        )
        cla.verify_document(b"text", config)


class CheckRunBodyTests(unittest.TestCase):
    def test_body_is_a_completed_cla_check_run_tagged_with_the_version(self) -> None:
        body = cla.check_run_body(HEAD, True, "title", "summary", CONFIG)
        self.assertEqual(
            body,
            {
                "name": "CLA",
                "head_sha": HEAD,
                "status": "completed",
                "conclusion": "success",
                "external_id": "cla:3",
                "output": {"title": "title", "summary": "summary"},
            },
        )

    def test_failure_conclusion(self) -> None:
        self.assertEqual(
            cla.check_run_body(HEAD, False, "t", "s", CONFIG)["conclusion"], "failure"
        )

    def test_summary_is_truncated_to_the_api_limit(self) -> None:
        body = cla.check_run_body(HEAD, False, "t", "x" * 70000, CONFIG)
        self.assertLessEqual(len(body["output"]["summary"]), 65535)


class ContributorTests(unittest.TestCase):
    def test_contributors_are_the_opener_the_authors_and_noreply_coauthors(
        self,
    ) -> None:
        msg = "m\n\nCo-authored-by: Z <55+zed@users.noreply.github.com>\nCo-authored-by: Y <y@x.io>"
        commits = [commit(7, "dev", message=msg), commit(8, "eve"), commit(None)]
        # a login is known only for the opener and linked authors; a trailer gives an id and no login
        self.assertEqual(
            cla.contributors(commits, (9, "opener")),
            {9: "opener", 7: "dev", 55: None, 8: "eve"},
        )

    def test_a_trailer_cannot_lend_an_allowlisted_login_to_an_unsigned_id(self) -> None:
        forged = "m\n\nCo-authored-by: X <55+dependabot[bot]@users.noreply.github.com>"
        verdict = cla.evaluate(
            [commit(7, "dev", message=forged)], (7, "dev"), sigs((7, "3")), CONFIG
        )
        self.assertEqual(verdict.unsigned, [(55, None)])

    def test_the_forged_login_is_not_kept_when_the_id_later_appears_as_a_linked_author(
        self,
    ) -> None:
        forged = "m\n\nCo-authored-by: X <55+dependabot[bot]@users.noreply.github.com>"
        commits = [commit(7, "dev", message=forged), commit(55, "late")]
        verdict = cla.evaluate(commits, (7, "dev"), sigs((7, "3")), CONFIG)
        self.assertEqual(verdict.unsigned, [(55, "late")])
        reversed_order = cla.evaluate(commits[::-1], (7, "dev"), sigs((7, "3")), CONFIG)
        self.assertEqual(reversed_order.unsigned, [(55, "late")])

    def test_a_signed_coauthor_id_passes_whatever_the_trailer_says(self) -> None:
        forged = "m\n\nCo-authored-by: X <55+dependabot[bot]@users.noreply.github.com>"
        verdict = cla.evaluate(
            [commit(7, "dev", message=forged)],
            (7, "dev"),
            sigs((7, "3"), (55, "3")),
            CONFIG,
        )
        self.assertTrue(verdict.ok)

    def test_an_unknown_login_is_reported_by_id(self) -> None:
        forged = "m\n\nCo-authored-by: X <55+zed@users.noreply.github.com>"
        verdict = cla.evaluate(
            [commit(7, "dev", message=forged)], (7, "dev"), sigs((7, "3")), CONFIG
        )
        self.assertIn("user id 55", cla.report(verdict, CONFIG))

    def test_the_same_set_drives_checking_and_signing(self) -> None:
        msg = "m\n\nCo-authored-by: Z <55+zed@users.noreply.github.com>"
        commits = [commit(7, "dev", message=msg)]
        verdict = cla.evaluate(commits, (7, "dev"), sigs(), CONFIG)
        self.assertEqual(
            {uid for uid, _ in verdict.unsigned},
            set(cla.contributors(commits, (7, "dev"))),
        )


class SignFlowTests(unittest.TestCase):
    def setUp(self) -> None:
        import tempfile

        self._tmp = tempfile.TemporaryDirectory()
        self.tmp = Path(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)

    def go(self, env: dict[str, str], api: FakeGitHub, now: str = "t") -> int:
        return cla.run(env, api, api, CONFIG, now=now)

    def comment_env(
        self,
        body: str = PHRASE,
        user_id: int = 7,
        action: str = "created",
        previous: str | None = None,
    ) -> dict[str, str]:
        payload: dict[str, Any] = {
            "action": action,
            "issue": {
                "number": 4,
                "pull_request": {"url": "u"},
                "user": {"id": 7, "login": "dev"},
            },
            "comment": {
                "id": 99,
                "body": body,
                "user": {"id": user_id, "login": "dev"},
                "html_url": COMMENT_URL,
            },
        }
        if previous is not None:
            payload["changes"] = {"body": {"from": previous}}
        return event(self.tmp, "issue_comment", payload)

    def record(
        self, uid: int, comment_id: int, repo: str = "o/r", version: str = "3"
    ) -> dict[str, Any]:
        return {
            "id": uid,
            "login": "u",
            "cla_version": version,
            "repo": repo,
            "pr": 4,
            "comment_id": comment_id,
            "signed_at": "t",
        }

    def signed_doc(self) -> dict[str, Any]:
        return {"signatures": [self.record(7, 99)]}

    def test_sign_records_signature_in_the_signatures_repo_and_posts_a_passing_check(
        self,
    ) -> None:
        api = FakeGitHub([commit(7, "dev")], doc=sigs())
        self.assertEqual(
            self.go(self.comment_env(), api, now="2026-01-01T00:00:00Z"), 0
        )
        self.assertEqual(
            api.doc["signatures"][0],  # type: ignore[index]
            {
                "id": 7,
                "login": "dev",
                "cla_version": "3",
                "cla_sha256": "d" * 64,
                "repo": "o/r",
                "pr": 4,
                "head_sha": HEAD,
                "comment_id": 99,
                "comment_url": COMMENT_URL,
                "comment_body_sha256": hashlib.sha256(PHRASE.encode()).hexdigest(),
                "signed_at": "2026-01-01T00:00:00Z",
            },
        )
        posted = api.posted()
        self.assertEqual(len(posted), 1)
        self.assertEqual(
            (
                posted[0]["name"],
                posted[0]["head_sha"],
                posted[0]["conclusion"],
                posted[0]["external_id"],
            ),
            ("CLA", HEAD, "success", "cla:3"),
        )

    def test_sign_posts_a_failing_check_while_another_author_is_unsigned(self) -> None:
        api = FakeGitHub([commit(7, "dev"), commit(8, "late")], doc=sigs())
        self.assertEqual(self.go(self.comment_env(), api), 0)
        posted = api.posted()
        self.assertEqual(posted[0]["conclusion"], "failure")
        self.assertIn("@late", posted[0]["output"]["summary"])

    def test_sign_ignores_non_phrase_comment(self) -> None:
        api = FakeGitHub([commit(7, "dev")], doc=sigs())
        self.assertEqual(self.go(self.comment_env("LGTM"), api), 0)
        self.assertEqual(api.calls, [])

    def test_sign_refuses_commenter_who_is_neither_author_nor_opener(self) -> None:
        api = FakeGitHub([commit(7, "dev")], doc=sigs())
        self.assertEqual(self.go(self.comment_env(user_id=666), api), 0)
        self.assertEqual(api.doc, sigs())
        self.assertEqual(api.writes(), [])

    def test_sign_accepts_commit_author_who_is_not_the_opener(self) -> None:
        api = FakeGitHub([commit(5, "co")], doc=sigs())
        self.assertEqual(self.go(self.comment_env(user_id=5), api), 0)
        self.assertEqual(api.doc["signatures"][0]["id"], 5)  # type: ignore[index]

    def test_sign_twice_writes_the_record_once_but_still_refreshes_the_check(
        self,
    ) -> None:
        api = FakeGitHub([commit(7, "dev")], doc=sigs((7, "3")))
        self.assertEqual(self.go(self.comment_env(), api), 0)
        self.assertEqual([c for c in api.calls if c[0] == "PUT"], [])
        self.assertEqual(len(api.posted()), 1)

    def test_sign_retries_conflicts_then_succeeds(self) -> None:
        api = FakeGitHub([commit(7, "dev")], doc=sigs(), put_failures=[409, 422])
        self.assertEqual(self.go(self.comment_env(), api), 0)
        self.assertEqual(len([c for c in api.calls if c[0] == "PUT"]), 3)
        self.assertEqual(len(api.doc["signatures"]), 1)  # type: ignore[index]

    def test_sign_gives_up_after_bounded_retries_without_posting(self) -> None:
        api = FakeGitHub([commit(7, "dev")], doc=sigs(), put_failures=[409] * 20)
        self.assertEqual(self.go(self.comment_env(), api), 1)
        self.assertEqual(len([c for c in api.calls if c[0] == "PUT"]), cla.MAX_ATTEMPTS)
        self.assertEqual(api.posted(), [])

    def test_sign_creates_the_signature_file_when_missing(self) -> None:
        api = FakeGitHub([commit(7, "dev")], doc=None)
        self.assertEqual(self.go(self.comment_env(), api), 0)
        put = next(c for c in api.calls if c[0] == "PUT")
        self.assertNotIn("sha", put[2])
        self.assertEqual(api.doc["signatures"][0]["id"], 7)  # type: ignore[index]

    def test_editing_a_comment_into_the_phrase_signs(self) -> None:
        api = FakeGitHub([commit(7, "dev")], doc=sigs())
        self.assertEqual(
            self.go(self.comment_env(action="edited", previous="draft"), api), 0
        )
        self.assertEqual(api.doc["signatures"][0]["comment_id"], 99)  # type: ignore[index]

    def test_match_only_is_true_for_sign_and_revoke_events_only(self) -> None:
        self.assertTrue(cla.matches(self.comment_env(), CONFIG))
        self.assertTrue(cla.matches(self.comment_env(action="deleted"), CONFIG))
        self.assertTrue(
            cla.matches(self.comment_env("x", action="edited", previous=PHRASE), CONFIG)
        )
        self.assertFalse(cla.matches(self.comment_env("nope"), CONFIG))
        self.assertFalse(
            cla.matches(self.comment_env("nope", action="deleted"), CONFIG)
        )

    # ---- reconcile: a signature stands only while its source comment still holds the phrase ----

    def test_signing_drops_the_signers_records_whose_comment_is_gone_or_changed(
        self,
    ) -> None:
        doc = {
            "signatures": [
                self.record(7, 50, version="2"),
                self.record(7, 51, version="2"),
            ]
        }
        api = FakeGitHub(
            [commit(7, "dev")], doc=doc, comments={99: PHRASE, 51: "edited away"}
        )
        self.assertEqual(self.go(self.comment_env(), api), 0)
        self.assertEqual([r["comment_id"] for r in api.doc["signatures"]], [99])  # type: ignore[index]

    def test_reconcile_keeps_a_record_whose_comment_still_holds_its_own_versions_phrase(
        self,
    ) -> None:
        old = self.record(7, 50, version="2")
        api = FakeGitHub(
            [commit(7, "dev")],
            doc={"signatures": [old]},
            comments={99: PHRASE, 50: cla.signing_phrase("2")},
        )
        self.assertEqual(self.go(self.comment_env(), api), 0)
        self.assertEqual([r["comment_id"] for r in api.doc["signatures"]], [50, 99])  # type: ignore[index]

    def test_reconcile_leaves_other_users_and_other_repos_alone(self) -> None:
        doc = {
            "signatures": [
                self.record(8, 60),
                self.record(7, 61, repo="other/r", version="2"),
            ]
        }
        api = FakeGitHub([commit(7, "dev")], doc=doc, comments={99: PHRASE})
        self.assertEqual(self.go(self.comment_env(), api), 0)
        self.assertEqual([r["comment_id"] for r in api.doc["signatures"]], [60, 61, 99])  # type: ignore[index]
        self.assertFalse(
            any("/comments/60" in c[1] or "/comments/61" in c[1] for c in api.calls)
        )

    def test_reconcile_keeps_a_record_when_the_comment_lookup_errors(self) -> None:
        doc = {"signatures": [self.record(7, 50, version="2")]}
        api = FakeGitHub(
            [commit(7, "dev")], doc=doc, comments={99: PHRASE}, comment_status={50: 500}
        )
        self.assertEqual(self.go(self.comment_env(), api), 0)
        self.assertEqual([r["comment_id"] for r in api.doc["signatures"]], [50, 99])  # type: ignore[index]

    def test_a_signing_comment_deleted_before_processing_records_nothing(self) -> None:
        api = FakeGitHub([commit(7, "dev")], doc=sigs(), comments={})
        self.assertEqual(self.go(self.comment_env(), api), 0)
        self.assertEqual(api.doc, sigs())

    # ---- revocation is global: every open PR of the user gets a failing check ----

    def test_deleting_the_signing_comment_removes_the_record(self) -> None:
        api = FakeGitHub([commit(7, "dev")], doc=self.signed_doc(), comments={})
        self.assertEqual(self.go(self.comment_env(action="deleted"), api), 0)
        self.assertEqual(api.doc, {"signatures": []})

    def test_editing_the_phrase_away_removes_the_record(self) -> None:
        api = FakeGitHub(
            [commit(7, "dev")], doc=self.signed_doc(), comments={99: "never mind"}
        )
        env = self.comment_env("never mind", action="edited", previous=PHRASE)
        self.assertEqual(self.go(env, api), 0)
        self.assertEqual(api.doc, {"signatures": []})

    def test_deleting_a_comment_also_drops_the_users_other_stale_records(self) -> None:
        doc = {"signatures": [self.record(7, 50, version="2"), self.record(7, 99)]}
        api = FakeGitHub([commit(7, "dev")], doc=doc, comments={})
        self.assertEqual(self.go(self.comment_env(action="deleted"), api), 0)
        self.assertEqual(api.doc, {"signatures": []})

    def test_deleting_a_comment_with_no_record_changes_nothing_but_still_invalidates(
        self,
    ) -> None:
        # a retry after a failed invalidation finds the record already gone; it must still post
        api = FakeGitHub([commit(7, "dev")], doc=sigs(), comments={})
        self.assertEqual(self.go(self.comment_env(action="deleted"), api), 0)
        self.assertEqual(api.puts(), [])
        self.assertEqual([p["conclusion"] for p in api.posted()], ["failure"])

    def test_a_retry_after_a_failed_invalidation_completes_it(self) -> None:
        api = FakeGitHub(
            [commit(7, "dev")],
            doc=self.signed_doc(),
            comments={},
            check_post_status={HEAD: 403},
        )
        self.assertEqual(self.go(self.comment_env(action="deleted"), api), 1)
        self.assertEqual(api.doc, {"signatures": []})
        api.check_post_status = {}
        self.assertEqual(self.go(self.comment_env(action="deleted"), api), 0)
        self.assertEqual(
            [p["conclusion"] for p in api.posted()], ["failure", "failure"]
        )

    def test_deleting_an_unrelated_comment_makes_no_call(self) -> None:
        api = FakeGitHub([commit(7, "dev")], doc=self.signed_doc())
        self.assertEqual(self.go(self.comment_env("LGTM", action="deleted"), api), 0)
        self.assertEqual(api.calls, [])

    def test_a_concurrent_signers_record_is_preserved_by_the_blob_sha_retry(
        self,
    ) -> None:
        other = {
            "id": 8,
            "login": "late",
            "cla_version": "3",
            "repo": "o/r",
            "pr": 9,
            "comment_id": 7,
            "signed_at": "t",
        }
        api = FakeGitHub(
            [commit(7, "dev")],
            doc=sigs(),
            comments={99: PHRASE, 7: PHRASE},
            concurrent=[other],
        )
        self.assertEqual(self.go(self.comment_env(), api), 0)
        self.assertEqual(sorted(r["id"] for r in api.doc["signatures"]), [7, 8])  # type: ignore[index]
        self.assertEqual(
            len(api.puts()), 2
        )  # the first PUT carried a stale sha and was rejected

    def test_a_concurrent_signer_does_not_lose_a_revocation_either(self) -> None:
        other = {
            "id": 8,
            "login": "late",
            "cla_version": "3",
            "repo": "o/r",
            "pr": 9,
            "comment_id": 7,
            "signed_at": "t",
        }
        api = FakeGitHub(
            [commit(7, "dev")],
            doc=self.signed_doc(),
            comments={7: PHRASE},
            concurrent=[other],
        )
        self.assertEqual(self.go(self.comment_env(action="deleted"), api), 0)
        self.assertEqual([r["id"] for r in api.doc["signatures"]], [8])  # type: ignore[index]

    def test_a_comment_deleted_while_the_signature_is_written_is_not_resurrected(
        self,
    ) -> None:
        api = FakeGitHub([commit(7, "dev")], doc=sigs())
        api.after_put = [
            lambda: api.comments.pop(99)
        ]  # the deletion lands after validation, before the end
        self.assertEqual(self.go(self.comment_env(), api), 0)
        self.assertEqual(api.doc, sigs())
        self.assertEqual(api.posted()[-1]["conclusion"], "failure")

    def test_a_comment_edited_while_the_signature_is_written_is_not_resurrected(
        self,
    ) -> None:
        api = FakeGitHub([commit(7, "dev")], doc=sigs())
        api.after_put = [lambda: api.comments.__setitem__(99, "never mind")]
        self.assertEqual(self.go(self.comment_env(), api), 0)
        self.assertEqual(api.doc, sigs())

    def test_a_comment_observation_is_not_reused_across_retries(self) -> None:
        old = self.record(7, 50, version="2")
        api = FakeGitHub(
            [commit(7, "dev")],
            doc={"signatures": [old]},
            comments={99: PHRASE, 50: cla.signing_phrase("2")},
            put_failures=[409],
        )
        api.on_put = [
            lambda: api.comments.pop(50),
            lambda: None,
        ]  # 50 is deleted between the attempts
        self.assertEqual(self.go(self.comment_env(), api), 0)
        self.assertEqual([r["comment_id"] for r in api.doc["signatures"]], [99])  # type: ignore[index]

    def test_a_late_removal_refreshes_the_users_other_open_prs_too(self) -> None:
        a, b = "1" * 40, "2" * 40
        api = FakeGitHub(
            [commit(7, "dev")], doc=sigs(), prs=[Pr(4, 7, a, [7]), Pr(5, 7, b, [7])]
        )
        api.after_put = [lambda: api.comments.pop(99)]
        self.assertEqual(self.go(self.comment_env(), api), 0)
        last = {}
        for p in api.posted():
            last[p["head_sha"]] = p["conclusion"]
        self.assertEqual(last, {a: "failure", b: "failure"})

    def test_a_failed_recheck_of_the_comment_rolls_the_signature_back_and_fails_the_job(
        self,
    ) -> None:
        api = FakeGitHub([commit(7, "dev")], doc=sigs())
        api.after_put = [lambda: api.comment_status.__setitem__(99, 500)]
        self.assertEqual(self.go(self.comment_env(), api), 1)
        self.assertEqual(api.doc, sigs())  # the record that was just written is gone
        self.assertFalse(
            cla.evaluate([commit(7, "dev")], (7, "dev"), api.doc, CONFIG).ok
        )
        self.assertEqual(api.posted()[-1]["conclusion"], "failure")

    def test_a_failed_recheck_keeps_a_signature_this_run_did_not_insert(self) -> None:
        existing = self.record(
            7, 99
        )  # this comment was already recorded by an earlier run
        api = FakeGitHub(
            [commit(7, "dev")], doc={"signatures": [existing]}, comment_status={99: 500}
        )
        self.assertEqual(self.go(self.comment_env(), api), 1)
        self.assertEqual(api.doc, {"signatures": [existing]})
        self.assertEqual(api.puts(), [])

    def test_a_failed_recheck_after_a_conflicting_write_keeps_the_other_writers_record(
        self,
    ) -> None:
        stale = self.record(7, 50)  # same version, its comment was deleted
        theirs = {**self.record(7, 99), "signed_at": "other-writer"}
        api = FakeGitHub(
            [commit(7, "dev")],
            doc={"signatures": [stale]},
            comments={99: PHRASE},
            comment_status={99: 500},
        )

        def other_writer_lands() -> (
            None
        ):  # between our read and our PUT, so the first PUT is rejected
            api.doc = {"signatures": [stale, theirs]}
            api.version += 1

        api.on_put = [other_writer_lands]
        self.assertEqual(self.go(self.comment_env(), api), 1)
        # attempt 2 dropped the stale record and found the signature already there (not inserted by us)
        self.assertEqual(api.doc, {"signatures": [theirs]})
        self.assertEqual(len(api.puts()), 2)
        self.assertEqual([p["conclusion"] for p in api.posted()], ["success"])

    def test_resigning_replaces_a_stale_record_of_the_same_version(self) -> None:
        stale = self.record(7, 50)  # same version, its comment was deleted
        api = FakeGitHub(
            [commit(7, "dev")], doc={"signatures": [stale]}, comments={99: PHRASE}
        )
        self.assertEqual(self.go(self.comment_env(), api), 0)
        self.assertEqual([r["comment_id"] for r in api.doc["signatures"]], [99])  # type: ignore[index]
        self.assertEqual(api.posted()[-1]["conclusion"], "success")

    def test_signing_one_pr_does_not_hide_a_failure_of_another_pr_with_the_same_head(
        self,
    ) -> None:
        prs = [
            Pr(4, 7, HEAD, [7], "o/r", "feat"),
            Pr(5, 7, HEAD, [7, 8], "x/r", "other"),
        ]
        api = FakeGitHub([commit(7, "dev")], doc=sigs(), prs=prs)
        self.assertEqual(self.go(self.comment_env(), api), 0)
        posted = api.posted()
        self.assertEqual([p["head_sha"] for p in posted], [HEAD])
        self.assertEqual(posted[0]["conclusion"], "failure")
        self.assertIn("#5", posted[0]["output"]["summary"])

    def test_revocation_aggregates_prs_sharing_a_head_and_posts_once_per_head(
        self,
    ) -> None:
        prs = [
            Pr(4, 7, HEAD, [7], "o/r", "feat"),
            Pr(5, 8, HEAD, [8], "o/r", "feat"),
            Pr(6, 7, "2" * 40, [7], "o/r", "other"),
            Pr(8, 9, HEAD, [9], "y/r", "third"),
        ]
        api = FakeGitHub(
            [commit(7, "dev")], doc=self.signed_doc(), comments={}, prs=prs
        )
        self.assertEqual(self.go(self.comment_env(action="deleted"), api), 0)
        posted = api.posted()
        self.assertEqual(
            sorted(p["head_sha"] for p in posted), sorted([HEAD, "2" * 40])
        )
        shared = next(p for p in posted if p["head_sha"] == HEAD)
        self.assertIn("#4", shared["output"]["summary"])
        self.assertIn("#5", shared["output"]["summary"])
        self.assertIn("#8", shared["output"]["summary"])

    def test_the_fake_rejects_a_stale_blob_sha(self) -> None:
        api = FakeGitHub([], doc=sigs())
        status, _ = api(
            "PUT", SIG_URL, {"branch": "main", "content": "e30=", "sha": "blob0"}
        )
        self.assertEqual(status, 409)

    def test_a_co_author_resolved_by_noreply_id_can_sign(self) -> None:
        msg = "m\n\nCo-authored-by: Z <55+zed@users.noreply.github.com>"
        api = FakeGitHub([commit(7, "dev", message=msg)], doc=sigs())
        self.assertEqual(self.go(self.comment_env(user_id=55), api), 0)
        self.assertEqual([r["id"] for r in api.doc["signatures"]], [55])  # type: ignore[index]

    def test_revoke_retries_a_conflict(self) -> None:
        api = FakeGitHub(
            [commit(7, "dev")], doc=self.signed_doc(), comments={}, put_failures=[409]
        )
        self.assertEqual(self.go(self.comment_env(action="deleted"), api), 0)
        self.assertEqual(api.doc, {"signatures": []})

    def test_revocation_fails_the_check_on_every_open_pr_of_the_user(self) -> None:
        a, b, c, d = "1" * 40, "2" * 40, "3" * 40, "4" * 40
        prs = [Pr(4, 7, a, [7]), Pr(5, 8, b, [8, 7]), Pr(6, 9, c, [9]), Pr(7, 7, d, [])]
        api = FakeGitHub(
            [commit(7, "dev")], doc=self.signed_doc(), comments={}, prs=prs
        )
        self.assertEqual(self.go(self.comment_env(action="deleted"), api), 0)
        posted = {p["head_sha"]: p["conclusion"] for p in api.posted()}
        self.assertEqual(posted, {a: "failure", b: "failure", d: "failure"})

    def test_revocation_still_updates_the_comments_own_pr_when_it_is_not_listed_open(
        self,
    ) -> None:
        api = FakeGitHub([commit(7, "dev")], doc=self.signed_doc(), comments={})
        api.open_only: list[Pr] = []  # type: ignore[misc]
        listing = api.prs
        api.prs = []
        real = api.__call__

        def closed_pr(method: str, path: str, body: Any = None) -> tuple[int, Any]:
            if method == "GET" and path.endswith("/pulls/4"):
                api.calls.append((method, path, body))
                return 200, api.pr_json(listing[0])
            return real(method, path, body)

        env = self.comment_env(action="deleted")
        self.assertEqual(cla.run(env, closed_pr, closed_pr, CONFIG), 0)
        self.assertEqual([p["head_sha"] for p in api.posted()], [HEAD])

    def test_open_pr_scan_is_fully_paginated_up_to_the_cap(self) -> None:
        prs = [
            Pr(1000 + i, 9, f"{i:040x}", [9])
            for i in range(100 * cla.MAX_OPEN_PR_PAGES - 2)
        ]
        prs.append(Pr(4, 7, HEAD, [7]))
        api = FakeGitHub(
            [commit(7, "dev")], doc=self.signed_doc(), comments={}, prs=prs
        )
        self.assertEqual(self.go(self.comment_env(action="deleted"), api), 0)
        listed = [
            c for c in api.calls if "/pulls?state=open" in c[1] and "head=" not in c[1]
        ]
        self.assertEqual(len(listed), cla.MAX_OPEN_PR_PAGES)

    def test_a_scan_that_hits_the_cap_fails_the_job_but_still_updates_what_it_found(
        self,
    ) -> None:
        prs = [
            Pr(1000 + i, 9, f"{i:040x}", [9])
            for i in range(100 * (cla.MAX_OPEN_PR_PAGES + 2))
        ]
        api = FakeGitHub(
            [commit(7, "dev")],
            doc=self.signed_doc(),
            comments={},
            prs=prs + [Pr(4, 7, HEAD, [7])],
        )
        self.assertEqual(self.go(self.comment_env(action="deleted"), api), 1)
        listed = [
            c for c in api.calls if "/pulls?state=open" in c[1] and "head=" not in c[1]
        ]
        self.assertEqual(len(listed), cla.MAX_OPEN_PR_PAGES)
        self.assertIn(HEAD, [p["head_sha"] for p in api.posted()])

    def test_one_failed_post_does_not_stop_the_others_but_fails_the_job(self) -> None:
        a, b = "1" * 40, "2" * 40
        api = FakeGitHub(
            [commit(7, "dev")],
            doc=self.signed_doc(),
            comments={},
            prs=[Pr(4, 7, a, [7]), Pr(5, 7, b, [7])],
            check_post_status={a: 403},
        )
        self.assertEqual(self.go(self.comment_env(action="deleted"), api), 1)
        self.assertEqual({p["head_sha"] for p in api.posted()}, {a, b})

    def test_a_grant_updates_only_the_signed_pr(self) -> None:
        a, b = "1" * 40, "2" * 40
        api = FakeGitHub(
            [commit(7, "dev")], doc=sigs(), prs=[Pr(4, 7, a, [7]), Pr(5, 7, b, [7])]
        )
        self.assertEqual(self.go(self.comment_env(), api), 0)
        self.assertEqual([p["head_sha"] for p in api.posted()], [a])


def run_payload(
    event_name: str, head_sha: str, branch: str, repo: str = "fork/r"
) -> dict[str, Any]:
    owner = repo.split("/")[0]
    return {
        "action": "completed",
        "workflow_run": {
            "event": event_name,
            "head_sha": head_sha,
            "head_branch": branch,
            "head_repository": {"full_name": repo, "owner": {"login": owner}},
            "conclusion": "success",
        },
    }


class WorkflowRunPullRequestTests(unittest.TestCase):
    def setUp(self) -> None:
        import tempfile

        self._tmp = tempfile.TemporaryDirectory()
        self.tmp = Path(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)

    def go(self, api: FakeGitHub, payload: dict[str, Any]) -> int:
        return cla.run(event(self.tmp, "workflow_run", payload), api, api, CONFIG)

    def fork_run(self, sha: str = HEAD, branch: str = "feature") -> dict[str, Any]:
        return run_payload("pull_request", sha, branch)

    def test_fork_pr_is_found_by_head_repo_branch_and_sha_and_passes_when_signed(
        self,
    ) -> None:
        api = FakeGitHub(
            [commit(7, "dev")], doc=sigs((7, "3")), prs=[Pr(4, 7, HEAD, [7])]
        )
        self.assertEqual(self.go(api, self.fork_run()), 0)
        posted = api.posted()
        self.assertEqual(len(posted), 1)
        self.assertEqual(
            (
                posted[0]["name"],
                posted[0]["head_sha"],
                posted[0]["conclusion"],
                posted[0]["external_id"],
            ),
            ("CLA", HEAD, "success", "cla:3"),
        )
        lookup = next(c[1] for c in api.calls if "/pulls?state=open" in c[1])
        self.assertNotIn(
            "head=", lookup
        )  # matching is by sha across every head, never narrowed

    def test_unsigned_author_posts_a_failure_with_the_phrase(self) -> None:
        api = FakeGitHub([commit(7, "dev")], doc=sigs(), prs=[Pr(4, 7, HEAD, [7])])
        self.assertEqual(self.go(api, self.fork_run()), 0)
        body = api.posted()[0]
        self.assertEqual(body["conclusion"], "failure")
        self.assertIn("@dev", body["output"]["summary"])
        self.assertIn(PHRASE, body["output"]["summary"])

    def test_no_matching_pr_posts_nothing(self) -> None:
        api = FakeGitHub([commit(7, "dev")], doc=sigs((7, "3")), prs=[])
        self.assertEqual(self.go(api, self.fork_run()), 0)
        self.assertEqual(api.posted(), [])

    def test_a_pr_whose_head_moved_on_is_not_matched(self) -> None:
        api = FakeGitHub(
            [commit(7, "dev")], doc=sigs((7, "3")), prs=[Pr(4, 7, "9" * 40, [7])]
        )
        self.assertEqual(self.go(api, self.fork_run()), 0)
        self.assertEqual(api.posted(), [])

    def test_prs_from_different_forks_sharing_a_sha_are_all_evaluated(self) -> None:
        prs = [
            Pr(4, 7, HEAD, [7]),
            Pr(5, 8, HEAD, [7, 8], head_repo="other/r", branch="elsewhere"),
        ]
        api = FakeGitHub([commit(7, "dev")], doc=sigs((7, "3")), prs=prs)
        self.assertEqual(self.go(api, self.fork_run()), 0)
        posted = api.posted()
        self.assertEqual(len(posted), 1)
        self.assertEqual(posted[0]["conclusion"], "failure")
        self.assertIn("#5", posted[0]["output"]["summary"])

    def test_the_run_does_not_have_to_name_the_head_repository(self) -> None:
        api = FakeGitHub(
            [commit(7, "dev")], doc=sigs((7, "3")), prs=[Pr(4, 7, HEAD, [7])]
        )
        payload = self.fork_run()
        payload["workflow_run"]["head_repository"] = None
        payload["workflow_run"]["head_branch"] = None
        self.assertEqual(self.go(api, payload), 0)
        self.assertEqual(api.posted()[0]["conclusion"], "success")

    def test_a_pr_with_a_deleted_head_repo_is_still_evaluated(self) -> None:
        api = FakeGitHub(
            [commit(7, "dev")], doc=sigs((7, "3")), prs=[Pr(4, 7, HEAD, [7])]
        )
        real = api.__call__

        def gone(method: str, path: str, body: Any = None) -> tuple[int, Any]:
            status, data = real(method, path, body)
            if "/pulls?state=open" in path:
                for item in data:
                    item["head"]["repo"] = None
            return status, data

        self.assertEqual(
            cla.run(
                event(self.tmp, "workflow_run", self.fork_run()), gone, gone, CONFIG
            ),
            0,
        )
        self.assertEqual([p["conclusion"] for p in api.posted()], ["success"])

    def test_an_incomplete_open_pr_listing_posts_a_failure_not_a_pass(self) -> None:
        prs = [
            Pr(1000 + i, 9, f"{i:040x}", [9])
            for i in range(100 * cla.MAX_OPEN_PR_PAGES)
        ]
        api = FakeGitHub(
            [commit(7, "dev")], doc=sigs((7, "3")), prs=prs + [Pr(4, 7, HEAD, [7])]
        )
        self.assertEqual(self.go(api, self.fork_run()), 0)
        self.assertEqual([p["conclusion"] for p in api.posted()], ["failure"])

    def test_several_prs_from_one_head_must_all_pass(self) -> None:
        prs = [Pr(4, 7, HEAD, [7]), Pr(5, 7, HEAD, [7, 8])]
        api = FakeGitHub([commit(7, "dev")], doc=sigs((7, "3")), prs=prs)
        self.assertEqual(self.go(api, self.fork_run()), 0)
        posted = api.posted()
        self.assertEqual(len(posted), 1)
        self.assertEqual(posted[0]["conclusion"], "failure")

    def test_an_undetermined_result_is_a_failure_never_a_pass(self) -> None:
        api = FakeGitHub(
            [commit(7, "dev")],
            doc=sigs((7, "3")),
            prs=[Pr(4, 7, HEAD, [7])],
            commits_status=500,
        )
        self.assertEqual(self.go(api, self.fork_run()), 0)
        self.assertEqual([p["conclusion"] for p in api.posted()], ["failure"])

    def test_a_revocation_landing_during_evaluation_is_reposted(self) -> None:
        api = FakeGitHub(
            [commit(7, "dev")], doc=sigs((7, "3")), prs=[Pr(4, 7, HEAD, [7])]
        )
        api.after_post = [lambda: setattr(api, "doc", sigs())]
        self.assertEqual(self.go(api, self.fork_run()), 0)
        self.assertEqual(
            [p["conclusion"] for p in api.posted()], ["success", "failure"]
        )

    def test_a_signature_landing_during_evaluation_is_reposted(self) -> None:
        api = FakeGitHub([commit(7, "dev")], doc=sigs(), prs=[Pr(4, 7, HEAD, [7])])
        api.after_post = [lambda: setattr(api, "doc", sigs((7, "3")))]
        self.assertEqual(self.go(api, self.fork_run()), 0)
        self.assertEqual(
            [p["conclusion"] for p in api.posted()], ["failure", "success"]
        )

    def test_re_evaluation_is_bounded(self) -> None:
        api = FakeGitHub([commit(7, "dev")], doc=sigs(), prs=[Pr(4, 7, HEAD, [7])])
        flip = [
            lambda i=i: setattr(api, "doc", sigs((7, "3")) if i % 2 == 0 else sigs())
            for i in range(20)
        ]
        api.after_post = flip
        self.assertEqual(self.go(api, self.fork_run()), 0)
        posted = api.posted()
        self.assertEqual(len(posted), cla.MAX_SETTLE + 1)
        self.assertEqual(
            posted[-1]["conclusion"], "failure"
        )  # the last word is never a stale pass

    def test_a_pr_with_250_commits_fails_closed_with_a_clear_message(self) -> None:
        api = FakeGitHub(
            [commit(7, "dev")] * 250, doc=sigs((7, "3")), prs=[Pr(4, 7, HEAD, [7])]
        )
        self.assertEqual(self.go(api, self.fork_run()), 0)
        body = api.posted()[0]
        self.assertEqual(body["conclusion"], "failure")
        self.assertIn("250", body["output"]["summary"])

    def test_249_commits_are_evaluated_normally(self) -> None:
        api = FakeGitHub(
            [commit(7, "dev")] * 249, doc=sigs((7, "3")), prs=[Pr(4, 7, HEAD, [7])]
        )
        self.assertEqual(self.go(api, self.fork_run()), 0)
        self.assertEqual(api.posted()[0]["conclusion"], "success")

    def test_a_failed_post_fails_the_job(self) -> None:
        api = FakeGitHub(
            [commit(7, "dev")],
            doc=sigs((7, "3")),
            prs=[Pr(4, 7, HEAD, [7])],
            check_post_status={HEAD: 403},
        )
        self.assertEqual(self.go(api, self.fork_run()), 1)

    def test_a_malformed_sha_is_rejected_before_any_api_call(self) -> None:
        api = FakeGitHub([commit(7, "dev")], doc=sigs((7, "3")))
        self.assertEqual(self.go(api, self.fork_run(sha="../../x")), 1)
        self.assertEqual(api.calls, [])

    def test_other_triggering_events_are_ignored(self) -> None:
        api = FakeGitHub([commit(7, "dev")], doc=sigs((7, "3")))
        self.assertEqual(self.go(api, run_payload("push", HEAD, "main")), 0)
        self.assertEqual(api.calls, [])

    def test_the_workflow_conclusion_is_not_trusted(self) -> None:
        payload = self.fork_run()
        payload["workflow_run"]["conclusion"] = "success"
        api = FakeGitHub([commit(7, "dev")], doc=sigs(), prs=[Pr(4, 7, HEAD, [7])])
        self.assertEqual(self.go(api, payload), 0)
        self.assertEqual(api.posted()[0]["conclusion"], "failure")


class WorkflowRunMergeGroupTests(unittest.TestCase):
    def setUp(self) -> None:
        import tempfile

        self._tmp = tempfile.TemporaryDirectory()
        self.tmp = Path(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)

    def go(self, api: FakeGitHub, branch: str | None = None, sha: str = MQ) -> int:
        branch = branch if branch is not None else f"gh-readonly-queue/main/pr-5-{MQ}"
        payload = run_payload("merge_group", sha, branch, repo="o/r")
        return cla.run(event(self.tmp, "workflow_run", payload), api, api, CONFIG)

    def api(
        self,
        doc: dict[str, Any] | None = None,
        checks: dict[str, list[dict[str, Any]]] | None = None,
        graphql: dict[str, Any] | None = None,
        prs: list[Pr] | None = None,
    ) -> FakeGitHub:
        a, b = "1" * 40, "2" * 40
        prs = (
            prs
            if prs is not None
            else [Pr(4, 7, a, [7], "o/r"), Pr(5, 8, b, [8], "o/r")]
        )
        checks = (
            checks if checks is not None else {a: [good_check()], b: [good_check()]}
        )
        graphql = graphql if graphql is not None else queue((1, M1, 4), (2, MQ, 5))
        return FakeGitHub(
            [commit(7, "dev")],
            doc=doc if doc is not None else sigs((7, "3"), (8, "3")),
            prs=prs,
            app_checks=checks,
            graphql=graphql,
        )

    def result(self, api: FakeGitHub) -> tuple[str, str]:
        posted = api.posted()
        self.assertEqual(len(posted), 1)
        self.assertEqual(posted[0]["head_sha"], MQ)
        self.assertEqual(posted[0]["name"], "CLA")
        return posted[0]["conclusion"], posted[0]["output"]["summary"]

    def test_all_prs_in_the_group_signed_and_checked_passes(self) -> None:
        api = self.api()
        self.assertEqual(self.go(api), 0)
        self.assertEqual(self.result(api)[0], "success")

    def test_the_prs_ahead_in_the_queue_are_checked_too(self) -> None:
        api = self.api(doc=sigs((8, "3")))  # author 7 of the PR ahead is not signed
        self.assertEqual(self.go(api), 0)
        conclusion, summary = self.result(api)
        self.assertEqual(conclusion, "failure")
        self.assertIn("#4", summary)

    def test_entries_behind_the_group_head_are_not_required(self) -> None:
        graphql = queue((1, M1, 4), (2, MQ, 5), (3, "d" * 40, 6))
        api = self.api(
            graphql=graphql,
            prs=[
                Pr(4, 7, "1" * 40, [7], "o/r"),
                Pr(5, 8, "2" * 40, [8], "o/r"),
                Pr(6, 9, "3" * 40, [9], "o/r"),
            ],
        )
        self.assertEqual(self.go(api), 0)
        self.assertEqual(self.result(api)[0], "success")

    def test_a_missing_pr_check_run_fails(self) -> None:
        a, b = "1" * 40, "2" * 40
        api = self.api(checks={a: [good_check()], b: []})
        self.assertEqual(self.go(api), 0)
        conclusion, summary = self.result(api)
        self.assertEqual(conclusion, "failure")
        self.assertIn("#5", summary)

    def test_a_check_run_for_another_cla_version_fails(self) -> None:
        a, b = "1" * 40, "2" * 40
        api = self.api(checks={a: [good_check()], b: [good_check(version="2")]})
        self.assertEqual(self.go(api), 0)
        self.assertEqual(self.result(api)[0], "failure")

    def test_a_failing_pr_check_run_fails(self) -> None:
        a, b = "1" * 40, "2" * 40
        api = self.api(
            checks={a: [good_check()], b: [good_check(conclusion="failure")]}
        )
        self.assertEqual(self.go(api), 0)
        self.assertEqual(self.result(api)[0], "failure")

    def test_the_newest_check_run_decides(self) -> None:
        a, b = "1" * 40, "2" * 40
        revoked = [good_check(1), good_check(2, conclusion="failure")]
        api = self.api(checks={a: [good_check()], b: revoked})
        self.assertEqual(self.go(api), 0)
        self.assertEqual(self.result(api)[0], "failure")
        recovered = [good_check(1, conclusion="failure"), good_check(2)]
        api = self.api(checks={a: [good_check()], b: recovered})
        self.assertEqual(self.go(api), 0)
        self.assertEqual(self.result(api)[0], "success")

    def test_a_signature_revoked_since_the_pr_passed_fails_even_with_a_stale_pass(
        self,
    ) -> None:
        api = self.api(doc=sigs((7, "3")))  # author 8 of PR 5 no longer has a signature
        self.assertEqual(self.go(api), 0)
        self.assertEqual(self.result(api)[0], "failure")

    def test_an_unreadable_queue_fails_closed(self) -> None:
        for graphql in [
            {"errors": [{"message": "x"}]},
            {"data": {"repository": {"mergeQueue": None}}},
        ]:
            api = self.api(graphql=graphql)
            self.assertEqual(self.go(api), 0)
            self.assertEqual(self.result(api)[0], "failure")

    def test_a_group_head_missing_from_the_queue_fails_closed(self) -> None:
        api = self.api(graphql=queue((1, M1, 4)))
        self.assertEqual(self.go(api), 0)
        self.assertEqual(self.result(api)[0], "failure")

    def test_the_ref_pr_must_be_in_the_matched_entries(self) -> None:
        api = self.api(graphql=queue((1, M1, 4), (2, MQ, 9)))
        self.assertEqual(self.go(api), 0)
        self.assertEqual(self.result(api)[0], "failure")

    def test_a_truncated_queue_listing_fails_closed(self) -> None:
        api = self.api(graphql=queue((1, M1, 4), (2, MQ, 5), total=500))
        self.assertEqual(self.go(api), 0)
        self.assertEqual(self.result(api)[0], "failure")

    def test_a_ref_that_is_not_a_queue_ref_posts_nothing(self) -> None:
        bad = [
            "main",
            "feature",
            f"gh-readonly-queue/main/pr-5-{'e' * 40}",
            "gh-readonly-queue/main/pr-x-" + MQ,
        ]
        for branch in bad:
            api = self.api()
            self.assertEqual(self.go(api, branch=branch), 1, branch)
            self.assertEqual(api.posted(), [])

    def test_the_graphql_query_names_the_target_branch_from_the_ref(self) -> None:
        api = self.api()
        self.go(api, branch=f"gh-readonly-queue/release/1.x/pr-5-{MQ}")
        body = next(c[2] for c in api.calls if c[1] == "/graphql")
        self.assertEqual(
            body["variables"], {"owner": "o", "name": "r", "branch": "release/1.x"}
        )


class _Recorder(BaseHTTPRequestHandler):
    """Answers from the server's ``responses`` and records what the adapter sent."""

    def _serve(self) -> None:
        length = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(length) if length else b""
        self.server.seen.append(
            {  # type: ignore[attr-defined]
                "method": self.command,
                "path": self.path,
                "body": body,
                "headers": {k.lower(): v for k, v in self.headers.items()},
            }
        )
        status, payload = self.server.responses.pop(0)  # type: ignore[attr-defined]
        data = payload if isinstance(payload, bytes) else json.dumps(payload).encode()
        self.send_response(status)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    do_GET = do_POST = do_PUT = _serve

    def log_message(self, *args: Any) -> None:  # keep test output quiet
        pass


class HttpAdapterTests(unittest.TestCase):
    """The production transport against a local server: headers, bodies, error mapping."""

    def setUp(self) -> None:
        self.server = ThreadingHTTPServer(("127.0.0.1", 0), _Recorder)
        self.server.seen = []  # type: ignore[attr-defined]
        self.server.responses = []  # type: ignore[attr-defined]
        thread = threading.Thread(
            target=self.server.serve_forever,
            kwargs={"poll_interval": 0.01},
            daemon=True,
        )
        thread.start()
        self.addCleanup(thread.join)
        self.addCleanup(self.server.server_close)
        self.addCleanup(self.server.shutdown)
        self.url = f"http://127.0.0.1:{self.server.server_address[1]}"

    def reply(self, status: int, payload: Any) -> None:
        self.server.responses.append((status, payload))  # type: ignore[attr-defined]

    def seen(self) -> list[dict[str, Any]]:
        return self.server.seen  # type: ignore[attr-defined,no-any-return]

    def test_get_sends_auth_accept_and_api_version_and_parses_json(self) -> None:
        self.reply(200, {"ok": True})
        status, data = cla.github_transport("tok", self.url)(
            "GET", "/repos/o/r/pulls?state=open", None
        )
        self.assertEqual((status, data), (200, {"ok": True}))
        sent = self.seen()[0]
        self.assertEqual(
            (sent["method"], sent["path"]), ("GET", "/repos/o/r/pulls?state=open")
        )
        self.assertEqual(sent["headers"]["authorization"], "Bearer tok")
        self.assertEqual(sent["headers"]["accept"], "application/vnd.github+json")
        self.assertEqual(sent["headers"]["x-github-api-version"], "2022-11-28")

    def test_post_and_put_send_a_json_body_with_a_json_content_type(self) -> None:
        self.reply(201, {"id": 1})
        self.reply(200, {})
        call = cla.github_transport("tok", self.url)
        self.assertEqual(
            call("POST", "/repos/o/r/check-runs", {"name": "CLA"}), (201, {"id": 1})
        )
        self.assertEqual(call("PUT", "/repos/o/c/contents/f", {"sha": "s"}), (200, {}))
        post, put = self.seen()
        self.assertEqual(json.loads(post["body"]), {"name": "CLA"})
        self.assertEqual(json.loads(put["body"]), {"sha": "s"})
        for sent in (post, put):
            self.assertEqual(sent["headers"]["content-type"], "application/json")

    def test_no_token_means_no_authorization_header(self) -> None:
        self.reply(200, {})
        cla.github_transport("", self.url)("GET", "/x", None)
        self.assertNotIn("authorization", self.seen()[0]["headers"])

    def test_json_error_body_is_returned_with_its_status(self) -> None:
        self.reply(409, {"message": "sha does not match"})
        self.assertEqual(
            cla.github_transport("t", self.url)("PUT", "/x", {}),
            (409, {"message": "sha does not match"}),
        )

    def test_a_404_is_a_status_not_an_exception(self) -> None:
        self.reply(404, {"message": "Not Found"})
        self.reply(404, b"")
        call = cla.github_transport("t", self.url)
        self.assertEqual(call("GET", "/a", None), (404, {"message": "Not Found"}))
        self.assertEqual(call("GET", "/b", None), (404, {}))

    def test_a_non_json_error_body_becomes_a_message(self) -> None:
        self.reply(502, b"<html>Bad gateway</html>")
        status, data = cla.github_transport("t", self.url)("GET", "/x", None)
        self.assertEqual((status, data), (502, {"message": "<html>Bad gateway</html>"}))

    def test_an_empty_success_body_is_an_empty_object(self) -> None:
        self.reply(204, b"")
        self.assertEqual(
            cla.github_transport("t", self.url)("GET", "/x", None), (204, {})
        )

    def test_an_unreachable_server_is_a_runtime_error_with_a_clear_message(
        self,
    ) -> None:
        self.server.shutdown()
        self.server.server_close()  # the port now refuses connections
        with self.assertRaisesRegex(RuntimeError, "GET /x"):
            cla.github_transport("t", self.url)("GET", "/x", None)
