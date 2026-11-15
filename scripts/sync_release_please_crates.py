#!/usr/bin/env python3
"""Keep release-please's Cargo.lock extra-files in step with the workspace crates.

Every workspace crate needs one Cargo.lock version entry in
release-please-config.json, or release PRs leave its lock version stale.
Run without arguments to rewrite the entries; use --check to fail on drift.
Reads Cargo.toml files with tomllib, so it needs no Rust toolchain.
"""

from __future__ import annotations

import argparse
import difflib
import json
import sys
import tomllib
from pathlib import Path

CONFIG = "release-please-config.json"
COMMAND = "python3 scripts/sync_release_please_crates.py"
JSONPATH = "$.package[?(@.name.value=='{name}')].version"


def workspace_crate_names(root: Path) -> list[str]:
    """Package names of all workspace members, sorted."""
    manifest = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
    members = manifest.get("workspace", {}).get("members", [])
    excluded = {
        p for pattern in manifest.get("workspace", {}).get("exclude", [])
        for p in root.glob(pattern)
    }
    names = set()
    for pattern in members:
        for directory in root.glob(pattern):
            cargo = directory / "Cargo.toml"
            if directory in excluded or not cargo.is_file():
                continue
            package = tomllib.loads(cargo.read_text(encoding="utf-8"))["package"]
            names.add(package["name"])
    return sorted(names)


def _is_lock_entry(entry: object) -> bool:
    return isinstance(entry, dict) and entry.get("path") == "Cargo.lock"


def render(root: Path) -> str:
    """The config text with one sorted Cargo.lock entry per workspace crate."""
    config = json.loads((root / CONFIG).read_text(encoding="utf-8"))
    package = config["packages"]["."]
    extra = package["extra-files"]
    existing = {
        e["jsonpath"]: e for e in extra if _is_lock_entry(e) and "jsonpath" in e
    }
    entries = []
    for name in workspace_crate_names(root):
        jsonpath = JSONPATH.format(name=name)
        entries.append(
            existing.get(jsonpath)
            or {"type": "toml", "path": "Cargo.lock", "jsonpath": jsonpath}
        )
    first = next((i for i, e in enumerate(extra) if _is_lock_entry(e)), len(extra))
    kept = [e for e in extra if not _is_lock_entry(e)]
    before = sum(1 for e in extra[:first] if not _is_lock_entry(e))
    package["extra-files"] = kept[:before] + entries + kept[before:]
    return json.dumps(config, indent=2, ensure_ascii=False) + "\n"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--check", action="store_true", help="fail on drift")
    parser.add_argument("--root", type=Path, default=Path(__file__).parents[1])
    args = parser.parse_args(argv)
    path = args.root / CONFIG
    current = path.read_text(encoding="utf-8")
    wanted = render(args.root)
    if current == wanted:
        return 0
    if args.check:
        sys.stdout.writelines(
            difflib.unified_diff(
                current.splitlines(keepends=True),
                wanted.splitlines(keepends=True),
                CONFIG,
                f"{CONFIG} (expected)",
            )
        )
        print(f"\n{CONFIG} is out of sync with the workspace crates.")
        print(f"Regenerate with: {COMMAND}")
        return 1
    path.write_text(wanted, encoding="utf-8")
    print(f"updated {CONFIG}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
