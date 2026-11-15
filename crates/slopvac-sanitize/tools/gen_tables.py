#!/usr/bin/env python3
"""Generate the unsafe-character tables from pinned Unicode Character Database data.

    gen_tables.py extract <ucd-dir>   # refresh tools/ucd/<version>/ from a download
    gen_tables.py                     # write src/tables.rs and the Python SDK table
    gen_tables.py --check             # fail if the checked-in tables differ

The table lists every code point that can forge or hide terminal output:
general categories Cc, Cf, Co, Zl and Zp; the Default_Ignorable_Code_Point
property (which includes reserved code points such as U+2065); and the
noncharacters (a stable subset of Cn). Other unassigned code points are left
out, because their set changes with every Unicode version and a plain new
letter or emoji must not get escaped.

Provenance: `extract` refuses any download whose SHA-256 differs from the pinned
digests of the upstream files below. It copies the lines that the generator
needs into `tools/ucd/<version>/*.subset.txt`, which are checked in. The
generator verifies the digests of those subsets before it reads them, so an
edited or truncated subset fails the run. To move to a newer Unicode version,
download both files from https://www.unicode.org/Public/<version>/ucd/, replace
the constants here, and run `extract` and then the generator.
"""

import hashlib
import sys
from pathlib import Path

UNICODE_VERSION = "17.0.0"
CATEGORIES = {"Cc", "Cf", "Co", "Zl", "Zp"}
PROPERTY = "Default_Ignorable_Code_Point"

# SHA-256 of the files on https://www.unicode.org/Public/17.0.0/ucd/ .
UPSTREAM_SHA256 = {
    "UnicodeData.txt": "2e1efc1dcb59c575eedf5ccae60f95229f706ee6d031835247d843c11d96470c",
    "DerivedCoreProperties.txt": "24c7fed1195c482faaefd5c1e7eb821c5ee1fb6de07ecdbaa64b56a99da22c08",
}
# SHA-256 of the checked-in subsets that `extract` writes.
SUBSET_SHA256 = {
    "UnicodeData.subset.txt": "053cc6774c2439c7382b0db1463b0a4e9fe651780e9229260b2008b520304489",
    "DerivedCoreProperties.subset.txt": "cad9289eddbc282ab5d8225cab7e0de90d2f3dd538488452e6ce4308071872c2",
}

HERE = Path(__file__).resolve().parent
CRATE = HERE.parent
REPO = HERE.parents[2]
SUBSET_DIR = HERE / "ucd" / UNICODE_VERSION
RUST_OUT = CRATE / "src" / "tables.rs"
PYTHON_OUT = (
    REPO / "packages/slopvac-plugin/src/slopvac_plugin/conformance/_unsafe_ranges.py"
)


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def read_verified(path: Path, want: str) -> str:
    data = path.read_bytes()
    got = sha256(data)
    if got != want:
        sys.exit(f"{path.name}: SHA-256 is {got}, pinned {want}")
    return data.decode("utf-8")


def unicode_data_subset(text: str) -> list[str]:
    """Lines of the wanted categories, with both ends of every First/Last range."""
    keep = []
    for line in text.splitlines():
        if line.split(";")[2] in CATEGORIES:
            keep.append(line)
    return keep


def default_ignorable_subset(text: str) -> list[str]:
    keep = []
    for line in text.splitlines():
        body = line.split("#", 1)[0]
        fields = [part.strip() for part in body.split(";")]
        if len(fields) >= 2 and fields[1] == PROPERTY:
            keep.append(line)
    return keep


def extract(ucd: Path) -> None:
    SUBSET_DIR.mkdir(parents=True, exist_ok=True)
    pairs = [
        ("UnicodeData.txt", "UnicodeData.subset.txt", unicode_data_subset),
        (
            "DerivedCoreProperties.txt",
            "DerivedCoreProperties.subset.txt",
            default_ignorable_subset,
        ),
    ]
    for upstream, subset, pick in pairs:
        text = read_verified(ucd / upstream, UPSTREAM_SHA256[upstream])
        body = "\n".join(pick(text)) + "\n"
        (SUBSET_DIR / subset).write_text(body, encoding="utf-8", newline="\n")
        print(f"{subset}: sha256 {sha256(body.encode())}", file=sys.stderr)


