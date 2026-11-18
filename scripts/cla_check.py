"""In-repo CLA check: verify that every contributor on a pull request signed the current CLA.

The CLA result is a check run named ``CLA`` that the CLA GitHub App creates. Workflow jobs cannot
supply it: a fork PR can edit any workflow that runs on ``pull_request`` and make a job pass. Three
flows run from the default branch (``route``), never from PR code:

* ``workflow_run`` (the trigger-only ``CLA`` workflow finished): for a ``pull_request`` run, find the
  open PR by head repository, branch and sha, evaluate its contributors and post the check run on
  the head sha. For a ``merge_group`` run, resolve the queue entries up to that group through the
  merge queue API, require every PR to pass now and to hold a current-version passing app check
  run, and post on the group commit. The workflow's own conclusion is never trusted.
* ``sign`` (issue_comment on a PR): when the comment is exactly the signing phrase and the commenter
  is a contributor of that PR, record the signature, then post a fresh check run for that PR.
* ``revoke`` (a signing comment was edited away or deleted): remove the signature, reconcile the
  user's other records against their source comments, then post a fresh check run for every open
  PR of that user. Posting happens whether or not storage changed, so a retry finishes a half-done
  invalidation.

Identity is the numeric GitHub user id; logins change. Only the allowlist matches by login.
Nothing here executes PR content, and event data is read from the payload file, never from a shell
expression. Stdlib only. The pure functions come first; HTTP and file access sit at the end.
"""

from __future__ import annotations

import base64
import hashlib
import json
import os
import re
import sys
import urllib.error
import urllib.parse
import urllib.request
from collections.abc import Callable
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

CHECK_NAME = "CLA"
MAX_PR_COMMITS = 250  # the pull request commits endpoint lists at most this many
MAX_SETTLE = 3  # evaluations posted per event while signatures keep changing underneath
SHA = re.compile(r"[0-9a-f]{40}")
QUEUE_REF = re.compile(r"gh-readonly-queue/(?P<branch>.+)/pr-(?P<number>\d+)-(?P<sha>[0-9a-f]{40})")
QUEUE_QUERY = (
    "query($owner:String!,$name:String!,$branch:String!){repository(owner:$owner,name:$name)"
    "{mergeQueue(branch:$branch){entries(first:100){totalCount nodes{position headCommit{oid}"
    " pullRequest{number}}}}}}"
)
MAX_ATTEMPTS = 5
MAX_OPEN_PR_PAGES = 20  # 2000 open pull requests; beyond that the scan is incomplete and fails closed
PAGE_SIZE = 100
CONFIG_PATH = Path(__file__).resolve().parent.parent / ".github" / "cla.json"
COAUTHOR = re.compile(r"^co-authored-by:\s*(.*?)\s*<([^<>]*)>\s*$", re.IGNORECASE | re.MULTILINE)
NOREPLY = re.compile(r"^(\d+)\+([A-Za-z0-9-]+(?:\[bot\])?)@users\.noreply\.github\.com$")

Transport = Callable[[str, str, Any], "tuple[int, Any]"]


@dataclass(frozen=True)
class Config:
    version: str
    document: str
    allowlist: tuple[str, ...]
    signatures_repo: str = "slopvac/cla-signers"
    signatures_branch: str = "main"
    signatures_path: str = "signatures/v1/cla.json"
    app_id: int = 0
    document_sha256: str = ""


@dataclass
class Verdict:
    unsigned: list[tuple[int, str | None]] = field(default_factory=list)  # login None: only an id is known
    unlinked: list[tuple[str, str, str]] = field(default_factory=list)  # sha, email, role
    warnings: list[str] = field(default_factory=list)

    @property
    def ok(self) -> bool:
        return not self.unsigned and not self.unlinked


def parse_config(raw: dict[str, Any]) -> Config:
    return Config(
        version=str(raw["version"]),
        document=str(raw["document"]),
        allowlist=tuple(raw.get("allowlist", [])),
        signatures_repo=str(raw["signatures_repo"]),
        signatures_branch=str(raw["signatures_branch"]),
        signatures_path=str(raw["signatures_path"]),
        app_id=int(raw.get("app_id", 0)),
        document_sha256=str(raw["document_sha256"]),
    )


