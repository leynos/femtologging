"""Parse and trim captured stack output so snapshots stay stable.

``stack_info=True`` records capture the whole call stack, including pytest,
pluggy and interpreter frames. :func:`parse_frames` reports the frames a raw
output names, :func:`parse_frame` parses one frame for the normalizer, and
:func:`strip_external_stack_frames` removes the frames owned by installed code
before normalization, so a third-party release cannot change a snapshot.
"""

from __future__ import annotations

import dataclasses
import re
import sysconfig
import typing as typ
from pathlib import Path

if typ.TYPE_CHECKING:
    import collections.abc as cabc


_TRACEBACK_FRAME_PATTERN: re.Pattern[str] = re.compile(
    r'^(?P<indent>\s*)File "(?P<file>[^"]+)", line (?P<line>[^,]+), '
    r"in (?P<func>.+)$",
)
_STACK_HEADING: typ.Final[str] = "Stack (most recent call last):"
_TRACEBACK_HEADING: typ.Final[str] = "Traceback (most recent call last):"
_SECTION_HEADINGS: typ.Final[frozenset[str]] = frozenset({
    _STACK_HEADING,
    _TRACEBACK_HEADING,
})
_CARET_LINE_PATTERN: re.Pattern[str] = re.compile(r"^\s*[\^~]+$")
_FRAME_SOURCE_INDENT: typ.Final[str] = "    "
_FROZEN_FILENAME_PATTERN: re.Pattern[str] = re.compile(r"^<frozen\b[^>]*>$")
_EXTERNAL_ROOT_KEYS: typ.Final[tuple[str, ...]] = (
    "stdlib",
    "platstdlib",
    "purelib",
    "platlib",
    "scripts",
)


@dataclasses.dataclass(slots=True, frozen=True)
class FrameInfo:
    """Parsed traceback frame plus its optional first source line."""

    frame_line: str
    frame_file: str
    frame_func: str
    code_line: str
    next_index: int


def parse_frames(output: str) -> list[tuple[Path, str]]:
    r"""Return each frame's resolved file and function name, in order.

    Parameters
    ----------
    output : str
        Raw output containing traceback or stack frames.

    Returns
    -------
    list[tuple[pathlib.Path, str]]
        One ``(resolved file, function name)`` pair per frame line.

    Examples
    --------
    >>> parse_frames('  File "/repo/app.py", line 2, in main\n')
    [(PosixPath('/repo/app.py'), 'main')]

    """
    frames: list[tuple[Path, str]] = []
    for line in output.splitlines():
        match = _TRACEBACK_FRAME_PATTERN.match(line)
        if match is not None:
            frames.append((Path(match["file"]).resolve(), match["func"].strip()))
    return frames


def parse_frame(lines: list[str], index: int) -> FrameInfo | None:
    r"""Return parsed frame information when the line starts a traceback frame.

    Parameters
    ----------
    lines : list[str]
        The output split into lines, without line terminators.
    index : int
        The position of the line to parse.

    Returns
    -------
    FrameInfo | None
        The frame's header, filename, function name, first source line and the
        index just past that source line, or ``None`` when ``lines[index]`` is
        not a frame header.

    Examples
    --------
    >>> frame = parse_frame(
    ...     ['  File "/repo/app.py", line 2, in main', "    app.run()"], 0
    ... )
    >>> (frame.frame_file, frame.frame_func, frame.code_line, frame.next_index)
    ('/repo/app.py', 'main', 'app.run()', 2)
    >>> parse_frame(["plain text"], 0) is None
    True
    """
    frame_line = lines[index]
    frame_match = _TRACEBACK_FRAME_PATTERN.match(frame_line)
    if frame_match is None:
        return None

    code_line = ""
    next_index = index + 1
    if next_index < len(lines):
        candidate = lines[next_index]
        if (
            candidate.startswith(_FRAME_SOURCE_INDENT)
            and _TRACEBACK_FRAME_PATTERN.match(candidate) is None
        ):
            code_line = candidate.strip()
            next_index += 1

    return FrameInfo(
        frame_line=frame_line,
        frame_file=frame_match.group("file"),
        frame_func=frame_match.group("func"),
        code_line=code_line,
        next_index=next_index,
    )


def strip_external_stack_frames(
    output: str | None,
    external_roots: cabc.Iterable[Path] | None = None,
) -> str:
    r"""Remove installed and standard-library frames from a captured stack.

    A ``stack_info=True`` record captures the whole call stack, including the
    frames of pytest, pluggy and the interpreter. Normalizing paths and line
    numbers keeps their locations stable, but not their source text: an
    upstream release may wrap a call across lines and change the snapshot
    without any change to this repository. Dropping those frame blocks before
    normalization leaves a snapshot that records only the frames the project
    controls.

    Only the section between the ``Stack (most recent call last):`` heading
    and the next ``Traceback (most recent call last):`` heading or the end of
    the output is considered, so exception tracebacks are untouched. A frame
    is external when its filename is frozen or resolves beneath one of the
    interpreter's standard-library or site-package roots. Relative and
    unrecognized filenames are kept, because they cannot be attributed
    reliably.

    Stripping deletes whole lines and keeps every surviving line with the
    terminator it already had, so an output that uses ``\r\n`` is returned with
    ``\r\n`` intact and a traceback stays byte-identical.

    Parameters
    ----------
    output : str | None
        The raw logging output, or ``None``.
    external_roots : collections.abc.Iterable[pathlib.Path] | None
        Directories whose frames are removed. Defaults to the ``stdlib``,
        ``platstdlib``, ``purelib``, ``platlib`` and ``scripts`` entries of
        :func:`sysconfig.get_paths`. Supplied roots are resolved, because a
        frame's filename is resolved before it is compared.

    Returns
    -------
    str
        The output with external frame blocks removed, or an empty string
        when ``output`` is ``None``.

    Examples
    --------
    >>> raw = (
    ...     "Stack (most recent call last):\n"
    ...     '  File "/usr/lib/python3.12/runpy.py", line 1, in run\n'
    ...     "    run_code()\n"
    ...     '  File "/repo/app.py", line 2, in main\n'
    ...     "    app.run()\n"
    ... )
    >>> kept = strip_external_stack_frames(
    ...     raw, external_roots=(Path("/usr/lib/python3.12"),)
    ... )
    >>> "runpy.py" not in kept and "/repo/app.py" in kept
    True

    """
    if output is None:
        return ""
    roots = (
        tuple(Path(root).resolve() for root in external_roots)
        if external_roots is not None
        else _external_roots_from_sysconfig()
    )
    lines, endings = _split_lines(output)
    return _rejoin_lines(lines, endings, _strip_frames(lines, roots))


