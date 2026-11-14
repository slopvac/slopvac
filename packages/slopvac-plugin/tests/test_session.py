"""The conformance session treats the plugin under test as hostile."""

import os
import signal
import subprocess
import sys
import threading
import time
from contextlib import suppress
from pathlib import Path

import pytest
from conftest import FIXTURES, bounded, script
from slopvac_plugin.conformance.schemas import Schemas
from slopvac_plugin.conformance.session import (
    CaseFailure,
    Session,
    plugin_environment,
    taskkill_command,
)

POSIX_ONLY = pytest.mark.skipif(
    os.name == "nt", reason="process groups are POSIX; Windows uses taskkill"
)


def hostile(*args: str) -> list[str]:
    return [sys.executable, str(FIXTURES / "hostile_plugin.py"), *args]


def wait_for(predicate, seconds: float = 30) -> bool:
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if predicate():
            return True
        time.sleep(0.05)
    return False


# Environment ----------------------------------------------------------------


def test_environment_keeps_the_allowlist_and_named_extras_only():
    base = {
        "PATH": "/bin",
        "LANG": "C",
        "GITHUB_TOKEN": "secret",
        "HOME": "/home/x",
        "OPTED_IN": "yes",
        "NOT_SET_HERE": "no",
    }
    got = plugin_environment(["OPTED_IN", "MISSING"], base)
    expected = {"PATH": "/bin", "LANG": "C", "OPTED_IN": "yes"}
    assert got == expected


@pytest.mark.parametrize("name", ["", "A=B"])
def test_environment_rejects_a_malformed_extra_name(name):
    with pytest.raises(ValueError, match="environment variable name"):
        plugin_environment([name], {})