def signing_phrase(version: str) -> str:
    return f"I have read the slopvac CLA version {version} and I hereby sign it."


def normalise_body(body: str) -> str:
    """The comparable form of a comment body: LF line endings, outer whitespace trimmed."""
    return body.replace("\r\n", "\n").strip()


def is_signing_comment(body: str, config: Config) -> bool:
    return normalise_body(body) == signing_phrase(config.version)


def document_sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def verify_document(data: bytes, config: Config) -> None:
    """Refuse to run when the agreement text is not the text that the version id and hash name."""
    actual = document_sha256(data)
    if actual != config.document_sha256:
        raise RuntimeError(
            f"{config.document} (sha256 {actual}) does not match the hash in .github/cla.json "
            f"({config.document_sha256}); change the version and the hash together."
        )


def allowlisted(login: str, logins: tuple[str, ...]) -> bool:
    """Case-insensitive exact login match. There is no wildcard: each exempt account is listed."""
    return login.lower() in {entry.lower() for entry in logins}


def parse_coauthors(message: str) -> list[tuple[str, str]]:
    return [(name, email) for name, email in COAUTHOR.findall(message)]


def noreply_identity(email: str) -> tuple[int, str] | None:
    match = NOREPLY.match(email)
    return (int(match.group(1)), match.group(2)) if match else None


def signed_ids(doc: dict[str, Any], version: str) -> set[int]:
    return {s["id"] for s in doc.get("signatures", []) if s.get("cla_version") == version}


def contributors(commits: list[dict[str, Any]], opener: tuple[int, str]) -> dict[int, str | None]:
    """Everyone who must have signed, and who may sign: the opener, the linked commit authors and
    committers, and the co-authors named by a GitHub noreply address. Checking and signing share this one set.

    A login is known only where the API supplied it (the opener and the commit author objects).
    A Co-authored-by trailer is free text: it contributes an id and never a login, so it cannot
    lend an allowlisted name such as ``dependabot[bot]`` to someone who has not signed."""
    people: dict[int, str | None] = {opener[0]: opener[1]}
    for c in commits:
        for role in ("author", "committer"):
            if c.get(role) is not None:
                people.setdefault(c[role]["id"], c[role]["login"])
    for c in commits:
        for _name, email in parse_coauthors(c["commit"].get("message", "")):
            identity = noreply_identity(email)
            if identity is not None:
                people.setdefault(identity[0], None)
    return people


def evaluate(
    commits: list[dict[str, Any]], opener: tuple[int, str], doc: dict[str, Any], config: Config
) -> Verdict:
    """Decide who still has to sign. Unlinked authors fail; unverifiable co-authors only warn."""
    verdict = Verdict()
    for c in commits:
        author_email = c["commit"].get("author", {}).get("email", "")
        if c.get("author") is None:
            verdict.unlinked.append((c["sha"], author_email, "author"))
        committer_email = c["commit"].get("committer", {}).get("email", "")
        if c.get("committer") is None and not (c.get("author") is None and committer_email == author_email):
            verdict.unlinked.append((c["sha"], committer_email, "committer"))
        for _name, email in parse_coauthors(c["commit"].get("message", "")):
            if noreply_identity(email) is None and email not in verdict.warnings:
                verdict.warnings.append(email)
    have = signed_ids(doc, config.version)
    verdict.unsigned = [
        (uid, login)
        for uid, login in contributors(commits, opener).items()
        if uid not in have and not (login is not None and allowlisted(login, config.allowlist))
    ]
    return verdict


def report(verdict: Verdict, config: Config) -> str:
    lines: list[str] = []
    if verdict.unsigned:
        who = ", ".join(
            f"@{login}" if login else f"user id {uid} (named in a Co-authored-by trailer)"
            for uid, login in verdict.unsigned
        )
        lines += [
            f"CLA not signed by: {who}.",
            f"Read {config.document}, then comment exactly this on the pull request:",
            f"    {signing_phrase(config.version)}",
        ]
    for sha, email, role in verdict.unlinked:
        lines.append(
            f"Commit {sha[:7]} has {role} email {email or '(none)'}, which is not linked to a "
            "GitHub account. Add that email to your GitHub account settings, or rewrite the commit "
            "with an email that GitHub links to you."
        )
    for email in verdict.warnings:
        lines.append(f"Warning: Co-authored-by {email} cannot be verified and was not checked.")
    return "\n".join(lines) or "CLA satisfied."


