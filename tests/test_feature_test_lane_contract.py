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

import pytest
import yaml

from tests.make_contract_helpers import (
    repo_root,
    sole_workflow_step,
    workflow_job,
)

if typ.TYPE_CHECKING:
    import collections.abc as cabc

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


def _tokenize(script: str) -> list[list[str]]:
    """Return one script's tokens, one entry per command.

    ``comments=True`` reads a trailing ``#`` as a comment, so a comment
    containing an apostrophe does not open a quotation the line never closes.
    Quoted ``#`` characters stay argument content. A comment-only line
    tokenizes to nothing and is dropped.

    Returns
    -------
        The tokenized commands, in the order the script runs them.
    """
    return [
        tokens
        for line in script.splitlines()
        if (tokens := shlex.split(line, comments=True))
    ]


def _commands() -> list[list[str]]:
    """Return the step's shell script tokens, one entry per command.

    Returns
    -------
        The tokenized commands, in the order the step runs them.
    """
    return _tokenize(_run_text())


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
    ``uv run --script`` exactly once, and that invocation is the whole
    command. Cargo itself must not be looped over inline: a shell loop reports
    only its last command's status. The whole-command check is what refuses a
    suppression suffix such as ``|| true``, which would otherwise pass every
    prefix assertion while discarding the driver's verdict.
    """
    drivers = [
        command
        for command in _commands()
        if command[:4] == ["uv", "run", "--script", _RUNNER]
    ]
    assert drivers == [["uv", "run", "--script", _RUNNER]], (
        f"the {_STEP!r} step must run {_RUNNER} once as the whole command, "
        f"found {drivers!r}"
    )


def _declared_shell(mapping: cabc.Mapping[str, object]) -> object:
    """Return one mapping's ``defaults.run.shell``, or ``None`` when absent.

    The parameter is a ``Mapping`` rather than a ``dict``, which is invariant
    in its value type: the parsed workflow's nested ``dict[str, Any]`` is not
    a ``dict[str, object]``, and the narrower annotation rejects a real
    workflow even though every value it reads is typed.

    Returns
    -------
        The declared shell template, or ``None``.
    """
    defaults = mapping.get("defaults")
    if not isinstance(defaults, dict):
        return None
    run = defaults.get("run")
    if not isinstance(run, dict):
        return None
    return run.get("shell")


def _effective_shell() -> object:
    """Return the shell the step actually runs under.

    GitHub resolves ``shell`` from the step, then the job's
    ``defaults.run.shell``, then the workflow's, then the implicit default for
    the runner's platform. Reading only the step would accept an absent key
    while a job-level template replaced the fail-fast default.

    Returns
    -------
        The effective shell template, or ``None`` for the built-in default.

    Raises
    ------
    TypeError
        If the workflow does not parse to a mapping.
    """
    document = yaml.safe_load((repo_root() / _WORKFLOW).read_text(encoding="utf-8"))
    if not isinstance(document, dict):
        message = f"{_WORKFLOW} must parse to a mapping"
        raise TypeError(message)
    job = workflow_job(_WORKFLOW, _JOB)
    return (
        _step().get("shell")
        or _declared_shell(job)
        or _declared_shell(typ.cast("dict[str, object]", document))
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
    step = _step()
    assert "if" not in step, f"the {_STEP!r} step must run unconditionally"
    assert step.get("continue-on-error") in {None, False}, (
        f"the {_STEP!r} step must not tolerate its own failure"
    )
    job = workflow_job(_WORKFLOW, _JOB)
    assert job.get("continue-on-error") in {None, False}, (
        f"the {_JOB!r} job must not tolerate its own failure"
    )
    shell = _effective_shell()
    # An absent shell means GitHub's built-in `bash -e {0}` on Linux, and the
    # built-in `bash` and `sh` names also run with `-e`. A custom template has
    # to say so itself.
    assert shell is None or shell in {"bash", "sh"} or "-e" in str(shell), (
        f"the {_STEP!r} step's effective shell must exit on error, saw {shell!r}"
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


@pytest.mark.parametrize(
    ("script", "expected"),
    [
        pytest.param(
            "# Check the driver's result\nuv run --script x.py",
            [["uv", "run", "--script", "x.py"]],
            id="comment-apostrophe",
        ),
        pytest.param(
            "uv run --script x.py  # trailing note",
            [["uv", "run", "--script", "x.py"]],
            id="trailing-comment",
        ),
        pytest.param(
            'echo "a # b"',
            [["echo", "a # b"]],
            id="quoted-hash",
        ),
        pytest.param("# only a comment", [], id="comment-only"),
    ],
)
def test_the_reader_ignores_shell_comments(
    script: str, expected: list[list[str]]
) -> None:
    """Scenario: the step gains a comment, with or without an apostrophe.

    Invariant: comments are not read as syntax, and a quoted ``#`` stays
    argument content. Without comment handling, ``shlex.split`` raises on the
    apostrophe and a comment-only workflow edit breaks the contract.
    """
    assert _tokenize(script) == expected, script


def test_a_custom_job_shell_default_is_resolved() -> None:
    """Scenario: the job declares a shell template that does not fail fast.

    Invariant: the resolver reports the job's template rather than the absent
    step-level key, so ``bash {0}`` cannot pass as the safe built-in default.
    """
    assert _effective_shell() is None, (
        "the committed workflow must run under the built-in fail-fast shell"
    )
    job = {"defaults": {"run": {"shell": "bash {0}"}}}
    assert _declared_shell(job) == "bash {0}", "a job-level shell default must be read"
    workflow = {"defaults": {"run": {"shell": "python {0}"}}}
    assert _declared_shell(workflow) == "python {0}", (
        "a workflow-level shell default must be read"
    )
