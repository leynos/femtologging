"""Load marked Python examples from published documentation."""

from __future__ import annotations

import re
import typing as typ

if typ.TYPE_CHECKING:
    from pathlib import Path

_IDENTIFIER_PATTERN = re.compile(r"[a-z0-9]+(?:-[a-z0-9]+)*")
# The source may not contain a closing-fence line, so the match cannot run past
# the first fence to a later one. The closing fence must then carry nothing but
# optional same-line whitespace: a language label or other text after it means
# the fence was a language-tagged opening fence, not the end of this example.
_PYTHON_FENCE_PATTERN = re.compile(
    r"\r?\n```python\r?\n"
    r"(?P<source>(?:(?!\r?\n```).)*?)"
    r"\r?\n```[ \t]*(?:\r?\n|\Z)",
    re.DOTALL,
)


def load_tested_python_example(document_path: Path, identifier: str) -> str:
    """Load the uniquely marked Python fence from *document_path*.

    Parameters
    ----------
    document_path : Path
        Published Markdown document containing the example.
    identifier : str
        Stable identifier from its ``tested-example`` marker.

    Returns
    -------
    str
        The source inside the marked Python fence.

    Raises
    ------
    ValueError
        If the identifier is invalid, missing, duplicated, or not followed
        immediately by a Python code fence.
    """
    if _IDENTIFIER_PATTERN.fullmatch(identifier) is None:
        message = f"Invalid tested-example identifier: {identifier!r}"
        raise ValueError(message)

    marker = f"<!-- tested-example: {identifier} -->"
    document = document_path.read_text(encoding="utf-8")
    if document.count(marker) != 1:
        message = (
            f"Expected one {marker!r} marker in {document_path}, "
            f"found {document.count(marker)}"
        )
        raise ValueError(message)

    marker_end = document.index(marker) + len(marker)
    match = _PYTHON_FENCE_PATTERN.match(document[marker_end:])
    if match is None:
        message = f"{marker!r} must immediately precede a Python fence"
        raise ValueError(message)

    return match.group("source")
