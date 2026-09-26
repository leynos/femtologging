"""Behaviour of the Loom model runner.

The runner exists because ``cargo test`` passes when no model ran: without
``--cfg loom`` every model is compiled out, a filter can match nothing,
``--ignored`` selects a subset, and ``--no-run`` executes nothing. These tests
pin each of those as a failure, and pin the one shape that must pass: every
listed model reported ``ok`` and nothing else.

Most cases hand :func:`run_loom_models.check_models` a stand-in runner, so no
process starts. One case shims ``cargo`` with ``cmd-mox`` to show the real
runner passes the command through and reads what it printed. Run this file
through ``make loom-runner-test``, which loads the plugin.
"""

from __future__ import annotations

import typing as typ

import pytest
import run_loom_models

if typ.TYPE_CHECKING:
    import collections.abc as cabc
    import pathlib

    from cmd_mox import CmdMox

manual_lifecycle = pytest.mark.cmd_mox(auto_lifecycle=False)

MODELS = ("loom_push::loom_first", "loom_topologies::loom_second")
CARGO_COMMAND = ("cargo", "test", "--test", "heavy", "loom_")


def libtest_output(results: dict[str, str]) -> str:
    """Render libtest's plain-text output for a name-to-outcome map.

    Returns
    -------
    str
        One ``test <name> ... <outcome>`` line per entry, then a result line.
    """
    lines = [f"test {name} ... {outcome}" for name, outcome in results.items()]
    lines.append("test result: done")
    return "\n".join(lines) + "\n"


def reporting(
    results: dict[str, str], returncode: int = 0
) -> cabc.Callable[[cabc.Sequence[str]], tuple[int, str]]:
    """Return a runner that reports ``results`` with ``returncode``.

    Returns
    -------
    cabc.Callable[[cabc.Sequence[str]], tuple[int, str]]
        A stand-in for :func:`run_loom_models.run_command`.
    """

    def run(_command: cabc.Sequence[str]) -> tuple[int, str]:
        return returncode, libtest_output(results)

    return run


@pytest.fixture
def expected(tmp_path: pathlib.Path) -> pathlib.Path:
    """Return an expected-model list naming both models, with a comment."""
    path = tmp_path / "loom-models.txt"
    path.write_text("# models\n" + "\n".join(MODELS) + "\n", encoding="utf-8")
    return path


def test_every_listed_model_passing_succeeds(expected: pathlib.Path) -> None:
    """Scenario: both listed models pass and nothing else runs.

    Invariant: this is the one report that is accepted.
    """
    run = reporting(dict.fromkeys(MODELS, "ok"))
    assert run_loom_models.check_models(CARGO_COMMAND, expected, run) == 0


@pytest.mark.parametrize(
    ("results", "returncode"),
    [
        pytest.param({}, 0, id="configuration dropped, nothing compiled in"),
        pytest.param({MODELS[0]: "ok"}, 0, id="one model missing"),
        pytest.param({MODELS[0]: "ok", MODELS[1]: "FAILED"}, 101, id="a model failed"),
        pytest.param(
            {**dict.fromkeys(MODELS, "ok"), "ordinary_test": "ok"},
            0,
            id="an ordinary test stands in",
        ),
        pytest.param({MODELS[0]: "ok", MODELS[1]: "ignored"}, 0, id="a model ignored"),
        pytest.param(dict.fromkeys(MODELS, "ok"), 101, id="cargo failed afterwards"),
    ],
)
def test_a_report_that_does_not_prove_every_model_fails(
    expected: pathlib.Path, results: dict[str, str], returncode: int
) -> None:
    """Scenario: a run that does not show every listed model passing.

    Invariant: the checker fails, whether or not Cargo itself exited zero.
    """
    run = reporting(results, returncode)
    assert run_loom_models.check_models(CARGO_COMMAND, expected, run) == 1


def test_only_ok_lines_count_as_passed() -> None:
    """Scenario: a report mixes passed, failed and ignored tests.

    Invariant: only ``ok`` counts. The exit code alone would catch a failed
    model today, but a parser that also counted ``FAILED`` would leave the
    missing-model rule unable to see a model that did not pass.
    """
    results = {MODELS[0]: "ok", MODELS[1]: "FAILED", "ignored_test": "ignored"}
    output = libtest_output(results)
    assert run_loom_models.passed_tests(output) == {MODELS[0]}


def test_an_empty_model_list_is_refused(tmp_path: pathlib.Path) -> None:
    """Scenario: the list names no models.

    Invariant: an empty list is itself a problem, because every run satisfies
    it vacuously.
    """
    path = tmp_path / "loom-models.txt"
    path.write_text("# nothing listed\n", encoding="utf-8")
    found = run_loom_models.problems(
        0, frozenset(), run_loom_models.read_expected(path)
    )
    assert found == ["the expected-model list is empty, so no run could prove anything"]


def test_no_command_is_a_usage_error(expected: pathlib.Path) -> None:
    """Scenario: nothing follows ``--``.

    Invariant: the checker refuses to run rather than judging an empty report.
    """
    assert run_loom_models.check_models((), expected) == 2


@manual_lifecycle
def test_the_real_runner_reads_what_cargo_printed(
    cmd_mox: CmdMox, expected: pathlib.Path
) -> None:
    """Scenario: the checker runs a real ``cargo`` process.

    Invariant: the command reaches the process unchanged and its standard
    output is what the checker judges, so a passing report through the real
    runner is accepted.
    """
    spy = cmd_mox.spy("cargo").returns(
        stdout=libtest_output(dict.fromkeys(MODELS, "ok"))
    )
    cmd_mox.replay()
    status = run_loom_models.check_models(CARGO_COMMAND, expected)
    cmd_mox.verify()
    assert status == 0
    assert [list(call.args) for call in spy.invocations] == [list(CARGO_COMMAND[1:])]