def unicode_data(text: str) -> set[int]:
    points: set[int] = set()
    first = None
    for line in text.splitlines():
        fields = line.split(";")
        code, name, category = int(fields[0], 16), fields[1], fields[2]
        if name.endswith(", First>"):
            first = code
            continue
        low = first if name.endswith(", Last>") and first is not None else code
        first = None
        if category in CATEGORIES:
            points.update(range(low, code + 1))
    return points


def default_ignorable(text: str) -> set[int]:
    points: set[int] = set()
    for line in text.splitlines():
        body = line.split("#", 1)[0].strip()
        if not body:
            continue
        span, prop, *_ = (part.strip() for part in body.split(";"))
        if prop != PROPERTY:
            continue
        low, _, high = span.partition("..")
        points.update(range(int(low, 16), int(high or low, 16) + 1))
    return points


def noncharacters() -> set[int]:
    points = set(range(0xFDD0, 0xFDF0))
    for plane in range(17):
        points.update((plane << 16 | 0xFFFE, plane << 16 | 0xFFFF))
    return points


def ranges(points: set[int]) -> list[tuple[int, int]]:
    out: list[tuple[int, int]] = []
    for p in sorted(points):
        if out and out[-1][1] + 1 == p:
            out[-1] = (out[-1][0], p)
        else:
            out.append((p, p))
    return out


def hex_literal(n: int) -> str:
    """Hex with `_` every four digits, as clippy's unreadable_literal wants."""
    digits = f"{n:x}"
    if len(digits) <= 4:
        return f"0x{digits}"
    digits = digits.zfill(-(-len(digits) // 4) * 4)
    return "0x" + "_".join(digits[i : i + 4] for i in range(0, len(digits), 4))


def table() -> list[tuple[int, int]]:
    ud = read_verified(
        SUBSET_DIR / "UnicodeData.subset.txt", SUBSET_SHA256["UnicodeData.subset.txt"]
    )
    dcp = read_verified(
        SUBSET_DIR / "DerivedCoreProperties.subset.txt",
        SUBSET_SHA256["DerivedCoreProperties.subset.txt"],
    )
    return ranges(unicode_data(ud) | default_ignorable(dcp) | noncharacters())


def render_rust(rows: list[tuple[int, int]]) -> str:
    out = [
        "// @generated by tools/gen_tables.py. Do not edit by hand.",
        f"// Source: Unicode Character Database {UNICODE_VERSION}.",
        "",
        "/// Version of the Unicode Character Database behind the table.",
        f'pub const UNICODE_VERSION: &str = "{UNICODE_VERSION}";',
        "",
        "/// Sorted, non-overlapping, inclusive ranges of code points to escape.",
        "pub(crate) const UNSAFE: &[(u32, u32)] = &[",
    ]
    out += [f"    ({hex_literal(lo)}, {hex_literal(hi)})," for lo, hi in rows]
    out.append("];")
    return "\n".join(out) + "\n"


def render_python(rows: list[tuple[int, int]]) -> str:
    out = [
        '"""Characters `escape_text` escapes. Generated; do not edit by hand.',
        "",
        "Written by crates/slopvac-sanitize/tools/gen_tables.py from the same",
        f"Unicode Character Database {UNICODE_VERSION} data as the Rust table of",
        "`slopvac-sanitize`, so both sides escape the same code points.",
        '"""',
        "",
        f'UNICODE_VERSION = "{UNICODE_VERSION}"',
        "",
        "# Sorted, non-overlapping, inclusive (low, high) code point ranges.",
        "UNSAFE_RANGES: tuple[tuple[int, int], ...] = (",
    ]
    out += [f"    (0x{lo:04X}, 0x{hi:04X})," for lo, hi in rows]
    out.append(")")
    return "\n".join(out) + "\n"


def main(argv: list[str]) -> None:
    if argv[:1] == ["extract"]:
        extract(Path(argv[1]))
        return
    rows = table()
    outputs = {RUST_OUT: render_rust(rows), PYTHON_OUT: render_python(rows)}
    if argv == ["--check"]:
        stale = [p for p, text in outputs.items() if p.read_text() != text]
        if stale:
            sys.exit("stale generated file: " + ", ".join(str(p) for p in stale))
        return
    for path, text in outputs.items():
        path.write_text(text, encoding="utf-8", newline="\n")


if __name__ == "__main__":
    main(sys.argv[1:])
