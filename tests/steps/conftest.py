"""Shared BDD steps reused across feature modules."""

from __future__ import annotations

import dataclasses
import enum
import re
import sysconfig
import typing as typ
from pathlib import Path, PurePath

from pytest_bdd import given, parsers, then

from femtologging import get_logger, reset_manager

if typ.TYPE_CHECKING:
    import collections.abc as cabc

    from syrupy.assertion import SnapshotAssertion


_PYTEST_RUNTEST_HOOK_LINE_PATTERN: re.Pattern[str] = re.compile(
    r"^(?P<indent>\s*)(?P<prefix>(?:lambda:\s*)?runtest_hook)\(.*\),.*$",
    flags=re.MULTILINE,
)
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
_FROZEN_FILENAME_PATTERN: re.Pattern[str] = re.compile(r"^<frozen\b[^>]*>$")
_EXTERNAL_ROOT_KEYS: typ.Final[tuple[str, ...]] = (
    "stdlib",
    "platstdlib",
    "purelib",
    "platlib",
    "scripts",
)


class LauncherFrameFunc(enum.StrEnum):
    """Function names emitted by Python launcher traceback frames."""

    RUN_MODULE_AS_MAIN = "_run_module_as_main"
    RUN_CODE = "_run_code"
    RUN_MODULE = "run_module"
    RUN_MODULE_CODE = "_run_module_code"


_LAUNCHER_FRAME_FUNCS: frozenset[str] = frozenset(
    member.value for member in LauncherFrameFunc
)
_SYSTEM_EXIT_PYTEST_LINES: frozenset[str] = frozenset({
    "raise SystemExit(pytest.console_main())",
    "raise SystemExit(pytest._console_main())",
    "raise SystemExit(_console_main())",
    "sys.exit(_console_main())",
})
_PYTEST_MAIN_INVOCATION_PREFIX = "code = _main("
_PYTEST_CMDLINE_MAIN_PREFIX = "ret: ExitCode | int = "
_PYTEST_PREPARECONFIG_PREFIX = "config = prepareconfig("
_SYSTEM_EXIT_MAIN_LINE = "raise SystemExit(main())"
_RUNPY_INVOCATION_SNIPPET = "runpy.run_module("


@dataclasses.dataclass(slots=True, frozen=True)
class _FrameInfo:
    """Parsed traceback frame plus optional source line."""

    frame_line: str
    frame_file: str
    frame_func: str
    code_line: str
    next_index: int


def normalize_traceback_output(output: str | None, placeholder: str = "<file>") -> str:
    """Normalize traceback output for snapshot comparison.

    Replaces file paths and line numbers with stable placeholders.

    Parameters
    ----------
    output : str | None
        The traceback output string to normalize, or ``None``.
    placeholder : str
        The placeholder used for file paths. Defaults to ``"<file>"``.

    Returns
    -------
    str
        Normalized output string, or an empty string when ``output`` is
        ``None``.

    """
    if output is None:
        return ""

    result = _normalize_launcher_frames(output)

    # Replace file paths with placeholder
    result = re.sub(
        r'File "[^"]+"',
        f'File "{placeholder}"',
        result,
    )
    # Replace line numbers
    result = re.sub(r", line \d+,", ", line <N>,", result)
    # Pytest can render runtest_hook lines with variable args/kwargs across
    # versions. Canonicalize the full call to a stable placeholder.
    return _PYTEST_RUNTEST_HOOK_LINE_PATTERN.sub(
        r"\g<indent>\g<prefix>(...),",
        result,
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

    Parameters
    ----------
    output : str | None
        The raw logging output, or ``None``.
    external_roots : collections.abc.Iterable[pathlib.Path] | None
        Directories whose frames are removed. Defaults to the ``stdlib``,
        ``platstdlib``, ``purelib``, ``platlib`` and ``scripts`` entries of
        :func:`sysconfig.get_paths`.

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
        tuple(external_roots)
        if external_roots is not None
        else _external_roots_from_sysconfig()
    )
    return _rejoin_lines(_strip_frames(output.splitlines(), roots), output)


def _strip_frames(lines: list[str], roots: cabc.Sequence[Path]) -> list[str]:
    """Return the lines with external frame blocks removed from stack sections."""
    kept: list[str] = []
    index = 0
    in_stack_section = False

    while index < len(lines):
        heading = lines[index].strip()
        if heading in _SECTION_HEADINGS:
            # A heading always ends the previous section and starts its own,
            # so a second stack heading inside one section is still a heading.
            in_stack_section = heading == _STACK_HEADING
            kept.append(lines[index])
            index += 1
        elif not in_stack_section:
            kept.append(lines[index])
            index += 1
        else:
            index = _keep_or_drop_frame(lines, index, roots, kept)
    return kept


def _keep_or_drop_frame(
    lines: list[str],
    index: int,
    roots: cabc.Sequence[Path],
    kept: list[str],
) -> int:
    """Append one stack frame's block to ``kept`` unless it is external.

    Returns
    -------
    int
        The index just past the frame block, whether kept or dropped.
    """
    frame = _parse_frame(lines, index)
    if frame is None:
        kept.append(lines[index])
        return index + 1
    block_end = _frame_block_end(lines, frame)
    if not _is_external_frame_file(frame.frame_file, roots):
        kept.extend(lines[index:block_end])
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