def external_id(config: Config) -> str:
    return f"cla:{config.version}"


def check_run_body(head_sha: str, ok: bool, title: str, summary: str, config: Config) -> dict[str, Any]:
    return {
        "name": CHECK_NAME,
        "head_sha": head_sha,
        "status": "completed",
        "conclusion": "success" if ok else "failure",
        "external_id": external_id(config),
        "output": {"title": title, "summary": summary[:65535]},
    }


def merge_signature(doc: dict[str, Any], record: dict[str, Any]) -> dict[str, Any]:
    """Add the record unless that user already signed that version; the first record wins."""
    existing = doc.get("signatures", [])
    if any(s.get("id") == record["id"] and s.get("cla_version") == record["cla_version"] for s in existing):
        return {**doc, "signatures": list(existing)}
    return {**doc, "signatures": [*existing, record]}


def route(event_name: str, payload: dict[str, Any]) -> str:
    action = payload.get("action")
    if event_name == "workflow_run" and action == "completed":
        return "workflow_run"
    if (
        event_name == "issue_comment"
        and action in {"created", "edited", "deleted"}
        and payload.get("issue", {}).get("pull_request")
    ):
        return "comment"
    return "ignore"


def comment_action(payload: dict[str, Any], config: Config) -> str:
    """What a PR comment event means: ``sign``, ``revoke`` (a signing comment was edited away or
    deleted) or ``ignore``. The previous body of an edit comes from ``changes.body.from``."""
    action = payload.get("action")
    body = payload["comment"]["body"]
    if action in {"created", "edited"} and is_signing_comment(body, config):
        return "sign"
    if action == "edited":
        previous = payload.get("changes", {}).get("body", {}).get("from", "")
        return "revoke" if is_signing_comment(previous, config) else "ignore"
    if action == "deleted" and is_signing_comment(body, config):
        return "revoke"
    return "ignore"


def drop_comments(doc: dict[str, Any], repo: str, comment_ids: set[int]) -> dict[str, Any]:
    """Drop the records written for those comments of that repository."""
    kept = [
        s for s in doc.get("signatures", [])
        if not (s.get("repo") == repo and s.get("comment_id") in comment_ids)
    ]
    return {**doc, "signatures": kept}


def remove_signatures(doc: dict[str, Any], repo: str, comment_id: int) -> dict[str, Any]:
    """Drop the records from one comment; a record from another comment stays."""
    return drop_comments(doc, repo, {comment_id})


# ---- I/O ----------------------------------------------------------------------------------------


def github_transport(token: str, api_url: str = "https://api.github.com") -> Transport:
    def call(method: str, path: str, body: Any = None) -> tuple[int, Any]:
        data = None if body is None else json.dumps(body).encode()
        request = urllib.request.Request(api_url + path, data=data, method=method)
        request.add_header("Accept", "application/vnd.github+json")
        request.add_header("X-GitHub-Api-Version", "2022-11-28")
        if data is not None:
            request.add_header("Content-Type", "application/json")
        if token:
            request.add_header("Authorization", f"Bearer {token}")
        try:
            with urllib.request.urlopen(request, timeout=30) as response:
                text = response.read().decode()
                return response.status, json.loads(text) if text else {}
        except urllib.error.HTTPError as err:
            text = err.read().decode()
            try:
                return err.code, json.loads(text) if text else {}
            except ValueError:
                return err.code, {"message": text}
        except OSError as err:  # refused, reset, timed out: surface it as a failed check, not a traceback
            raise RuntimeError(f"{method} {path} failed: {err}") from err

    return call


def expect(status: int, ok: tuple[int, ...], what: str) -> None:
    if status not in ok:
        raise RuntimeError(f"{what} failed with HTTP {status}")


