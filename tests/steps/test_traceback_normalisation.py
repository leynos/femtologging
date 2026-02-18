"""Tests for traceback output normalisation helpers."""

from .conftest import normalise_traceback_output


def test_normalise_traceback_output_strips_pytest_lambda_kwargs() -> None:
    output = (
        "Stack (most recent call last):\n"
        '  File "/tmp/run.py", line 42, in <lambda>\n'
        "    lambda: runtest_hook(item=item, **kwds), when=when, reraise=reraise\n"
    )

    assert normalise_traceback_output(output) == (
        "Stack (most recent call last):\n"
        '  File "<file>", line <N>, in <lambda>\n'
        "    lambda: runtest_hook(item=item, **kwds),\n"
    )
