r"""Load fenced examples from femtologging's user-facing Markdown.

``docs/documentation-style-guide.md`` asks for every example in a user-facing
document to be tested as published. Each one sits directly beneath a
``<!-- tested-example: <id> -->`` marker, and the tests load the published
text through this module rather than keeping copied fixtures, so an example
that drifts from the library fails the suite instead of misleading a reader.

``scan_document()`` enforces the marker contract: a malformed marker, a marker
with no fence directly beneath it, a fence without a language, an unterminated
fence, or a repeated identifier raises ``DocumentationExampleError``. A fence
with no marker is not an error here. It is reported by ``unmarked_fences()``,
so a test can require every fence to be marked once a document has been
brought up to that standard.

Examples
--------
>>> examples = parse_document(
...     "doc.md",
...     "<!-- tested-example: hello -->\n```python\nassert True\n```\n",
... )
>>> [(example.id, example.language, example.body) for example in examples]
[('hello', 'python', 'assert True\n')]
>>> unmarked_fences("doc.md", "Intro.\n```bash\nls\n```\n")
['doc.md:2: fence is missing a tested-example marker']
"""

from __future__ import annotations

import dataclasses
import re
import subprocess  # ruff: ignore[suspicious-subprocess-import] examples run in the current interpreter
import sys
from pathlib import Path

REPOSITORY_ROOT = Path(__file__).resolve().parents[1]
DOCUMENTS = (
    "README.md",
    "docs/users-guide.md",
    "docs/v0-2-0-migration-guide.md",
)
MARKER = re.compile(r"<!-- tested-example: (?P<id>[^ ]*) -->")
MARKER_LIKE = re.compile(r"<!-- tested-example:")
# Both CommonMark fence characters open a block, so neither may hide an
# example from marker validation.
FENCE = re.compile(r"(?P<delimiter>`{3,}|~{3,})(?P<info>.*)")
VALID_ID = re.compile(r"[a-z0-9]+(?:-[a-z0-9]+)*")


class DocumentationExampleError(ValueError):
    """Raised when a document's fences do not follow the marker contract."""


@dataclasses.dataclass(frozen=True, slots=True)
class DocumentedExample:
    """One marked fenced example loaded from a user-facing document."""

    id: str
    language: str
    body: str
    source: str
    line: int


@dataclasses.dataclass(frozen=True, slots=True)
class DocumentScan:
    """The marked examples and the unmarked fences found in one document."""

    examples: tuple[DocumentedExample, ...]
    unmarked: tuple[str, ...]


@dataclasses.dataclass(frozen=True, slots=True)
class _Fence:
    """The opening line of a fenced code block."""

    delimiter: str
    info: str

    @classmethod
    def parse(cls, line: str) -> _Fence | None:
        """Return the fence ``line`` opens, or ``None`` when it opens none."""
        match = FENCE.fullmatch(line.strip())
        if match is None:
            return None
        return cls(match["delimiter"], match["info"].strip())

    def is_closed_by(self, line: str) -> bool:
        """Report whether ``line`` closes this fence."""
        # A fence closes only on a run of its own character at least as long
        # as the opening run, with nothing after it but whitespace. A line
        # such as "```python" therefore stays in the body, as CommonMark
        # requires, and a longer fence can quote a shorter one.
        candidate = line.strip()
        return len(candidate) >= len(self.delimiter) and set(candidate) == {
            self.delimiter[0]
        }


def _located(source: str, index: int, message: str) -> str:
    """Prefix ``message`` with the document and one-based line number."""
    return f"{source}:{index + 1}: {message}"


def _read_marker(source: str, lines: list[str], index: int) -> str | None:
    """Return the identifier declared on ``lines[index]``, if any."""
    line = lines[index]
    match = MARKER.fullmatch(line.strip())
    if match is None:
        if MARKER_LIKE.match(line.strip()):
            raise DocumentationExampleError(
                _located(source, index, "malformed tested-example marker")
            )
        return None
    identifier = match["id"]
    if VALID_ID.fullmatch(identifier) is None:
        raise DocumentationExampleError(
            _located(source, index, f"invalid tested-example identifier {identifier!r}")
        )
    return identifier


def _next_fence(source: str, lines: list[str], index: int) -> tuple[int, _Fence]:
    """Return the fence opened on the line after the marker at ``index``."""
    # The style guide places each marker immediately above its fence, and the
    # Rust loader in build.rs enforces the same rule, so even a blank line
    # between them is a breach rather than a layout choice.
    candidate = index + 1
    if candidate >= len(lines):
        raise DocumentationExampleError(_located(source, index, "marker has no fence"))
    line = lines[candidate]
    fence = _Fence.parse(line)
    if fence is None or line[:1].isspace():
        raise DocumentationExampleError(
            _located(source, index, "marker is not followed by a fence")
        )
    if not fence.info:
        raise DocumentationExampleError(
            _located(source, candidate, "fence should declare a language")
        )
    return candidate, fence