def list_commits(api: Transport, repo: str, number: int) -> list[dict[str, Any]]:
    """All commits of a PR. The endpoint stops at 250, so a list that long may be truncated: fail
    closed rather than check part of a PR."""
    commits: list[dict[str, Any]] = []
    page = 1
    while True:
        status, data = api("GET", f"/repos/{repo}/pulls/{number}/commits?per_page={PAGE_SIZE}&page={page}", None)
        expect(status, (200,), "listing pull request commits")
        commits += data
        if len(data) < PAGE_SIZE:
            break
        page += 1
    if len(commits) >= MAX_PR_COMMITS:
        raise RuntimeError(
            f"#{number} has {MAX_PR_COMMITS} or more commits, and GitHub lists at most {MAX_PR_COMMITS}, "
            "so the CLA cannot be checked completely. Split the pull request."
        )
    return commits


def read_signatures(api: Transport, config: Config) -> tuple[dict[str, Any], str | None]:
    path = f"/repos/{config.signatures_repo}/contents/{config.signatures_path}?ref={config.signatures_branch}"
    status, data = api("GET", path, None)
    if status == 404:
        return {"signatures": []}, None
    expect(status, (200,), "reading signatures")
    return json.loads(base64.b64decode(data["content"])), data["sha"]


def update_signatures(
    api: Transport, config: Config, change: Callable[[dict[str, Any]], dict[str, Any]], message: str
) -> str:
    """Apply ``change`` to the signature file with the blob sha, retrying conflicts a bounded number
    of times. Returns ``unchanged``, ``written`` or ``conflict``."""
    for _ in range(MAX_ATTEMPTS):
        doc, sha = read_signatures(api, config)
        updated = change(doc)
        if updated == doc:
            return "unchanged"
        body: dict[str, Any] = {
            "message": message,
            "content": base64.b64encode((json.dumps(updated, indent=2) + "\n").encode()).decode(),
            "branch": config.signatures_branch,
        }
        if sha is not None:
            body["sha"] = sha
        status, _ = api("PUT", f"/repos/{config.signatures_repo}/contents/{config.signatures_path}", body)
        if status in (200, 201):
            return "written"
        if status not in (409, 422):
            raise RuntimeError(f"writing signatures failed with HTTP {status}")
    return "conflict"


def comment_holds_phrase(api: Transport, repo: str, comment_id: int, version: str) -> bool | None:
    """True while the comment exists with exactly the phrase of ``version``; False once it is gone
    (404) or changed; None when the lookup failed, which is no proof either way."""
    status, data = api("GET", f"/repos/{repo}/issues/comments/{comment_id}", None)
    if status == 404:
        return False
    if status != 200:
        return None
    return normalise_body(str(data.get("body", ""))) == signing_phrase(version)


def stale_comments(api: Transport, repo: str, doc: dict[str, Any], user_id: int) -> set[int]:
    """Comment ids of the user's records whose source comment is gone or no longer exactly the phrase
    of the record's version. Each call looks again: nothing is remembered across write retries. A
    lookup error keeps the record, since only proof drops it."""
    stale: set[int] = set()
    for rec in doc.get("signatures", []):
        cid = rec.get("comment_id")
        if rec.get("id") != user_id or rec.get("repo") != repo or cid is None:
            continue
        if comment_holds_phrase(api, repo, cid, str(rec.get("cla_version"))) is False:
            stale.add(cid)
    return stale


def reconcile(api: Transport, repo: str, doc: dict[str, Any], user_id: int) -> dict[str, Any]:
    return drop_comments(doc, repo, stale_comments(api, repo, doc, user_id))


@dataclass
class OpenPulls:
    pulls: list[dict[str, Any]]
    complete: bool  # False when the scan stopped at the page cap with more pages possible


def scan_open_pulls(api: Transport, repo: str) -> OpenPulls:
    pulls: list[dict[str, Any]] = []
    for page in range(1, MAX_OPEN_PR_PAGES + 1):
        status, data = api("GET", f"/repos/{repo}/pulls?state=open&per_page={PAGE_SIZE}&page={page}", None)
        expect(status, (200,), "listing open pull requests")
        pulls += data
        if len(data) < PAGE_SIZE:
            return OpenPulls(pulls, True)
    return OpenPulls(pulls, False)


