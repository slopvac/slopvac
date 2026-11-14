"""Refuse to run Vale-dependent tests under a Vale other than the mise pin.

CI installs the Vale version pinned in `mise.toml`. A developer whose shell
resolves another `vale` first (for example a Homebrew install) gets failures
that look like rule bugs but come from Vale output drift.

Tests that need Vale carry the `vale` marker. After collection this plugin:

- skips marked tests when no `vale` is on PATH, without reading `mise.toml`;
- stops the run with a usage error when the `vale` on PATH is not the pin, so
  a mismatch never turns into a skip that hides a real failure.

Unmarked tests and `-k` or node-id selections that do not touch marked tests
are unaffected. Each xdist worker runs its own collection, so each enforces
the check on the items it collected.
"""

from __future__ import annotations

import re
import shutil
import subprocess
import tomllib
from pathlib import Path

import pytest

MARKER = "vale"
_VERSION = re.compile(r"vale version (\S+)")
_HINT = "Run the suite with `mise exec -- uv run pytest` so the pinned Vale is used."


def pytest_configure(config: pytest.Config) -> None:
    config.addinivalue_line(
        "markers", f"{MARKER}: test runs Vale; needs the pinned Vale version"
    )


def pinned_vale_version(rootpath: Path) -> str:
    path = next(
        (
            folder / "mise.toml"
            for folder in (rootpath, *rootpath.parents)
            if (folder / "mise.toml").is_file()
        ),
        rootpath / "mise.toml",
    )
    try:
        text = path.read_text(encoding="utf-8")
    except OSError as error:
        raise pytest.UsageError(
            f"cannot find mise.toml above {rootpath} to read the pinned Vale "
            f"version ({error}). Vale-dependent tests need the repository's mise.toml."
        ) from error
    try:
        data = tomllib.loads(text)
    except tomllib.TOMLDecodeError as error:
        raise pytest.UsageError(f"{path} is not valid TOML: {error}") from error
    pinned = data.get("tools", {}).get("vale") if isinstance(data, dict) else None
    if not isinstance(pinned, str) or not pinned:
        raise pytest.UsageError(f"{path} has no string pin at [tools].vale")
    return pinned


def installed_vale_version(binary: str) -> str:
    result = subprocess.run(
        [binary, "--version"], capture_output=True, text=True, check=False
    )
    match = _VERSION.search(result.stdout)
    if match is None:
        raise pytest.UsageError(
            f"cannot read the version from `{binary} --version`: "
            f"{result.stdout.strip() or result.stderr.strip()!r}"
        )
    return match.group(1)


@pytest.hookimpl(trylast=True)
def pytest_collection_modifyitems(
    config: pytest.Config, items: list[pytest.Item]
) -> None:
    selected = [item for item in items if item.get_closest_marker(MARKER)]
    if not selected:
        return
    binary = shutil.which("vale")
    if binary is None:
        skip = pytest.mark.skip(reason="vale is not on PATH")
        for item in selected:
            item.add_marker(skip)
        return
    pinned = pinned_vale_version(config.rootpath)
    installed = installed_vale_version(binary)
    if installed != pinned:
        raise pytest.UsageError(
            f"vale {installed} is on PATH but mise.toml pins {pinned}. {_HINT}"
        )