def _frame_block_end(lines: list[str], frame: _FrameInfo) -> int:
    """Return the index just past a frame's header, source and caret lines."""
    end = frame.next_index
    while end < len(lines) and _CARET_LINE_PATTERN.match(lines[end]):
        end += 1
    return end


def _rejoin_lines(lines: list[str], original: str) -> str:
    """Join split lines, preserving whether the original ended with a newline."""
    joined = "\n".join(lines)
    return f"{joined}\n" if original.endswith("\n") else joined


def _normalize_launcher_frames(output: str) -> str:
    """Remove interpreter launcher frames and normalize entrypoint calls."""
    lines = output.splitlines()
    normalized_lines: list[str] = []
    index = 0

    while index < len(lines):
        frame = _parse_frame(lines, index)
        if frame is None:
            normalized_lines.append(lines[index])
            index += 1
            continue

        index = frame.next_index
        if _should_drop_launcher_frame(
            frame.frame_file,
            frame.frame_func,
            frame.code_line,
        ):
            continue

        normalized_lines.append(
            _normalize_frame_line(frame.frame_line, frame.frame_func, frame.code_line)
        )
        if frame.code_line:
            normalized_lines.append(_normalize_frame_code_line(frame.code_line))

    rebuilt = "\n".join(normalized_lines)
    if output.endswith("\n"):
        return f"{rebuilt}\n"
    return rebuilt


def _parse_frame(lines: list[str], index: int) -> _FrameInfo | None:
    """Return parsed frame information when the line starts a traceback frame."""
    frame_line = lines[index]
    frame_match = _TRACEBACK_FRAME_PATTERN.match(frame_line)
    if frame_match is None:
        return None

    code_line = ""
    next_index = index + 1
    if next_index < len(lines):
        candidate = lines[next_index]
        if (
            candidate.startswith("    ")
            and _TRACEBACK_FRAME_PATTERN.match(candidate) is None
        ):
            code_line = candidate.strip()
            next_index += 1

    return _FrameInfo(
        frame_line=frame_line,
        frame_file=frame_match.group("file"),
        frame_func=frame_match.group("func"),
        code_line=code_line,
        next_index=next_index,
    )


def _normalize_frame_code_line(code_line: str) -> str:
    """Normalize volatile traceback source lines to stable output."""
    if code_line in _SYSTEM_EXIT_PYTEST_LINES:
        return "    sys.exit(console_main())"
    if code_line.startswith(_PYTEST_MAIN_INVOCATION_PREFIX):
        return "    code = main()"
    return f"    {code_line}"


def _normalize_frame_line(frame_line: str, frame_func: str, code_line: str) -> str:
    """Normalize volatile traceback frame names to stable output."""
    if frame_func == "_console_main":
        return frame_line.removesuffix(" in _console_main") + " in console_main"
    if _is_pytest_command_main_frame(frame_func, code_line):
        return frame_line.removesuffix(" in _main") + " in main"
    return frame_line


def _is_pytest_command_main_frame(frame_func: str, code_line: str) -> bool:
    """Return whether a pytest command-line ``_main`` frame should be stable."""
    if frame_func != "_main":
        return False
    return code_line.startswith((
        _PYTEST_CMDLINE_MAIN_PREFIX,
        _PYTEST_PREPARECONFIG_PREFIX,
    ))


def _is_runpy_launcher_path(frame_file: str) -> bool:
    """Return whether a traceback frame comes from runpy launcher internals."""
    if frame_file == "<frozen runpy>":
        return True
    return PurePath(frame_file).name == "runpy.py"


def _should_drop_launcher_frame(
    frame_file: str,
    frame_func: str,
    code_line: str,
) -> bool:
    """Return whether a traceback frame is launcher noise."""
    in_launcher_runtime = _is_runpy_launcher_path(frame_file)
    return (
        (in_launcher_runtime and frame_func in _LAUNCHER_FRAME_FUNCS)
        or (frame_func == "<module>" and code_line == _SYSTEM_EXIT_MAIN_LINE)
        or (frame_func == "main" and _RUNPY_INVOCATION_SNIPPET in code_line)
    )


@given("the logging system is reset")
def reset_logging() -> None:
    """Reset global logging state for scenario isolation."""
    reset_manager()


@then(parsers.parse('logging "{msg}" at "{level}" from root matches snapshot'))
def log_matches_snapshot(msg: str, level: str, snapshot: SnapshotAssertion) -> None:
    """Assert root logger output matches snapshot, handling DEBUG specially."""
    logger = get_logger("root")
    formatted = logger.log(level, msg)
    if level.upper() == "DEBUG":
        assert formatted is None, (
            "root logger must suppress DEBUG records at its default level, "
            f"but it formatted {formatted!r}"
        )
    else:
        assert formatted is not None, (
            f"root logger must emit a record logged at level {level!r}"
        )
        assert formatted == snapshot, (
            f"formatted root output at level {level!r} must match the recorded snapshot"
        )