INCOMPLETE_SCAN = (
    f"More than {MAX_OPEN_PR_PAGES * PAGE_SIZE} pull requests are open, so every pull request that "
    "shares this commit cannot be found."
)


def pulls_sharing_head(scan: OpenPulls, pull: dict[str, Any]) -> list[dict[str, Any]]:
    """Every open PR with the same head sha, whatever fork or branch it comes from. A check run
    belongs to a sha, so its result must cover all of them. Raises when the scan is incomplete."""
    if not scan.complete:
        raise RuntimeError(INCOMPLETE_SCAN)
    same = [p for p in scan.pulls if p["head"]["sha"] == pull["head"]["sha"]]
    if all(p["number"] != pull["number"] for p in same):
        same.append(pull)
    return same


def pulls_of_user(
    user_id: int, own: dict[str, Any], scan: OpenPulls, api: Transport, repo: str
) -> list[dict[str, Any]]:
    """Scanned open pull requests that count the user as a contributor, plus the comment's own. A PR
    whose commits cannot be listed completely counts as affected: it is re-evaluated and fails closed."""
    found = {own["number"]: own}
    for pull in scan.pulls:
        if pull["number"] in found:
            continue
        opener = (pull["user"]["id"], pull["user"]["login"])
        try:
            people = contributors(list_commits(api, repo, pull["number"]), opener)
        except RuntimeError:
            found[pull["number"]] = pull
            continue
        if user_id in people:
            found[pull["number"]] = pull
    return list(found.values())


# ---- check runs ---------------------------------------------------------------------------------

Result = tuple[bool, str, str]


def post_check(api: Transport, repo: str, body: dict[str, Any]) -> None:
    status, _ = api("POST", f"/repos/{repo}/check-runs", body)
    expect(status, (201,), "creating the CLA check run")


def describe(verdicts: list[tuple[int, Verdict]], config: Config) -> Result:
    ok = all(v.ok for _n, v in verdicts)
    if len(verdicts) == 1:
        summary = report(verdicts[0][1], config)
    else:
        summary = "\n\n".join(f"#{n}: {report(v, config)}" for n, v in verdicts)
    return ok, "CLA signed" if ok else "CLA not satisfied", summary


def pull_verdicts(
    api: Transport, repo: str, pulls: list[dict[str, Any]], doc: dict[str, Any], config: Config
) -> list[tuple[int, Verdict]]:
    out = []
    for pull in pulls:
        commits = list_commits(api, repo, pull["number"])
        opener = (pull["user"]["id"], pull["user"]["login"])
        out.append((pull["number"], evaluate(commits, opener, doc, config)))
    return out


def settle(
    api: Transport, sig_api: Transport, repo: str, config: Config, head_sha: str,
    compute: Callable[[dict[str, Any]], Result],
) -> None:
    """Post the result for the signatures as they are now, and again if they changed meanwhile.

    The signatures are read first and re-read after the post, so an evaluation that raced a sign or
    revoke is replaced by one that saw it. Every writer (this job, the sign job) posts after its own
    read, so the last post reflects the newest signatures except in one window: a write that lands
    after the re-read is followed by that writer's own post. If the signatures keep changing past
    ``MAX_SETTLE`` rounds, the last word is a failure: never a stale pass.
    """
    for _ in range(MAX_SETTLE):
        doc, _ = read_signatures(sig_api, config)
        try:
            ok, title, summary = compute(doc)
        except RuntimeError as err:  # undetermined is a failure, never a pass
            ok, title, summary = False, "CLA could not be determined", str(err)
        post_check(api, repo, check_run_body(head_sha, ok, title, summary, config))
        again, _ = read_signatures(sig_api, config)
        if again == doc:
            return
    post_check(api, repo, check_run_body(
        head_sha, False, "CLA could not be determined",
        "The signatures kept changing during evaluation. Comment or re-run the CLA workflow to refresh.",
        config,
    ))


