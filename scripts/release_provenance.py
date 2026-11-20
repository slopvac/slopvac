#!/usr/bin/env python3
"""Release tags and the provenance gate for stable releases.

A candidate is an ``rc/vX.Y.Z-rc.N`` tag cut on demand. A stable ``vX.Y.Z`` tag
may publish only when a passed candidate for the same version tested the exact
tree being promoted. Both publish from publish-pypi.yml.

Subcommands:
  parse-tag TAG
      print the Cargo and PyPI versions for a candidate or stable tag, whether
      it is stable, and ``active=true`` when publish-pypi.yml handles it (a
      candidate, or a stable tag of major 3 or later)
  check-stable TAG --repo OWNER/NAME --sha SHA
      exit 0 when TAG points at SHA (the commit the workflow runs on) and a
      passed candidate tested that tree
  verify-tag TAG SHA
      exit 0 when TAG on the remote still resolves to SHA; run it right before
      publishing, so a tag moved after the check cannot be released
  is-legacy VERSION
      print true when VERSION is below major 3, the only versions the
      publish-lint.yml workflow may publish
  check-dist DIR --version VERSION [--project NAME] [--sdist] [--wheels N]
                 [--wheel-targets-from WORKFLOW]
      exit 0 when DIR holds exactly the files an upload of NAME VERSION may
      carry: no extras, each file named for NAME and VERSION, and the project
      name and version inside each file's own metadata equal to them. Run it in
      an upload job, after the download and before the upload
  check-draft TAG --repo OWNER/NAME [--app-slug SLUG]
      exit 0 when the release for TAG is unpublished and was created by
      github-actions[bot] or SLUG[bot] (release-please); read-only
  route-tag TAG
      print ``gated=true`` for vX.Y.Z of major 3 or later (the dist pipeline) and
      ``gated=false`` for earlier majors (publish the release-please draft only);
      exit 1 for any other tag, so a bad tag never skips the gate
"""

from __future__ import annotations

import argparse
import email.parser
import json
import re
import subprocess
import sys
import tarfile
import zipfile
from collections.abc import Callable, Sequence
from dataclasses import dataclass
from pathlib import Path

# SemVer forbids leading zeros in every numeric field, and \d would also accept
# non-ASCII digits, so each field is spelled out.
_NUM = r"(?:0|[1-9][0-9]*)"
_BASE = rf"{_NUM}\.{_NUM}\.{_NUM}"
_RC_TAG = re.compile(rf"rc/v(?P<base>{_BASE})-rc\.(?P<n>[1-9][0-9]*)")
_STABLE_TAG = re.compile(rf"v(?P<base>{_BASE})")
_STABLE_VERSION = re.compile(rf"(?P<major>{_NUM})\.{_NUM}\.{_NUM}")
_PRE_VERSION = re.compile(rf"(?P<major>{_NUM})\.{_NUM}\.{_NUM}(?:a|b|rc)[0-9]+")
CANDIDATE_WORKFLOW = "publish-pypi.yml"
# Releases from this major version on need a passed candidate. 2.x candidates are
# untagged branch builds, so they cannot satisfy a candidate-tag gate.
GATED_FROM_MAJOR = 3
PER_PAGE = 100
MAX_PAGES = 10

Runner = Callable[[Sequence[str]], str]


@dataclass(frozen=True)
class Candidate:
    cargo_version: str
    pypi_version: str


@dataclass(frozen=True)
class ReleaseTag:
    cargo_version: str
    pypi_version: str
    stable: bool
    active: bool


def parse_candidate_tag(tag: str) -> Candidate:
    match = _RC_TAG.fullmatch(tag)
    if match is None:
        raise ValueError(f"not a candidate tag (expected rc/vX.Y.Z-rc.N): {tag}")
    base, n = match["base"], match["n"]
    return Candidate(cargo_version=f"{base}-rc.{n}", pypi_version=f"{base}rc{n}")


