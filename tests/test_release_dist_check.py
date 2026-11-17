"""check-dist binds an upload to the project and version inside each file."""

from __future__ import annotations

import importlib.util
import io
import subprocess
import sys
import tarfile
import zipfile
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "release_provenance.py"
SPEC = importlib.util.spec_from_file_location("release_provenance_dist", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
prov = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = prov
SPEC.loader.exec_module(prov)

TARGETS = ["macosx_11_0_arm64", "manylinux_2_17_x86_64"]


def metadata(name: str, version: str) -> bytes:
    return f"Metadata-Version: 2.4\nName: {name}\nVersion: {version}\n\nbody\n".encode()


def write_wheel(directory: Path, version: str, tag: str, name="slopvac", inner=None):
    inner = inner or version
    path = directory / f"slopvac-{version}-py3-none-{tag}.whl"
    with zipfile.ZipFile(path, "w") as wheel:
        wheel.writestr(f"{name}-{inner}.dist-info/METADATA", metadata(name, inner))
    return path


def write_sdist(directory: Path, version: str, name="slopvac", inner=None):
    inner = inner or version
    path = directory / f"slopvac-{version}.tar.gz"
    data = metadata(name, inner)
    info = tarfile.TarInfo(f"slopvac-{version}/PKG-INFO")
    info.size = len(data)
    with tarfile.open(path, "w:gz") as sdist:
        sdist.addfile(info, io.BytesIO(data))
    return path


@pytest.fixture
def dist(tmp_path) -> Path:
    directory = tmp_path / "dist"
    directory.mkdir()
    return directory


def refused(directory: Path, version: str, **kwargs) -> str:
    with pytest.raises(ValueError) as caught:
        prov.check_dist(directory, version, **kwargs)
    return str(caught.value)


def test_the_exact_python_set_passes(dist):
    write_sdist(dist, "2.19.0")
    write_wheel(dist, "2.19.0", "any")
    prov.check_dist(dist, "2.19.0", sdist=True, wheels=1)


def test_the_exact_wheel_set_passes(dist):
    for tag in TARGETS:
        write_wheel(dist, "3.0.0rc2", tag)
    prov.check_dist(dist, "3.0.0rc2", wheels=2)


def test_legitimate_2x_files_plus_an_extra_3x_sdist_are_refused(dist):
    write_sdist(dist, "2.19.0")
    write_wheel(dist, "2.19.0", "any")
    write_sdist(dist, "3.0.0")
    message = refused(dist, "2.19.0", sdist=True, wheels=1)
    assert "slopvac-3.0.0.tar.gz: unexpected file" in message


def test_a_3x_sdist_named_like_the_2x_one_is_refused_by_its_metadata(dist):
    write_sdist(dist, "2.19.0", inner="3.0.0")
    write_wheel(dist, "2.19.0", "any")
    assert "version is '3.0.0', not '2.19.0'" in refused(
        dist, "2.19.0", sdist=True, wheels=1
    )


def test_a_wheel_of_another_project_is_refused(dist):
    write_wheel(dist, "3.0.0", "any", name="other-project")
    assert "project is 'other-project', not 'slopvac'" in refused(
        dist, "3.0.0", wheels=1
    )


def test_a_wheel_whose_metadata_names_another_version_is_refused(dist):
    write_wheel(dist, "3.0.0", "any", inner="3.0.1")
    assert "version is '3.0.1', not '3.0.0'" in refused(dist, "3.0.0", wheels=1)


@pytest.mark.parametrize("name", ["Slopvac", "slopvac", "SLOPVAC"])
def test_the_project_name_is_compared_after_pep_503_normalisation(dist, name):
    write_wheel(dist, "3.0.0", "any", name=name)
    prov.check_dist(dist, "3.0.0", wheels=1)


@pytest.mark.parametrize(
    "extra",
    [
        "notes.txt",
        "slopvac-3.0.0.zip",
        "slopvac-3.0.0.tar.gz.asc",
        "slopvac-3.0.1-py3-none-any.whl",
        "other-3.0.0-py3-none-any.whl",
    ],
)
def test_any_other_file_in_the_directory_is_refused(dist, extra):
    write_wheel(dist, "3.0.0", "any")
    (dist / extra).write_bytes(b"x")
    assert f"{extra}: unexpected file" in refused(dist, "3.0.0", wheels=1)


def test_a_subdirectory_is_refused(dist):
    write_wheel(dist, "3.0.0", "any")
    (dist / "nested").mkdir()
    assert "nested: not a regular file" in refused(dist, "3.0.0", wheels=1)


@pytest.mark.parametrize(
    ("sdist", "wheels", "present", "message"),
    [
        (True, 1, "wheel", "expected one sdist, found 0"),
        (False, 1, "both", "expected no sdist, found 1"),
        (False, 2, "wheel", "expected 2 wheels, found 1"),
        (False, None, "none", "no wheels found"),
    ],
)
def test_a_wrong_number_of_files_is_refused(dist, sdist, wheels, present, message):
    if present in ("wheel", "both"):
        write_wheel(dist, "3.0.0", "any")
    if present == "both":
        write_sdist(dist, "3.0.0")
    assert message in refused(dist, "3.0.0", sdist=sdist, wheels=wheels)


def test_a_wheel_that_is_not_a_zip_is_refused(dist):
    (dist / "slopvac-3.0.0-py3-none-any.whl").write_bytes(b"not a zip")
    assert "unreadable wheel" in refused(dist, "3.0.0", wheels=1)


def test_a_wheel_with_two_metadata_files_is_refused(dist):
    path = write_wheel(dist, "3.0.0", "any")
    with zipfile.ZipFile(path, "a") as wheel:
        wheel.writestr("evil-9.dist-info/METADATA", metadata("slopvac", "3.0.0"))
    assert "expected one dist-info/METADATA, found 2" in refused(
        dist, "3.0.0", wheels=1
    )


def test_a_missing_directory_is_refused(tmp_path):
    assert "no distribution directory" in refused(tmp_path / "gone", "3.0.0")


def test_the_command_line_exits_zero_for_the_exact_set_and_one_for_a_stray(dist):
    write_wheel(dist, "3.0.0", "any")
    argv = [sys.executable, str(SCRIPT), "check-dist", str(dist), "--version", "3.0.0"]
    ok = subprocess.run(argv, capture_output=True, text=True, check=False)
    assert ok.returncode == 0, ok.stderr
    (dist / "stray.txt").write_text("x")
    bad = subprocess.run(argv, capture_output=True, text=True, check=False)
    assert bad.returncode == 1
    assert "stray.txt: unexpected file" in bad.stderr
