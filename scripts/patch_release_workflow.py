#!/usr/bin/env python3
"""Harden the cargo-dist generated release workflow.

`dist generate` writes `.github/workflows/release.yml` and interpolates the tag
name straight into shell commands. A git tag may contain `$(...)` or backticks,
so that is command injection. This script rewrites the generated text:

* tags starting with `v` and a digit trigger the workflow, so `rc/v*` candidate
  tags never do;
* the plan job rejects any tag that is not `vX.Y.Z` before dist runs;
* every tag reaches a shell through an environment variable, never `${{ }}`;
* the root token is read-only, and only the host job may write contents;
* the host job re-resolves the tag on the remote right before it publishes and
  fails unless it still points at the commit the workflow runs on (CWE-367);
* the build step runs under bash with `$TAG_FLAG` and `$DIST_ARGS` as environment
  variables, not as `${{ }}` text;
* the CycloneDX step output is read as `outputs`, which dist spells `output`;
* dist, cargo-auditable and cargo-cyclonedx come from pinned release archives checked
  against committed sha256 digests (`scripts/install_*.sh`), never from an installer
  script piped into a shell, and the generator fails if any such pipeline remains;
* artifacts are signed with minisign in a separate `sign` job that never installs or
  runs dist; the host job only verifies the signatures before it uploads them;
* the Homebrew job pushes with a token minted from the release App, not a stored secret,
  runs no brew command, and gives the token to the push step alone;
* the `sign` job runs in the `release` environment, where the signing key lives;
* every job and workflow-level block of the result is pinned by sha256 in
  `scripts/release_workflow_pins.json`, so any change needs a reviewed pin update;
* a draft pull request runs no job, and marking it ready for review starts the run.

Run `scripts/dist_generate.sh` instead of `dist generate`. Each rewrite must
match exactly once, so a dist upgrade that changes the template fails loudly
here instead of silently dropping a protection.
"""

from __future__ import annotations

import hashlib
import json
import os
import re
import sys
from pathlib import Path

WORKFLOW = Path(".github/workflows/release.yml")

_TRIGGER = ("      - '**[0-9]+.[0-9]+.[0-9]+*'\n", "      - 'v[0-9]+.[0-9]+.[0-9]+'\n")

# Marking a draft ready must start a run, and so must converting a PR back to a
# draft, so the draft run replaces a ready run in flight. The route job skips drafts.
_PR_TYPES = "[opened, synchronize, reopened, ready_for_review, converted_to_draft]"
_PR_TRIGGER = (
    "on:\n  pull_request:\n  push:\n",
    f"on:\n  pull_request:\n    types: {_PR_TYPES}\n  push:\n",
)

_VALIDATE = """\
      - name: Validate the tag
        if: ${{ !github.event.pull_request }}
        shell: bash
        env:
          REF_NAME: ${{ github.ref_name }}
        run: |
          if [[ ! "$REF_NAME" =~ ^v(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)$ ]]; then
            echo "refusing to release from a tag that is not vX.Y.Z" >&2
            exit 1
          fi
"""

_PLAN_RUN_OLD = """\
      - id: plan
        run: |
          dist ${{ (!github.event.pull_request && format('host --steps=create --tag={0}', github.ref_name)) || 'plan' }} --output-format=json > plan-dist-manifest.json
"""
_PLAN_RUN_NEW = """\
      - id: plan
        shell: bash
        env:
          REF_NAME: ${{ github.ref_name }}
          IS_PULL_REQUEST: ${{ github.event.pull_request != null }}
        run: |
          if [[ "$IS_PULL_REQUEST" == "true" ]]; then
            dist plan --output-format=json > plan-dist-manifest.json
          else
            dist host --steps=create --tag="$REF_NAME" --output-format=json > plan-dist-manifest.json
          fi
"""