def refresh_pull(
    api: Transport, sig_api: Transport, repo: str, pull: dict[str, Any], config: Config, scan: OpenPulls
) -> None:
    sha = pull["head"]["sha"]
    try:
        pulls = pulls_sharing_head(scan, pull)
    except RuntimeError as err:  # undetermined is a failure, never a pass
        reason = str(err)
        settle(api, sig_api, repo, config, sha, lambda doc: (False, "CLA could not be determined", reason))
        return
    settle(api, sig_api, repo, config, sha,
           lambda doc: describe(pull_verdicts(api, repo, pulls, doc, config), config))


def latest_check(api: Transport, repo: str, sha: str, config: Config) -> dict[str, Any] | None:
    query = f"check_name={CHECK_NAME}&app_id={config.app_id}&filter=latest&per_page=100"
    status, data = api("GET", f"/repos/{repo}/commits/{sha}/check-runs?{query}", None)
    expect(status, (200,), "reading CLA check runs")
    runs = data.get("check_runs", [])
    return max(runs, key=lambda r: r["id"]) if runs else None


def queued_prs(api: Transport, repo: str, branch: str, head_sha: str, ref_pr: int) -> list[int]:
    """Numbers of the PRs in the merge group whose head commit is ``head_sha``, and of every entry
    ahead of it (their commits are part of the group). Anything unclear raises."""
    owner, name = repo.split("/")
    status, data = api("POST", "/graphql", {
        "query": QUEUE_QUERY, "variables": {"owner": owner, "name": name, "branch": branch},
    })
    expect(status, (200,), "reading the merge queue")
    queue = ((data.get("data") or {}).get("repository") or {}).get("mergeQueue")
    if data.get("errors") or queue is None:
        raise RuntimeError(f"the merge queue for {branch} could not be read")
    entries = queue["entries"]
    nodes = entries["nodes"]
    if entries["totalCount"] > len(nodes):
        raise RuntimeError("the merge queue is longer than the listing; its entries cannot all be checked")
    matched = [n for n in nodes if (n.get("headCommit") or {}).get("oid") == head_sha]
    if ref_pr not in {n["pullRequest"]["number"] for n in matched}:
        raise RuntimeError(f"merge group {head_sha[:7]} is not a queue entry of #{ref_pr}")
    top = max(n["position"] for n in matched)
    return [n["pullRequest"]["number"] for n in nodes if n["position"] <= top]


def merge_group_result(
    api: Transport, repo: str, branch: str, sha: str, ref_pr: int, doc: dict[str, Any], config: Config
) -> Result:
    problems: list[str] = []
    for number in queued_prs(api, repo, branch, sha, ref_pr):
        status, pull = api("GET", f"/repos/{repo}/pulls/{number}", None)
        expect(status, (200,), f"reading #{number}")
        ((_n, verdict),) = pull_verdicts(api, repo, [{**pull, "number": number}], doc, config)
        if not verdict.ok:
            problems.append(f"#{number}: {report(verdict, config)}")
        latest = latest_check(api, repo, pull["head"]["sha"], config)
        if latest is None:
            problems.append(f"#{number}: no CLA check run from the CLA app on its head commit.")
        elif latest.get("conclusion") != "success":
            problems.append(f"#{number}: its latest CLA check run did not pass.")
        elif latest.get("external_id") != external_id(config):
            problems.append(f"#{number}: its CLA check run is for another CLA version.")
    if problems:
        return False, "CLA not satisfied", "\n\n".join(problems)
    return True, "CLA signed", "Every pull request in the merge group passed the CLA check."


def handle_workflow_run(api: Transport, sig_api: Transport, repo: str, run: dict[str, Any], config: Config) -> int:
    """Evaluate the commit a finished trigger workflow ran for. All of ``run`` is untrusted data:
    it only selects what to look up, and the lookup must find a real PR or queue entry."""
    kind, sha = run.get("event"), str(run.get("head_sha", ""))
    if kind not in {"pull_request", "merge_group"}:
        return 0
    if not SHA.fullmatch(sha):
        print("The triggering run has no usable head sha; nothing posted.")
        return 1
    if kind == "merge_group":
        queue_ref = QUEUE_REF.fullmatch(str(run.get("head_branch") or ""))
        if queue_ref is None or queue_ref["sha"] != sha:
            print("The merge group branch name does not match its commit; nothing posted.")
            return 1
        branch, ref_pr = queue_ref["branch"], int(queue_ref["number"])
        settle(api, sig_api, repo, config, sha,
               lambda doc: merge_group_result(api, repo, branch, sha, ref_pr, doc, config))
        return 0
    scan = scan_open_pulls(api, repo)
    pulls = [p for p in scan.pulls if p["head"]["sha"] == sha]
    if not scan.complete:
        settle(api, sig_api, repo, config, sha,
               lambda doc: (False, "CLA could not be determined", INCOMPLETE_SCAN))
        return 0
    if not pulls:
        print(f"No open pull request has head {sha[:7]}; nothing posted.")
        return 0
    settle(api, sig_api, repo, config, sha,
           lambda doc: describe(pull_verdicts(api, repo, pulls, doc, config), config))
    return 0


