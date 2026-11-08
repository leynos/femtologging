"""Cargo target directories stay out of the `interrogate` gate.

Build output (maturin artefacts, generated stubs, trybuild scratch crates) lives
under `target/` and is not owned by the repository. These tests run the real
tool through `make interrogate` and prove that such output is skipped while a
file that merely resembles it is still measured.
"""

from __future__ import annotations

from pathlib import Path

import pytest

from tests.test_interrogate_target import (
    _assignment,
    _make_interrogate,
    _run_real_tool,
)


def test_cargo_target_directories_are_not_measured(tmp_path: Path) -> None:
    """Undocumented build output under a root or nested `target/` is skipped."""
    result = _run_real_tool(
        tmp_path,
        definitions=5,
        undocumented=0,
        build_output=("target/stub.py", "crate/target/debug/stub.py"),
    )
    assert result.returncode == 0, result.stdout + result.stderr
    assert "stub.py" not in result.stdout, result.stdout


@pytest.mark.parametrize("name", ["[out]", "my build"])
def test_a_custom_cargo_target_directory_is_not_measured(
    tmp_path: Path, name: str
) -> None:
    """`CARGO_TARGET_DIR` is excluded; metacharacters and spaces stay literal."""
    build = tmp_path / "generated/build" / name
    build.mkdir(parents=True)
    (build / "stub.py").write_text("def generated() -> None:\n    pass\n")
    (tmp_path / "generated/mod.py").write_text('"""Documented."""\n')
    result = _make_interrogate(
        _assignment("INTERROGATE_TARGETS", tmp_path / "generated"),
        f"CARGO_TARGET_DIR={build}",
        workdir=tmp_path / "generated",
    )
    assert result.returncode == 0, result.stdout + result.stderr
    assert "stub.py" not in result.stdout, result.stdout


@pytest.mark.parametrize(
    "owned",
    ["target_utils.py", "targets/helper.py", "crate/target_utils.py"],
)
def test_excluding_target_directories_does_not_hide_owned_files(
    tmp_path: Path, owned: str
) -> None:
    """A file whose name only begins with `target` is still measured."""
    result = _run_real_tool(
        tmp_path, definitions=5, undocumented=0, build_output=("target/stub.py", owned)
    )
    assert result.returncode != 0, result.stdout + result.stderr
    assert Path(owned).name in result.stdout, result.stdout
    assert "stub.py" not in result.stdout, result.stdout


def test_excluding_target_directories_keeps_the_gate_strict(tmp_path: Path) -> None:
    """An undocumented owned file still fails when build output is present."""
    result = _run_real_tool(
        tmp_path, definitions=5, undocumented=1, build_output=("target/stub.py",)
    )
    assert result.returncode != 0, result.stdout + result.stderr
    assert "MISSED" in result.stdout, result.stdout
