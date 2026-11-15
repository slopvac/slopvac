#!/usr/bin/env python3
"""Regenerate the driver output for each fixture `*.rst`.

The Rust tests feed this JSON to the position mapper through a fake interpreter, so the
mapping runs on real docutils output without docutils on the test machine.

    uv run --with docutils==0.23 python generate.py          # write
    uv run --with docutils==0.23 python generate.py --check  # exit 1 when stale

The program is `src/rst/driver.py`, the file the adapter embeds, run the way the adapter
runs it: `python -E -B -X utf8 -c <driver>`.
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
DRIVER = HERE.parents[2] / "src" / "rst" / "driver.py"
DOCUTILS_VERSION = "0.23"


def convert(source: Path) -> str:
    done = subprocess.run(
        [
            sys.executable,
            "-E",
            "-B",
            "-X",
            "utf8",
            "-c",
            DRIVER.read_text(encoding="utf-8"),
        ],
        input=source.read_bytes(),
        capture_output=True,
        check=True,
    )
    result = json.loads(done.stdout.decode("utf-8"))
    if result["docutils"] != DOCUTILS_VERSION:
        sys.exit(f"docutils {DOCUTILS_VERSION} required, found {result['docutils']}")
    return json.dumps(result, ensure_ascii=False, indent=1) + "\n"


def main() -> int:
    check = "--check" in sys.argv[1:]
    stale = []
    for source in sorted(HERE.glob("*.rst")):
        target = source.with_suffix(".json")
        text = convert(source)
        if check:
            if not target.exists() or target.read_text(encoding="utf-8") != text:
                stale.append(target.name)
        else:
            target.write_text(text, encoding="utf-8")
    if stale:
        print("stale: " + ", ".join(stale), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
