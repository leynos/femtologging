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
            "    lambda: runtest_hook(item=item, **kwds),\n"
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
            "    lambda: runtest_hook( item=item, stage=stage, retry=retry ),\n"
        )

        assert normalise_traceback_output(output) == expected, (
            f"{class_name}: normalise_traceback_output should retain a stable "
            "runtest_hook call prefix across spacing and keyword changes"
        )