def _split_lines(output: str) -> tuple[list[str], list[str]]:
    r"""Split ``output`` into line texts and the terminator each line ended with.

    Splitting without terminators and rejoining with ``\n`` would rewrite every
    ``\r\n`` in the output, including the exception traceback that
    :func:`strip_external_stack_frames` promises to leave untouched. Keeping
    each terminator alongside its text means the rebuilt output differs from
    the original only by the removed frame blocks.

    Parameters
    ----------
    output : str
        The raw logging output.

    Returns
    -------
    tuple[list[str], list[str]]
        The line texts, and in the same order the terminator each line ended
        with. A final line without a newline has an empty terminator.

    Examples
    --------
    >>> _split_lines("a\r\nb")
    (['a', 'b'], ['\r\n', ''])
    """
    bodies: list[str] = []
    endings: list[str] = []
    for raw_line in output.splitlines(keepends=True):
        body = raw_line.splitlines()[0]
        bodies.append(body)
        endings.append(raw_line[len(body) :])
    return bodies, endings


def _rejoin_lines(lines: list[str], endings: list[str], kept: list[int]) -> str:
    """Rebuild the output from the kept line indices and their terminators."""
    return "".join(f"{lines[index]}{endings[index]}" for index in kept)


def _strip_frames(lines: list[str], roots: cabc.Sequence[Path]) -> list[int]:
    """Return the indices of lines kept after external frames are removed."""
    kept: list[int] = []
    index = 0
    in_stack_section = False

    while index < len(lines):
        heading = lines[index].strip()
        if heading in _SECTION_HEADINGS:
            # A heading always ends the previous section and starts its own,
            # so a second stack heading inside one section is still a heading.
            in_stack_section = heading == _STACK_HEADING
            kept.append(index)
            index += 1
        elif not in_stack_section:
            kept.append(index)
            index += 1
        else:
            index = _keep_or_drop_frame(lines, index, roots, kept)
    return kept


def _keep_or_drop_frame(
    lines: list[str],
    index: int,
    roots: cabc.Sequence[Path],
    kept: list[int],
) -> int:
    """Append one stack frame's block to ``kept`` unless it is external.

    Returns
    -------
    int
        The index just past the frame block, whether kept or dropped.
    """
    frame = parse_frame(lines, index)
    if frame is None:
        kept.append(index)
        return index + 1
    block_end = frame_block_end(lines, frame)
    if not _is_external_frame_file(frame.frame_file, roots):
        kept.extend(range(index, block_end))
    return block_end


def _external_roots_from_sysconfig() -> tuple[Path, ...]:
    """Return this interpreter's resolved standard-library and package roots."""
    scheme = sysconfig.get_paths()
    return tuple(
        Path(scheme[key]).resolve() for key in _EXTERNAL_ROOT_KEYS if scheme.get(key)
    )


def _is_external_frame_file(frame_file: str, roots: cabc.Sequence[Path]) -> bool:
    """Return whether a frame's filename is frozen or beneath an external root."""
    if _FROZEN_FILENAME_PATTERN.match(frame_file):
        return True
    candidate = Path(frame_file)
    if not candidate.is_absolute():
        return False
    resolved = candidate.resolve()
    return any(resolved.is_relative_to(root) for root in roots)


def frame_block_end(lines: list[str], frame: FrameInfo) -> int:
    """Return the index just past a frame's header and all its source lines.

    Python 3.11 and later print every line of a multi-line statement, followed
    by caret lines under the failing expression. A frame's block is therefore
    more than the single source line :func:`parse_frame` records; stopping at
    the first source line would leave the rest of a removed frame's body behind.

    Parameters
    ----------
    lines : list[str]
        The output split into lines, without line terminators.
    frame : FrameInfo
        The parsed frame whose block is measured.

    Returns
    -------
    int
        The index just past the frame's last source or caret line.
    """
    end = frame.next_index
    while end < len(lines) and _is_frame_source_line(lines[end]):
        end += 1
    return end


def _is_frame_source_line(line: str) -> bool:
    """Return whether a line is indented as frame source or a caret marker."""
    if _CARET_LINE_PATTERN.match(line) is not None:
        return True
    if not line.startswith(_FRAME_SOURCE_INDENT):
        return False
    return _TRACEBACK_FRAME_PATTERN.match(line) is None
