"""Contract tests for the heavy lane's Cargo feature matrix.

The heavy suite must run the ignored tests once per supported feature
selection, and every selection must be able to fail the step. That used to be
an inline shell ``for`` loop: a loop reports only the status of its last
command, so a rejection in an earlier selection was discarded unless every
call carried a guard. The workflow now delegates to
``scripts/run_cargo_feature_tests.py``; these tests hold the wiring in place
so the matrix cannot quietly shrink, lose its harness selection, or become an
inline loop again.

The assertions are over the parsed workflow document, not over the file's
text, so re-indenting the workflow or renaming a nearby step cannot break
them and cannot hide a change either.

Whether a failed driver fails the step is asked of bash rather than of the
text: the step's own script is run under the shell template the workflow
declares, with ``cargo``, ``make`` and ``uv`` stubbed, and a failing driver's
effect on the step's status is observed. Reading the text for that answer
means modelling quoting, separators, redirections, here-documents and shell
flags, and every model has constructs it does not handle — each one a script
that passes while discarding the verdict. See
:mod:`tests.workflow_step_execution`.
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
    effective_shell,
    environment,
    run_text,
    step,
)
from tests.make_contract_helpers import repo_root, workflow_job
from tests.workflow_step_execution import StepRun, driver_calls, run_script

if typ.TYPE_CHECKING:
    import pathlib

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

    Invariant: the step and its job run unconditionally and do not tolerate
    their own failure, so the YAML cannot absorb it.
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


def test_a_failing_driver_fails_the_step(tmp_path: pathlib.Path) -> None:
    """Scenario: the driver reports a failing feature selection.

    Invariant: the step's own script exits non-zero under the shell the
    workflow declares, so the job fails. This is asked of bash by running the
    script, not of the text: a suppression suffix, a disabling ``set``, a
    redirection that hides a later command, a here-document body, a
    conditional list, or a shell template without fail-fast behaviour all
    leave the driver failing while the step continues, and each has defeated
    a text-level check.

    The driver is stubbed to report failure, so no Rust is compiled.
    """
    status, calls = run_script(
        StepRun(
            script=run_text(),
            shell_template=effective_shell(),
            driver_exit=101,
            runner=_RUNNER,
            workdir=tmp_path,
        )
    )

    assert driver_calls(calls, _RUNNER), (
        f"the step must actually invoke the feature-matrix driver; it ran {calls!r}"
    )
    assert status != 0, (
        f"a failing driver must fail the step; the script exited 0 after {calls!r}"
    )


def test_a_passing_driver_lets_the_step_finish(tmp_path: pathlib.Path) -> None:
    """Scenario: the driver reports success.

    Invariant: the step's script exits zero, so the rule above is a real
    control rather than a step that always fails.
    """
    status, calls = run_script(
        StepRun(
            script=run_text(),
            shell_template=effective_shell(),
            driver_exit=0,
            runner=_RUNNER,
            workdir=tmp_path,
        )
    )

    assert driver_calls(calls, _RUNNER), "the step must invoke the driver"
    assert status == 0, f"a passing driver must let the step finish; saw {calls!r}"


#: A script that captures the driver's status and then hides it behind a
#: passing command. `set +e` stops the capture from aborting the script, and
#: the pipeline's status is its last command's, so `-e` alone does not see the
#: failure; only `-o pipefail` rejects it.
_STATUS_LAUNDERING: typ.Final[str] = (
    "set +e\nbash -c 'exit 101'\nstatus=$?\n(exit \"$status\") | cat\n"
)


def test_the_implicit_shell_matches_the_runner(tmp_path: pathlib.Path) -> None:
    """Scenario: a step declares no shell and launders the driver's status.

    Invariant: the harness reproduces GitHub's implicit default exactly, so
    this script really does exit 0 there and the contract only accepts a
    workflow whose shell rejects it. GitHub runs ``bash -e {0}`` when no
    ``shell`` is declared, and that is *not* ``shell: bash``, which adds
    ``-o pipefail``. A harness that ran the stricter command would report the
    failure the runner misses, and the contract would pass a step that cannot
    propagate the driver's verdict.
    """
    status, _ = run_script(
        StepRun(
            script=_STATUS_LAUNDERING,
            shell_template=None,
            driver_exit=101,
            runner=_RUNNER,
            workdir=tmp_path,
        )
    )
    assert status == 0, (
        "the implicit default has no pipefail, so the laundered status must "
        f"escape it; saw exit {status}"
    )

    strict, _ = run_script(
        StepRun(
            script=_STATUS_LAUNDERING,
            shell_template="bash",
            driver_exit=101,
            runner=_RUNNER,
            workdir=tmp_path / "strict",
        )
    )
    assert strict != 0, (
        "an explicit bash adds pipefail and must reject the laundered status; "
        f"saw exit {strict}"
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
    assert tuple(harness.split()) == (_HARNESS_ARGUMENT,), (
        f"the harness selection must be exactly {_HARNESS_ARGUMENT!r}, saw {harness!r}"
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
