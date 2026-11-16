"""Normalize captured traceback output so snapshots stay stable.

Pytest, pluggy and the interpreter change the frames and source lines they
emit between releases. :func:`normalize_traceback_output` masks paths and line
numbers and canonicalizes the launcher frames, so a snapshot records this
project's behaviour rather than a dependency's release.
"""

from __future__ import annotations

import enum
import re
from pathlib import PurePath

from tests.steps.stack_frames import frame_block_end, parse_frame

_PYTEST_RUNTEST_HOOK_LINE_PATTERN: re.Pattern[str] = re.compile(
    r"^(?P<indent>\s*)(?P<prefix>(?:lambda:\s*)?runtest_hook)\(.*\),.*$",
    flags=re.MULTILINE,
)
_STDLIB_DIRECTORY_PATTERN: re.Pattern[str] = re.compile(
    r"^python\d+(?:\.\d+)?[a-z]?$",
    flags=re.IGNORECASE,
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
SYSTEM_EXIT_PYTEST_LINES: frozenset[str] = frozenset({
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


def _normalize_launcher_frames(output: str) -> str:
    """Remove interpreter launcher frames and normalize entrypoint calls."""
    lines = output.splitlines()
    normalized_lines: list[str] = []
    index = 0

    while index < len(lines):
        frame = parse_frame(lines, index)
        if frame is None:
            normalized_lines.append(lines[index])
            index += 1
            continue

        if _should_drop_launcher_frame(
            frame.frame_file,
            frame.frame_func,
            frame.code_line,
        ):
            # A wrapped launcher call prints further source lines after the
            # first, so skip the whole block rather than leaving an orphan.
            index = frame_block_end(lines, frame)
            continue

        index = frame.next_index
        normalized_lines.append(
            _normalize_frame_line(frame.frame_line, frame.frame_func, frame.code_line)
        )
        if frame.code_line:
            normalized_lines.append(_normalize_frame_code_line(frame.code_line))

    rebuilt = "\n".join(normalized_lines)
    if output.endswith("\n"):
        return f"{rebuilt}\n"
    return rebuilt


def _normalize_frame_code_line(code_line: str) -> str:
    """Normalize volatile traceback source lines to stable output."""
    if code_line in SYSTEM_EXIT_PYTEST_LINES:
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
    """Return whether a traceback frame comes from runpy launcher internals.

    The basename alone is not enough: a project may ship its own ``runpy.py``
    with a ``run_module`` function. Only a ``runpy.py`` in an interpreter
    standard-library directory (``.../python3.15/`` or ``.../Python313/Lib``)
    is treated as launcher code.

    Returns
    -------
    bool
        Whether the frame belongs to interpreter launcher code.
    """
    if frame_file == "<frozen runpy>":
        return True
    path = PurePath(frame_file)
    if path.name != "runpy.py":
        return False
    return any(_STDLIB_DIRECTORY_PATTERN.match(part) for part in path.parent.parts)


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
