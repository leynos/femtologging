"""Traceback output normalization tests for BDD snapshot stability.

This module verifies that ``normalize_traceback_output`` produces deterministic
traceback text across Python and pytest versions so snapshot assertions stay
stable.

Example:
    normalized = normalize_traceback_output(raw_output)

"""

from __future__ import annotations

from pathlib import Path

import pytest

from tests.steps._hypothesis_support import _ENTRYPOINT_PROPERTY_CASES
from tests.steps.conftest import (
    normalize_traceback_output,
    strip_external_stack_frames,
)

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


_STDLIB_ROOT = Path("/opt/python/lib/python3.12")
_SITE_PACKAGES_ROOT = Path("/workspace/.venv/lib/python3.12/site-packages")
_EXTERNAL_ROOTS = (_STDLIB_ROOT, _SITE_PACKAGES_ROOT)

_STACK_HEADING = "Stack (most recent call last):\n"

_EXTERNAL_FRAME_CASES = (
    pytest.param(
        _STACK_HEADING
        + '  File "/opt/python/lib/python3.12/runpy.py", line 9, in _run\n'
        + "    return _run_code()\n"
        + '  File "/workspace/.venv/lib/python3.12/site-packages/pluggy/'
        '_callers.py", line 3, in _multicall\n'
        + "    res = hook_impl.function(*args)\n"
        + "    ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^\n"
        + '  File "/workspace/femtologging/app.py", line 7, in emit\n'
        + "    logger.log(level, message)\n",
        _STACK_HEADING
        + '  File "/workspace/femtologging/app.py", line 7, in emit\n'
        + "    logger.log(level, message)\n",
        "remove standard-library and site-package frames with their carets",
        id="removes_stdlib_and_site_package_frames",
    ),
    pytest.param(
        _STACK_HEADING
        + '  File "/workspace/femtologging/runpy.py", line 4, in run_module\n'
        + "    dispatch()\n"
        + '  File "/workspace/femtologging/main.py", line 8, in main\n'
        + "    return app.run()\n"
        + '  File "/workspace/femtologging/call.py", line 12, in _call\n'
        + "    app.run()\n"
        + '  File "/opt/python/lib/python3.12/runpy.py", line 9, in _run\n'
        + "    return _run_code()\n",
        _STACK_HEADING
        + '  File "/workspace/femtologging/runpy.py", line 4, in run_module\n'
        + "    dispatch()\n"
        + '  File "/workspace/femtologging/main.py", line 8, in main\n'
        + "    return app.run()\n"
        + '  File "/workspace/femtologging/call.py", line 12, in _call\n'
        + "    app.run()\n",
        "keep application frames whose names collide with launcher functions",
        id="keeps_colliding_application_frames_in_order",
    ),
    pytest.param(
        _STACK_HEADING + '  File "/workspace/.venv/lib/python3.12/site-packages/pytest/'
        '_pytest/python.py", line 21, in pytest_pyfunc_call\n'
        + '  File "/workspace/femtologging/app.py", line 7, in emit\n'
        + "    logger.log(level, message)\n",
        _STACK_HEADING
        + '  File "/workspace/femtologging/app.py", line 7, in emit\n'
        + "    logger.log(level, message)\n",
        "remove an external frame that has no source line",
        id="removes_external_frame_without_source",
    ),
    pytest.param(
        _STACK_HEADING
        + "  an explanatory line from the logger\n"
        + '  File "/workspace/.venv/lib/python3.12/site-packages/pytest/'
        '_pytest/runner.py", line 5, in runtest\n'
        + "    item.runtest()\n"
        + "    ~~~~^^^^\n"
        + "  continuation of the logger text\n",
        _STACK_HEADING
        + "  an explanatory line from the logger\n"
        + "  continuation of the logger text\n",
        "keep non-frame text and remove caret and continuation lines",
        id="keeps_non_frame_text_and_removes_carets",
    ),
    pytest.param(
        _STACK_HEADING
        + '  File "<frozen runpy>", line 198, in _run_module_as_main\n'
        + '  File "<frozen importlib._bootstrap>", line 1, in _find\n'
        + '  File "/workspace/femtologging/app.py", line 7, in emit\n'
        + "    logger.log(level, message)\n",
        _STACK_HEADING
        + '  File "/workspace/femtologging/app.py", line 7, in emit\n'
        + "    logger.log(level, message)\n",
        "remove frozen interpreter frames",
        id="removes_frozen_frames",
    ),
    pytest.param(
        _STACK_HEADING
        + '  File "app.py", line 7, in emit\n'
        + "    logger.log(level, message)\n"
        + '  File "<string>", line 1, in <module>\n',
        _STACK_HEADING
        + '  File "app.py", line 7, in emit\n'
        + "    logger.log(level, message)\n"
        + '  File "<string>", line 1, in <module>\n',
        "keep relative and unrecognized filenames",
        id="keeps_relative_and_unrecognized_filenames",
    ),
)


class TestStripExternalStackFrames:
    """Grouped tests for removing frames owned by installed code."""

    @staticmethod
    @pytest.mark.parametrize(("output", "expected", "reason"), _EXTERNAL_FRAME_CASES)
    def test_strip_external_stack_frames(
        output: str,
        expected: str,
        reason: str,
    ) -> None:
        """Drop external frame blocks and preserve everything else."""
        stripped = strip_external_stack_frames(output, external_roots=_EXTERNAL_ROOTS)
        assert stripped == expected, f"strip_external_stack_frames should {reason}"
        assert (
            strip_external_stack_frames(stripped, external_roots=_EXTERNAL_ROOTS)
            == stripped
        ), "strip_external_stack_frames should be idempotent"

    @staticmethod
    def test_a_none_output_becomes_an_empty_string() -> None:
        """Treat a missing output as empty rather than raising."""
        assert strip_external_stack_frames(None, external_roots=_EXTERNAL_ROOTS) == ""

    @staticmethod
    def test_a_traceback_after_the_stack_is_untouched() -> None:
        """Leave the exception section byte-identical while trimming the stack."""
        traceback = (
            "Traceback (most recent call last):\n"
            '  File "/opt/python/lib/python3.12/threading.py", line 3, in run\n'
            "    self._target(*self._args)\n"
            '  File "/workspace/femtologging/app.py", line 9, in work\n'
            "    raise ValueError('boom')\n"
            "ValueError: boom\n"
        )
        output = (
            "test [ERROR] debug\n"
            + _STACK_HEADING
            + '  File "/workspace/.venv/lib/python3.12/site-packages/pytest/'
            '_pytest/runner.py", line 5, in runtest\n'
            + "    item.runtest()\n"
            + '  File "/workspace/femtologging/app.py", line 7, in emit\n'
            + "    logger.log(level, message)\n"
            + traceback
        )
        stripped = strip_external_stack_frames(output, external_roots=_EXTERNAL_ROOTS)
        assert stripped == (
            "test [ERROR] debug\n"
            + _STACK_HEADING
            + '  File "/workspace/femtologging/app.py", line 7, in emit\n'
            + "    logger.log(level, message)\n"
            + traceback
        ), "the exception traceback must be preserved byte for byte"
