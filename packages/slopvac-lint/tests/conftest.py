"""Test-wide isolation.

THE SUITE MUST NOT WRITE TO THE USER'S CACHE. Several tests call
`compile_ruleset` without an `outdir`, which is the real code path and worth
exercising, and that path writes to `cache_root()`. Most of those calls also pass
`validate=False`, because they assert something about the fingerprint rather than
about Vale, and an unvalidated compile writes every rule INCLUDING the ones Vale
refuses to load.

The result was a poisoned cache entry in the developer's own `~/.cache/slopvac`:
a real `slopvac` run whose config happened to fingerprint the same way found a
manifest, trusted it, and handed Vale a style tree containing an emoji rule Go's
regex engine rejects. Vale answers E201 and then lints NOTHING while exiting 0,
so the file reads clean and the score silently drops to the native rules alone.

Redirecting the cache per test session fixes it at the source. The compiler also
now refuses to publish an unvalidated tree to the shared cache, so this fixture
is the belt to that suspenders -- either alone would have prevented it.

`load_ruleset` writes to the same root, and module-scoped fixtures call it before
any function-scoped fixture runs, so the session gets its own root as well.
"""

from __future__ import annotations

import pytest

# The Vale pin guard: its hooks skip or stop runs that select `vale`-marked tests.
from vale_pin_guard import (
    pytest_collection_modifyitems,
    pytest_configure,
)

__all__ = ["pytest_collection_modifyitems", "pytest_configure"]


@pytest.fixture(autouse=True, scope="session")
def _isolated_session_cache(tmp_path_factory):
    with pytest.MonkeyPatch.context() as patch:
        patch.setenv(
            "SLOPVAC_CACHE_DIR", str(tmp_path_factory.mktemp("slopvac-session-cache"))
        )
        yield


@pytest.fixture(autouse=True)
def _isolated_cache(tmp_path_factory, monkeypatch):
    monkeypatch.setenv("SLOPVAC_CACHE_DIR", str(tmp_path_factory.mktemp("slopvac-cache")))


@pytest.fixture(autouse=True)
def _outside_the_repository(tmp_path_factory, monkeypatch):
    """Start every test outside the repository checkout.

    The repository's own `slopvac.toml` is a 3.x file with `schema_version`, which this
    2.x reference package rejects. The 2.x CLI discovers a config by walking up from the
    working directory, so a test that never writes a config of its own would otherwise
    read the repository root file. A test that needs another directory calls
    `monkeypatch.chdir` itself.
    """
    monkeypatch.chdir(tmp_path_factory.mktemp("cwd"))