_TAG_FLAG = "${{ needs.plan.outputs.tag-flag }}"
# With `create-release = false` dist uploads to the draft release-please made, then
# undrafts it. Split that step so the tag check sits right before the undraft.
_RELEASE_OLD = """\
      - name: Create GitHub Release
        env:
          PRERELEASE_FLAG: "${{ fromJson(steps.host.outputs.manifest).announcement_is_prerelease && '--prerelease' || '' }}"
          RELEASE_COMMIT: "${{ github.sha }}"
        run: |
          # If we're editing a release in place, we need to upload things ahead of time
          gh release upload "${{ needs.plan.outputs.tag }}" artifacts/*

          gh release edit "${{ needs.plan.outputs.tag }}" --target "$RELEASE_COMMIT" $PRERELEASE_FLAG --draft=false
"""
_RELEASE_NEW = """\
{signatures}\
      - name: Upload the artifacts to the draft release
        env:
          GH_TOKEN: ${{ steps.release-token.outputs.token }}
          RELEASE_TAG: ${{ needs.plan.outputs.tag }}
        run: |
          gh release upload "$RELEASE_TAG" artifacts/*
{verify}\
      - name: Publish the release
        env:
          GH_TOKEN: ${{ steps.release-token.outputs.token }}
          RELEASE_TAG: ${{ needs.plan.outputs.tag }}
          PRERELEASE_FLAG: "${{ fromJson(steps.host.outputs.manifest).announcement_is_prerelease && '--prerelease' || '' }}"
          RELEASE_COMMIT: "${{ github.sha }}"
        run: |
          gh release edit "$RELEASE_TAG" --target "$RELEASE_COMMIT" $PRERELEASE_FLAG --draft=false
"""
_JOB_ENV = "      GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}\n"
# Every published file gets a minisign signature before the upload, so the draft never
# holds an unsigned asset. The `sign` job signs on a runner that never ran dist and
# is the only job that reads the secret; `host`, which ran dist, only verifies.
_SIGN_JOB = """\
  # Signs the built artifacts. It never installs or runs dist, so the one job that
  # reads the signing secret executes nothing that dist produced or downloaded.
  sign:
    name: Sign the release artifacts
    needs:
      - plan
      - custom-release-provenance
      - build-local-artifacts
      - build-global-artifacts
{condition}\
    runs-on: "ubuntu-22.04"
    # The signing key lives in this environment, which only main and v* tags can deploy to.
    environment: release
    steps:
      - uses: {checkout}
        with:
          persist-credentials: false
      - name: Confirm the tag still points at the built commit
        shell: bash
        env:
          RELEASE_TAG: ${{ needs.plan.outputs.tag }}
          RELEASE_COMMIT: ${{ github.sha }}
        run: |
          python3 scripts/release_provenance.py verify-tag "$RELEASE_TAG" "$RELEASE_COMMIT"
      - name: Fetch artifacts
        uses: {download}
        with:
          pattern: artifacts-*
          path: artifacts
          merge-multiple: true
      - name: Cleanup
        run: |
          # Remove the granular manifests
          rm -f artifacts/*-dist-manifest.json
          # The host job writes this one itself; it is not a signed release asset
          rm -f artifacts/dist-manifest.json
      - name: Install minisign
        run: bash scripts/install_minisign.sh
      - name: Sign the release artifacts
        env:
          MINISIGN_SECRET_KEY: ${{ secrets.MINISIGN_SECRET_KEY }}
          RELEASE_TAG: ${{ needs.plan.outputs.tag }}
        run: |
          python3 scripts/release_signing.py sign artifacts --version "${RELEASE_TAG#v}"
      - name: Upload the signatures
        uses: {upload}
        with:
          name: release-signatures
          path: artifacts/*.minisig
          if-no-files-found: error

"""
_HOST_SIGNATURE_STEPS = """\
      - name: Fetch the signatures
        uses: {download}
        with:
          name: release-signatures
          path: artifacts
      - name: Install minisign
        run: bash scripts/install_minisign.sh
      - name: Verify the signatures against the committed key
        shell: bash
        run: |
          find artifacts -type f ! -name '*.minisig' -print0 | xargs -0 python3 scripts/release_signing.py verify
"""
_HOST_CLEANUP_OLD = """\
          rm -f artifacts/*-dist-manifest.json
      - name: Create GitHub Release
"""
_HOST_CLEANUP_NEW = """\
          rm -f artifacts/*-dist-manifest.json
          # This job's own dist run writes this one; the sign job never sees it
          rm -f artifacts/dist-manifest.json
      - name: Create GitHub Release
"""
_USES = {
    "checkout": re.compile(r"uses: (actions/checkout@\S+ # \S+)\n"),
    "download": re.compile(r"uses: (actions/download-artifact@\S+ # \S+)\n"),
    "upload": re.compile(r"uses: (actions/upload-artifact@\S+ # \S+)\n"),
}
_HOST_IF = re.compile(r"^    if: \$\{\{ always\(\) && needs\.plan\.result .* \}\}\n", re.M)
_HOST_ENV = (
    "    env:\n"
    + _JOB_ENV
    + '    runs-on: "ubuntu-22.04"\n    outputs:\n      val: ${{ steps.host.outputs.manifest }}\n'
)


_SETUP_PYTHON = (
    "      - uses: actions/setup-python@6d4fd22b20465ccc0f7354476588e8df7c9ca0e0 # v7.1.0\n"
    "        with:\n"
    '          python-version: "3.14"\n'
)
_DIST_INSTALL_STEP = """\
      - name: Install dist
        run: bash scripts/install_cargo_dist.sh
"""
_TOOLCHAIN_STEP = (
    "      - name: Install the pinned toolchain\n"
    "        run: bash scripts/ci/install-rust-toolchain.sh\n"
)
# Where each Rust-running job installs the pinned toolchain: before the first dist step,
# after checkout. The helper reads rust-toolchain.toml with python3 3.11 or newer, which neither
# ubuntu-22.04 nor every dist container has, so setup-python comes first.
_TOOLCHAIN_AT = (
    (
        "  build-local-artifacts:\n",
        _DIST_INSTALL_STEP,
    ),
    (
        "  build-global-artifacts:\n",
        "      - name: Install cached dist\n",
    ),
)


def _install_toolchain(text: str) -> str:
    """Run the pinned-toolchain helper in the dist jobs that build or call cargo."""
    out: list[str] = []
    done = set()
    for chunk in _split_jobs(text):
        for job, anchor in _TOOLCHAIN_AT:
            if chunk.startswith(job):
                chunk = _replace_once(
                    chunk, anchor, _SETUP_PYTHON + _TOOLCHAIN_STEP + anchor, job.strip()
                )
                done.add(job)
        out.append(chunk)
    if len(done) != len(_TOOLCHAIN_AT):
        raise ValueError("expected the local and global artifact jobs in the workflow")
    return "".join(out)



def _replace_once(text: str, old: str, new: str, what: str) -> str:
    if text.count(old) != 1:
        raise ValueError(
            f"expected exactly one {what} in the generated workflow, found {text.count(old)}"
        )
    return text.replace(old, new)


