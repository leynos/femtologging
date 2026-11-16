"""Property tests for the documented-example fence loader.

``tests/test_documentation_examples_loader.py`` pins the CommonMark fence rules
with named and parameterized cases. Those cases sample two rules that hold over
a wide input space, so they are stated here as properties instead: a closing
fence ends a block exactly when its run of the fence character is at least as
long as the opening run and its indentation is at most three columns, and a
body reaches the reader exactly as published.
"""

from __future__ import annotations

import pytest
from hypothesis import given, settings
from hypothesis import strategies as st

from tests.documentation_examples import DocumentationExampleError, parse_document

# Both CommonMark fence characters behave alike, so the properties run against
# either rather than assuming backticks.
_FENCE_CHARACTERS = st.sampled_from("`~")

# A body may hold any character a document can hold except the two that fence
# it and the three CommonMark line endings that end its line.
_BODY_TEXT = st.text(
    alphabet=st.characters(blacklist_characters="`~\r\n"),
    max_size=40,
)


def _marked_document(opening: str, body: str, closing: str) -> str:
    """Return a document holding one marked example built from the parts.

    Parameters
    ----------
    opening
        The complete opening line, info string included.
    body
        The example body, line endings included.
    closing
        The closing line.

    Returns
    -------
    str
        A Markdown document whose single example holds ``body`` exactly.
    """
    return f"<!-- tested-example: sample -->\n{opening}\n{body}{closing}"


@settings(max_examples=100, deadline=None)
@given(
    opening_length=st.integers(min_value=3, max_value=10),
    closing_length=st.integers(min_value=1, max_value=10),
    fence_character=_FENCE_CHARACTERS,
)
def test_a_closer_ends_a_block_exactly_when_it_is_not_shorter(
    opening_length: int, closing_length: int, fence_character: str
) -> None:
    """A run of the fence character closes a block iff it is at least as long."""
    document = _marked_document(
        f"{fence_character * opening_length}text",
        "body\n",
        f"{fence_character * closing_length}\n",
    )
    if closing_length >= opening_length:
        [example] = parse_document("doc.md", document)
        assert example.body == "body\n", (
            "a closer at least as long as the opener must end the example"
        )
    else:
        with pytest.raises(DocumentationExampleError, match="not terminated"):
            parse_document("doc.md", document)


@settings(max_examples=100, deadline=None)
@given(
    indent=st.integers(min_value=0, max_value=3),
    trailing=st.from_regex(r"[ \t]{0,3}", fullmatch=True),
)
def test_a_closer_closes_when_indented_by_at_most_three_columns(
    indent: int, trailing: str
) -> None:
    """A closer inside the indentation allowance ends the example."""
    document = _marked_document("```text", "body\n", f"{' ' * indent}```{trailing}\n")
    [example] = parse_document("doc.md", document)
    assert example.body == "body\n", (
        f"a closer indented by {indent} column(s) and followed by {trailing!r}"
        " must end the example"
    )


@settings(max_examples=100, deadline=None)
@given(indent=st.integers(min_value=4, max_value=10))
def test_a_closer_indented_past_three_columns_is_content(indent: int) -> None:
    """Four or more columns of indent make the fence line part of the body."""
    quoted = f"{' ' * indent}```"
    document = _marked_document("```text", f"{quoted}\n", "```\n")
    [example] = parse_document("doc.md", document)
    assert example.body == f"{quoted}\n", (
        f"a fence line indented by {indent} column(s) must stay in the example"
    )


@settings(max_examples=100, deadline=None)
@given(body=_BODY_TEXT)
def test_a_body_reaches_the_reader_exactly_as_published(body: str) -> None:
    """Only Markdown line endings split lines, so a body survives unchanged."""
    document = _marked_document("```text", f"{body}\n", "```\n")
    [example] = parse_document("doc.md", document)
    assert example.body == f"{body}\n", (
        "the example body must reach the reader byte-for-byte"
    )
