"""Behaviour of the Cargo feature-matrix test driver.

The driver exists because the heavy suite must run the ignored tests once per
supported feature selection, and every invocation has to be able to fail the
step. A shell ``for`` loop reports the status of its last command, so a
rejection in an earlier selection is discarded unless every call carries a
guard. These tests pin the failure paths first: a selection that fails must
fail the run whether it is the first or the last, and the run must name it.

``cmd-mox`` shims ``cargo`` so no Rust is compiled and no Cargo cache is
touched. The shim is driven by a handler rather than by per-invocation
expectations because one run invokes ``cargo`` several times with different
arguments, and the interesting question is which selection failed rather than
which exact argument vector was used. The properties that walk arbitrary
selection lists live in ``test_run_cargo_feature_tests_properties.py``. Run
both through ``make feature-tests-runner-test``, which loads the plugin.
"""

from __future__ import annotations

import typing as typ

import pytest
import run_cargo_feature_tests

if typ.TYPE_CHECKING:
    import collections.abc as cabc
    import pathlib

    from cmd_mox import CmdMox, SpyCommand
    from cmd_mox.ipc import Invocation

# `cmd-mox`'s plugin is loaded by the `feature-tests-runner-test` Make target
# with `-p cmd_mox.pytest_plugin` rather than declared here, so that it is in
# place early enough to register its own marker.
#
# The plugin replays before the test body and verifies during teardown unless
# told otherwise. These tests drive record, replay and verify explicitly so
# that each expectation sits beside the assertion it supports.
manual_lifecycle = pytest.mark.cmd_mox(auto_lifecycle=False)

MANIFEST = "rust_extension/Cargo.toml"
FEATURE_SETS = ("none", "python", "log-compat", "python,log-compat")
HARNESS_ARGS = ("--ignored",)
REJECTED = 101

#: The feature flags each selection must reach Cargo with, in matrix order.
EXPECTED_SELECTIONS: typ.Final[tuple[tuple[str, ...], ...]] = (
    ("--no-default-features",),
    ("--no-default-features", "--features", "python"),
    ("--no-default-features", "--features", "log-compat"),
    ("--no-default-features", "--features", "python,log-compat"),
)


def manifest_path() -> pathlib.Path:
    """Return the manifest path the driver is given.

    The driver only ever stringifies this value, so the tests pass a plain
    string and narrow the type here in one place.

    Returns
    -------
        The manifest path, typed as the driver expects it.
    """
    return typ.cast("pathlib.Path", MANIFEST)


def _subsequence_at(tokens: cabc.Sequence[str], wanted: cabc.Sequence[str]) -> bool:
    """Return whether ``wanted`` appears in ``tokens`` as a contiguous run.

    Returns
    -------
        True when the whole subsequence appears contiguously.
    """
    width = len(wanted)
    return any(
        list(tokens[start : start + width]) == list(wanted)
        for start in range(len(tokens) - width + 1)
    )


def selects(argv: cabc.Sequence[str], feature_set: str) -> bool:
    """Report whether an argument vector carries the named selection.

    A selection is identified by the flags it adds rather than by its name:
    the ``none`` selection passes no ``--features`` flag and never the word
    ``none``. Matching the flags contiguously keeps ``log-compat`` from also
    matching ``python,log-compat``.

    Returns
    -------
        True when the vector selects exactly that feature set.
    """
    wanted = run_cargo_feature_tests.feature_selection(feature_set)
    if not wanted:
        return "--features" not in argv
    return _subsequence_at(argv, wanted)


def rejecting(feature_set: str) -> cabc.Callable[[Invocation], tuple[str, str, int]]:
    """Return a shim handler that rejects exactly one feature set.

    Returns
    -------
        A handler returning a non-zero result for that selection and success
        otherwise.
    """

    def handler(invocation: Invocation) -> tuple[str, str, int]:
        """Reject the invocation if it selects the feature set, else succeed."""
        if selects(invocation.args, feature_set):
            return ("", f"error: rejected in {feature_set}\n", REJECTED)
        return ("", "", 0)

    return handler


def invocations_in(spy: SpyCommand) -> list[list[str]]:
    """Return the argument vector of each recorded ``cargo`` invocation.

    Returns
    -------
        One argument list per invocation, in the order they were made.
    """
    return [list(invocation.args) for invocation in spy.invocations]


def run_feature_sets(feature_sets: cabc.Sequence[str]) -> int:
    """Invoke the driver over the given feature sets.

    Returns
    -------
        The driver's exit code.
    """
    return run_cargo_feature_tests.main(
        manifest=manifest_path(),
        feature_sets=list(feature_sets),
        test_args=list(HARNESS_ARGS),
    )


def test_the_reserved_name_selects_no_features() -> None:
    """Scenario: a selection is named ``none`` or after a feature list.

    Invariant: ``none`` adds no flag and any other name passes its list
    verbatim, so ``python,log-compat`` reaches Cargo as Cargo spells it.
    """
    assert not run_cargo_feature_tests.feature_selection("none"), (
        "the none selection must add no feature flags"
    )
    assert run_cargo_feature_tests.feature_selection("python,log-compat") == [
        "--features",
        "python,log-compat",
    ], "a named selection must be passed through unchanged"


def test_the_command_omits_an_empty_harness_separator() -> None:
    """Scenario: the driver is given no harness arguments.

    Invariant: no bare ``--`` is appended, so Cargo never sees a trailing
    separator with nothing behind it.
    """
    argv = run_cargo_feature_tests.cargo_test_command(manifest_path(), "none", ())
    assert "--" not in argv, "an empty harness argument list must add no separator"


