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
* the Windows build step runs under bash, where `$TAG_FLAG` is an environment
  variable (under PowerShell it expands to nothing);
* the CycloneDX step output is read as `outputs`, which dist spells `output`.

Run `scripts/dist_generate.sh` instead of `dist generate`. Each rewrite must
match exactly once, so a dist upgrade that changes the template fails loudly
here instead of silently dropping a protection.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

WORKFLOW = Path(".github/workflows/release.yml")

_TRIGGER = ("      - '**[0-9]+.[0-9]+.[0-9]+*'\n", "      - 'v[0-9]+.[0-9]+.[0-9]+'\n")

_VALIDATE = """\
      - name: Validate the tag
        if: ${{ !github.event.pull_request }}
        shell: bash
        env:
          REF_NAME: ${{ github.ref_name }}
        run: |
          if [[ ! "$REF_NAME" =~ ^v[0-9]+\\.[0-9]+\\.[0-9]+$ ]]; then
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
      - name: Upload the artifacts to the draft release
        env:
          RELEASE_TAG: ${{ needs.plan.outputs.tag }}
        run: |
          gh release upload "$RELEASE_TAG" artifacts/*
{verify}\
      - name: Publish the release
        env:
          RELEASE_TAG: ${{ needs.plan.outputs.tag }}
          PRERELEASE_FLAG: "${{ fromJson(steps.host.outputs.manifest).announcement_is_prerelease && '--prerelease' || '' }}"
          RELEASE_COMMIT: "${{ github.sha }}"
        run: |
          gh release edit "$RELEASE_TAG" --target "$RELEASE_COMMIT" $PRERELEASE_FLAG --draft=false
"""
_JOB_ENV = "      GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}\n"
_HOST_ENV = (
    "    env:\n"
    + _JOB_ENV
    + '    runs-on: "ubuntu-22.04"\n    outputs:\n      val: ${{ steps.host.outputs.manifest }}\n'
)


def _replace_once(text: str, old: str, new: str, what: str) -> str:
    if text.count(old) != 1:
        raise ValueError(
            f"expected exactly one {what} in the generated workflow, found {text.count(old)}"
        )
    return text.replace(old, new)


def harden(text: str) -> str:
    text = _replace_once(text, *_TRIGGER, "tag trigger")
    text = _insert_validation(text)
    text = _replace_once(text, _PLAN_RUN_OLD, _PLAN_RUN_NEW, "plan command")
    text = _replace_once(
        text,
        _RELEASE_OLD,
        _RELEASE_NEW.replace("{verify}", _VERIFY_STEP),
        "release step",
    )
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
        '    permissions:\n      "contents": "write"\n' + _HOST_ENV,
        "host job env",
    )
    text = _add_tag_flag_env(text)
    text = _gate_on_provenance(text)
    text = _replace_once(text, _BUILD_OLD, _BUILD_NEW, "build artifacts step")
    text = _replace_once(text, _SBOM_OLD, _SBOM_NEW, "SBOM output reference")
    text = _verify_tag_before_publishing(text)
    text = _route_by_major_version(text)
    _assert_clean(text)
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
"""
_HOST_NEEDS_OLD = "  host:\n    needs:\n      - plan\n"
_HOST_NEEDS_NEW = (
    "  host:\n    needs:\n      - plan\n      - custom-release-provenance\n"
)
_HOST_IF_OLD = "if: ${{ always() && needs.plan.result == 'success' && needs.plan.outputs.publishing == 'true' &&"
_HOST_IF_NEW = (
    "if: ${{ always() && needs.plan.result == 'success' && needs.custom-release-provenance.result == 'success'"
    " && needs.plan.outputs.publishing == 'true' &&"
)


_BUILD_OLD = "      - name: Build artifacts\n        run: |\n"
_BUILD_NEW = "      - name: Build artifacts\n        shell: bash\n        run: |\n"
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


def _verify_tag_before_publishing(text: str) -> str:
    """A tag-writer may move the tag after the provenance check; catch it at publish time.

    One check runs before dist touches the release, and one sits right before the
    step that undrafts it (see ``_RELEASE_NEW``).
    """
    return _replace_once(text, _HOST_STEP, _VERIFY_STEP + _HOST_STEP, "host step")


_ROUTE_JOB = """\
  # Tags of major 3 and later go through dist. Earlier tags (2.x) only publish the
  # draft release release-please made, so they release as they always did.
  route:
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
      "contents": "write"
    env:
      GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
      RELEASE_TAG: ${{ github.ref_name }}
      REPO: ${{ github.repository }}
    steps:
      - name: Publish the draft release
        shell: bash
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
    return text.rstrip("\n") + "\n" + _LEGACY_JOB


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


def main() -> int:
    original = WORKFLOW.read_text(encoding="utf-8")
    try:
        WORKFLOW.write_text(harden(original), encoding="utf-8")
    except ValueError as err:
        print(f"error: {err}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
