#!/usr/bin/env python3
"""Regenerate the HTML that docutils writes for each fixture `*.rst`.

The Rust tests feed this HTML to the position mapper through a fake converter, so the
mapping runs on real docutils output without docutils on the test machine.

    uv run --with docutils==0.23 python generate.py          # write
    uv run --with docutils==0.23 python generate.py --check  # exit 1 when stale

The arguments come from `ARGS` in `src/rst/run.rs`, so the fixtures cannot drift from
what the adapter runs.
"""

from __future__ import annotations

import os
import re
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
RUN_RS = HERE.parents[2] / "src" / "rst" / "run.rs"
DOCUTILS_VERSION = "0.23"


def converter_args() -> list[str]:
    block = re.search(
        r"const ARGS: \[&str; \d+\] = \[(.*?)\];", RUN_RS.read_text(), re.S
    )
    if block is None:
        sys.exit(f"cannot find ARGS in {RUN_RS}")
    return re.findall(r'"([^"]+)"', block.group(1))


def convert(source: Path) -> str:
    import docutils

    if docutils.__version__ != DOCUTILS_VERSION:
        sys.exit(f"docutils {DOCUTILS_VERSION} required, found {docutils.__version__}")
    converter = Path(sys.executable).parent / "rst2html"
    env = {"PATH": os.environ["PATH"], "DOCUTILSCONFIG": os.devnull}
    done = subprocess.run(
        [str(converter), *converter_args()],
        input=source.read_bytes(),
        capture_output=True,
        env=env,
        check=True,
    )
    html = done.stdout.decode("utf-8")
    # The stylesheet path is the local install path; pin it so the output is portable.
    return re.sub(r'href="[^"]*html4css1\.css"', 'href="html4css1.css"', html)


def main() -> int:
    check = "--check" in sys.argv[1:]
    stale = []
    for source in sorted(HERE.glob("*.rst")):
        target = source.with_suffix(".html")
        html = convert(source)
        if check:
            if not target.exists() or target.read_text() != html:
                stale.append(target.name)
        else:
            target.write_text(html)
    if stale:
        print("stale: " + ", ".join(stale), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