def test_the_command_places_harness_arguments_behind_the_separator() -> None:
    """Scenario: the driver is given harness arguments.

    Invariant: they follow ``--`` and the feature selection precedes it, so
    Cargo reads the selection and the harness reads the arguments.
    """
    argv = run_cargo_feature_tests.cargo_test_command(
        manifest_path(), "python", HARNESS_ARGS
    )
    separator = argv.index("--")
    assert argv[separator - 2 : separator] == ["--features", "python"], (
        "the feature selection must precede the separator"
    )
    assert argv[separator + 1 :] == list(HARNESS_ARGS), (
        "harness arguments must follow the separator"
    )


@manual_lifecycle
def test_every_feature_set_passing_succeeds(cmd_mox: CmdMox) -> None:
    """Scenario: every feature selection passes.

    Invariant: the run succeeds and every selection is invoked, so a clean
    result cannot come from skipping the work.
    """
    spy = cmd_mox.spy("cargo").runs(rejecting("no-such-feature"))

    cmd_mox.replay()
    assert run_feature_sets(FEATURE_SETS) == 0, "every selection passing must succeed"
    cmd_mox.verify()

    assert spy.call_count == len(FEATURE_SETS), (
        "every feature selection must actually be tested"
    )


@manual_lifecycle
def test_a_failing_first_feature_set_fails_the_run(cmd_mox: CmdMox) -> None:
    """Scenario: the first of two selections rejects the code.

    Invariant: the run fails with that selection's exit code and stops before
    the passing selection. This is the defect the shell loop had, where the
    later selection's success became the target's exit status.
    """
    spy = cmd_mox.spy("cargo").runs(rejecting("none"))

    cmd_mox.replay()
    assert run_feature_sets(("none", "python")) == REJECTED, (
        "a failing first selection must fail the run"
    )
    cmd_mox.verify()

    assert spy.call_count == 1, "the run must stop at the failing selection"


@manual_lifecycle
def test_a_failing_last_feature_set_fails_the_run(cmd_mox: CmdMox) -> None:
    """Scenario: the last of two selections rejects the code.

    Invariant: the run fails. The shell loop got this case right, so it is
    kept to show the fix did not trade one blind spot for another.
    """
    spy = cmd_mox.spy("cargo").runs(rejecting("python"))

    cmd_mox.replay()
    assert run_feature_sets(("none", "python")) == REJECTED, (
        "a failing last selection must fail the run"
    )
    cmd_mox.verify()

    assert spy.call_count == 2, "both selections must run before the failure"


@manual_lifecycle
def test_the_failing_feature_set_is_named(
    cmd_mox: CmdMox, capsys: pytest.CaptureFixture[str]
) -> None:
    """Scenario: a selection fails partway through the list.

    Invariant: the output names it and the run stops there, so a contributor
    knows which feature set to reproduce rather than inferring it from
    whichever selection printed last.
    """
    spy = cmd_mox.spy("cargo").runs(rejecting("log-compat"))

    cmd_mox.replay()
    assert run_feature_sets(("none", "log-compat", "python")) == REJECTED, (
        "a failing middle selection must fail the run"
    )
    cmd_mox.verify()

    assert (
        f"# Cargo test failed for feature set: log-compat (exit {REJECTED})"
        in capsys.readouterr().out
    ), "the failing feature set must be named in the output"
    assert not any(selects(argv, "python") for argv in invocations_in(spy)), (
        "the run must stop before the selection after the failure"
    )


@manual_lifecycle
def test_each_feature_set_selects_its_own_features(cmd_mox: CmdMox) -> None:
    """Scenario: the driver walks all four selections.

    Invariant: each invocation carries that selection and nothing else, so a
    selection cannot silently test the same feature set as its neighbour.
    """
    spy = cmd_mox.spy("cargo").runs(rejecting("no-such-feature"))

    cmd_mox.replay()
    assert run_feature_sets(FEATURE_SETS) == 0, "every selection must pass"
    cmd_mox.verify()

    selections = [
        argv[argv.index("--manifest-path") + 2 : argv.index("--")]
        for argv in invocations_in(spy)
    ]
    assert selections == [list(flags) for flags in EXPECTED_SELECTIONS], (
        "each selection must carry its own feature flags"
    )


@manual_lifecycle
def test_the_default_harness_arguments_are_the_ignored_suite(cmd_mox: CmdMox) -> None:
    """Scenario: the caller omits the harness selection.

    Invariant: ``--ignored`` still reaches the harness. An omitted selection
    would run the ordinary tests instead, which pass without touching the
    ignored suite the heavy lane exists to run.
    """
    spy = cmd_mox.spy("cargo").returns(exit_code=0)

    cmd_mox.replay()
    status = run_cargo_feature_tests.main(
        manifest=manifest_path(), feature_sets=["none"]
    )
    cmd_mox.verify()

    assert status == 0, "the default selection must run the ignored suite"
    assert invocations_in(spy)[0][-2:] == ["--", "--ignored"], (
        "an omitted harness selection must default to --ignored"
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
    """Scenario: the caller supplies an empty harness selection.

    Invariant: the run fails and says so. Cargo would accept the command and
    run the ordinary suite, which passes without touching the ignored tests.

    ``test_args=[]`` models ``INPUT_TEST_ARGS=``, which Cyclopts reads as an
    empty list rather than as an omission, so the ``None`` default does not
    apply.
    """
    status = run_cargo_feature_tests.main(
        manifest=manifest_path(), feature_sets=list(FEATURE_SETS), test_args=[]
    )
    assert status == 2, "an empty harness selection must not report success"
    assert "no harness arguments requested" in capsys.readouterr().err, (
        "an empty harness selection must explain the refusal"
    )
