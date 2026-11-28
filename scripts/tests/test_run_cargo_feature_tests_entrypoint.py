"""Entry-point behaviour of the Cargo feature-matrix driver.

The behavioural tests in :mod:`test_run_cargo_feature_tests` call the driver's
`main` function directly. That is convenient, but it bypasses the Cyclopts
entry point the workflow actually uses: no command-line arguments, with all
configuration read from ``INPUT_``-prefixed environment variables. A broken
prefix or an environment variable that stopped binding would leave every one
of those tests passing while the workflow tested nothing.

These tests drive `app([])` instead, and pin the two refusals that belong to
the entry point rather than to the matrix walk. Run them through
``make feature-tests-runner-test``.
"""

from __future__ import annotations

import typing as typ

import pytest
import run_cargo_feature_tests
from test_run_cargo_feature_tests import (
    EXPECTED_SELECTIONS,
    FEATURE_SETS,
    HARNESS_ARGS,
    MANIFEST,
    REJECTED,
    invocations_in,
    manifest_path,
    rejecting,
)

if typ.TYPE_CHECKING:
    from cmd_mox import CmdMox

manual_lifecycle = pytest.mark.cmd_mox(auto_lifecycle=False)


def _run_app(monkeypatch: pytest.MonkeyPatch, **exported: str) -> int:
    """Invoke the driver the way the workflow does, through ``app``.

    The helper calls :func:`run_cargo_feature_tests.main` directly, which
    bypasses Cyclopts' environment binding entirely. This drives the same
    entry point the workflow uses: no command-line arguments, configuration
    read from ``INPUT_`` variables only.

    Returns
    -------
        The exit status the entry point raised.
    """
    for name, value in exported.items():
        monkeypatch.setenv(name, value)
    with pytest.raises(SystemExit) as exited:
        run_cargo_feature_tests.app([])
    assert isinstance(exited.value.code, int), exited.value.code
    return exited.value.code


@manual_lifecycle
def test_the_environment_drives_the_workflow_entrypoint(
    cmd_mox: CmdMox, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Scenario: the workflow runs the driver with no command-line arguments.

    Invariant: the three ``INPUT_`` variables reach the driver and each
    selection is tested, so a broken environment prefix cannot leave the
    helper's direct calls passing while the workflow tests nothing.
    """
    spy = cmd_mox.spy("cargo").returns(exit_code=0)

    cmd_mox.replay()
    status = _run_app(
        monkeypatch,
        INPUT_MANIFEST=MANIFEST,
        INPUT_FEATURE_SETS=" ".join(FEATURE_SETS),
        INPUT_TEST_ARGS=" ".join(HARNESS_ARGS),
    )
    cmd_mox.verify()

    assert status == 0, "the environment-driven run must succeed"
    selections = [
        argv[argv.index("--manifest-path") + 2 : argv.index("--")]
        for argv in invocations_in(spy)
    ]
    assert selections == [list(flags) for flags in EXPECTED_SELECTIONS], (
        "every selection must arrive through the environment, in matrix order"
    )


@manual_lifecycle
def test_the_environment_driven_entrypoint_reports_a_failure(
    cmd_mox: CmdMox, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Scenario: the first selection fails in an environment-driven run.

    Invariant: the process exits with that selection's code, so the workflow
    step fails rather than reporting a green run.
    """
    cmd_mox.spy("cargo").runs(rejecting("none"))

    cmd_mox.replay()
    status = _run_app(
        monkeypatch,
        INPUT_MANIFEST=MANIFEST,
        INPUT_FEATURE_SETS=" ".join(FEATURE_SETS),
        INPUT_TEST_ARGS=" ".join(HARNESS_ARGS),
    )
    cmd_mox.verify()

    assert status == REJECTED, (
        "an environment-driven run must exit with the failing selection's code"
    )


def test_an_empty_feature_set_list_is_not_success(
    capsys: pytest.CaptureFixture[str],
) -> None:
    """Scenario: the selection list is empty.

    Invariant: the run fails and says so. A matrix that checked nothing must
    not report a clean result, the failure mode the script exists to prevent.
    """
    status = run_cargo_feature_tests.main(
        manifest=manifest_path(), feature_sets=[], test_args=list(HARNESS_ARGS)
    )
    assert status == 2, "an empty selection list must not report success"
    assert "no feature sets requested" in capsys.readouterr().err, (
        "an empty selection list must explain the refusal"
    )


def test_an_empty_harness_selection_is_not_success(
    capsys: pytest.CaptureFixture[str],
) -> None:
    """Scenario: a caller passes an empty harness selection directly.

    Invariant: the run fails and says so. Cargo would accept the command and
    run the ordinary suite, which passes without touching the ignored tests.
    The environment path is covered separately: Cyclopts discards a blank
    variable, so it reaches :func:`main` as ``None`` rather than as ``[]``.
    """
    status = run_cargo_feature_tests.main(
        manifest=manifest_path(), feature_sets=list(FEATURE_SETS), test_args=[]
    )
    assert status == 2, "an empty harness selection must not report success"
    assert "no harness arguments requested" in capsys.readouterr().err, (
        "an empty harness selection must explain the refusal"
    )
