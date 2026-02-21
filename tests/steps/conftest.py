"""Shared BDD steps reused across feature modules."""

from __future__ import annotations

import dataclasses
import enum
import re
import typing as typ
from pathlib import PurePath

from pytest_bdd import given, parsers, then

from femtologging import get_logger, reset_manager

if typ.TYPE_CHECKING:
    from syrupy.assertion import SnapshotAssertion


_PYTEST_RUNTEST_HOOK_LINE_PATTERN: re.Pattern[str] = re.compile(
    r"^(?P<indent>\s*)(?P<prefix>(?:lambda:\s*)?runtest_hook)\(.*\),.*$",
    flags=re.MULTILINE,
)
_TRACEBACK_FRAME_PATTERN: re.Pattern[str] = re.compile(
    r'^(?P<indent>\s*)File "(?P<file>[^"]+)", line (?P<line>[^,]+), '
    r"in (?P<func>.+)$",
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
_SYSTEM_EXIT_PYTEST_LINE = "raise SystemExit(pytest.console_main())"
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


def normalise_traceback_output(output: str | None, placeholder: str = "<file>") -> str:
    """Normalise traceback output for snapshot comparison.

    Replaces file paths and line numbers with stable placeholders.

    Parameters
    ----------
    output : str | None
        The traceback output string to normalise, or ``None``.
    placeholder : str
        The placeholder used for file paths. Defaults to ``"<file>"``.

    Returns
    -------
    str
        Normalised output string, or an empty string when ``output`` is
        ``None``.

    """
    if output is None:
        return ""

    result = _normalise_launcher_frames(output)

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


def _normalise_launcher_frames(output: str) -> str:
    """Remove interpreter launcher frames and normalise entrypoint calls."""
    lines = output.splitlines()
    normalised_lines: list[str] = []
    index = 0

    while index < len(lines):
        frame = _parse_frame(lines, index)
        if frame is None:
            normalised_lines.append(lines[index])
            index += 1
            continue

        index = frame.next_index
        if _should_drop_launcher_frame(
            frame.frame_file,
            frame.frame_func,
            frame.code_line,
        ):
            continue

        normalised_lines.append(frame.frame_line)
        if frame.code_line:
            normalised_lines.append(_normalise_frame_code_line(frame.code_line))

    rebuilt = "\n".join(normalised_lines)
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


def _normalise_frame_code_line(code_line: str) -> str:
    """Normalise volatile traceback source lines to stable output."""
    if code_line == _SYSTEM_EXIT_PYTEST_LINE:
        return "    sys.exit(console_main())"
    return f"    {code_line}"


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
        assert formatted is None
    else:
        assert formatted is not None
        assert formatted == snapshot