# ---- signing and revoking -----------------------------------------------------------------------


def invalidate_user(
    api: Transport, sig_api: Transport, repo: str, user_id: int, comment_id: int,
    own: dict[str, Any], config: Config,
) -> int:
    """Remove the signature written from a comment that is gone or changed, drop the user's other
    stale records, and post fresh results for every open PR the user is part of.

    Posting happens whether or not storage changed: a retry after a failed post finds the record
    already gone and must still invalidate. Signatures are global, so every open PR of the user is
    refreshed, not just the one with the comment. An incomplete scan fails the job."""
    outcome = update_signatures(
        sig_api, config,
        lambda doc: reconcile(api, repo, remove_signatures(doc, repo, comment_id), user_id),
        f"cla: remove the signature from edited or deleted comment {comment_id} on {repo}#{own['number']}",
    )
    if outcome == "conflict":
        print("Could not remove the signature after repeated conflicts; re-run this job.")
        return 1
    if outcome == "written":
        print(f"Removed the signature recorded from comment {comment_id}.")
    scan = scan_open_pulls(api, repo)
    failed = 0
    if not scan.complete:
        print(f"{INCOMPLETE_SCAN} Pull requests beyond the scan keep their old result: re-run after closing some.")
        failed = 1
    done: set[str] = set()
    for pull in pulls_of_user(user_id, own, scan, api, repo):
        if pull["head"]["sha"] in done:
            continue
        done.add(pull["head"]["sha"])
        try:
            refresh_pull(api, sig_api, repo, pull, config, scan)
        except RuntimeError as err:
            print(f"Could not update the CLA check for #{pull['number']}: {err}")
            failed = 1
    return failed


def sign(api: Transport, sig_api: Transport, repo: str, payload: dict[str, Any], config: Config, now: str) -> int:
    comment, number = payload["comment"], payload["issue"]["number"]
    user = comment["user"]
    status, pull = api("GET", f"/repos/{repo}/pulls/{number}", None)
    expect(status, (200,), "reading the pull request")
    pull = {**pull, "number": number}
    people = contributors(list_commits(api, repo, number), (pull["user"]["id"], pull["user"]["login"]))
    if user["id"] not in people:
        print(f"@{user['login']} is neither the opener nor a contributor of this pull request; not recorded.")
        return 0
    record = {
        "id": user["id"], "login": user["login"], "cla_version": config.version,
        "cla_sha256": config.document_sha256, "repo": repo, "pr": number,
        "head_sha": pull["head"]["sha"], "comment_id": comment["id"],
        "comment_url": comment["html_url"],
        "comment_body_sha256": hashlib.sha256(normalise_body(comment["body"]).encode()).hexdigest(),
        "signed_at": now,
    }
    # Reconcile first: a stale record of this version would otherwise shadow the new signature.
    # `inserted` is whether the last attempt added this run's record; a record that was already there
    # (an earlier run of the same comment, or an older comment of this version) is not ours to undo.
    inserted = False

    def add_record(doc: dict[str, Any]) -> dict[str, Any]:
        nonlocal inserted
        merged = merge_signature(reconcile(api, repo, doc, user["id"]), record)
        inserted = record in merged["signatures"] and record not in doc.get("signatures", [])
        return merged

    outcome = update_signatures(
        sig_api, config, add_record,
        f"cla: {user['login']} signed version {config.version} on {repo}#{number}",
    )
    if outcome == "conflict":
        print("Could not record the signature after repeated conflicts; comment again to retry.")
        return 1
    # The comment may have been deleted or edited after it was validated and before the write
    # landed; the revoking job saw no record then. Check again now that the record exists.
    holds = comment_holds_phrase(api, repo, comment["id"], config.version)
    if holds is None:
        if inserted and outcome == "written":
            # Cannot tell whether the comment still stands: do not leave our record behind. Roll it
            # back (blob-sha safe), refresh the affected checks, and still fail the job.
            print("Could not re-check the signing comment; rolling the signature back.")
            invalidate_user(api, sig_api, repo, user["id"], comment["id"], pull, config)
        else:
            print("Could not re-check the signing comment; the existing signatures are left as they are.")
            if outcome == "written":  # stale records were dropped: the checks need the new result
                refresh_pull(api, sig_api, repo, pull, config, scan_open_pulls(api, repo))
        return 1
    if not holds:
        print("The signing comment changed while the signature was recorded; removing it.")
        return invalidate_user(api, sig_api, repo, user["id"], comment["id"], pull, config)
    print(f"Recorded @{user['login']} for CLA version {config.version}.")
    refresh_pull(api, sig_api, repo, pull, config, scan_open_pulls(api, repo))
    return 0