def test_a_started_plugin_does_not_see_ci_secrets(
    schemas: Schemas, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    monkeypatch.setenv("CI_DEPLOY_TOKEN", "hunter2")
    monkeypatch.setenv("OPTED_IN", "yes")
    dump = tmp_path / "env.txt"
    s = Session(hostile("env-probe", str(dump)), schemas, 10, env_names=["OPTED_IN"])
    try:
        assert wait_for(dump.exists)
        assert s.wait(10) == 0
    finally:
        s.close()
    names = set(dump.read_text("utf-8").split())
    assert "CI_DEPLOY_TOKEN" not in names
    assert "OPTED_IN" in names
    assert "PATH" in names


def test_taskkill_runs_from_system32_and_ends_the_tree():
    assert taskkill_command(4242, r"C:\Windows") == [
        str(Path(r"C:\Windows") / "System32" / "taskkill.exe"),
        "/PID",
        "4242",
        "/T",
        "/F",
    ]


# Output bounds --------------------------------------------------------------


def test_a_flood_without_newlines_on_stderr_ends_the_plugin(schemas: Schemas):
    s = Session(hostile("stderr-flood"), schemas, 10, max_output_bytes=1024 * 1024)
    try:
        events = []
        assert bounded(lambda: events.extend(s.drain_events(20)), 40)
        kind, message = events[-1]
        assert kind == "violation"
        assert "more than" in message
        assert wait_for(lambda: s.proc.poll() is not None)
        assert len(s.stderr_tail()) <= 400
    finally:
        s.close()


def test_unread_frames_over_the_count_limit_end_the_plugin(schemas: Schemas):
    s = Session(hostile("frame-flood", "300"), schemas, 10, max_queued_messages=100)
    try:
        # Read nothing until the session has reacted, so the queue really fills.
        assert wait_for(lambda: s.proc.poll() is not None)
        events = s.drain_events(5)
        assert events[-1][0] == "violation"
        assert "unread messages" in events[-1][1]
        assert len(events) <= 101
    finally:
        s.close()


def test_unread_frames_over_the_byte_limit_end_the_plugin(schemas: Schemas):
    s = Session(hostile("big-frames", "10"), schemas, 10, max_queued_bytes=256 * 1024)
    try:
        assert wait_for(lambda: s.proc.poll() is not None)
        events = s.drain_events(5)
        assert events[-1][0] == "violation"
        assert "unread messages" in events[-1][1]
    finally:
        s.close()


def test_total_output_over_the_cap_ends_the_plugin_even_when_read(schemas: Schemas):
    s = Session(hostile("big-frames", "60"), schemas, 10, max_output_bytes=1024 * 1024)
    try:
        last = []

        def consume() -> None:
            while True:
                kind, value = s.next_event(20)
                if kind != "frame":
                    last.append((kind, value))
                    return

        assert bounded(consume, 40)
        assert last[0][0] == "violation"
        assert "more than" in last[0][1]
    finally:
        s.close()


# Deadlines ------------------------------------------------------------------


def test_a_write_to_a_plugin_that_never_reads_fails_within_the_timeout(
    schemas: Schemas,
):
    s = Session(hostile("deaf"), schemas, 1)
    try:
        outcome = []

        def write() -> None:
            try:
                s.send_bytes(b"x" * (16 * 1024 * 1024))
            except CaseFailure as exc:
                outcome.append(str(exc))

        assert bounded(write, 30), "the write blocked past its deadline"
        assert "did not read" in outcome[0]
        assert wait_for(lambda: s.proc.poll() is not None)  # the plugin was ended
    finally:
        s.close()


# Process groups -------------------------------------------------------------


def alive(pid: int) -> bool:
    """True while ``pid`` runs. A zombie that nobody reaps counts as ended."""
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    state = subprocess.run(
        ["ps", "-o", "stat=", "-p", str(pid)],
        check=False,
        capture_output=True,
        text=True,
    ).stdout.strip()
    return bool(state) and not state.startswith("Z")


@POSIX_ONLY
@pytest.mark.parametrize("mode", ["spawner", "spawner-exit"])
def test_close_ends_a_descendant_that_holds_the_pipes(
    schemas: Schemas, tmp_path: Path, mode: str
):
    pidfile = tmp_path / "child.pid"
    s = Session(hostile(mode, str(pidfile)), schemas, 10)
    child = None
    try:
        assert wait_for(lambda: pidfile.exists() and pidfile.read_text("utf-8") != "")
        child = int(pidfile.read_text("utf-8"))
        assert alive(child)
        assert bounded(s.close, 30), "close() hung on a descendant"
        assert wait_for(lambda: not alive(child), 10), "the descendant survived"
    finally:
        if child is not None and alive(child):
            os.kill(child, signal.SIGKILL)
        s.close()


WINDOWS_ONLY = pytest.mark.skipif(os.name != "nt", reason="Job Objects are Windows")


def windows_alive(pid: int) -> bool:
    """True while the Windows process ``pid`` has not exited."""
    import ctypes
    from ctypes import wintypes

    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    kernel32.OpenProcess.restype = wintypes.HANDLE
    kernel32.GetExitCodeProcess.argtypes = [
        wintypes.HANDLE,
        ctypes.POINTER(wintypes.DWORD),
    ]
    kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
    handle = kernel32.OpenProcess(
        0x1000, False, pid
    )  # PROCESS_QUERY_LIMITED_INFORMATION
    if not handle:
        return False  # no such process
    try:
        code = wintypes.DWORD()
        kernel32.GetExitCodeProcess(handle, ctypes.byref(code))
        return code.value == 259  # STILL_ACTIVE
    finally:
        kernel32.CloseHandle(handle)


@WINDOWS_ONLY
@pytest.mark.parametrize("mode", ["spawner", "spawner-exit"])
def test_windows_close_ends_a_descendant_after_its_parent_exited(
    schemas: Schemas, tmp_path: Path, mode: str
):
    """taskkill /T cannot find the child of a dead parent. The job still holds it."""
    pidfile = tmp_path / "child.pid"
    s = Session(hostile(mode, str(pidfile)), schemas, 10)
    child = None
    try:
        assert wait_for(lambda: pidfile.exists() and pidfile.read_text("utf-8") != "")
        child = int(pidfile.read_text("utf-8"))
        if mode == "spawner-exit":
            assert s.proc.wait(timeout=20) is not None
        assert windows_alive(child)
        assert bounded(s.close, 30), "close() hung on a descendant"
        assert wait_for(lambda: not windows_alive(child), 10), "the descendant survived"
    finally:
        s.close()


@pytest.mark.skipif(os.name == "nt", reason="the job exists on Windows")
def test_no_job_object_off_windows(schemas: Schemas):
    s = Session(script("raw_plugin.py"), schemas, 10)
    try:
        assert s._job is None
    finally:
        s.close()


@POSIX_ONLY
def test_the_plugin_leads_its_own_process_group(schemas: Schemas):
    s = Session(script("raw_plugin.py"), schemas, 10)
    try:
        assert os.getpgid(s.proc.pid) == s.proc.pid
        assert os.getpgid(s.proc.pid) != os.getpgrp()
    finally:
        s.close()


def test_a_cancel_write_that_blocks_uses_up_the_grace(schemas: Schemas):
    """Section 6.3: the grace covers the cancel write too, not only the wait."""
    s = Session(hostile("deaf"), schemas, 30)
    try:
        assert s.proc.stdin is not None
        # Fill the pipe so the cancel notification cannot be written.
        stdin = s.proc.stdin

        def fill() -> None:
            with suppress(OSError, ValueError):
                stdin.write(b"x" * (8 * 1024 * 1024))

        filler = threading.Thread(target=fill, daemon=True)
        filler.start()
        time.sleep(0.5)
        started = time.monotonic()
        with pytest.raises(CaseFailure, match="did not read its input within 1"):
            s.cancel_and_wait(1, 1)
        assert time.monotonic() - started < 10  # not the 30 s session timeout
    finally:
        s.close()


def test_the_schema_integer_check_refuses_a_value_beyond_2_53_minus_1(schemas: Schemas):
    from slopvac_plugin.framing import UnsafeInt, loads_strict

    reply = {
        "protocolVersion": 1,
        "pluginInfo": {"name": "p", "version": "0"},
        "capabilities": {
            "hooks": [],
            "features": {"required": [], "optional": []},
            "requiresNetwork": False,
        },
        "limits": {"maxMessageBytes": 65536, "maxBatchDocuments": 1, "maxInFlight": 1},
    }
    assert schemas.errors("InitializeResult", reply) == []
    # The strict parser turns a token beyond 2^53-1 into an UnsafeInt, an int subclass.
    parsed = loads_strict(b'{"n": 9007199254740993}')["n"]
    assert isinstance(parsed, UnsafeInt)
    reply["limits"]["maxBatchDocuments"] = parsed
    assert schemas.errors("InitializeResult", reply) != []