def parse_release_tag(tag: str) -> ReleaseTag:
    """Parse a candidate or a stable tag of the single 3.x PyPI publisher.

    ``active`` is false for a stable tag below major 3: those releases do not
    publish binary wheels, so publish-pypi.yml does nothing for them.
    """
    stable = _STABLE_TAG.fullmatch(tag)
    if stable is None:
        candidate = parse_candidate_tag(tag)
        return ReleaseTag(
            candidate.cargo_version, candidate.pypi_version, stable=False, active=True
        )
    base = stable["base"]
    return ReleaseTag(base, base, stable=True, active=requires_gate(base))


def is_legacy_version(version: str) -> bool:
    """True for a version of a major below 3, stable or prerelease.

    publish-lint.yml builds the pure-Python 2.x package, so it may publish only
    these. Anything else, including a version it cannot parse, is refused.
    """
    match = _STABLE_VERSION.fullmatch(version) or _PRE_VERSION.fullmatch(version)
    if match is None:
        raise ValueError(f"not a version: {version}")
    return int(match["major"]) < GATED_FROM_MAJOR


def requires_gate(version: str) -> bool:
    """True for a stable X.Y.Z version of major 3 or later.

    A prerelease (``3.0.0rc1``) is a candidate itself and is never gated.
    """
    match = _STABLE_VERSION.fullmatch(version)
    if match is not None:
        return int(match["major"]) >= GATED_FROM_MAJOR
    if _PRE_VERSION.fullmatch(version):
        return False
    raise ValueError(f"not a version: {version}")


def route_tag(tag: str) -> bool:
    """True when the dist pipeline and its provenance gate apply to ``tag``."""
    match = _STABLE_TAG.fullmatch(tag)
    if match is None:
        raise ValueError(f"not a stable tag (expected vX.Y.Z): {tag}")
    return requires_gate(match["base"])


PROJECT = "slopvac"
MAX_METADATA_BYTES = 1 << 20
_DIST_INFO_METADATA = re.compile(r"[^/]+\.dist-info/METADATA")
_WHEEL_TAGS = r"[A-Za-z0-9_.]+-[A-Za-z0-9_.]+-[A-Za-z0-9_.]+"


def _normalise(name: str) -> str:
    return re.sub(r"[-_.]+", "-", name).lower()


def _metadata_problems(text: bytes, what: str, project: str, version: str) -> list[str]:
    fields = email.parser.BytesParser().parsebytes(text, headersonly=True)
    problems = []
    if _normalise(fields.get("Name", "")) != _normalise(project):
        problems.append(f"{what}: project is {fields.get('Name')!r}, not {project!r}")
    if fields.get("Version") != version:
        problems.append(
            f"{what}: version is {fields.get('Version')!r}, not {version!r}"
        )
    return problems


def _read_capped(member) -> bytes:
    if member is None:
        raise ValueError("metadata is not a file")
    with member:
        data = member.read(MAX_METADATA_BYTES + 1)
    if len(data) > MAX_METADATA_BYTES:
        raise ValueError(f"metadata is larger than {MAX_METADATA_BYTES} bytes")
    return data


def _wheel_problems(path: Path, project: str, version: str) -> list[str]:
    try:
        with zipfile.ZipFile(path) as wheel:
            found = [n for n in wheel.namelist() if _DIST_INFO_METADATA.fullmatch(n)]
            if len(found) != 1:
                return [
                    f"{path.name}: expected one dist-info/METADATA, found {len(found)}"
                ]
            data = _read_capped(wheel.open(found[0]))
    except (zipfile.BadZipFile, ValueError, OSError) as err:
        return [f"{path.name}: unreadable wheel: {err}"]
    return _metadata_problems(data, path.name, project, version)


def _sdist_problems(path: Path, project: str, version: str) -> list[str]:
    try:
        with tarfile.open(path, "r:gz") as sdist:
            data = _read_capped(sdist.extractfile(f"{project}-{version}/PKG-INFO"))
    except (tarfile.TarError, KeyError, ValueError, OSError) as err:
        return [f"{path.name}: unreadable sdist: {err}"]
    return _metadata_problems(data, path.name, project, version)


_TARGET_LINE = re.compile(r"^\s+target:\s*(\S+)\s*$", re.MULTILINE)