def revoke(api: Transport, sig_api: Transport, repo: str, payload: dict[str, Any], config: Config) -> int:
    comment, number = payload["comment"], payload["issue"]["number"]
    status, own = api("GET", f"/repos/{repo}/pulls/{number}", None)
    expect(status, (200,), "reading the pull request")
    return invalidate_user(api, sig_api, repo, comment["user"]["id"], comment["id"], {**own, "number": number}, config)


def load_payload(env: dict[str, str]) -> dict[str, Any]:
    return json.loads(Path(env["GITHUB_EVENT_PATH"]).read_text(encoding="utf-8"))


def matches(env: dict[str, str], config: Config) -> bool:
    """True when the event needs the app token: a signing comment, or one edited away or deleted."""
    payload = load_payload(env)
    return route(env["GITHUB_EVENT_NAME"], payload) == "comment" and comment_action(payload, config) != "ignore"


def run(env: dict[str, str], api: Transport, sig_api: Transport, config: Config, now: str | None = None) -> int:
    payload = load_payload(env)
    repo = env["GITHUB_REPOSITORY"]
    try:
        match route(env["GITHUB_EVENT_NAME"], payload):
            case "workflow_run":
                return handle_workflow_run(api, sig_api, repo, payload["workflow_run"], config)
            case "comment":
                match comment_action(payload, config):
                    case "sign":
                        stamp = now or datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
                        return sign(api, sig_api, repo, payload, config, stamp)
                    case "revoke":
                        return revoke(api, sig_api, repo, payload, config)
                    case _:
                        return 0
            case _:
                return 0
    except RuntimeError as err:
        print(f"CLA check failed: {err}")
        return 1


def main(argv: list[str]) -> int:
    env = dict(os.environ)
    config = parse_config(json.loads(CONFIG_PATH.read_text(encoding="utf-8")))
    if argv == ["--match-only"]:
        out = env.get("GITHUB_OUTPUT")
        line = f"matches={'true' if matches(env, config) else 'false'}\n"
        if out:
            with open(out, "a", encoding="utf-8") as handle:
                handle.write(line)
        else:
            print(line, end="")
        return 0
    url = env.get("GITHUB_API_URL", "https://api.github.com")
    try:
        verify_document((CONFIG_PATH.parent.parent / config.document).read_bytes(), config)
    except (RuntimeError, OSError) as err:  # no check run is posted, so the required check stays unmet
        print(f"CLA check failed: {err}")
        return 1
    # GITHUB_TOKEN (the app's, scoped to this repo) reads pull requests and creates check runs.
    # SIGNATURES_TOKEN writes the signatures repo; without it the public repo is read with GITHUB_TOKEN.
    api = github_transport(env.get("GITHUB_TOKEN", ""), url)
    sig_api = github_transport(env.get("SIGNATURES_TOKEN") or env.get("GITHUB_TOKEN", ""), url)
    return run(env, api, sig_api, config)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
