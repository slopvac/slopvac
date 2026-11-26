"""Escape untrusted text for the GitHub Actions log, for every script that prints it.

The runner reads a workflow command from any line of a job log that starts with `::` or
`##[`, and reads `%0A` style escapes in its data. A file name, a path or a message from a
pull request can carry those. The functions here are the one Python escaper, imported by the
Action's script (`action_v3.py`) and by the Markdown audit. They ship in `scripts/` next to
`action_v3.py`, so the Action finds them through its own path.
"""

from __future__ import annotations

# Inclusive code point ranges that slopvac-sanitize escapes (`UNSAFE` in
# crates/slopvac-sanitize/src/tables.rs). tests/fixtures/workflow-escape-vectors.json
# holds both edges of every range and is checked by the Rust and the Python side.
_UNSAFE_RANGES = (
    (0x0, 0x1F), (0x7F, 0x9F), (0xAD, 0xAD), (0x34F, 0x34F), (0x600, 0x605),
    (0x61C, 0x61C), (0x6DD, 0x6DD), (0x70F, 0x70F), (0x890, 0x891), (0x8E2, 0x8E2),
    (0x115F, 0x1160), (0x17B4, 0x17B5), (0x180B, 0x180F), (0x200B, 0x200F),
    (0x2028, 0x202E), (0x2060, 0x206F), (0x3164, 0x3164), (0xE000, 0xF8FF),
    (0xFDD0, 0xFDEF), (0xFE00, 0xFE0F), (0xFEFF, 0xFEFF), (0xFFA0, 0xFFA0),
    (0xFFF0, 0xFFFB), (0xFFFE, 0xFFFF), (0x110BD, 0x110BD), (0x110CD, 0x110CD),
    (0x13430, 0x1343F), (0x1BCA0, 0x1BCA3), (0x1D173, 0x1D17A),
    *((plane << 16 | 0xFFFE, plane << 16 | 0xFFFF) for plane in range(1, 15)),
    (0xE0000, 0xE0FFF), (0xEFFFE, 0x10FFFF),
)  # fmt: skip


def _is_unsafe(char: str) -> bool:
    code = ord(char)
    return any(low <= code <= high for low, high in _UNSAFE_RANGES)


def escape_workflow_data(text: str) -> str:
    r"""Escape untrusted text so the runner reads no workflow command in it.

    The same function as ``workflow_data`` in crates/slopvac-sanitize
    (TM-13/O4): ``%`` becomes ``%25``, CR ``%0D``, LF ``%0A``, ``::`` becomes
    ``%3A%3A``, ``##[`` is neutralised, a backslash is doubled, and an unsafe
    character becomes ``\u{..}``. Tab stays. The shared vectors in
    tests/fixtures/workflow-escape-vectors.json keep the two in step.
    """
    out = []
    for char in text:
        if char == "%":
            out.append("%25")
        elif char == "\r":
            out.append("%0D")
        elif char == "\n":
            out.append("%0A")
        elif char == "\\":
            out.append("\\\\")
        elif char != "\t" and _is_unsafe(char):
            out.append(f"\\u{{{ord(char):x}}}")
        else:
            out.append(char)
    return "".join(out).replace("::", "%3A%3A").replace("##[", "\\u{23}#[")
