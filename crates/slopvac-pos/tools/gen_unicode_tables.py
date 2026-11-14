#!/usr/bin/env python3
"""Generate (or check) src/unicode_tables.rs from Go's unicode package.

prose v3.3.1 classifies characters with Go's `unicode.IsUpper`, `IsLower`,
`IsDigit`, `IsPunct` and `IsSymbol`, so the Rust port uses Go's tables, not
Rust's Unicode properties (`char::is_uppercase` includes Other_Uppercase,
`is_numeric` includes No and Nl). The tables must come from the Unicode version
of the Go that built the reference tagger: Vale 3.27.0 is built with
go1.27.3, which ships Unicode 17.0.0, as does go1.27.4.

  gen_unicode_tables.py          write src/unicode_tables.rs
  gen_unicode_tables.py --check  fail if the committed file is stale

Needs `go` on PATH (any release with the same Unicode version).
"""

from __future__ import annotations

import argparse
import pathlib
import re
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
OUT = HERE.parent / "src" / "unicode_tables.rs"


def generate() -> str:
    proc = subprocess.run(
        ["go", "run", str(HERE / "unicode_tables.go")],
        capture_output=True, text=True, check=False,
    )  # fmt: skip
    if proc.returncode != 0:
        sys.exit(f"go run failed: {proc.stderr.strip()}")
    return proc.stdout


def comparable(text: str) -> str:
    """Drops the Go release from the header: only the Unicode version matters."""
    return re.sub(r"\(Go [^,]+, ", "(", text, count=1)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--check", action="store_true", help="compare, do not write")
    args = ap.parse_args()
    text = generate()
    if args.check:
        if not OUT.exists() or comparable(OUT.read_text()) != comparable(text):
            print(f"{OUT.name} is stale; run gen_unicode_tables.py", file=sys.stderr)
            return 1
        return 0
    OUT.write_text(text)
    return 0


if __name__ == "__main__":
    sys.exit(main())
