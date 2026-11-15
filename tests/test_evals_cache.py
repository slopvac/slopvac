"""Offline tests for the fetch cache and the twin record lifecycle."""

from __future__ import annotations

import copy
import threading
from concurrent.futures import ThreadPoolExecutor

from evals import corpuslib as lib
from evals import twins


def test_concurrent_cold_cache_downloads_share_one_entry(tmp_path):
    calls = []
    gate = threading.Barrier(8)

    def getter(url, token):
        calls.append(url)
        gate.wait(timeout=10)  # hold every thread inside the miss path
        return b"payload"

    url = "https://example.invalid/file.json"
    with ThreadPoolExecutor(8) as pool:
        results = list(
            pool.map(lambda _: lib._cached_get(url, None, tmp_path, getter), range(8))
        )
    assert results == [b"payload"] * 8
    assert len(calls) == 8  # every thread raced past the miss
    assert [p for p in tmp_path.iterdir() if p.suffix == ".tmp"] == []
    assert len(list(tmp_path.iterdir())) == 1


def test_twin_record_lifecycle_yields_a_valid_manifest(tmp_path):
    manifest = copy.deepcopy(lib.load_manifest())
    slot = twins.slots(manifest)[0]
    cache = tmp_path / "cache"
    (cache / "twins").mkdir(parents=True)
    (cache / "twins" / f"{slot['id']}.txt").write_text("A generated twin.\n", "utf-8")
    out = tmp_path / "manifest.json"

    assert (
        twins.cmd_record(manifest, cache, "some-model", today="2026-11-14", out=out)
        == 0
    )

    written = lib.load_manifest(out)
    recorded = next(d for d in written["documents"] if d["id"] == slot["id"])
    assert recorded["status"] == "generated"
    assert recorded["date"] == "2026-11-14"
    assert recorded["sha256"] == lib.sha256_text("A generated twin.\n")
    assert recorded["source"]["model"] == "some-model"
    assert lib.validate_manifest(written) == []