def _read_body(
    source: str, lines: list[str], opening: int, fence: _Fence
) -> tuple[int, str]:
    """Return the index after the closing fence and the exact fence body."""
    for candidate in range(opening + 1, len(lines)):
        if fence.is_closed_by(lines[candidate]):
            body = "".join(f"{line}\n" for line in lines[opening + 1 : candidate])
            return candidate + 1, body
    raise DocumentationExampleError(
        _located(source, opening, "fence is not terminated")
    )


def _unmarked_reason(line: str) -> str:
    """Explain why the unmarked fence opened by ``line`` is not an example."""
    # An indented fence would carry its indentation into the executed body,
    # so a fence must start at column 0 before it can be marked at all.
    if line[:1].isspace():
        return "fence must start at column 0"
    return "fence is missing a tested-example marker"


def scan_document(source: str, text: str) -> DocumentScan:
    """Return the marked examples and unmarked fences in ``text``.

    Parameters
    ----------
    source
        The document's repository-relative path, used in diagnostics.
    text
        The document's Markdown text.

    A malformed marker, a marker with no fence directly beneath it, a marked
    fence with no language, an unterminated fence, or a repeated identifier
    raises ``DocumentationExampleError``.

    Returns
    -------
    DocumentScan
        The examples in document order, and one located diagnostic for each
        fence that has no marker.
    """
    lines = text.splitlines()
    examples: list[DocumentedExample] = []
    unmarked: list[str] = []
    index = 0
    while index < len(lines):
        identifier = _read_marker(source, lines, index)
        if identifier is not None:
            opening, fence = _next_fence(source, lines, index)
            index, body = _read_body(source, lines, opening, fence)
            examples.append(
                DocumentedExample(identifier, fence.info, body, source, opening + 1)
            )
            continue
        fence = _Fence.parse(lines[index])
        if fence is None:
            index += 1
            continue
        # Skip the whole unmarked block, so text inside it that looks like a
        # marker or a fence is never read as one.
        unmarked.append(_located(source, index, _unmarked_reason(lines[index])))
        index, _ = _read_body(source, lines, index, fence)
    _reject_duplicates(examples)
    return DocumentScan(tuple(examples), tuple(unmarked))


def parse_document(source: str, text: str) -> list[DocumentedExample]:
    """Return every marked example in ``text``, in document order.

    A contract breach raises ``DocumentationExampleError``, as it does in
    ``scan_document()``.

    Returns
    -------
    list[DocumentedExample]
        The marked examples.
    """
    return list(scan_document(source, text).examples)


def unmarked_fences(source: str, text: str) -> list[str]:
    """Return a located diagnostic for every fence in ``text`` with no marker.

    A contract breach raises ``DocumentationExampleError``, as it does in
    ``scan_document()``.

    Returns
    -------
    list[str]
        One ``document:line: reason`` entry per unmarked fence.
    """
    return list(scan_document(source, text).unmarked)


def _reject_duplicates(examples: list[DocumentedExample]) -> None:
    """Raise when two examples share an identifier."""
    seen: set[str] = set()
    for example in examples:
        if example.id in seen:
            msg = f"duplicate tested-example identifier {example.id!r}"
            raise DocumentationExampleError(msg)
        seen.add(example.id)


def read_document(path: str) -> str:
    """Return the text of the published document at repository path ``path``."""
    return (REPOSITORY_ROOT / path).read_text(encoding="utf-8")


def load_documented_examples() -> list[DocumentedExample]:
    """Load every marked example across the published documents.

    A document that breaks the marker contract, or an identifier shared by
    two documents, raises ``DocumentationExampleError``.

    Returns
    -------
    list[DocumentedExample]
        The examples of each document in ``DOCUMENTS`` order.
    """
    examples: list[DocumentedExample] = []
    for path in DOCUMENTS:
        examples.extend(parse_document(path, read_document(path)))
    _reject_duplicates(examples)
    return examples


def documented_example(identifier: str) -> DocumentedExample:
    """Return the published example named ``identifier``.

    Returns
    -------
    DocumentedExample
        The example whose marker declares ``identifier``.

    Raises
    ------
    KeyError
        When no document declares ``identifier``.
    """
    for example in load_documented_examples():
        if example.id == identifier:
            return example
    raise KeyError(identifier)


def run_example(
    example: DocumentedExample, cwd: Path
) -> subprocess.CompletedProcess[str]:
    """Run a published Python example in a fresh interpreter under ``cwd``.

    A fresh interpreter starts with an empty logger registry, as a reader's
    would, and ``cwd`` keeps any files the example writes out of the
    repository.

    Returns
    -------
    subprocess.CompletedProcess[str]
        The finished interpreter, with its captured output.
    """
    return subprocess.run(  # ruff: ignore[subprocess-without-shell-equals-true] runs the current interpreter on published example text
        [sys.executable, "-c", example.body],
        capture_output=True,
        check=False,
        cwd=cwd,
        text=True,
        timeout=25,
    )
