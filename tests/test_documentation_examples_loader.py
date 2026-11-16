"""Contracts for the documented-example fence loader."""

from __future__ import annotations

import re

import pytest

from tests import documentation_examples
from tests.documentation_examples import (
    DocumentationExampleError,
    documented_example,
    load_documented_examples,
    parse_document,
    unmarked_fences,
)


def test_examples_keep_their_order_and_exact_text() -> None:
    """Each marked fence, either fence style, yields its exact body."""
    text = (
        "<!-- tested-example: first -->\n```python\nassert 1\n```\n"
        "Prose between examples.\n"
        "<!-- tested-example: second -->\n~~~text\nshown\n~~~\n"
    )
    examples = parse_document("doc.md", text)
    assert [(example.id, example.line) for example in examples] == [
        ("first", 2),
        ("second", 7),
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


def test_a_fence_line_with_an_info_string_stays_in_the_body() -> None:
    """Only a bare fence closes an example, as CommonMark requires.

    A line such as "```python" opens a fence; read as a closer, it would end
    the example early and leave its remaining lines outside it.
    """
    text = (
        "<!-- tested-example: nested -->\n```markdown\n```python\nprint('two')\n```\n"
    )
    [example] = parse_document("doc.md", text)
    assert example.body == "```python\nprint('two')\n", (
        "the inner opening fence and its code should stay in the body"
    )


@pytest.mark.parametrize(
    "closing",
    [
        pytest.param("```\n", id="bare"),
        pytest.param("```   \n", id="trailing-spaces"),
        pytest.param("```\t\n", id="trailing-tab"),
        pytest.param(" ```\n", id="one-space-indent"),
        pytest.param("   ```\n", id="three-space-indent"),
        pytest.param("```", id="end-of-file"),
    ],
)
def test_closing_fence_allows_trailing_whitespace(closing: str) -> None:
    """Whitespace after a closing fence, or the end of the file, still closes it."""
    text = f"<!-- tested-example: sample -->\n```python\nprint('one')\n{closing}"
    [example] = parse_document("doc.md", text)
    assert example.body == "print('one')\n", (
        f"the closing fence {closing!r} must end the example"
    )


def test_a_closing_fence_indented_four_spaces_stays_in_the_body() -> None:
    """Four spaces make the line content, not a closer, as CommonMark requires."""
    text = "<!-- tested-example: indented -->\n```markdown\n    ```\n```\n"
    [example] = parse_document("doc.md", text)
    assert example.body == "    ```\n", (
        "a four-space-indented fence line must stay inside the example"
    )


def test_unicode_separators_stay_inside_an_example_line() -> None:
    """Only Markdown line endings split lines; NEL and LINE SEPARATOR are content."""
    # U+0085 NEXT LINE and U+2028 LINE SEPARATOR are characters, not Markdown
    # line endings, so splitting on them would rewrite the published example.
    body = "first\u2028second\u0085third\n"
    text = f"<!-- tested-example: unicode -->\n```text\n{body}```\n"
    [example] = parse_document("doc.md", text)
    assert example.body == body, "the example body must survive byte-for-byte"


def test_prose_without_fences_yields_nothing() -> None:
    """A document with no fences has no examples and no errors."""
    assert not parse_document("doc.md", "# Title\n\nJust prose.\n"), (
        "prose alone should yield no examples"
    )


@pytest.mark.parametrize(
    ("text", "expected"),
    [
        pytest.param(
            "```python\nassert True\n```\n",
            ["doc.md:1: fence is missing a tested-example marker"],
            id="unmarked-fence",
        ),
        pytest.param(
            "~~~bash\nls\n~~~\n",
            ["doc.md:1: fence is missing a tested-example marker"],
            id="unmarked-tilde-fence",
        ),
        pytest.param(
            "- item\n\n  ```python\n  assert True\n  ```\n",
            ["doc.md:3: fence must start at column 0"],
            id="indented-fence",
        ),
        pytest.param(
            "<!-- tested-example: marked -->\n```python\nassert 1\n```\n",
            [],
            id="marked-fence",
        ),
    ],
)
def test_unmarked_fences_are_reported(text: str, expected: list[str]) -> None:
    """Every fence without a marker is named with its location."""
    assert unmarked_fences("doc.md", text) == expected, (
        "each unmarked fence should be reported once, at its opening line"
    )


def test_a_marker_inside_an_unmarked_fence_is_not_read() -> None:
    """An unmarked block is skipped whole, so its contents cannot pose as markup."""
    text = (
        "````markdown\n"
        "<!-- tested-example: quoted -->\n"
        "```python\nassert 1\n```\n"
        "````\n"
    )
    assert not parse_document("doc.md", text), (
        "a marker quoted inside another fence must not become an example"
    )
    assert unmarked_fences("doc.md", text) == [
        "doc.md:1: fence is missing a tested-example marker"
    ], "only the outer fence should be reported"


@pytest.mark.parametrize(
    ("text", "diagnostic"),
    [
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
            "<!-- tested-example: gap -->\n\n```python\nassert 1\n```\n",
            "doc.md:1: marker is not followed by a fence",
            id="marker-separated-by-blank-line",
        ),
        pytest.param(
            "<!-- tested-example: indented -->\n  ```python\n  assert 1\n  ```\n",
            "doc.md:1: marker is not followed by a fence",
            id="marker-above-indented-fence",
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
            "Intro.\n```python\nassert 1\n",
            "doc.md:2: fence is not terminated",
            id="unterminated-unmarked-fence",
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


@pytest.mark.parametrize(
    "indent",
    [
        pytest.param("\t", id="tab"),
        pytest.param(" \t", id="space-tab"),
        pytest.param("  \t", id="two-spaces-tab"),
        pytest.param("   \t", id="three-spaces-tab"),
    ],
)
def test_a_tab_indented_fence_line_stays_in_the_body(indent: str) -> None:
    """A tab reaches the next four-column stop, so the line is never a closer.

    Counting a tab as a single column would keep these lines within the
    three-column limit and let them close the example early, cutting the body
    short. The example's own trailing fence still closes it.
    """
    text = f"<!-- tested-example: tabbed -->\n```markdown\n{indent}```\n```\n"
    [example] = parse_document("doc.md", text)

    assert example.body == f"{indent}```\n", (
        "a tab-indented fence line must stay inside the example"
    )


def test_load_rejects_an_identifier_shared_by_two_documents(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """An identifier reused across documents is refused, not merely in one file."""
    documents = {
        "first.md": "<!-- tested-example: shared -->\n```python\nassert 1\n```\n",
        "second.md": "<!-- tested-example: shared -->\n```python\nassert 2\n```\n",
    }
    monkeypatch.setattr(documentation_examples, "DOCUMENTS", tuple(documents))
    monkeypatch.setattr(documentation_examples, "read_document", documents.__getitem__)

    with pytest.raises(
        DocumentationExampleError,
        match="duplicate tested-example identifier 'shared'",
    ):
        load_documented_examples()


def test_an_unknown_identifier_raises_key_error(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """A lookup finds a declared identifier and names one that no document has."""
    documents = {
        "only.md": "<!-- tested-example: known -->\n```python\nassert 1\n```\n",
    }
    monkeypatch.setattr(documentation_examples, "DOCUMENTS", tuple(documents))
    monkeypatch.setattr(documentation_examples, "read_document", documents.__getitem__)

    assert documented_example("known").id == "known", (
        "the declared identifier should be found"
    )
    with pytest.raises(KeyError, match="unknown"):
        documented_example("unknown")
