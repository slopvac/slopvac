"""The privileged fork-review consumer must reject a hostile artifact.

The valid report is the rdjsonl the real slopvac binary writes for a sloppy
document (see real_binary.py).
"""

from __future__ import annotations

import importlib.util
import json
import os
import sys
import zipfile
from pathlib import Path

import pytest

pytest_plugins = ["real_binary"]  # the real-binary fixtures

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "fork_review.py"
SPEC = importlib.util.spec_from_file_location("fork_review", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
fork = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = fork
SPEC.loader.exec_module(fork)

PR = "42"
SHA = "0123456789abcdef0123456789abcdef01234567"
OTHER_SHA = "f" * 40


@pytest.fixture(scope="module")
def valid_report(slop_project, lint_with_binary, tmp_path_factory) -> bytes:
    out = tmp_path_factory.mktemp("report") / "findings.rdjsonl"
    assert lint_with_binary(slop_project, "rdjsonl", out, "slop.md") == 1
    return out.read_bytes()


def make_archive(
    path: Path, entries: dict[str, bytes], method: int = zipfile.ZIP_DEFLATED
) -> Path:
    with zipfile.ZipFile(path, "w", method) as zf:
        for name, data in entries.items():
            zf.writestr(name, data)
    return path


@pytest.fixture
def good(valid_report):
    return {
        "findings.rdjsonl": valid_report,
        "pr-number": f"{PR}\n".encode(),
        "head-sha": f"{SHA}\n".encode(),
    }


@pytest.fixture
def check(tmp_path):
    def go(entries, *, pr=PR, sha=SHA):
        archive = make_archive(tmp_path / "a.zip", entries)
        return fork.validate(archive, expected_pr=pr, expected_sha=sha)

    return go


def line(**overrides):
    base = {"message": "m", "location": {"path": "docs/a.md"}}
    return json.dumps({**base, **overrides}).encode() + b"\n"


def test_the_real_binary_report_is_accepted(check, good, valid_report):
    assert check(good)["findings.rdjsonl"] == valid_report


def test_an_empty_report_is_accepted(check, good):
    good["findings.rdjsonl"] = b""
    assert check(good)["findings.rdjsonl"] == b""


@pytest.mark.parametrize(
    "report",
    [
        line(severity="ERROR", source={"name": "slopvac"}, code={"value": "r"}),
        line(location={"path": "a.md", "range": {"start": {"line": 1, "column": 2}}}),
        line(
            suggestions=[
                {"range": {"start": {"line": 1}, "end": {"line": 1}}, "text": "x"}
            ]
        ),
    ],
)
def test_other_valid_diagnostic_shapes_are_accepted(check, good, report):
    good["findings.rdjsonl"] = report
    assert check(good)["findings.rdjsonl"] == report


def test_an_oversize_archive_is_rejected(check, good):
    # Incompressible data makes the archive itself exceed its cap.
    good["findings.rdjsonl"] = os.urandom(1_000_001)
    with pytest.raises(fork.Rejected, match="over the cap"):
        check(good)


def test_a_zip_bomb_is_rejected_by_its_declared_size(check, good):
    good["findings.rdjsonl"] = b"\n" * (2_000_001)
    with pytest.raises(fork.Rejected, match="over the cap"):
        check(good)


def test_an_oversize_metadata_file_is_rejected(check, good):
    good["pr-number"] = b"1" * (65)
    with pytest.raises(fork.Rejected, match="over the cap"):
        check(good)


def understate_size(archive: Path, name: str, size: int) -> None:
    """Rewrite the central directory's uncompressed size of one member."""
    raw = bytearray(archive.read_bytes())
    start = 0
    while (start := raw.find(b"PK\x01\x02", start)) != -1:
        name_len = int.from_bytes(raw[start + 28 : start + 30], "little")
        if bytes(raw[start + 46 : start + 46 + name_len]) == name.encode():
            raw[start + 24 : start + 28] = size.to_bytes(4, "little")
            archive.write_bytes(raw)
            return
        start += 4
    raise AssertionError(name)


def test_a_deflate_bomb_with_an_understated_header_is_rejected(tmp_path, good):
    # 3 MB of newlines deflates to a few KB, under the archive cap.
    good["findings.rdjsonl"] = b"\n" * 3_000_000
    archive = make_archive(tmp_path / "a.zip", good)
    assert archive.stat().st_size < 1_000_000
    understate_size(archive, "findings.rdjsonl", 10)
    with pytest.raises(fork.Rejected):
        fork.validate(archive, expected_pr=PR, expected_sha=SHA)


@pytest.mark.parametrize("method", [zipfile.ZIP_BZIP2, zipfile.ZIP_LZMA])
def test_unbounded_compression_methods_are_rejected_before_opening(
    tmp_path, good, method
):
    archive = make_archive(tmp_path / "a.zip", good, method)
    with pytest.raises(fork.Rejected, match="compression method"):
        fork.validate(archive, expected_pr=PR, expected_sha=SHA)


def test_stored_members_are_accepted(tmp_path, good):
    archive = make_archive(tmp_path / "a.zip", good, zipfile.ZIP_STORED)
    assert fork.validate(archive, expected_pr=PR, expected_sha=SHA)


@pytest.mark.parametrize(
    "name",
    ["../findings.rdjsonl", "/etc/passwd", "sub/findings.rdjsonl", "..\\x", "extra"],
)
def test_unexpected_or_traversing_entry_names_are_rejected(check, good, name):
    good[name] = b"x"
    with pytest.raises(fork.Rejected, match="entries"):
        check(good)


def test_a_traversal_name_in_place_of_a_real_entry_is_rejected(check, good):
    good["../pr-number"] = good.pop("pr-number")
    with pytest.raises(fork.Rejected, match="entries"):
        check(good)


def test_a_symlink_entry_is_rejected(tmp_path, good):
    out = tmp_path / "b.zip"
    with zipfile.ZipFile(out, "w") as zf:
        link = zipfile.ZipInfo("findings.rdjsonl")
        link.external_attr = 0o120777 << 16
        zf.writestr(link, "/etc/passwd")
        zf.writestr("pr-number", good["pr-number"])
        zf.writestr("head-sha", good["head-sha"])
    with pytest.raises(fork.Rejected, match="regular file"):
        fork.validate(out, expected_pr=PR, expected_sha=SHA)


def test_too_many_files_are_rejected(tmp_path, good):
    archive = make_archive(tmp_path / "a.zip", {**good, "x": b"", "y": b""})
    with pytest.raises(fork.Rejected, match="entries"):
        fork.validate(archive, expected_pr=PR, expected_sha=SHA)


def test_duplicate_entries_are_rejected(tmp_path, good):
    archive = tmp_path / "a.zip"
    with zipfile.ZipFile(archive, "w") as zf:
        for name, data in good.items():
            zf.writestr(name, data)
        with pytest.warns(UserWarning, match="Duplicate name"):
            zf.writestr("pr-number", good["pr-number"])
    with pytest.raises(fork.Rejected, match="entries|duplicate"):
        fork.validate(archive, expected_pr=PR, expected_sha=SHA)


def test_a_missing_entry_is_rejected(check, good):
    del good["head-sha"]
    with pytest.raises(fork.Rejected, match="entries"):
        check(good)


def test_a_file_that_is_not_a_zip_is_rejected(tmp_path):
    bad = tmp_path / "a.zip"
    bad.write_bytes(b"not a zip")
    with pytest.raises(fork.Rejected, match="not a zip"):
        fork.validate(bad, expected_pr=PR, expected_sha=SHA)


@pytest.mark.parametrize("claimed", ["41", "042", "42 43", "abc", ""])
def test_an_artifact_for_another_pull_request_is_rejected(check, good, claimed):
    good["pr-number"] = claimed.encode()
    with pytest.raises(fork.Rejected, match="different pull request"):
        check(good)


@pytest.mark.parametrize("claimed", [OTHER_SHA, SHA[:10], SHA.upper(), ""])
def test_an_artifact_for_another_head_commit_is_rejected(check, good, claimed):
    good["head-sha"] = claimed.encode()
    with pytest.raises(fork.Rejected, match="different commit"):
        check(good)


@pytest.mark.parametrize(
    ("kwargs", "match"),
    [
        ({"pr": "0"}, "expected PR"),
        ({"pr": "4; rm"}, "expected PR"),
        ({"sha": "abc"}, "expected head"),
    ],
)
def test_the_event_side_bindings_must_be_well_formed(check, good, kwargs, match):
    with pytest.raises(fork.Rejected, match=match):
        check(good, **kwargs)


@pytest.mark.parametrize(
    ("report", "match"),
    [
        (b"not json\n", "not valid JSON"),
        (b"[1]\n", "not an object"),
        (b'"text"\n', "not an object"),
        (b"\xff\xfe\n", "not UTF-8"),
        (b"{}\n", "message"),
        (line(message=""), "message"),
        (line(message=7), "message"),
        (b'{"message": "m"}\n', "no location"),
        (line(extra=1), "unknown fields"),
        (line(location={"path": "a", "extra": 1}), "unknown fields"),
        (line(location={"path": "/etc/passwd"}), "relative repository path"),
        (line(location={"path": "../x"}), "relative repository path"),
        (line(location={"path": "a\\..\\b"}), "relative repository path"),
        (line(location={"path": "a", "range": {"end": {"line": 1}}}), "no start"),
        (
            line(location={"path": "a", "range": {"start": {"line": 0}}}),
            "invalid",
        ),
        (
            line(location={"path": "a", "range": {"start": {"line": True}}}),
            "invalid",
        ),
        (line(severity="FATAL"), "severity"),
        (line(severity=["ERROR"]), "severity"),
        (
            line(
                location={
                    "path": "a",
                    "range": {"start": {"line": 5}, "end": {"line": 4}},
                }
            ),
            "ends before",
        ),
        (
            line(
                location={
                    "path": "a",
                    "range": {
                        "start": {"line": 3, "column": 9},
                        "end": {"line": 3, "column": 2},
                    },
                }
            ),
            "ends before",
        ),
        (
            line(location={"path": "a", "range": {"start": {"line": 10_000_001}}}),
            "invalid",
        ),
        (line(source={"name": "s", "x": 1}), "unknown fields"),
        (line(suggestions="x"), "suggestions"),
        (line(suggestions=[{"text": "x"}]), "range"),
        (b"[" * 100_000 + b"\n", "not valid JSON"),
    ],
)
def test_a_malformed_report_is_rejected(check, good, report, match):
    good["findings.rdjsonl"] = report
    with pytest.raises(fork.Rejected, match=match):
        check(good)


@pytest.mark.parametrize("separator", ["\u2028", "\u0085"])
def test_a_line_separator_inside_a_message_is_not_a_line_break(check, good, separator):
    # ensure_ascii=False puts the raw character in the bytes. Escaped, the
    # parser would never see it and str.splitlines() would pass this test.
    raw = json.dumps(
        {"message": f"one{separator}two", "location": {"path": "a.md"}},
        ensure_ascii=False,
    ).encode()
    assert separator.encode() in raw
    report = raw + b"\n" + line(message="x")
    good["findings.rdjsonl"] = report
    assert check(good)["findings.rdjsonl"] == report


def test_crlf_line_endings_are_accepted(check, good):
    good["findings.rdjsonl"] = line().replace(b"\n", b"\r\n") * 2
    assert check(good)


def test_too_many_diagnostics_are_rejected(check, good):
    good["findings.rdjsonl"] = line() * (2_001)
    with pytest.raises(fork.Rejected, match="more than"):
        check(good)


def test_the_diagnostic_cap_itself_is_accepted(check, good):
    good["findings.rdjsonl"] = line() * 2_000
    assert check(good)


def run_cli(tmp_path, archive, *, pr=PR, sha=SHA):
    out = tmp_path / "out"
    env = {"EXPECTED_PR": pr, "EXPECTED_SHA": sha}
    code = fork.main(["fork_review.py", "extract", str(archive), str(out)], env)
    return code, out


def test_extract_writes_only_the_report_under_a_fixed_name(tmp_path, good):
    archive = make_archive(tmp_path / "a.zip", good)
    code, out = run_cli(tmp_path, archive)
    assert code == 0
    assert [p.name for p in out.iterdir()] == ["findings.rdjsonl"]
    assert (out / "findings.rdjsonl").read_bytes() == good["findings.rdjsonl"]


def test_extract_refuses_an_existing_destination(tmp_path, good):
    archive = make_archive(tmp_path / "a.zip", good)
    (tmp_path / "out").mkdir()
    assert run_cli(tmp_path, archive)[0] == 1


def test_usage_error(capsys):
    assert fork.main(["fork_review.py"], {}) == 2


def test_the_documented_template_writes_the_files_the_consumer_requires():
    import re

    doc = (ROOT / "docs" / "ci" / "github-action.md").read_text(encoding="utf-8")
    template = next(
        b for b in re.findall(r"```yaml\n(.*?)```", doc, re.DOTALL) if "head-sha" in b
    )
    written = set(re.findall(r"> findings/([\w.-]+)", template))
    written.add(re.search(r"rdjsonl-file: findings/([\w.-]+)", template).group(1))
    assert written == fork.ENTRIES


def malicious_cases(good):
    """name -> (entries, expected PR, expected head, rejection reason)."""
    swapped = {k: v for k, v in good.items() if k != "pr-number"}
    return {
        "oversize archive": (
            {**good, "findings.rdjsonl": os.urandom(1_000_001)},
            PR,
            SHA,
            "over the cap",
        ),
        "path traversal": (
            {**good, "../../etc/cron.d/x": b"x"},
            PR,
            SHA,
            "archive has 4 entries",
        ),
        "traversal replaces entry": (
            {"../pr-number": good["pr-number"], **swapped},
            PR,
            SHA,
            "entries must be exactly",
        ),
        "wrong head": (good, PR, OTHER_SHA, "different commit"),
        "wrong pull request": (good, "43", SHA, "different pull request"),
    }


@pytest.mark.parametrize("case", list(malicious_cases({"pr-number": b"", "x": b""})))
def test_the_command_line_rejects_a_malicious_artifact(tmp_path, good, capsys, case):
    entries, pr, sha, reason = malicious_cases(good)[case]
    archive = make_archive(tmp_path / "a.zip", entries)
    code, out = run_cli(tmp_path, archive, pr=pr, sha=sha)
    printed = capsys.readouterr().out
    assert code == 1
    assert printed.startswith("::error title=fork review::")
    assert reason in printed
    assert not out.exists()
