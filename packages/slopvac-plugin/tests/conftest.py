from __future__ import annotations

import os
import signal
import sys
import threading
from collections.abc import Callable, Iterator
from contextlib import suppress
from pathlib import Path

import pytest
from slopvac_plugin.conformance.schemas import Schemas
from slopvac_plugin.conformance.session import Session

SRC = Path(__file__).parents[1] / "src"
SDK_ENV = ("PYTHONPATH",)
FIXTURES = Path(__file__).parent / "fixtures"
EXAMPLE = Path(__file__).parents[1] / "examples" / "fence-tagger"


def script(name: str, *args: str) -> list[str]:
    return [sys.executable, str(FIXTURES / name), *args]


def bounded(fn: Callable[[], object], seconds: float) -> bool:
    """Run ``fn`` on a helper thread. False when it is still running after ``seconds``.

    A hang then fails one test instead of the whole run.
    """
    thread = threading.Thread(target=fn, daemon=True)
    thread.start()
    thread.join(seconds)
    return not thread.is_alive()


@pytest.fixture(autouse=True)
def sdk_on_pythonpath(monkeypatch: pytest.MonkeyPatch) -> None:
    """Plugins under test import the SDK from the source tree, installed or not.

    The session gives a plugin PATH and LANG only, so tests name PYTHONPATH in
    ``pass_env`` when the plugin needs it.
    """
    existing = os.environ.get("PYTHONPATH")
    monkeypatch.setenv(
        "PYTHONPATH", os.pathsep.join([str(SRC), *([existing] if existing else [])])
    )


@pytest.fixture(scope="session")
def schemas() -> Schemas:
    return Schemas()


@pytest.fixture
def start(schemas: Schemas) -> Iterator[Callable[..., Session]]:
    """Spawn a plugin command; every session is closed at the end of the test."""
    sessions: list[Session] = []

    def spawn(command: list[str]) -> Session:
        sessions.append(Session(command, schemas, timeout=10, env_names=SDK_ENV))
        return sessions[-1]

    yield spawn
    for session in sessions:
        session.close()


@pytest.fixture
def quiet_cwd(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    """Run the suite from an empty directory, so P17's file snapshot is stable."""
    monkeypatch.chdir(tmp_path)
    return tmp_path


@pytest.fixture
def escape_pidfile(
    tmp_path_factory: pytest.TempPathFactory, monkeypatch: pytest.MonkeyPatch
) -> Iterator[Path]:
    """Where raw_plugin.py records descendants that left its process group.

    The suite cannot end them, so the test does.
    """
    path = tmp_path_factory.mktemp("escaped") / "pids"
    path.touch()
    monkeypatch.setenv("ESCAPE_PIDFILE", str(path))
    yield path
    for pid in path.read_text("utf-8").split():
        with suppress(ProcessLookupError):
            os.kill(int(pid), signal.SIGKILL)
