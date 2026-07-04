"""Traceback output normalization tests for BDD snapshot stability.

This module verifies that ``normalize_traceback_output`` produces deterministic
traceback text across Python and pytest versions so snapshot assertions stay
stable.

Example:
    normalized = normalize_traceback_output(raw_output)

"""

from __future__ import annotations

import pytest
from hypothesis import given
from hypothesis import strategies as st

from tests.steps.conftest import (
    _SYSTEM_EXIT_PYTEST_LINES,
    normalize_traceback_output,
)

_PYTEST_COMPAT_CASES = (
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
        "normalize_traceback_output should strip pytest lambda kwargs and "
        "normalize file/line markers",
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
        "normalize_traceback_output should retain a stable runtest_hook "
        "placeholder across spacing and keyword changes",
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
            "Stack (most recent call last):\n"
            '  File "<file>", line <N>, in <module>\n'
            "    sys.exit(console_main())\n"
            '  File "<file>", line <N>, in console_main\n'
            "    code = main()\n"
            '  File "<file>", line <N>, in main\n'
            "    config = prepareconfig(args, plugins)\n"
        ),
        "normalize_traceback_output should keep pytest entrypoint snapshots "
        "stable when the helper is private",
        id="accepts_private_pytest_entrypoint",
    ),
    pytest.param(
        (
            "Stack (most recent call last):\n"
            '  File "/tmp/pytest/__main__.py", line 22, in <module>\n'
            "    raise SystemExit(pytest._console_main())\n"
            '  File "/tmp/pytest.py", line 25, in _console_main\n'
            "    code = _main(prog=_get_prog_name(sys.argv))\n"
        ),
        (
            "Stack (most recent call last):\n"
            '  File "<file>", line <N>, in <module>\n'
            "    sys.exit(console_main())\n"
            '  File "<file>", line <N>, in console_main\n'
            "    code = main()\n"
        ),
        "normalize_traceback_output should normalize qualified private pytest "
        "entrypoint calls",
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
        (
            "Stack (most recent call last):\n"
            '  File "<file>", line <N>, in <module>\n'
            "    sys.exit(console_main())\n"
            '  File "<file>", line <N>, in console_main\n'
            "    code = main()\n"
        ),
        "normalize_traceback_output should normalize bare private pytest "
        "entrypoint calls",
        id="accepts_bare_private_pytest_entrypoint",
    ),
)


