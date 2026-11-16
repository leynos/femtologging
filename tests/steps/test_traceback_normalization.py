"""Traceback output normalization tests for BDD snapshot stability.

This module verifies that ``normalize_traceback_output`` produces deterministic
traceback text across Python and pytest versions so snapshot assertions stay
stable.

Example:
    normalized = normalize_traceback_output(raw_output)

"""

from __future__ import annotations

import pytest

from tests.steps._hypothesis_support import _ENTRYPOINT_PROPERTY_CASES
from tests.steps.conftest import normalize_traceback_output

_STABLE_ENTRYPOINT = (
    "Stack (most recent call last):\n"
    '  File "<file>", line <N>, in <module>\n'
    "    sys.exit(console_main())\n"
    '  File "<file>", line <N>, in console_main\n'
    "    code = main()\n"
)

_NORMALIZATION_CASES = (
    pytest.param(
        (
            "Stack (most recent call last):\n"
            '  File "/tmp/run.py", line 42, in <lambda>\n'
            "    lambda: runtest_hook(item=item, **kwds), when=when, reraise=reraise\n"
        ),
        (
            "Stack (most recent call last):\n"
            '  File "<file>", line <N>, in <lambda>\n'
            "    lambda: runtest_hook(...),\n"
        ),
        "strip pytest lambda kwargs and normalize file/line markers",
        id="strips_pytest_lambda_kwargs",
    ),
    pytest.param(
        (
            "Stack (most recent call last):\n"
            '  File "/tmp/run.py", line 9, in <lambda>\n'
            "    lambda: runtest_hook( item=item, stage=stage, retry=retry ), "
            "when=phase, reroute=reroute\n"
        ),
        (
            "Stack (most recent call last):\n"
            '  File "<file>", line <N>, in <lambda>\n'
            "    lambda: runtest_hook(...),\n"
        ),
        "retain a stable runtest_hook placeholder across spacing and keyword changes",
        id="relaxes_runtest_hook_signature",
    ),
    pytest.param(
        (
            "Stack (most recent call last):\n"
            '  File "/tmp/pytest/__main__.py", line 22, in <module>\n'
            "    sys.exit(_console_main())\n"
            '  File "/tmp/pytest.py", line 25, in _console_main\n'
            "    code = _main(prog=_get_prog_name(sys.argv))\n"
            '  File "/tmp/pytest.py", line 30, in _main\n'
            "    config = prepareconfig(args, plugins)\n"
        ),
        (
            f"{_STABLE_ENTRYPOINT}"
            '  File "<file>", line <N>, in main\n'
            "    config = prepareconfig(args, plugins)\n"
        ),
        "keep pytest entrypoint snapshots stable when the helper is private",
        id="accepts_private_pytest_entrypoint",
    ),
    pytest.param(
        (
            "Stack (most recent call last):\n"
            '  File "/tmp/pytest/__main__.py", line 22, in <module>\n'
            "    sys.exit(_console_main())\n"
            '  File "/tmp/pytest.py", line 25, in _console_main\n'
            "    code = _main(prog=_get_prog_name(sys.argv))\n"
            '  File "/tmp/pytest.py", line 31, in _main\n'
            "    ret: ExitCode | int = "
            "config.hook.pytest_cmdline_main(config=config)\n"
        ),
        (
            f"{_STABLE_ENTRYPOINT}"
            '  File "<file>", line <N>, in main\n'
            "    ret: ExitCode | int = "
            "config.hook.pytest_cmdline_main(config=config)\n"
        ),
        "preserve the trailing hook call while canonicalizing the private "
        "entrypoint chain",
        id="preserves_frame_after_private_entrypoint",
    ),
    pytest.param(
        (
            "Stack (most recent call last):\n"
            '  File "/tmp/pytest/__main__.py", line 22, in <module>\n'
            "    raise SystemExit(pytest._console_main())\n"
            '  File "/tmp/pytest.py", line 25, in _console_main\n'
            "    code = _main(prog=_get_prog_name(sys.argv))\n"
        ),
        _STABLE_ENTRYPOINT,
        "normalize qualified private pytest entrypoint calls",
        id="accepts_qualified_private_pytest_entrypoint",
    ),
    pytest.param(
        (
            "Stack (most recent call last):\n"
            '  File "/tmp/pytest/__main__.py", line 22, in <module>\n'
            "    raise SystemExit(_console_main())\n"
            '  File "/tmp/pytest.py", line 25, in _console_main\n'
            "    code = _main(prog=_get_prog_name(sys.argv))\n"
        ),
        _STABLE_ENTRYPOINT,
        "normalize bare private pytest entrypoint calls",
        id="accepts_bare_private_pytest_entrypoint",
    ),
    pytest.param(
        (
            "Stack (most recent call last):\n"
            '  File "/tmp/pytest/__main__.py", line 9, in <module>\n'
            "    raise SystemExit(_console_main())\n"
            '  File "/tmp/_pytest/config/__init__.py", line 201, '
            "in _console_main\n"
            "    code = _main(prog=_get_prog_name(sys.argv))\n"
        ),
        _STABLE_ENTRYPOINT,
        "normalize private entrypoints defined under the _pytest package",
        id="accepts_private_entrypoint_from_pytest_internals",
    ),
    pytest.param(
        (
            "Stack (most recent call last):\n"
            '  File "/usr/lib/python3.15/runpy.py", line 198, in _run_module_as_main\n'
            '  File "/usr/lib/python3.15/runpy.py", line 88, in _run_code\n'
            '  File "/tmp/__main__.py", line 12, in <module>\n'
            "    raise SystemExit(main())\n"
            '  File "/tmp/pytest.py", line 20, in main\n'
            "    runpy.run_module(*args.module, run_name='__main__', alter_sys=True)\n"
            '  File "/usr/lib/python3.15/runpy.py", line 229, in run_module\n'
            '  File "/usr/lib/python3.15/runpy.py", line 98, in _run_module_code\n'
            '  File "/usr/lib/python3.15/runpy.py", line 88, in _run_code\n'
            '  File "/tmp/pytest/__main__.py", line 22, in <module>\n'
            "    raise SystemExit(pytest.console_main())\n"
            '  File "/tmp/pytest.py", line 25, in console_main\n'
            "    code = main()\n"
        ),
        _STABLE_ENTRYPOINT,
        "drop volatile runpy/python launcher frames that vary across "
        "interpreter versions",
        id="strips_python_launcher_frames",
    ),
    pytest.param(
        (
            "Stack (most recent call last):\n"
            '  File "/tmp/app.py", line 11, in main\n'
            "    process_request()\n"
        ),
        (
            "Stack (most recent call last):\n"
            '  File "<file>", line <N>, in main\n'
            "    process_request()\n"
        ),
        "keep application main frames so snapshots still catch regressions",
        id="keeps_non_launcher_main_frame",
    ),
    pytest.param(
        (
            "Stack (most recent call last):\n"
            '  File "/tmp/app.py", line 11, in _main\n'
            "    return application.run()\n"
        ),
        (
            "Stack (most recent call last):\n"
            '  File "<file>", line <N>, in _main\n'
            "    return application.run()\n"
        ),
        "keep application _main frames that are not launcher wrappers",
        id="keeps_non_launcher_private_main_frame",
    ),
    pytest.param(
        (
            "Stack (most recent call last):\n"
            '  File "/workspace/app/runtime.py", line 47, in run_module\n'
            "    dispatch(request)\n"
            '  File "/workspace/app/runtime.py", line 20, in dispatch\n'
            "    raise RuntimeError('boom')\n"
        ),
        (
            "Stack (most recent call last):\n"
            '  File "<file>", line <N>, in run_module\n'
            "    dispatch(request)\n"
            '  File "<file>", line <N>, in dispatch\n'
            "    raise RuntimeError('boom')\n"
        ),
        "keep user-defined run_module frames outside stdlib launcher code",
        id="keeps_non_launcher_run_module_frame",
    ),
)


