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
nearby step cannot break them and cannot hide a change either.
"""

from __future__ import annotations

import shlex
import tomllib
import typing as typ

from tests.make_contract_helpers import repo_root, sole_workflow_step

_WORKFLOW: typ.Final = ".github/workflows/heavy-tests.yml"
_JOB: typ.Final = "heavy"
_STEP: typ.Final = "Run formatters and tests"

#: The driver the step delegates the matrix to.
_RUNNER: typ.Final = "scripts/run_cargo_feature_tests.py"

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


def _step() -> dict[str, object]:
    """Return the heavy job's formatter-and-test step.

    Returns
    -------
        The step mapping from the parsed workflow.
    """
    return sole_workflow_step(_WORKFLOW, _JOB, _STEP)


def _run_text() -> str:
    """Return the step's shell script.

    Returns
    -------
        The ``run`` script, with backslash-newline continuations joined so
        each command is one line.

    Raises
    ------
    TypeError
        If the step has no string ``run`` command.
    """
    command = _step().get("run")
    if not isinstance(command, str):
        message = f"the {_STEP!r} step must have a string `run` command"
        raise TypeError(message)
    return command.replace("\\\n", " ")


def _commands() -> list[list[str]]:
    """Return the step's shell script tokens, one entry per command.

    Returns
    -------
        The tokenized commands, in the order the step runs them.
    """
    return [shlex.split(line) for line in _run_text().splitlines() if line.strip()]


def _environment() -> dict[str, object]:
    """Return the step's own environment mapping.

    Returns
    -------
        The ``env`` mapping, or an empty mapping when the step has none.
    """
    environment = _step().get("env")
    return environment if isinstance(environment, dict) else {}


def test_the_matrix_runs_through_the_python_driver() -> None:
    """Scenario: the lane is asked what runs the feature matrix.

    Invariant: the step invokes ``scripts/run_cargo_feature_tests.py`` under
    ``uv run --script`` exactly once. Cargo itself must not be looped over
    inline: a shell loop reports only its last command's status.
    """
    drivers = [
        command
        for command in _commands()
        if command[:4] == ["uv", "run", "--script", _RUNNER]
    ]
    assert len(drivers) == 1, (
        f"the {_STEP!r} step must run {_RUNNER} once, found {drivers!r}"
    )


def test_the_driver_is_given_every_supported_selection() -> None:
    """Scenario: a feature is added to or removed from the matrix.

    Invariant: the driver receives exactly the four selections the nightly
    lane is defined to cover, including the reserved ``none`` entry that
    selects no features.
    """
    configured = _environment().get("INPUT_FEATURE_SETS")
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
    environment = _environment()
    assert environment.get("INPUT_MANIFEST") == _MANIFEST, (
        f"the driver must test {_MANIFEST}, saw {environment.get('INPUT_MANIFEST')!r}"
    )
    harness = environment.get("INPUT_TEST_ARGS")
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
    interpreter = _environment().get("PYO3_PYTHON")
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
    text = _run_text()
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
    commands = _commands()
    driver_at = next(
        at
        for at, command in enumerate(commands)
        if command[:4] == ["uv", "run", "--script", _RUNNER]
    )
    pytest_at = next(
        at
        for at, command in enumerate(commands)
        if any(word == "pytest" for word in command)
    )
    assert driver_at < pytest_at, (
        f"the feature matrix must run before pytest, saw {commands!r}"
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