def wheel_targets(workflow: Path) -> list[str]:
    """Return the build targets listed in a wheels workflow's matrix, in order."""
    found = _TARGET_LINE.findall(workflow.read_text(encoding="utf-8"))
    if not found:
        raise ValueError(f"no build targets in {workflow}")
    return found


def _tag_target(platform_tag: str) -> str | None:
    """Map one wheel platform tag to the Rust target that builds it."""
    arch = next(
        (
            rust
            for suffix, rust in (
                ("_x86_64", "x86_64"),
                ("_arm64", "aarch64"),
                ("_aarch64", "aarch64"),
            )
            if platform_tag.endswith(suffix)
        ),
        None,
    )
    if arch is None:
        return None
    if platform_tag.startswith("macosx_"):
        return f"{arch}-apple-darwin"
    if platform_tag.startswith("manylinux"):
        return f"{arch}-unknown-linux-gnu"
    if platform_tag.startswith("musllinux_"):
        return f"{arch}-unknown-linux-musl"
    return None


def _wheel_target(name: str) -> str | None:
    """Return the one target all of a wheel's platform tags name, else None."""
    platform = name.removesuffix(".whl").rsplit("-", 1)[-1]
    targets = {_tag_target(tag) for tag in platform.split(".")}
    return targets.pop() if len(targets) == 1 else None


def _target_problems(wheel_files: list[Path], targets: Sequence[str]) -> list[str]:
    problems: list[str] = []
    seen: dict[str, int] = {}
    for path in wheel_files:
        target = _wheel_target(path.name)
        if target is None or target not in targets:
            problems.append(
                f"{path.name}: unexpected wheel, not built for a listed target"
            )
        else:
            seen[target] = seen.get(target, 0) + 1
    for target in targets:
        if target not in seen:
            problems.append(f"missing wheel for {target}")
        elif seen[target] > 1:
            problems.append(f"two wheels for {target}")
    return problems


def check_dist(
    directory: Path,
    version: str,
    *,
    project: str = PROJECT,
    sdist: bool = False,
    wheels: int | None = None,
    targets: Sequence[str] | None = None,
) -> None:
    """Fail unless ``directory`` holds exactly the files an upload may carry.

    File names alone prove nothing, since a build backend can write any file next
    to the expected ones. Each file's own metadata must name ``project`` and
    ``version``. Expect one sdist when ``sdist`` is set and none otherwise, and
    ``wheels`` wheels (at least one when ``wheels`` is None). When ``targets`` is
    given, the wheels must be exactly one per listed Rust target, judged by each
    wheel's platform tag, so a right count of wrong wheels fails.
    """
    if not (_STABLE_VERSION.fullmatch(version) or _PRE_VERSION.fullmatch(version)):
        raise ValueError(f"not a version: {version}")
    if not directory.is_dir():
        raise ValueError(f"no distribution directory: {directory}")
    wheel_name = re.compile(
        rf"{re.escape(project)}-{re.escape(version)}-{_WHEEL_TAGS}\.whl"
    )
    sdist_name = f"{project}-{version}.tar.gz"
    problems: list[str] = []
    wheel_files: list[Path] = []
    sdist_files: list[Path] = []
    for entry in sorted(directory.iterdir()):
        if entry.is_symlink() or not entry.is_file():
            problems.append(f"{entry.name}: not a regular file")
        elif wheel_name.fullmatch(entry.name):
            wheel_files.append(entry)
        elif entry.name == sdist_name:
            sdist_files.append(entry)
        else:
            problems.append(f"{entry.name}: unexpected file for {project} {version}")
    if sdist != bool(sdist_files):
        problems.append(
            f"expected {'one sdist' if sdist else 'no sdist'}, found {len(sdist_files)}"
        )
    if wheels is None and not wheel_files:
        problems.append("no wheels found")
    if wheels is not None and len(wheel_files) != wheels:
        problems.append(f"expected {wheels} wheels, found {len(wheel_files)}")
    if targets is not None:
        problems += _target_problems(wheel_files, targets)
    for path in wheel_files:
        problems += _wheel_problems(path, project, version)
    for path in sdist_files:
        problems += _sdist_problems(path, project, version)
    if problems:
        raise ValueError("; ".join(problems))