class TestTracebackNormalization:
    """Grouped tests for traceback normalization behavior."""

    @pytest.mark.parametrize(("output", "expected", "reason"), _PYTEST_COMPAT_CASES)
    def test_normalize_traceback_output_pytest_compat_cases(
        self,
        output: str,
        expected: str,
        reason: str,
    ) -> None:
        """Normalize pytest compatibility frames to stable snapshot forms.

        Returns
        -------
        None
            Asserts pytest compatibility frames normalize for stable snapshots.

        """
        class_name = self.__class__.__name__

        assert normalize_traceback_output(output) == expected, f"{class_name}: {reason}"

    def test_normalize_traceback_output_strips_python_launcher_frames(self) -> None:
        """Drop volatile runpy/python launcher frames before pytest frames.

        Returns
        -------
        None
            Asserts that runpy launcher frames are removed and pytest entrypoint
            formatting remains stable.

        Notes
        -----
        This protects snapshot output from Python launcher implementation
        details that vary across versions.

        """
        class_name = self.__class__.__name__
        output = (
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
        )
        expected = (
            "Stack (most recent call last):\n"
            '  File "<file>", line <N>, in <module>\n'
            "    sys.exit(console_main())\n"
            '  File "<file>", line <N>, in console_main\n'
            "    code = main()\n"
        )

        assert normalize_traceback_output(output) == expected, (
            f"{class_name}: normalize_traceback_output should remove launcher "
            "frames and keep a stable pytest entrypoint frame"
        )

    @pytest.mark.parametrize(
        ("description", "output", "expected"),
        [
            (
                "qualified private entrypoint",
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
                    "Stack (most recent call last):\n"
                    '  File "<file>", line <N>, in <module>\n'
                    "    sys.exit(console_main())\n"
                    '  File "<file>", line <N>, in console_main\n'
                    "    code = main()\n"
                    '  File "<file>", line <N>, in main\n'
                    "    ret: ExitCode | int = "
                    "config.hook.pytest_cmdline_main(config=config)\n"
                ),
            ),
            (
                "unqualified private entrypoint",
                (
                    "Stack (most recent call last):\n"
                    '  File "/tmp/pytest/__main__.py", line 9, in <module>\n'
                    "    raise SystemExit(_console_main())\n"
                    '  File "/tmp/_pytest/config/__init__.py", line 201, '
                    "in _console_main\n"
                    "    code = _main(prog=_get_prog_name(sys.argv))\n"
                ),
                (
                    "Stack (most recent call last):\n"
                    '  File "<file>", line <N>, in <module>\n'
                    "    sys.exit(console_main())\n"
                    '  File "<file>", line <N>, in console_main\n'
                    "    code = main()\n"
                ),
            ),
        ],
        ids=[
            "qualified private entrypoint",
            "unqualified private entrypoint",
        ],
    )
    def test_normalize_traceback_output_canonicalizes_pytest_entrypoint(
        self, description: str, output: str, expected: str
    ) -> None:
        """Handle pytest private entrypoint spelling used by newer versions.

        Returns
        -------
        None
            Asserts that ``_console_main`` normalizes to the stable public
            entrypoint spelling used by snapshots.

        """
        assert normalize_traceback_output(output) == expected, (
            f"{self.__class__.__name__}: normalize_traceback_output should "
            f"canonicalize pytest entrypoint calls ({description})"
        )

    @staticmethod
    @given(
        segment=st.text(
            alphabet=st.characters(
                blacklist_characters='"\n\r\v\f\x1c\x1d\x1e\x85\u2028\u2029'
            ),
            max_size=120,
        ),
        line_no=st.integers(min_value=1, max_value=999_999),
        entrypoint_line=st.sampled_from(sorted(_SYSTEM_EXIT_PYTEST_LINES)),
    )
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
            Pytest entrypoint source line variant accepted by the normaliser.

        Returns
        -------
        None
            Asserts pytest entrypoint frames canonicalise their source line,
            scrub concrete line numbers, and remain stable when normalised more
            than once.

        """
        output = (
            "Stack (most recent call last):\n"
            f'  File "/tmp/{segment}/__main__.py", line {line_no}, in <module>\n'
            f"    {entrypoint_line}\n"
        )

        normalized = normalize_traceback_output(output)

        assert "sys.exit(console_main())" in normalized
        assert f"line {line_no}" not in normalized
        assert normalize_traceback_output(normalized) == normalized

    def test_normalize_traceback_output_keeps_non_launcher_main_frame(self) -> None:
        """Keep application main frames that are not runpy wrappers.

        Returns
        -------
        None
            Asserts that non-launcher frames are preserved after normalization.

        Notes
        -----
        User entrypoints must remain visible so snapshots catch application
        regressions.

        """
        class_name = self.__class__.__name__
        output = (
            "Stack (most recent call last):\n"
            '  File "/tmp/app.py", line 11, in main\n'
            "    process_request()\n"
        )
        expected = (
            "Stack (most recent call last):\n"
            '  File "<file>", line <N>, in main\n'
            "    process_request()\n"
        )

        assert normalize_traceback_output(output) == expected, (
            f"{class_name}: normalize_traceback_output should keep non-launcher "
            "main frames"
        )

    def test_normalize_traceback_output_keeps_non_launcher_run_module_frame(
        self,
    ) -> None:
        """Keep user-defined run_module frames outside stdlib launcher code.

        Returns
        -------
        None
            Asserts application ``run_module`` frames are retained.

        """
        class_name = self.__class__.__name__
        output = (
            "Stack (most recent call last):\n"
            '  File "/workspace/app/runtime.py", line 47, in run_module\n'
            "    dispatch(request)\n"
            '  File "/workspace/app/runtime.py", line 20, in dispatch\n'
            "    raise RuntimeError('boom')\n"
        )
        expected = (
            "Stack (most recent call last):\n"
            '  File "<file>", line <N>, in run_module\n'
            "    dispatch(request)\n"
            '  File "<file>", line <N>, in dispatch\n'
            "    raise RuntimeError('boom')\n"
        )

        assert normalize_traceback_output(output) == expected, (
            f"{class_name}: normalize_traceback_output should keep user "
            "run_module frames"
        )
