"""Behaviour of the per-lane Rust lint driver.

The script exists because the shell loop it replaced could not fail: a shell
``for`` loop reports the status of its last command, so a rejection in any
earlier lane was discarded. These tests pin the failure paths first. A lane
that fails must fail the run whether it is the first lane or the last, and the
run must name the lane that failed.

``cmd-mox`` shims ``cargo`` so no Rust is compiled and no Cargo cache is
touched. The shim is driven by a handler rather than by per-invocation
expectations because one run invokes ``cargo`` several times with different
arguments, and the interesting question is which lane failed rather than which
exact argument vector was used.
"""

from __future__ import annotations

import typing as typ

import lint_rust_lanes
import pytest

if typ.TYPE_CHECKING:
    import collections.abc as cabc
    import pathlib

    from cmd_mox import CmdMox, SpyCommand
    from cmd_mox.ipc import Invocation

# `cmd-mox`'s plugin is loaded by the `lint-lanes-test` Make target with
# `-p cmd_mox.pytest_plugin` rather than declared here, so that it is in place
# early enough to register its own marker. Run this file through that target.
#
# The plugin replays before the test body and verifies during teardown unless
# told otherwise. These tests drive record, replay and verify explicitly so
# that each expectation sits beside the assertion it supports.
manual_lifecycle = pytest.mark.cmd_mox(auto_lifecycle=False)

MANIFEST = "rust_extension/Cargo.toml"
LINT_ARGS = ("-D", "clippy::disallowed_methods")
REJECTED = 101


def manifest_path() -> pathlib.Path:
    """Return the manifest path the driver is given.

    The driver only ever stringifies this value, so the tests pass a plain
    string and narrow the type here in one place.

    Returns
    -------
        The manifest path, typed as the driver expects it.
    """
    return typ.cast("pathlib.Path", MANIFEST)


def selects(argv: cabc.Sequence[str], lane: str) -> bool:
    """Report whether an argument vector carries the named lane's selection.

    A lane is identified by the flags it selects rather than by its name: the
    `none` lane passes `--no-default-features` and never the word "none".
    Matching the selection contiguously keeps `python` from also matching a
    lane that merely mentions it elsewhere.

    Returns
    -------
        True when the vector contains the lane's selection contiguously.
    """
    wanted = lint_rust_lanes.lane_selection(lane)
    window = len(wanted)
    return any(
        list(argv[start : start + window]) == wanted
        for start in range(len(argv) - window + 1)
    )


def rejecting(lane: str) -> cabc.Callable[[Invocation], tuple[str, str, int]]:
    """Return a shim handler that rejects exactly one lane.

    Returns
    -------
        A handler returning a non-zero result for that lane and success
        otherwise.
    """

    def handler(invocation: Invocation) -> tuple[str, str, int]:
        if selects(invocation.args, lane):
            return ("", f"error: rejected in {lane}\n", REJECTED)
        return ("", "", 0)

    return handler


def lanes_invoked(spy: SpyCommand) -> list[list[str]]:
    """Return the argument vector of each recorded ``cargo`` invocation.

    Returns
    -------
        One argument list per invocation, in the order they were made.
    """
    return [list(invocation.args) for invocation in spy.invocations]


def run_lanes(lanes: cabc.Sequence[str]) -> int:
    """Invoke the driver over the given lanes.

    Returns
    -------
        The driver's exit code.
    """
    return lint_rust_lanes.main(
        manifest=manifest_path(),
        lanes=list(lanes),
        lint_args=list(LINT_ARGS),
    )


def test_lane_selection_maps_the_two_reserved_names() -> None:
    """Scenario: a lane is named ``none``, ``all``, or after a feature.

    Invariant: ``none`` and ``all`` select the two whole-crate arms and any
    other name selects that feature alone. ``--all-features`` on its own would
    never compile a ``cfg(not(feature))`` block, which is why ``none`` exists.
    """
    assert lint_rust_lanes.lane_selection("none") == ["--no-default-features"], (
        "the none lane must select no features"
    )
    assert lint_rust_lanes.lane_selection("all") == ["--all-features"], (
        "the all lane must select every feature"
    )
    assert lint_rust_lanes.lane_selection("python") == [
        "--no-default-features",
        "--features",
        "python",
    ], "a named lane must select that feature and nothing else"


def test_lane_command_omits_an_empty_lint_separator() -> None:
    """Scenario: a caller supplies no lint arguments.

    Invariant: no bare ``--`` is appended, so Cargo never sees a trailing
    separator with nothing behind it.
    """
    argv = lint_rust_lanes.lane_command(manifest_path(), "none", (), ())
    assert "--" not in argv, "an empty lint argument list must add no separator"