def harden(text: str) -> str:
    text = _replace_once(text, *_TRIGGER, "tag trigger")
    text = _replace_once(text, *_PR_TRIGGER, "pull request trigger")
    text = _insert_validation(text)
    text = _replace_once(text, _PLAN_RUN_OLD, _PLAN_RUN_NEW, "plan command")
    text = _replace_once(text, _HOST_CLEANUP_OLD, _HOST_CLEANUP_NEW, "host cleanup")
    text = _replace_once(
        text,
        _RELEASE_OLD,
        _RELEASE_NEW.replace("{verify}", _VERIFY_STEP).replace(
            "{signatures}",
            _HOST_SIGNATURE_STEPS.replace("{download}", _pin(text, "download")),
        ),
        "release step",
    )
    text = _pin_dist(text)
    text = _pin_tool_installers(text)
    # Every remaining `needs.plan.outputs.tag-flag` is a shell word in `dist ...`.
    count = text.count(_TAG_FLAG + " ")
    if count < 1:
        raise ValueError("expected tag-flag uses in dist commands")
    text = text.replace("dist build " + _TAG_FLAG, "dist build $TAG_FLAG").replace(
        "dist host " + _TAG_FLAG, "dist host $TAG_FLAG"
    )
    text = _replace_once(
        text,
        'permissions:\n  "contents": "write"\n',
        'permissions:\n  "contents": "read"\n',
        "root permissions",
    )
    text = _replace_once(
        text,
        _HOST_ENV,
        '    permissions:\n      "contents": "read"\n' + _HOST_ENV,
        "host job env",
    )
    text = _add_tag_flag_env(text)
    text = _gate_on_provenance(text)
    text = _add_sign_job(text)
    text = _replace_once(text, _BUILD_OLD, _BUILD_NEW, "build artifacts step")
    text = _replace_once(text, _DIST_ARGS_OLD, _DIST_ARGS_NEW, "dist_args expression")
    for run, what in _COMMAND_RUNS:
        first, _, rest = run.partition("\n")
        text = _replace_once(text, run, first + _ZIZMOR_WHY + rest, what)
    text = _replace_once(text, _SBOM_OLD, _SBOM_NEW, "SBOM output reference")
    text = _verify_tag_before_publishing(text)
    text = _publish_without_dist(text)
    text = _mint_tap_token(text)
    text = _isolate_tap_push(text)
    text = _route_by_major_version(text)
    text = _install_toolchain(text)
    _assert_clean(text)
    _assert_no_download_to_shell(text)
    _assert_permissions_understood(text)
    return text


_CUSTOM_OLD = """\
  custom-release-provenance:
    uses: $/.github/workflows/release-provenance.yml
    secrets: inherit
"""
_CUSTOM_NEW = """\
  custom-release-provenance:
    uses: ./.github/workflows/release-provenance.yml
    permissions:
      "actions": "read"
      "contents": "read"
    with:
      # The plan also runs on pull requests, which cannot publish.
      require-candidate: ${{ !github.event.pull_request }}
    secrets:
      RELEASE_APP_PRIVATE_KEY: ${{ secrets.RELEASE_APP_PRIVATE_KEY }}
"""
_HOST_NEEDS_OLD = "  host:\n    needs:\n      - plan\n"
_HOST_NEEDS_NEW = (
    "  host:\n    needs:\n      - plan\n      - custom-release-provenance\n      - sign\n"
)
_HOST_IF_OLD = "if: ${{ always() && needs.plan.result == 'success' && needs.plan.outputs.publishing == 'true' &&"
_HOST_IF_NEW = (
    "if: ${{ always() && needs.plan.result == 'success' && needs.custom-release-provenance.result == 'success'"
    " && needs.plan.outputs.publishing == 'true' &&"
)


_BUILD_OLD = "      - name: Build artifacts\n        run: |\n"
_BUILD_NEW = (
    "      - name: Build artifacts\n"
    "        shell: bash\n"
    "        env:\n"
    "          DIST_ARGS: ${{ matrix.dist_args }}\n"
    "        run: |\n"
)
# dist_args is a space-separated flag list; bash splits `$DIST_ARGS` the way the inline
# expression was split, but the value no longer sits inside the script text.
_DIST_ARGS_OLD = "--output-format=json ${{ matrix.dist_args }} >"
_DIST_ARGS_NEW = "--output-format=json $DIST_ARGS >"
# The matrix comes from `dist plan` on the tagged tree's dist-workspace.toml. These
# values are commands by design, so they stay expressions and carry a justification.
_ZIZMOR_WHY = (
    " # zizmor: ignore[template-injection] value comes from dist plan of the tagged "
    "tree's dist-workspace.toml, not from untrusted input\n"
)
_COMMAND_RUNS = (
    (
        "        run: |\n          ${{ matrix.packages_install }}\n",
        "packages_install command",
    ),
)
_SBOM_OLD = "steps.cargo-cyclonedx.output.paths"
_SBOM_NEW = "steps.cargo-cyclonedx.outputs.paths"
_VERIFY_STEP = """\
      - name: Confirm the tag still points at the built commit
        shell: bash
        env:
          RELEASE_TAG: ${{ needs.plan.outputs.tag }}
          RELEASE_COMMIT: ${{ github.sha }}
        run: |
          python3 scripts/release_provenance.py verify-tag "$RELEASE_TAG" "$RELEASE_COMMIT"
"""
_HOST_STEP = "      - id: host\n"
_HOST_STEP_OLD = _HOST_STEP + "        shell: bash\n"
_HOST_STEP_NEW = (
    _HOST_STEP
    + "        shell: bash\n        env:\n          GH_TOKEN: ${{ steps.release-token.outputs.token }}\n"
)

# The release is written with a token minted from the release App, scoped to this
# repository and to contents: write, never with GITHUB_TOKEN. A failed mint fails the job.
_RELEASE_TOKEN_STEP = """\
      - name: Mint the release token
        id: release-token
        uses: actions/create-github-app-token@64ab729a6294000f5aa4a4bc0b73d1f32bc45510 # v3.3.0
        with:
          client-id: ${{ vars.RELEASE_APP_CLIENT_ID }}
          private-key: ${{ secrets.RELEASE_APP_PRIVATE_KEY }}
          owner: ${{ github.repository_owner }}
          repositories: ${{ github.event.repository.name }}
          permission-contents: write
"""

# cargo-dist reads a long-lived HOMEBREW_TAP_TOKEN secret to push the formula. Mint a
# short-lived installation token from the release App instead, scoped to the tap.
_TAP_TOKEN_OLD = "          token: ${{ secrets.HOMEBREW_TAP_TOKEN }}\n"
_TAP_TOKEN_NEW = "          token: ${{ steps.tap-token.outputs.token }}\n"
_TAP_MINT_STEP = """\
      # `client-id`, not `app-id`, and the same App credentials as release-please.yml.
      - name: Mint the Homebrew tap token
        id: tap-token
        uses: actions/create-github-app-token@64ab729a6294000f5aa4a4bc0b73d1f32bc45510 # v3.3.0
        with:
          client-id: ${{ vars.RELEASE_APP_CLIENT_ID }}
          private-key: ${{ secrets.RELEASE_APP_PRIVATE_KEY }}
          owner: slopvac
          repositories: homebrew-tap
          permission-contents: write
"""