def run_command(argv: Sequence[str]) -> str:
    done = subprocess.run(argv, capture_output=True, text=True, check=False)
    if done.returncode != 0:
        raise RuntimeError(f"{' '.join(argv)} failed: {done.stderr.strip()}")
    return done.stdout


def _run_proves(run: dict, candidate: str, sha: str) -> bool:
    """A run proves a candidate only when it is that tag's push run at that commit."""
    return (
        run.get("conclusion") == "success"
        and run.get("status") == "completed"
        and run.get("event") == "push"
        and run.get("head_sha") == sha
        and run.get("head_branch") == candidate
        and run.get("path") == f".github/workflows/{CANDIDATE_WORKFLOW}"
    )


def _candidate_runs(repo: str, sha: str, runner: Runner) -> list[dict]:
    """Every completed push run of the candidate workflow at ``sha``, page by page."""
    runs: list[dict] = []
    for page in range(1, MAX_PAGES + 1):
        data = json.loads(
            runner(
                [
                    "gh",
                    "api",
                    (
                        f"repos/{repo}/actions/workflows/{CANDIDATE_WORKFLOW}/runs"
                        f"?head_sha={sha}&event=push&status=completed"
                        f"&per_page={PER_PAGE}&page={page}"
                    ),
                ]
            )
        )
        batch = data.get("workflow_runs", [])
        runs.extend(batch)
        if len(batch) < PER_PAGE:
            return runs
    raise ValueError(
        f"more than {MAX_PAGES * PER_PAGE} candidate runs at {sha}; refusing to guess"
    )


def check_stable(
    tag: str, repo: str, event_sha: str, runner: Runner = run_command
) -> str:
    """Return the candidate tag that proves ``tag``, or raise ``ValueError``.

    ``event_sha`` is the commit the release workflow runs on. The tag must still
    point at it, so a tag moved after the event cannot borrow this check.
    """
    match = _STABLE_TAG.fullmatch(tag)
    if match is None:
        raise ValueError(f"not a stable tag (expected vX.Y.Z): {tag}")
    base = match["base"]
    stable_commit = runner(["git", "rev-parse", f"{tag}^{{commit}}"]).strip()
    if stable_commit != event_sha:
        raise ValueError(
            f"{tag} points at {stable_commit}, not the workflow commit {event_sha}"
        )
    stable_tree = runner(["git", "rev-parse", f"{tag}^{{tree}}"]).strip()
    refs = runner(["git", "tag", "--list", f"rc/v{base}-rc.*"]).split()
    candidates = [t for t in refs if _RC_TAG.fullmatch(t)]
    if not candidates:
        raise ValueError(f"no candidate tag rc/v{base}-rc.N exists")
    # Newest candidate first, so the message names the closest miss.
    candidates.sort(key=lambda t: int(_RC_TAG.fullmatch(t)["n"]), reverse=True)
    problems: list[str] = []
    for candidate in candidates:
        tree = runner(["git", "rev-parse", f"{candidate}^{{tree}}"]).strip()
        if tree != stable_tree:
            problems.append(f"{candidate}: tree differs from {tag}")
            continue
        sha = runner(["git", "rev-parse", f"{candidate}^{{commit}}"]).strip()
        if any(
            _run_proves(run, candidate, sha)
            for run in _candidate_runs(repo, sha, runner)
        ):
            return candidate
        problems.append(
            f"{candidate}: no successful {CANDIDATE_WORKFLOW} run of that tag at {sha}"
        )
    raise ValueError(
        "no passed candidate matches the stable tag: " + "; ".join(problems)
    )


def remote_tag_commit(tag: str, runner: Runner = run_command) -> str:
    """The commit ``tag`` resolves to on the remote, peeled for annotated tags."""
    ref = f"refs/tags/{tag}"
    lines = runner(["git", "ls-remote", "--tags", "origin", ref, f"{ref}^{{}}"])
    found: dict[str, str] = {}
    for line in lines.splitlines():
        sha, _, name = line.partition("\t")
        found[name.strip()] = sha.strip()
    resolved = found.get(f"{ref}^{{}}") or found.get(ref)
    if not resolved:
        raise ValueError(f"{tag} does not exist on the remote")
    return resolved


