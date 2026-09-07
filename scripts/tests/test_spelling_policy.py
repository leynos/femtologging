"""Regression coverage for spelling visibility inside authored code examples."""

from __future__ import annotations

import importlib
import re
import tomllib
import typing as typ
from pathlib import Path

if typ.TYPE_CHECKING:
    import pytest


def test_generator_keeps_authored_code_visible(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Shared broad code exemptions cannot hide native names in this repository."""
    monkeypatch.syspath_prepend(str(Path(__file__).resolve().parents[1]))
    generator = importlib.import_module("generate_typos_config")
    cache = tmp_path / ".typos-oxendict-base.toml"
    cache.write_text(
        'schema = 1\n[patterns]\nignore = ["(?s)```.*?```", '
        "'`[^`\\n]+`', 'rust-analyzer']\n",
        encoding="utf-8",
    )

    config = generator.render_config(tmp_path)
    patterns = tomllib.loads(config)["default"]["extend-ignore-re"]
    for example in ("`native_identifier`", "```rust\nnative_identifier\n```"):
        assert not any(re.search(pattern, example) for pattern in patterns)
    assert any(re.search(pattern, "rust-analyzer") for pattern in patterns)
