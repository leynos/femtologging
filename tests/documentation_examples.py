r"""Load fenced examples from femtologging's user-facing Markdown.

Every fence in ``README.md``, ``docs/users-guide.md``, and
``docs/v0-2-0-migration-guide.md`` must sit directly beneath a
``<!-- tested-example: <id> -->`` marker. The documentation tests load the
published text through this module rather than keeping copied fixtures, so an
example that drifts from the library fails the suite instead of misleading a
reader.

Examples
--------
>>> examples = parse_document(
...     "doc.md",
...     "<!-- tested-example: hello -->\n\n```python\nassert True\n```\n",
... )
>>> [(example.id, example.language, example.body) for example in examples]
[('hello', 'python', 'assert True\n')]
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
class _Fence:
    """The opening line of a fenced code block."""

    delimiter: str
    info: str

    @classmethod
    def parse(cls, line: str) -> _Fence | None:
        """Return the fence ``line`` opens, or ``None`` when it opens none."""
        match = FENCE.fullmatch(line.rstrip())
        if match is None:
            return None
        return cls(match["delimiter"], match["info"].strip())

    def is_closed_by(self, line: str) -> bool:
        """Report whether ``line`` closes this fence."""
        # A fence closes only on a run of its own character at least as long
        # as the opening run, so a longer fence can quote a shorter one.
        candidate = line.rstrip()
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
    """Return the fence opened after the marker at ``index``."""
    # Only blank lines may separate a marker from its fence.
    for candidate in range(index + 1, len(lines)):
        if not lines[candidate].strip():
            continue
        fence = _Fence.parse(lines[candidate])
        if fence is None:
            raise DocumentationExampleError(
                _located(source, index, "marker is not followed by a fence")
            )
        if not fence.info:
            raise DocumentationExampleError(
                _located(source, candidate, "fence should declare a language")
            )
        return candidate, fence
    raise DocumentationExampleError(_located(source, index, "marker has no fence"))


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


def parse_document(source: str, text: str) -> list[DocumentedExample]:
    """Return every marked example in ``text``, rejecting unmarked fences.

    Parameters
    ----------
    source
        The document's repository-relative path, used in diagnostics.
    text
        The document's Markdown text.

    Returns
    -------
    list[DocumentedExample]
        The examples in document order.

    Raises
    ------
    DocumentationExampleError
        When a marker is malformed or orphaned, a fence is unmarked,
        unterminated, or has no language, or an identifier repeats.
    """
    lines = text.splitlines()
    examples: list[DocumentedExample] = []
    index = 0
    while index < len(lines):
        identifier = _read_marker(source, lines, index)
        if identifier is None:
            line = lines[index]
            if line[:1].isspace() and _Fence.parse(line.lstrip()) is not None:
                # An indented fence would carry its indentation into the
                # executed body, so every example starts at column 0.
                raise DocumentationExampleError(
                    _located(source, index, "fence must start at column 0")
                )
            if _Fence.parse(line) is not None:
                raise DocumentationExampleError(
                    _located(source, index, "fence is missing a tested-example marker")
                )
            index += 1
            continue
        opening, fence = _next_fence(source, lines, index)
        index, body = _read_body(source, lines, opening, fence)
        examples.append(
            DocumentedExample(identifier, fence.info, body, source, opening + 1)
        )
    _reject_duplicates(examples)
    return examples


def _reject_duplicates(examples: list[DocumentedExample]) -> None:
    """Raise when two examples share an identifier."""
    seen: set[str] = set()
    for example in examples:
        if example.id in seen:
            msg = f"duplicate tested-example identifier {example.id!r}"
            raise DocumentationExampleError(msg)
        seen.add(example.id)


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
        text = (REPOSITORY_ROOT / path).read_text(encoding="utf-8")
        examples.extend(parse_document(path, text))
    _reject_duplicates(examples)
    return examples


def documented_example(identifier: str) -> DocumentedExample:
    """Return the published example named ``identifier``.

    Returns
    -------
    DocumentedExample
        The first example whose marker declares ``identifier``.

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
