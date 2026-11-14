#!/usr/bin/env python3
"""Set the workspace version in Cargo.toml and refresh Cargo.lock.

Release candidates build with a version such as ``3.0.0-rc.1`` that no commit
carries. CI calls this before building, so the binary, the archives and the
wheels all report the candidate version.
"""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

_SEMVER = re.compile(r"\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?")
_VERSION_LINE = re.compile(r'^version = "[^"]*"', re.MULTILINE)


def set_workspace_version(text: str, version: str) -> str:
    """Return ``text`` with ``[workspace.package] version`` set to ``version``."""
    if _SEMVER.fullmatch(version) is None:
        raise ValueError(f"not a SemVer version: {version}")
    start = text.find("[workspace.package]")
    if start < 0:
        raise ValueError("Cargo.toml has no [workspace.package] table")
    end = text.find("\n[", start + 1)
    end = len(text) if end < 0 else end
    section = text[start:end]
    if _VERSION_LINE.search(section) is None:
        raise ValueError("[workspace.package] has no version")
    return (
        text[:start]
        + _VERSION_LINE.sub(f'version = "{version}"', section, count=1)
        + text[end:]
    )


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print("usage: set_workspace_version.py VERSION", file=sys.stderr)
        return 2
    manifest = Path("Cargo.toml")
    try:
        manifest.write_text(
            set_workspace_version(manifest.read_text(encoding="utf-8"), argv[1]),
            encoding="utf-8",
        )
    except ValueError as err:
        print(f"error: {err}", file=sys.stderr)
        return 1
    return subprocess.run(
        ["cargo", "update", "--workspace", "--offline"], check=False
    ).returncode


if __name__ == "__main__":
    sys.exit(main(sys.argv))