def verify_tag(tag: str, sha: str, runner: Runner = run_command) -> None:
    """Fail unless the remote tag still points at ``sha``."""
    if _STABLE_TAG.fullmatch(tag) is None:
        raise ValueError(f"not a stable tag (expected vX.Y.Z): {tag}")
    current = remote_tag_commit(tag, runner)
    if current != sha:
        raise ValueError(f"{tag} moved to {current}; the release was built from {sha}")


def check_draft(
    tag: str, repo: str, app_slug: str = "", runner: Runner = run_command
) -> None:
    """Fail unless ``tag``'s release exists, is unpublished, and a release bot made it."""
    if _STABLE_TAG.fullmatch(tag) is None:
        raise ValueError(f"not a stable tag (expected vX.Y.Z): {tag}")
    release = json.loads(
        runner(
            ["gh", "release", "view", tag, "--repo", repo, "--json", "isDraft,author"]
        )
    )
    if release.get("isDraft") is not True:
        raise ValueError(f"the release for {tag} is already published")
    allowed = {"github-actions[bot]"}
    if app_slug:
        allowed.add(f"{app_slug}[bot]")
    login = (release.get("author") or {}).get("login")
    if login not in allowed:
        raise ValueError(
            f"the release for {tag} was created by {login!r}, not by {sorted(allowed)}"
        )


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawTextHelpFormatter
    )
    sub = parser.add_subparsers(dest="command", required=True)
    parse = sub.add_parser("parse-tag")
    parse.add_argument("tag")
    check = sub.add_parser("check-stable")
    check.add_argument("tag")
    check.add_argument("--repo", required=True, help="owner/name")
    check.add_argument("--sha", required=True, help="commit the workflow runs on")
    verify = sub.add_parser("verify-tag")
    verify.add_argument("tag")
    verify.add_argument("sha")
    legacy = sub.add_parser("is-legacy")
    legacy.add_argument("version")
    dist = sub.add_parser("check-dist")
    dist.add_argument("directory", type=Path)
    dist.add_argument("--version", required=True)
    dist.add_argument("--project", default=PROJECT)
    dist.add_argument("--sdist", action="store_true")
    dist.add_argument("--wheels", type=int, default=None)
    dist.add_argument(
        "--wheel-targets-from",
        type=Path,
        default=None,
        help="wheels workflow whose build matrix lists the expected targets",
    )
    draft = sub.add_parser("check-draft")
    draft.add_argument("tag")
    draft.add_argument("--repo", required=True, help="owner/name")
    draft.add_argument("--app-slug", default="")
    route = sub.add_parser("route-tag")
    route.add_argument("tag")
    args = parser.parse_args(argv)
    try:
        if args.command == "parse-tag":
            release = parse_release_tag(args.tag)
            print(f"cargo={release.cargo_version}")
            print(f"pypi={release.pypi_version}")
            print(f"stable={'true' if release.stable else 'false'}")
            print(f"active={'true' if release.active else 'false'}")
        elif args.command == "check-stable":
            print(f"candidate={check_stable(args.tag, args.repo, args.sha)}")
        elif args.command == "verify-tag":
            verify_tag(args.tag, args.sha)
        elif args.command == "check-dist":
            check_dist(
                args.directory,
                args.version,
                project=args.project,
                sdist=args.sdist,
                wheels=args.wheels,
                targets=(
                    wheel_targets(args.wheel_targets_from)
                    if args.wheel_targets_from
                    else None
                ),
            )
        elif args.command == "check-draft":
            check_draft(args.tag, args.repo, args.app_slug)
        elif args.command == "route-tag":
            print(f"gated={'true' if route_tag(args.tag) else 'false'}")
        else:
            print("true" if is_legacy_version(args.version) else "false")
    except (ValueError, RuntimeError) as err:
        print(f"error: {err}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
