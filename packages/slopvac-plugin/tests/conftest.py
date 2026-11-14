from __future__ import annotations

import sys
from collections.abc import Callable, Iterator
from pathlib import Path

import pytest
from slopvac_plugin.conformance.schemas import Schemas
from slopvac_plugin.conformance.session import Session

FIXTURES = Path(__file__).parent / "fixtures"
EXAMPLE = Path(__file__).parents[1] / "examples" / "fence-tagger"


def script(name: str, *args: str) -> list[str]:
    return [sys.executable, str(FIXTURES / name), *args]


@pytest.fixture(scope="session")
def schemas() -> Schemas:
    return Schemas()


@pytest.fixture
def start(schemas: Schemas) -> Iterator[Callable[..., Session]]:
    """Spawn a plugin command; every session is closed at the end of the test."""
    sessions: list[Session] = []

    def spawn(command: list[str]) -> Session:
        sessions.append(Session(command, schemas, timeout=10))
        return sessions[-1]

    yield spawn
    for session in sessions:
        session.close()


@pytest.fixture
def quiet_cwd(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    """Run the suite from an empty directory, so P17's file snapshot is stable."""
    monkeypatch.chdir(tmp_path)
    return tmp_path