class TestTracebackNormalization:
    """Grouped tests for traceback normalization behaviour."""

    @staticmethod
    @pytest.mark.parametrize(("output", "expected", "reason"), _NORMALIZATION_CASES)
    def test_normalize_traceback_output_stabilizes_frames(
        output: str,
        expected: str,
        reason: str,
    ) -> None:
        """Normalize traceback frames to their stable snapshot forms."""
        assert normalize_traceback_output(output) == expected, (
            f"normalize_traceback_output should {reason}"
        )

    @staticmethod
    @_ENTRYPOINT_PROPERTY_CASES
    def test_normalize_traceback_output_canonicalizes_entrypoints_property(
        segment: str,
        line_no: int,
        entrypoint_line: str,
    ) -> None:
        """Normalize pytest entrypoint frames across arbitrary source locations.

        Parameters
        ----------
        segment : str
            Generated path segment inserted into the synthetic traceback frame.
        line_no : int
            Generated source line number for the synthetic traceback frame.
        entrypoint_line : str
            Pytest entrypoint source line variant accepted by the normalizer.

        """
        output = (
            "Stack (most recent call last):\n"
            f'  File "/tmp/{segment}/__main__.py", line {line_no}, in <module>\n'
            f"    {entrypoint_line}\n"
        )

        normalized = normalize_traceback_output(output)

        assert "sys.exit(console_main())" in normalized, (
            "expected canonical sys.exit(console_main()) call in normalized output"
        )
        assert f"line {line_no}" not in normalized, (
            f"expected line number {line_no} to be scrubbed from normalized output"
        )
        assert normalize_traceback_output(normalized) == normalized, (
            "normalize_traceback_output should be idempotent"
        )
