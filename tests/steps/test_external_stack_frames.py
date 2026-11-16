"""Tests for removing installed and standard-library frames from a stack.

``stack_info=True`` records capture the whole call stack. These tests pin the
rule that frames owned by the interpreter or an installed package are dropped
before normalizing, so a third-party release cannot change a snapshot.
"""

from __future__ import annotations

from pathlib import Path

import pytest

from tests.steps.conftest import strip_external_stack_frames

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
        assert not strip_external_stack_frames(None, external_roots=_EXTERNAL_ROOTS)

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
