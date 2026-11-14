"""Check executable Python examples from the published user guide."""

from __future__ import annotations

import runpy
from pathlib import Path

import pytest

from tests.documented_examples import load_tested_python_example

_USER_GUIDE = Path(__file__).resolve().parents[1] / "docs" / "users-guide.md"

_MARKER = "<!-- tested-example: sample -->"


def _write_document(tmp_path: Path, body: str) -> Path:
    """Write a Markdown document whose only content is *body*."""
    document = tmp_path / "document.md"
    document.write_text(body, encoding="utf-8")
    return document


def test_example_source_is_captured_verbatim(tmp_path: Path) -> None:
    """The loader returns the fenced source without the surrounding fences."""
    document = _write_document(
        tmp_path,
        f"{_MARKER}\n```python\nprint('one')\nprint('two')\n```\n",
    )

    assert load_tested_python_example(document, "sample") == (
        "print('one')\nprint('two')"
    ), "the loader must return the source between the fences unchanged"


@pytest.mark.parametrize(
    "closing_fence",
    [
        pytest.param("```python", id="language-label-after-fence"),
        pytest.param("``` trailing text", id="prose-after-fence"),
    ],
)
def test_closing_fence_must_not_carry_a_language_label(
    tmp_path: Path, closing_fence: str
) -> None:
    """A fence followed by non-whitespace is an opening fence, not a closer.

    Without this rule the loader would run past the real closing fence and
    capture the following block's opening fence as part of the example.
    """
    document = _write_document(
        tmp_path,
        f"{_MARKER}\n```python\nprint('one')\n{closing_fence}\nprint('two')\n```\n",
    )

    with pytest.raises(ValueError, match="must immediately precede a Python fence"):
        load_tested_python_example(document, "sample")


@pytest.mark.parametrize(
    "trailer",
    [
        pytest.param("", id="no-trailer"),
        pytest.param("   ", id="trailing-spaces"),
        pytest.param("\t", id="trailing-tab"),
    ],
)
def test_closing_fence_allows_trailing_whitespace(tmp_path: Path, trailer: str) -> None:
    """Whitespace after the closing fence does not end the example early."""
    document = _write_document(
        tmp_path,
        f"{_MARKER}\n```python\nprint('one')\n```{trailer}\n",
    )

    assert load_tested_python_example(document, "sample") == "print('one')", (
        f"whitespace after the closing fence ({trailer!r}) must not end the example"
    )


def test_closing_fence_may_end_the_file(tmp_path: Path) -> None:
    """A document ending immediately after the closing fence is accepted."""
    document = _write_document(tmp_path, f"{_MARKER}\n```python\nprint('one')\n```")

    assert load_tested_python_example(document, "sample") == "print('one')", (
        "a closing fence at end of file must still end the example"
    )


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
