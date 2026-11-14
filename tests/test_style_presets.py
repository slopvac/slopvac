"""The bundled STE preset must match the 2.x normal profile it replaces."""

from __future__ import annotations

import tomllib
from pathlib import Path

from slopvac.profiles import _NORMAL

PRESET = (
    Path(__file__).resolve().parents[1]
    / "crates"
    / "slopvac-styles"
    / "presets"
    / "ste.toml"
)


def test_ste_preset_reproduces_normal_profile_ste_categories() -> None:
    preset = tomllib.loads(PRESET.read_text())["categories"]
    expected = {k: v for k, v in _NORMAL.items() if k.startswith("ste-")}
    assert set(preset) == set(expected)
    for name, settings in expected.items():
        got = preset[name]
        if isinstance(got, str):
            assert (got, None, None) == (
                settings.severity.value,
                settings.max_per_100_words,
                settings.weight,
            ), name
            continue
        assert got["severity"] == settings.severity.value, name
        assert got["max_per_100_words"] == settings.max_per_100_words, name
        assert got["weight"] == settings.weight, name
