"""Check executable Python examples from the published user guide."""

from __future__ import annotations

import runpy
import typing as typ
from pathlib import Path

if typ.TYPE_CHECKING:
    import pytest

from tests.documented_examples import load_tested_python_example

_USER_GUIDE = Path(__file__).resolve().parents[1] / "docs" / "users-guide.md"


def test_user_guide_flush_example_writes_before_closing(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The published flush example drains its attached file handler."""
    example_source = load_tested_python_example(_USER_GUIDE, "flush-handlers-usage")
    example_path = tmp_path / "flush_handlers_example.py"
    example_path.write_text(example_source, encoding="utf-8")
    monkeypatch.chdir(tmp_path)

    runpy.run_path(str(example_path))

    output = (tmp_path / "service.log").read_text(encoding="utf-8")
    assert "shutting down" in output, "the published example did not flush its record"
