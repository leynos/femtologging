"""Contract tests for the heavy lane's Cargo feature matrix.

The heavy suite must run the ignored tests once per supported feature
selection, and every selection must be able to fail the step. That used to be
an inline shell ``for`` loop: a loop reports only the status of its last
command, so a rejection in an earlier selection was discarded unless every
call carried a guard. The workflow now delegates to
``scripts/run_cargo_feature_tests.py``; these tests hold the wiring in place
so the matrix cannot quietly shrink, lose its harness selection, or become an
inline loop again.

The assertions are over the parsed workflow document and the tokenized
command, not over the file's text, so re-indenting the workflow or renaming a
nearby step cannot break them and cannot hide a change either. The readers
live in :mod:`tests.feature_lane_step_readers`, and the reader properties are
in ``test_feature_lane_step_reader_properties.py``.
"""

from __future__ import annotations

import shlex
import tomllib
import typing as typ

from tests.feature_lane_step_readers import (
    JOB,
    RUNNER,
    RUNNER_COMMAND,
    STEP,
    WORKFLOW,
    commands,
    disables_errexit,
    effective_shell,
    environment,
    run_text,
    step,
)
from tests.make_contract_helpers import repo_root, workflow_job

_WORKFLOW: typ.Final = WORKFLOW
_JOB: typ.Final = JOB
_STEP: typ.Final = STEP
_RUNNER: typ.Final = RUNNER

#: The manifest every invocation tests.
_MANIFEST: typ.Final = "rust_extension/Cargo.toml"

#: The selections the nightly matrix must cover. ``none`` is the driver's
#: reserved name for ``--no-default-features`` with no ``--features`` flag.
_FEATURE_SETS: typ.Final[tuple[str, ...]] = (
    "none",
    "python",
    "log-compat",
    "python,log-compat",
)

#: The harness selection. Without it the run tests the ordinary suite, which
#: passes without touching the ignored tests the heavy lane exists to run.
_HARNESS_ARGUMENT: typ.Final = "--ignored"

#: The inline shell shape the driver replaced.
_INLINE_LOOP_MARKERS: typ.Final[tuple[str, ...]] = (
    "feature_sets=(",
    "for feature_set in",
)


def test_the_matrix_runs_through_the_python_driver() -> None:
    """Scenario: the lane is asked what runs the feature matrix.

    Invariant: the step invokes ``scripts/run_cargo_feature_tests.py`` under
    ``uv run --script`` exactly once, and that invocation is the whole
    command. Cargo itself must not be looped over inline: a shell loop reports
    only its last command's status. The whole-command check is what refuses a
    suppression suffix such as ``|| true``, which would otherwise pass every
    prefix assertion while discarding the driver's verdict.
    """
    drivers = [
        command
        for command in commands()
        if command[: len(RUNNER_COMMAND)] == list(RUNNER_COMMAND)
    ]
    assert drivers == [list(RUNNER_COMMAND)], (
        f"the {_STEP!r} step must run {_RUNNER} once as the whole command, "
        f"found {drivers!r}"
    )


def test_the_driver_failure_cannot_be_suppressed() -> None:
    """Scenario: a selection fails and the step swallows the verdict.

    Invariant: the step carries no ``if``, neither the step nor the job
    tolerates its own failure, and the shell running it exits on error.
    ``|| true`` is refused by requiring the driver to be the whole command;
    these rules close the surrounding ways to lose the same failure.

    The shell is resolved through the step, the job and the workflow, because
    a ``defaults.run.shell`` template such as ``bash {0}`` replaces the
    built-in ``bash -e {0}`` and an absent step-level key would otherwise read
    as the safe default.
    """
    found = step()
    assert "if" not in found, f"the {_STEP!r} step must run unconditionally"
    assert found.get("continue-on-error") in {None, False}, (
        f"the {_STEP!r} step must not tolerate its own failure"
    )
    job = workflow_job(_WORKFLOW, _JOB)
    assert job.get("continue-on-error") in {None, False}, (
        f"the {_JOB!r} job must not tolerate its own failure"
    )
    shell = effective_shell()
    # An absent shell means GitHub's built-in `bash -e {0}` on Linux, and the
    # built-in `bash` and `sh` names also run with `-e`. A custom template has
    # to say so itself.
    assert shell is None or shell in {"bash", "sh"} or "-e" in str(shell), (
        f"the {_STEP!r} step's effective shell must exit on error, saw {shell!r}"
    )