_TAP_JOB = "  publish-homebrew-formula:\n"
# The one checkout of the tap, inside the Homebrew job; the token line is its last input.
_TAP_CHECKOUT = re.compile(
    r"      - uses: actions/checkout@\S+ # \S+\n"
    r"        with:\n"
    r"(?:          [^\n]*\n)*?"
    r"          repository: \"slopvac/homebrew-tap\"\n"
    r"(?:          [^\n]*\n)*?" + re.escape(_TAP_TOKEN_OLD)
)


_HOST_DIST_SETUP = re.compile(
    r"      - name: Install cached dist\n.*?(?=" + re.escape(_VERIFY_STEP) + r")", re.S
)
_HOST_DIST_RUN = re.compile(r"      - id: host\n.*?(?=      # Create a GitHub Release)", re.S)
_HOST_DIST_ENV = "    env:\n      GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}\n      TAG_FLAG: ${{ needs.plan.outputs.tag-flag }}\n"
_HOST_DIST_OUTPUT = "    outputs:\n      val: ${{ steps.host.outputs.manifest }}\n"
_HOST_OWN_MANIFEST = (
    "          # This job's own dist run writes this one; the sign job never sees it\n"
    "          rm -f artifacts/dist-manifest.json\n"
)
_HOST_PRERELEASE_OLD = "fromJson(steps.host.outputs.manifest).announcement_is_prerelease"
_HOST_PRERELEASE_NEW = "fromJson(needs.plan.outputs.val).announcement_is_prerelease"
_HOST_UPLOAD_STEP = "      - name: Upload the artifacts to the draft release\n"


def _publish_without_dist(text: str) -> str:
    """Make `host` publish with a minted App token and without running any dist.

    `host` used to run a cached dist binary while it held the write token, before any
    signature was checked. It now installs and runs no dist and executes no downloaded
    file: it fetches the artifacts and signatures, verifies each signature against the
    committed key, and only then mints the contents-write token (the mint sits right
    before the upload). The token goes only to the `gh` steps that write.
    """
    chunks = _split_jobs(text)
    hosts = [i for i, c in enumerate(chunks) if c.startswith("  host:\n")]
    if len(hosts) != 1:
        raise ValueError(f"expected exactly one host job, found {len(hosts)}")
    host = chunks[hosts[0]]
    for old, new, what in (
        (_HOST_DIST_ENV, "", "host job env"),
        (_HOST_DIST_OUTPUT, "", "host job outputs"),
        (_HOST_OWN_MANIFEST, "", "host manifest cleanup"),
        (_HOST_PRERELEASE_OLD, _HOST_PRERELEASE_NEW, "host prerelease flag"),
        (
            _HOST_UPLOAD_STEP,
            _RELEASE_TOKEN_STEP + _HOST_UPLOAD_STEP,
            "host upload step",
        ),
    ):
        host = _replace_once(host, old, new, what)
    for pattern, what in ((_HOST_DIST_SETUP, "dist setup"), (_HOST_DIST_RUN, "dist run")):
        host, count = pattern.subn("", host)
        if count != 1:
            raise ValueError(f"expected exactly one host {what} block, found {count}")
    chunks[hosts[0]] = host
    return "".join(chunks)

def _mint_tap_token(text: str) -> str:
    """Replace the HOMEBREW_TAP_TOKEN secret with a token minted from the release App.

    The rewrite is confined to the Homebrew job and needs exactly one tap checkout
    there, so a reordered or renamed template fails here instead of moving the step.
    """
    chunks = _split_jobs(text)
    jobs = [i for i, chunk in enumerate(chunks) if chunk.startswith(_TAP_JOB)]
    if len(jobs) != 1:
        raise ValueError(
            f"expected exactly one Homebrew publish job, found {len(jobs)}"
        )
    job = chunks[jobs[0]]
    matches = list(_TAP_CHECKOUT.finditer(job))
    if len(matches) != 1 or job.count(_TAP_TOKEN_OLD) != 1:
        raise ValueError(
            f"expected exactly one Homebrew tap checkout, found {len(matches)}"
        )
    start = matches[0].start()
    job = job.replace(_TAP_TOKEN_OLD, _TAP_TOKEN_NEW)
    chunks[jobs[0]] = job[:start] + _TAP_MINT_STEP + job[start:]
    if "HOMEBREW_TAP_TOKEN" in "".join(chunks):
        raise ValueError("a HOMEBREW_TAP_TOKEN reference remains after the rewrite")
    return "".join(chunks)


# dist's Homebrew job runs `brew update` and `brew style --fix` after it checked out the
# tap with the write token persisted in .git/config. `brew update` fetches unpinned
# upstream code and the next command runs it, so that code could read the token. The
# style pass only reformats a formula that dist generated, so it goes. The job now
# validates the file names, commits without credentials and hands the token to the push
# step alone.
_TAP_PERSIST_OLD = (
    '          persist-credentials: true\n          repository: "slopvac/homebrew-tap"\n'
)
_TAP_PERSIST_NEW = (
    '          persist-credentials: false\n          repository: "slopvac/homebrew-tap"\n'
)
_TAP_COMMIT_OLD = """\
          for release in $(echo "$PLAN" | jq --compact-output '.releases[] | select([.artifacts[] | endswith(".rb")] | any)'); do
            filename=$(echo "$release" | jq '.artifacts[] | select(endswith(".rb"))' --raw-output)
            name=$(echo "$filename" | sed "s/\\.rb$//")
            version=$(echo "$release" | jq .app_version --raw-output)

            export PATH="/home/linuxbrew/.linuxbrew/bin:$PATH"
            brew update
            # We avoid reformatting user-provided data such as the app description and homepage.
            brew style --except-cops FormulaAudit/Homepage,FormulaAudit/Desc,FormulaAuditStrict --fix "Formula/${filename}" || true

            git add "Formula/${filename}"
            git commit -m "${name} ${version}"
          done
          git push
"""
_TAP_COMMIT_NEW = """\
          set -euo pipefail
          for release in $(echo "$PLAN" | jq --compact-output '.releases[] | select([.artifacts[] | endswith(".rb")] | any)'); do
            filename=$(echo "$release" | jq '.artifacts[] | select(endswith(".rb"))' --raw-output)
            name=$(echo "$filename" | sed "s/\\.rb$//")
            version=$(echo "$release" | jq .app_version --raw-output)

            [[ "$filename" =~ ^[A-Za-z0-9][A-Za-z0-9._-]*\\.rb$ ]] || { echo "unexpected formula file name: $filename" >&2; exit 1; }
            [[ -s "Formula/${filename}" ]] || { echo "missing or empty formula: $filename" >&2; exit 1; }

            git add "Formula/${filename}"
            git commit -m "${name} ${version}"
          done
      # The one step that sees the token. It runs no fetched code, only git.
      - name: Push the formula files
        env:
          TAP_TOKEN: ${{ steps.tap-token.outputs.token }}
        run: |
          set -euo pipefail
          # The header goes to git through its environment, never its argument list, which
          # any process on the runner can read. printf is a shell builtin, so the token
          # is not in a command line there either.
          GIT_CONFIG_VALUE_0="AUTHORIZATION: basic $(printf 'x-access-token:%s' "$TAP_TOKEN" | base64 | tr -d '\\n')"
          export GIT_CONFIG_COUNT=1
          export GIT_CONFIG_KEY_0=http.https://github.com/.extraheader
          export GIT_CONFIG_VALUE_0
          git push origin HEAD
"""


