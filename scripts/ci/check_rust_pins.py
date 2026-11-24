"""Fail when the Rust pin in mise.toml differs from rust-toolchain.toml.

Both files pin the toolchain: rust-toolchain.toml for rustup, mise.toml so a
project-level mise ``rust`` entry wins over a global one. The check compares
version, components and profile, and prints nothing when they agree.

Usage: python3 scripts/ci/check_rust_pins.py [ROOT]
"""

from __future__ import annotations

import sys
import tomllib
from pathlib import Path


def _components(value: object) -> list[str]:
    if isinstance(value, str):
        items = value.split(",")
    elif isinstance(value, list):
        items = [str(item) for item in value]
    else:
        items = []
    return sorted(item.strip() for item in items if item.strip())


def pins(root: Path) -> tuple[dict[str, object], dict[str, object]]:
    toolchain = tomllib.loads((root / "rust-toolchain.toml").read_text())["toolchain"]
    rust = tomllib.loads((root / "mise.toml").read_text())["tools"]["rust"]
    if not isinstance(rust, dict):
        rust = {"version": rust}
    want = {
        "version": str(toolchain["channel"]),
        "components": _components(toolchain.get("components")),
        "profile": toolchain.get("profile", "default"),
    }
    got = {
        "version": str(rust.get("version")),
        "components": _components(rust.get("components")),
        "profile": rust.get("profile", "default"),
    }
    return got, want


def main(argv: list[str]) -> int:
    root = Path(argv[1]) if len(argv) > 1 else Path.cwd()
    got, want = pins(root)
    if got != want:
        print(
            f"mise.toml rust {got} differs from rust-toolchain.toml {want}",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