def test_the_script_never_disables_errexit_before_the_driver() -> None:
    """Scenario: the step runs ``set +e`` ahead of the driver.

    Invariant: no command turns exit-on-error off, so a failed driver still
    fails the step. Without this, ``set +e`` followed by the driver and a
    passing ``pytest`` reports a green step while every selection may have
    failed.
    """
    offenders = [command for command in commands() if disables_errexit(command)]
    assert not offenders, (
        f"the {_STEP!r} step must not disable exit-on-error, found {offenders!r}"
    )


def test_the_driver_is_given_every_supported_selection() -> None:
    """Scenario: a feature is added to or removed from the matrix.

    Invariant: the driver receives exactly the four selections the nightly
    lane is defined to cover, including the reserved ``none`` entry that
    selects no features.
    """
    configured = environment().get("INPUT_FEATURE_SETS")
    assert isinstance(configured, str), (
        f"the {_STEP!r} step must set INPUT_FEATURE_SETS, saw {configured!r}"
    )
    assert tuple(shlex.split(configured)) == _FEATURE_SETS, (
        f"the matrix must cover {_FEATURE_SETS}, saw {configured!r}"
    )


def test_the_driver_is_given_the_manifest_and_the_harness_selection() -> None:
    """Scenario: the lane is asked what it tests and which tests it runs.

    Invariant: the manifest names the Rust crate, and ``--ignored`` reaches
    the harness. Without the harness argument the ordinary suite runs and
    passes without touching the ignored tests.
    """
    exported = environment()
    assert exported.get("INPUT_MANIFEST") == _MANIFEST, (
        f"the driver must test {_MANIFEST}, saw {exported.get('INPUT_MANIFEST')!r}"
    )
    harness = exported.get("INPUT_TEST_ARGS")
    assert isinstance(harness, str), (
        f"the {_STEP!r} step must set INPUT_TEST_ARGS, saw {harness!r}"
    )
    assert _HARNESS_ARGUMENT in harness.split(), (
        f"the harness selection must include {_HARNESS_ARGUMENT}, saw {harness!r}"
    )


def test_the_driver_links_the_setup_python_interpreter() -> None:
    """Scenario: ``uv run --script`` puts its own interpreter first on PATH.

    Invariant: ``PYO3_PYTHON`` names the interpreter ``setup-python``
    installed. PyO3 links whichever Python it finds, and a test binary linked
    against a different minor version cannot load on the runner.
    """
    interpreter = environment().get("PYO3_PYTHON")
    assert isinstance(interpreter, str), (
        f"the {_STEP!r} step must set PYO3_PYTHON, saw {interpreter!r}"
    )
    assert "env.pythonLocation" in interpreter, (
        f"PYO3_PYTHON must derive from the setup-python location, saw {interpreter!r}"
    )


def test_the_matrix_is_not_an_inline_shell_loop() -> None:
    """Scenario: someone reintroduces the loop the driver replaced.

    Invariant: neither the bash array nor the ``for`` loop returns to the
    step. Both are what let an earlier selection's failure be discarded.
    """
    text = run_text()
    for marker in _INLINE_LOOP_MARKERS:
        assert marker not in text, (
            f"the {_STEP!r} step must not regain an inline loop ({marker!r})"
        )


def test_the_matrix_runs_before_pytest() -> None:
    """Scenario: a feature selection fails.

    Invariant: the driver runs before ``pytest``, matching the order the
    inline loop had. A selection that fails stops the step, so it must not
    become invisible behind an earlier test run.
    """
    found = commands()
    driver_at = next(
        at
        for at, command in enumerate(found)
        if command[: len(RUNNER_COMMAND)] == list(RUNNER_COMMAND)
    )
    pytest_at = next(at for at, command in enumerate(found) if "pytest" in command)
    assert driver_at < pytest_at, (
        f"the feature matrix must run before pytest, saw {found!r}"
    )


def test_every_named_selection_is_a_declared_cargo_feature() -> None:
    """Scenario: a feature is renamed in ``Cargo.toml``.

    Invariant: every name in the matrix other than the reserved ``none``
    exists in the crate's ``[features]`` table, so a rename fails here rather
    than in the nightly run.
    """
    manifest = tomllib.loads((repo_root() / _MANIFEST).read_text(encoding="utf-8"))
    declared = set(manifest.get("features", {}))
    named = {
        feature
        for selection in _FEATURE_SETS
        for feature in selection.split(",")
        if feature != "none"
    }
    assert named <= declared, (
        f"undeclared Cargo features in the matrix: {sorted(named - declared)}"
    )


def test_the_driver_exists() -> None:
    """Scenario: the workflow names a script that is not committed.

    Invariant: the driver file exists in the repository.
    """
    assert (repo_root() / _RUNNER).is_file(), f"{_RUNNER} must exist"