def _isolate_tap_push(text: str) -> str:
    """Keep fetched code out of the job that holds the tap token; see the note above."""
    chunks = _split_jobs(text)
    jobs = [i for i, chunk in enumerate(chunks) if chunk.startswith(_TAP_JOB)]
    if len(jobs) != 1:
        raise ValueError(
            f"expected exactly one Homebrew publish job, found {len(jobs)}"
        )
    job = chunks[jobs[0]]
    job = _replace_once(job, _TAP_PERSIST_OLD, _TAP_PERSIST_NEW, "tap credential persistence")
    job = _replace_once(job, _TAP_COMMIT_OLD, _TAP_COMMIT_NEW, "formula commit script")
    chunks[jobs[0]] = job
    return "".join(chunks)


_PERMISSION_LINE = re.compile(
    r"^\s+[\"']?[\w-]+[\"']?:\s*[\"']?(read|write|none)[\"']?\s*(#.*)?$"
)


def _writes(block: list[str], where: str) -> bool:
    """Whether a `permissions:` block (its first line, then the lines it owns) can write.

    Only block mappings of `scope: read|write|none`, `write-all`, `read-all` and `{}` are
    understood. Anything else, such as a flow mapping or an expression, is an error so
    that new syntax cannot hide a write grant.
    """
    head = block[0].split("permissions:", 1)[1].split("#", 1)[0].strip()
    if head == "write-all":
        return True
    if head in {"read-all", "{}"}:
        return False
    if head:
        raise ValueError(f"unsupported permissions syntax in {where}: {head}")
    wrote = False
    for line in block[1:]:
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        found = _PERMISSION_LINE.match(line)
        if found is None:
            raise ValueError(f"unsupported permissions syntax in {where}: {line.strip()}")
        wrote = wrote or found[1] == "write"
    return wrote


def _permissions_block(chunk: str, indent: int, where: str) -> list[str] | None:
    """The `permissions:` entry at `indent` spaces, or None when there is none."""
    lines = chunk.splitlines()
    pad = " " * indent
    if any(re.match(rf"{pad}[\"']permissions[\"']\s*:", line) for line in lines):
        raise ValueError(f"unsupported permissions syntax in {where}: quoted key")
    starts = [i for i, line in enumerate(lines) if line.startswith(pad + "permissions:")]
    if len(starts) > 1:
        raise ValueError(f"unsupported permissions syntax in {where}: repeated key")
    if not starts:
        return None
    body: list[str] = []
    for line in lines[starts[0] + 1 :]:
        content = line.strip() and not line.lstrip().startswith("#")
        if content and len(line) - len(line.lstrip()) <= indent:
            break
        body.append(line)
    # Blank and comment lines after the last entry belong to what follows the block.
    while body and (not body[-1].strip() or body[-1].lstrip().startswith("#")):
        body.pop()
    if any(not line.strip() or line.lstrip().startswith("#") for line in body):
        raise ValueError(
            f"unsupported permissions syntax in {where}: comment or blank line in the block"
        )
    return [lines[starts[0]], *body]


def _assert_permissions_understood(text: str) -> None:
    """Fail on any `permissions:` syntax the reader does not understand, in any job."""
    chunks = _split_jobs(text)
    top = _permissions_block(chunks[0], 0, "the workflow")
    if top is not None:
        _writes(top, "the workflow")
    for chunk in chunks[1:]:
        job = chunk.split(":", 1)[0].strip()
        own = _permissions_block(chunk, 4, f"job {job}")
        if own is not None:
            _writes(own, f"job {job}")


# --- the pin table ---------------------------------------------------------------
#
# Patterns cannot keep up with the ways a shell hides a command, so the finished
# workflow is pinned instead: every job and every workflow-level block has a sha256 in
# PINS. Any change to the generated text, in any job, fails generation until a person
# has reviewed the diff for that job and updated its pin.

PINS = Path(__file__).with_name("release_workflow_pins.json")
PINS_NAME = f"scripts/{PINS.name}"
_PIN_COMMENT = (
    "A pin update needs the same review as a workflow change. Review the generated "
    "diff for the named job before updating its pin."
)


def _digest(block: str) -> str:
    return hashlib.sha256(block.encode("utf-8")).hexdigest()


_PREAMBLE = "_preamble"
_JOBS_HEADER = "_jobs_header"
_SHA256 = re.compile(r"[0-9a-f]{64}")


