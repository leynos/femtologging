"""Failure contracts for the documented-example fence loader."""

from __future__ import annotations

import re

import pytest

from tests.documentation_examples import (
    DocumentationExampleError,
    parse_document,
)


def test_examples_keep_their_order_and_exact_text() -> None:
    """Each marked fence, either fence style, yields its exact body."""
    text = (
        "<!-- tested-example: first -->\n\n```python\nassert 1\n```\n"
        "Prose between examples.\n"
        "<!-- tested-example: second -->\n~~~text\nshown\n~~~\n"
    )
    examples = parse_document("doc.md", text)
    assert [(example.id, example.line) for example in examples] == [
        ("first", 3),
        ("second", 8),
    ], "examples must keep their order and opening line"
    assert [(example.language, example.body) for example in examples] == [
        ("python", "assert 1\n"),
        ("text", "shown\n"),
    ], "examples must keep their language and exact text"


def test_a_longer_fence_can_quote_a_shorter_one() -> None:
    """A four-backtick fence stays open across the three-backtick line."""
    text = "<!-- tested-example: quoted -->\n````markdown\n```\n````\n"
    [example] = parse_document("doc.md", text)
    assert example.body == "```\n", "the quoted fence should stay in the body"


def test_prose_without_fences_yields_nothing() -> None:
    """A document with no fences has no examples and no errors."""
    assert not parse_document("doc.md", "# Title\n\nJust prose.\n"), (
        "prose alone should yield no examples"
    )


@pytest.mark.parametrize(
    ("text", "diagnostic"),
    [
        pytest.param(
            "```python\nassert True\n```\n",
            "doc.md:1: fence is missing a tested-example marker",
            id="unmarked-fence",
        ),
        pytest.param(
            "~~~bash\nls\n~~~\n",
            "doc.md:1: fence is missing a tested-example marker",
            id="unmarked-tilde-fence",
        ),
        pytest.param(
            "- item\n\n  ```python\n  assert True\n  ```\n",
            "doc.md:3: fence must start at column 0",
            id="indented-fence",
        ),
        pytest.param(
            "<!-- tested-example: -->\n```python\nassert 1\n```\n",
            "doc.md:1: malformed tested-example marker",
            id="empty-identifier",
        ),
        pytest.param(
            "<!-- tested-example: Bad_Name -->\n```python\nassert 1\n```\n",
            "doc.md:1: invalid tested-example identifier 'Bad_Name'",
            id="invalid-identifier",
        ),
        pytest.param(
            "<!-- tested-example: gap -->\nProse.\n```python\nassert 1\n```\n",
            "doc.md:1: marker is not followed by a fence",
            id="marker-separated-by-text",
        ),
        pytest.param(
            "Intro.\n<!-- tested-example: orphan -->\n",
            "doc.md:2: marker has no fence",
            id="marker-without-fence",
        ),
        pytest.param(
            "<!-- tested-example: bare -->\n```\nassert 1\n```\n",
            "doc.md:2: fence should declare a language",
            id="fence-without-language",
        ),
        pytest.param(
            "<!-- tested-example: open -->\n```python\nassert 1\n",
            "doc.md:2: fence is not terminated",
            id="unterminated-fence",
        ),
        pytest.param(
            (
                "<!-- tested-example: twice -->\n```python\nassert 1\n```\n"
                "<!-- tested-example: twice -->\n```python\nassert 2\n```\n"
            ),
            "duplicate tested-example identifier 'twice'",
            id="duplicate-identifier",
        ),
    ],
)
def test_malformed_documents_report_their_location(text: str, diagnostic: str) -> None:
    """Every contract breach names the document and the offending line."""
    with pytest.raises(DocumentationExampleError, match=re.escape(diagnostic)):
        parse_document("doc.md", text)
