"""Traceback output normalisation tests for BDD snapshot stability.

This module verifies that ``normalise_traceback_output`` produces deterministic
traceback text across Python and pytest versions so snapshot assertions stay
stable.

Example:
    normalised = normalise_traceback_output(raw_output)

"""

from __future__ import annotations

from .conftest import normalise_traceback_output


class TestTracebackNormalisation:
    """Grouped tests for traceback normalisation behaviour."""

    def test_normalise_traceback_output_strips_pytest_lambda_kwargs(self) -> None:
        """Normalise pytest lambda frames to a stable snapshot-friendly form."""
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

        assert normalise_traceback_output(output) == expected, (
            f"{class_name}: normalise_traceback_output should strip pytest "
            "lambda kwargs and "
            "normalise file/line markers"
        )

    def test_normalise_traceback_output_relaxes_runtest_hook_signature(self) -> None:
        """Handle spacing and kwarg-name changes in runtest_hook renderings."""
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

        assert normalise_traceback_output(output) == expected, (
            f"{class_name}: normalise_traceback_output should retain a stable "
            "runtest_hook placeholder across spacing and keyword changes"
        )

    def test_normalise_traceback_output_strips_python_launcher_frames(self) -> None:
        """Drop volatile runpy/python launcher frames before pytest frames."""
        class_name = self.__class__.__name__
        output = (
            "Stack (most recent call last):\n"
            '  File "/tmp/python.py", line 100, in _run_module_as_main\n'
            '  File "/tmp/python.py", line 101, in _run_code\n'
            '  File "/tmp/__main__.py", line 12, in <module>\n'
            "    raise SystemExit(main())\n"
            '  File "/tmp/pytest.py", line 20, in main\n'
            "    runpy.run_module(*args.module, run_name='__main__', alter_sys=True)\n"
            '  File "/tmp/__main__.py", line 22, in <module>\n'
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

        assert normalise_traceback_output(output) == expected, (
            f"{class_name}: normalise_traceback_output should remove launcher "
            "frames and keep a stable pytest entrypoint frame"
        )

    def test_normalise_traceback_output_keeps_non_launcher_main_frame(self) -> None:
        """Keep application main frames that are not runpy wrappers."""
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

        assert normalise_traceback_output(output) == expected, (
            f"{class_name}: normalise_traceback_output should keep non-launcher "
            "main frames"
        )