def _regions(text: str) -> list[tuple[str, str, str]]:
    """Every byte of `text` as (section, name, text) regions, in file order.

    The leading comments and blank lines are `_preamble`, each top-level key before
    `jobs:` is a workflow block, `jobs:` with whatever follows it up to the first job is
    `_jobs_header`, and each job is a job block.
    """
    chunks = _split_jobs(text)
    regions: list[tuple[str, str, list[str]]] = []
    current = None
    for line in chunks[0].splitlines(keepends=True):
        if line[:1] not in {" ", "\t", "\n", "#", ""}:
            key = line.split(":", 1)[0].strip().strip("\"'")
            name = _JOBS_HEADER if key == "jobs" else key
            # Compare the stored region names: `jobs:` is stored as `_jobs_header`, so a
            # second `jobs:` line would otherwise pass as a new name and lose its digest.
            if any(r[0] == "workflow" and r[1] == name for r in regions):
                raise ValueError(f"the workflow repeats the region {name}")
            current = ("workflow", name, [])
            regions.append(current)
        elif current is None:
            current = ("workflow", _PREAMBLE, [])
            regions.append(current)
        current[2].append(line)
    out = [(section, name, "".join(lines)) for section, name, lines in regions]
    seen: set[str] = set()
    for chunk in chunks[1:]:
        job = chunk.split(":", 1)[0].strip()
        if job in seen:
            raise ValueError(f"the workflow repeats the region {job}")
        seen.add(job)
        out.append(("jobs", job, chunk))
    return out


def pin_table(text: str) -> dict[str, object]:
    """The sha256 of every region of `text`; together the regions are the whole file."""
    regions = _regions(text)
    if "".join(block for _, _, block in regions) != text:
        raise ValueError("the pinned regions do not add up to the whole workflow")
    table: dict[str, dict[str, str]] = {"jobs": {}, "workflow": {}}
    for section, name, block in regions:
        table[section][name] = _digest(block)
    return {
        "_comment": _PIN_COMMENT,
        "jobs": dict(sorted(table["jobs"].items())),
        "workflow": dict(sorted(table["workflow"].items())),
    }


