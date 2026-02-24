"""Traceback output normalization tests for BDD snapshot stability.

This module verifies that ``normalize_traceback_output`` produces deterministic
traceback text across Python and pytest versions so snapshot assertions stay
stable.

Example:
    normalized = normalize_traceback_output(raw_output)

"""

from __future__ import annotations

from .conftest import normalize_traceback_output


class TestTracebackNormalization:
    """Grouped tests for traceback normalization behavior."""

    def test_normalize_traceback_output_strips_pytest_lambda_kwargs(self) -> None:
        """Normalize pytest lambda frames to a stable snapshot-friendly form.

        Returns
        -------
        None
            Asserts pytest lambda kwargs are normalized for stable snapshots.

        """
        class_name = self.__class__.__name__
        output = (
            "Stack (most recent call last):\n"
            '  File "/tmp/run.py", line 42, in <lambda>\n'
            "    lambda: runtest_hook(item=item, **kwds), when=when, reraise=reraise\n"
        )
        expected = (
            "Stack (most recent call last):\n"
            '  File "<file>", line <N>, in <lambda>\n'
            "    lambda: runtest_hook(...),\n"
        )

        assert normalize_traceback_output(output) == expected, (
            f"{class_name}: normalize_traceback_output should strip pytest "
            "lambda kwargs and "
            "normalize file/line markers"
        )

    def test_normalize_traceback_output_relaxes_runtest_hook_signature(self) -> None:
        """Handle spacing and kwarg-name changes in runtest_hook renderings.

        Returns
        -------
        None
            Asserts runtest hook signatures normalize to a single stable form.

        """
        class_name = self.__class__.__name__
        output = (
            "Stack (most recent call last):\n"
            '  File "/tmp/run.py", line 9, in <lambda>\n'
            "    lambda: runtest_hook( item=item, stage=stage, retry=retry ), "
            "when=phase, reroute=reroute\n"
        )
        expected = (
            "Stack (most recent call last):\n"
            '  File "<file>", line <N>, in <lambda>\n'
            "    lambda: runtest_hook(...),\n"
        )

        assert normalize_traceback_output(output) == expected, (
            f"{class_name}: normalize_traceback_output should retain a stable "
            "runtest_hook placeholder across spacing and keyword changes"
        )

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