def test_lane_command_places_lint_arguments_behind_the_separator() -> None:
    """Scenario: a caller supplies lint arguments.

    Invariant: they follow ``--`` and the feature selection precedes it, so
    Cargo reads the selection and the lint driver reads the denials.
    """
    argv = lint_rust_lanes.lane_command(
        manifest_path(), "python", ("--all-targets",), LINT_ARGS
    )
    separator = argv.index("--")
    assert argv[separator - 1] == "--all-targets", (
        "Cargo arguments must precede the separator"
    )
    assert argv[separator + 1 :] == list(LINT_ARGS), (
        "lint arguments must follow the separator"
    )


@manual_lifecycle
def test_every_lane_passing_succeeds(cmd_mox: CmdMox) -> None:
    """Scenario: Clippy accepts the code in both lanes.

    Invariant: the run reports success and both lanes are actually invoked, so
    a clean result cannot come from skipping the work.
    """
    spy = cmd_mox.spy("cargo").runs(rejecting("no-such-feature"))

    cmd_mox.replay()
    assert run_lanes(["none", "python"]) == 0, "every lane passing must succeed"
    cmd_mox.verify()

    assert spy.call_count == 2, "both lanes must actually be linted"


@manual_lifecycle
def test_a_failing_first_lane_fails_the_run(cmd_mox: CmdMox) -> None:
    """Scenario: the first of two lanes rejects the code.

    Invariant: the run fails with that lane's exit code and stops before the
    passing lane. This is the defect the shell loop had, where the later
    lane's success became the target's exit status.
    """
    spy = cmd_mox.spy("cargo").runs(rejecting("none"))

    cmd_mox.replay()
    assert run_lanes(["none", "python"]) == REJECTED, (
        "a failing first lane must fail the run"
    )
    cmd_mox.verify()

    assert spy.call_count == 1, "the run must stop at the failing lane"


@manual_lifecycle
def test_a_failing_last_lane_fails_the_run(cmd_mox: CmdMox) -> None:
    """Scenario: the last of two lanes rejects the code.

    Invariant: the run fails. The shell loop got this case right, so it is
    kept to show the fix did not trade one blind spot for another.
    """
    spy = cmd_mox.spy("cargo").runs(rejecting("python"))

    cmd_mox.replay()
    assert run_lanes(["none", "python"]) == REJECTED, (
        "a failing last lane must fail the run"
    )
    cmd_mox.verify()

    assert spy.call_count == 2, "both lanes must run before the failure"


@manual_lifecycle
def test_the_failing_lane_is_named(
    cmd_mox: CmdMox, capsys: pytest.CaptureFixture[str]
) -> None:
    """Scenario: a lane fails partway through the list.

    Invariant: the output names that lane and the run stops there, so a
    contributor knows which feature selection to reproduce rather than
    inferring it from whichever lane printed last.
    """
    spy = cmd_mox.spy("cargo").runs(rejecting("test-util"))

    cmd_mox.replay()
    assert run_lanes(["none", "test-util", "python"]) == REJECTED, (
        "a failing middle lane must fail the run"
    )
    cmd_mox.verify()

    assert (
        f"# Rust lint lane failed: test-util (exit {REJECTED})"
        in capsys.readouterr().out
    ), "the failing lane must be named in the output"
    assert not any(selects(argv, "python") for argv in lanes_invoked(spy)), (
        "the run must stop before the lane after the failure"
    )


@manual_lifecycle
def test_each_lane_selects_its_own_features(cmd_mox: CmdMox) -> None:
    """Scenario: the driver walks a three-lane list.

    Invariant: each invocation carries that lane's selection and nothing else,
    so a lane cannot silently lint the same feature set as its neighbour.
    """
    spy = cmd_mox.spy("cargo").runs(rejecting("no-such-feature"))

    cmd_mox.replay()
    assert run_lanes(["none", "python", "all"]) == 0, "every lane must pass"
    cmd_mox.verify()

    selections = [
        argv[argv.index("--manifest-path") + 2 : argv.index("--")]
        for argv in lanes_invoked(spy)
    ]
    assert selections == [
        ["--no-default-features"],
        ["--no-default-features", "--features", "python"],
        ["--all-features"],
    ], "each lane must carry its own feature selection"


def test_an_empty_lane_list_is_not_success() -> None:
    """Scenario: the lane list is empty.

    Invariant: the run fails. A lint that checked nothing must not report a
    clean result, which is the same failure mode the script exists to prevent.
    """
    assert (
        lint_rust_lanes.main(
            manifest=manifest_path(),
            lanes=[],
            lint_args=list(LINT_ARGS),
        )
        != 0
    ), "an empty lane list must not report success"