def write_pins(text: str, path: Path = PINS) -> None:
    path.write_text(
        json.dumps(pin_table(text), indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )


def _no_duplicates(pairs: list[tuple[str, object]]) -> dict[str, object]:
    keys = [k for k, _ in pairs]
    if len(keys) != len(set(keys)):
        raise ValueError("duplicate key")
    return dict(pairs)


def load_pins(path: Path = PINS) -> dict[str, dict[str, str]]:
    """Read the pin table and reject anything that is not exactly the expected shape."""
    try:
        table = json.loads(
            path.read_text(encoding="utf-8"), object_pairs_hook=_no_duplicates
        )
    except (OSError, ValueError) as err:
        raise ValueError(f"invalid pin table {PINS_NAME}: {err}") from err
    if not isinstance(table, dict) or set(table) - {"_comment", "jobs", "workflow"}:
        raise ValueError(f"invalid pin table {PINS_NAME}: unexpected top-level fields")
    if not isinstance(table.get("_comment", ""), str):
        raise ValueError(f"invalid pin table {PINS_NAME}: _comment is not text")
    for section in ("jobs", "workflow"):
        entries = table.get(section)
        if not isinstance(entries, dict) or not all(
            isinstance(v, str) and _SHA256.fullmatch(v) for v in entries.values()
        ):
            raise ValueError(
                f"invalid pin table {PINS_NAME}: {section} must map names to sha256 hex"
            )
    return {"jobs": table["jobs"], "workflow": table["workflow"]}


def check_pins(text: str, path: Path = PINS) -> None:
    """Fail unless every region of the workflow matches its committed pin."""
    pinned = load_pins(path)
    actual = pin_table(text)
    problems: list[str] = []
    for section, noun in (("jobs", "job"), ("workflow", "workflow block")):
        have, want = actual[section], pinned[section]
        for name in sorted(have.keys() - want.keys()):
            problems.append(f"{noun} {name} is new")
        for name in sorted(want.keys() - have.keys()):
            problems.append(f"{noun} {name} was removed or renamed")
        for name in sorted(have.keys() & want.keys()):
            if have[name] != want[name]:
                problems.append(f"{noun} {name} changed")
    if problems:
        raise ValueError(
            "; ".join(problems)
            + f". Review the diff for each in .github/workflows/release.yml, then update "
            f"its pin in {PINS_NAME} with scripts/dist_generate.sh --update-pins"
        )


def _verify_tag_before_publishing(text: str) -> str:
    """A tag-writer may move the tag after the provenance check; catch it at publish time.

    One check runs before dist touches the release, and one sits right before the
    step that undrafts it (see ``_RELEASE_NEW``).
    """
    return _replace_once(text, _HOST_STEP, _VERIFY_STEP + _HOST_STEP, "host step")


_ROUTE_JOB = """\
  # Tags of major 3 and later go through dist. Earlier tags (2.x) only publish the
  # draft release release-please made, so they release as they always did.
  # Every other job needs this one, so a draft pull request skips them all.
  route:
    if: ${{ github.event.pull_request.draft != true }}
    runs-on: "ubuntu-22.04"
    outputs:
      gated: ${{ steps.route.outputs.gated }}
    steps:
      - uses: {checkout}
        with:
          persist-credentials: false
      - id: route
        shell: bash
        env:
          REF_NAME: ${{ github.ref_name }}
          IS_PULL_REQUEST: ${{ github.event.pull_request != null }}
        run: |
          if [[ "$IS_PULL_REQUEST" == "true" ]]; then
            echo "gated=true" >> "$GITHUB_OUTPUT"
          else
            python3 scripts/release_provenance.py route-tag "$REF_NAME" >> "$GITHUB_OUTPUT"
          fi

"""
_LEGACY_JOB = """
  publish-legacy-release:
    name: Publish the release-please release (before 3.0)
    needs: route
    if: ${{ needs.route.outputs.gated == 'false' }}
    runs-on: "ubuntu-22.04"
    permissions:
      "contents": "read"
    env:
      GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
      RELEASE_TAG: ${{ github.ref_name }}
      RELEASE_COMMIT: ${{ github.sha }}
      REPO: ${{ github.repository }}
    steps:
      - uses: {checkout}
        with:
          persist-credentials: false
      - name: Mint the ruleset read token
        id: ruleset-token
        uses: actions/create-github-app-token@64ab729a6294000f5aa4a4bc0b73d1f32bc45510 # v3.3.0
        with:
          client-id: ${{ vars.RELEASE_APP_CLIENT_ID }}
          private-key: ${{ secrets.RELEASE_APP_PRIVATE_KEY }}
          owner: ${{ github.repository_owner }}
          repositories: ${{ github.event.repository.name }}
          permission-administration: read
      - name: Explain a failed token mint
        if: ${{ failure() && steps.ruleset-token.outcome == 'failure' }}
        shell: bash
        run: |
          echo "::error::Could not mint the ruleset read token."
          echo "::error::The release app needs Administration: Read-only on the slopvac/slopvac installation."
          exit 1
      - name: Require an immutable tag ruleset
        shell: bash
        env:
          GH_TOKEN: ${{ steps.ruleset-token.outputs.token }}
        run: |
          python3 scripts/release_ruleset.py check "$RELEASE_TAG" --repo "$REPO"
      - name: Check the release is the unpublished release-please release
        shell: bash
        env:
          APP_SLUG: ${{ vars.RELEASE_APP_SLUG }}
        run: |
          python3 scripts/release_provenance.py check-draft "$RELEASE_TAG" --repo "$REPO" --app-slug "$APP_SLUG"
      - name: Confirm the tag still points at the workflow commit
        shell: bash
        run: |
          python3 scripts/release_provenance.py verify-tag "$RELEASE_TAG" "$RELEASE_COMMIT"
{mint}\
      - name: Publish the release
        shell: bash
        env:
          GH_TOKEN: ${{ steps.release-token.outputs.token }}
        run: |
          gh release edit "$RELEASE_TAG" --repo "$REPO" --draft=false
"""
_ROUTED = "    if: ${{ needs.route.outputs.gated == 'true' }}\n"
_CHECKOUT = re.compile(r"uses: (actions/checkout@\S+ # \S+)\n")


def _route_by_major_version(text: str) -> str:
    """Skip the dist pipeline for tags before major 3, and publish their draft instead."""
    checkout = _CHECKOUT.search(text)
    if checkout is None:
        raise ValueError("expected a pinned actions/checkout step, found 0")
    job = _ROUTE_JOB.replace("{checkout}", checkout[1])
    jobs_head, plan = (
        "jobs:\n",
        (
            "  # Run 'dist plan' (or host) to determine what tasks we need to do\n"
            "  plan:\n"
        ),
    )
    text = _replace_once(text, jobs_head + plan, jobs_head + job + plan, "jobs header")
    text = _replace_once(
        text,
        '  plan:\n    runs-on: "ubuntu-22.04"\n',
        "  plan:\n    needs: route\n" + _ROUTED + '    runs-on: "ubuntu-22.04"\n',
        "plan job",
    )
    text = _replace_once(
        text,
        "  custom-release-provenance:\n    uses: ./.github/workflows/release-provenance.yml\n",
        "  custom-release-provenance:\n    needs: route\n"
        + _ROUTED
        + "    uses: ./.github/workflows/release-provenance.yml\n",
        "routed provenance job",
    )
    legacy = _LEGACY_JOB.replace("{checkout}", checkout[1]).replace(
        "{mint}", _RELEASE_TOKEN_STEP
    )
    return text.rstrip("\n") + "\n" + legacy


# The plan job installs dist with the generated curl-to-sh line, and the build jobs run
# whatever `matrix.install_dist.run` holds, which is the same line. Both become the
# pinned, digest-checked script.
_DIST_INSTALL_OLD = (
    """\
      - name: Install dist
        # we specify bash to get pipefail; it guards against the `curl` command
        # failing. otherwise `sh` won't catch that `curl` returned non-0
        shell: bash
        run: "curl --proto '=https' --tlsv1.2 -LsSf https://github.com/axodotdev/cargo-dist/releases/download/v0.34.0/cargo-dist-installer.sh | sh"
""",
    """\
      - name: Install dist
        run: ${{ matrix.install_dist.run }}
""",
)


def _pin_dist(text: str) -> str:
    """Install dist from a release archive whose sha256 is committed."""
    for old, what in zip(_DIST_INSTALL_OLD, ("plan", "build"), strict=True):
        text = _replace_once(
            text, old, _DIST_INSTALL_STEP, f"{what} dist install step"
        )
    return text


_RUSTUP_BOOTSTRAP = """\
      - name: Install Rust non-interactively if not already installed
        if: ${{ matrix.container }}
        run: |
          if ! command -v cargo > /dev/null 2>&1; then
            curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
            echo "$HOME/.cargo/bin" >> $GITHUB_PATH
          fi
"""
_AUDITABLE_OLD = """\
      - name: Install cargo-auditable
        run: ${{ matrix.install_cargo_auditable.run }}
"""
_AUDITABLE_NEW = """\
      - name: Install cargo-auditable
        run: bash scripts/install_cargo_auditable.sh
"""
_CYCLONEDX_OLD = """\
      - name: Install cargo-cyclonedx
        # we specify bash to get pipefail; it guards against the `curl` command
        # failing. otherwise `sh` won't catch that `curl` return non-0
        run: "curl --proto '=https' --tlsv1.2 -LsSf https://github.com/CycloneDX/cyclonedx-rust-cargo/releases/download/cargo-cyclonedx-0.5.5/cargo-cyclonedx-installer.sh | sh"
        shell: bash
"""
_CYCLONEDX_NEW = """\
      - name: Install cargo-cyclonedx
        run: bash scripts/install_cargo_cyclonedx.sh
"""


def _pin_tool_installers(text: str) -> str:
    """Replace every other download-to-shell install with a digest-checked script.

    cargo-auditable and cargo-cyclonedx come from pinned archives. The rustup bootstrap
    only runs in a container job, which this matrix does not have; it is removed so a
    later matrix change fails at the pinned toolchain step instead of piping rustup's
    installer into a shell.
    """
    for old, new, what in (
        (_RUSTUP_BOOTSTRAP, "", "rustup bootstrap step"),
        (_AUDITABLE_OLD, _AUDITABLE_NEW, "cargo-auditable install step"),
        (_CYCLONEDX_OLD, _CYCLONEDX_NEW, "cargo-cyclonedx install step"),
    ):
        text = _replace_once(text, old, new, what)
    return text


# A command that feeds a download to a shell: `curl ... | sh`, `irm ... | iex`,
# `sh -c "$(curl ...)"`, `bash <(curl ...)`; or a matrix install command dist supplies.
_DOWNLOAD_TO_SHELL = re.compile(
    r"\b(curl|wget|irm|iwr|Invoke-RestMethod|Invoke-WebRequest)\b.*"
    r"\|\s*(sudo\s+)?(bash|zsh|dash|sh|iex|Invoke-Expression)\b"
    r"|\b(bash|zsh|dash|sh)\s+(-\w+\s+)*[\"']?(\$\(|<\()\s*(curl|wget)\b"
    r"|\$\{\{\s*matrix\.install_",
    re.I,
)


def _assert_no_download_to_shell(text: str) -> None:
    """Fail when any line runs an unchecked download, so a template change cannot add one."""
    for number, line in enumerate(text.splitlines(), 1):
        if _DOWNLOAD_TO_SHELL.search(line):
            raise ValueError(f"line {number}: a download is piped into a shell")


def _pin(text: str, kind: str) -> str:
    """The pinned `uses:` value the template already holds for an action."""
    found = _USES[kind].search(text)
    if found is None:
        raise ValueError(f"expected a pinned {kind} action, found 0")
    return found[1]


def _add_sign_job(text: str) -> str:
    """Add the `sign` job, and make `host` wait for it and require its success."""
    condition = _HOST_IF.findall(text)
    if len(condition) != 1:
        raise ValueError(
            f"expected exactly one host condition to copy, found {len(condition)}"
        )
    job = (
        _SIGN_JOB.replace("{condition}", condition[0])
        .replace("{checkout}", _pin(text, "checkout"))
        .replace("{download}", _pin(text, "download"))
        .replace("{upload}", _pin(text, "upload"))
    )
    host_condition = condition[0].replace(
        " }}\n", " && needs.sign.result == 'success' }}\n"
    )
    text = text.replace(condition[0], host_condition)
    anchor = "  # Determines if we should publish/announce\n  host:\n"
    return _replace_once(text, anchor, job + anchor, "host job header")


def _gate_on_provenance(text: str) -> str:
    """Run the provenance job without inherited secrets, and make `host` require it.

    dist lets `host` run when the build jobs were skipped, so a failed provenance
    job that skips the builds must still stop `host`.
    """
    text = _replace_once(text, _CUSTOM_OLD, _CUSTOM_NEW, "provenance plan job")
    text = _replace_once(text, _HOST_NEEDS_OLD, _HOST_NEEDS_NEW, "host needs")
    return _replace_once(text, _HOST_IF_OLD, _HOST_IF_NEW, "host condition")


def _insert_validation(text: str) -> str:
    """Make the tag check the first step of the plan job."""
    anchor = "      publishing: ${{ !github.event.pull_request }}\n"
    start = text.find(anchor)
    if text.count(anchor) != 1:
        raise ValueError("expected exactly one plan job outputs block")
    steps = text.index("    steps:\n", start) + len("    steps:\n")
    return text[:steps] + _VALIDATE + text[steps:]


def _add_tag_flag_env(text: str) -> str:
    """Give each job that runs a `dist ... $TAG_FLAG` command the flag as env."""
    out: list[str] = []
    for chunk in _split_jobs(text):
        if "$TAG_FLAG" in chunk:
            chunk = _replace_once(
                chunk,
                _JOB_ENV,
                _JOB_ENV + "      TAG_FLAG: ${{ needs.plan.outputs.tag-flag }}\n",
                "job env GH_TOKEN",
            )
        out.append(chunk)
    return "".join(out)


def _split_jobs(text: str) -> list[str]:
    lines = text.splitlines(keepends=True)
    chunks: list[str] = []
    current: list[str] = []
    in_jobs = False
    for line in lines:
        if line.startswith("jobs:"):
            in_jobs = True
        elif (
            in_jobs
            and line.startswith("  ")
            and not line.startswith("   ")
            and line.rstrip().endswith(":")
            and not line.lstrip().startswith("#")
        ):
            chunks.append("".join(current))
            current = []
        current.append(line)
    chunks.append("".join(current))
    return chunks


def _assert_clean(text: str) -> None:
    """No `${{ }}` expression of tag-controlled data may sit inside a `run:` script."""
    banned = ("github.ref_name", "needs.plan.outputs.tag")
    in_run = False
    indent = 0
    for number, line in enumerate(text.splitlines(), 1):
        stripped = line.lstrip()
        width = len(line) - len(stripped)
        if in_run and stripped and width <= indent:
            in_run = False
        if stripped.startswith(("run:", "- run:")):
            indent = width
            in_run = True
            body = stripped.split("run:", 1)[1]
            if any(b in body for b in banned) and "${{" in body:
                raise ValueError(f"line {number}: tag interpolated into a run command")
        elif in_run and "${{" in line and any(b in line for b in banned):
            raise ValueError(f"line {number}: tag interpolated into a run script")


# Set only by `scripts/dist_generate.sh --update-pins`, the one route that writes the pins.
UPDATE_PINS_ENV = "SLOPVAC_UPDATE_PINS"


def main(argv: list[str]) -> int:
    if argv:
        print("error: this script takes no arguments", file=sys.stderr)
        return 2
    original = WORKFLOW.read_text(encoding="utf-8")
    try:
        hardened = harden(original)
        WORKFLOW.write_text(hardened, encoding="utf-8")
        if os.environ.get(UPDATE_PINS_ENV) == "1":
            write_pins(hardened)
        else:
            check_pins(hardened)
    except ValueError as err:
        print(f"error: {err}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
