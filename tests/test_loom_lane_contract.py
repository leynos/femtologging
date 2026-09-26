"""Contract tests for the scheduled lane that executes the Loom models.

Loom models are only useful if something runs them. The ``heavy-tests``
workflow compiled them for months and ran none, and a green scheduled run said
the models type-checked and nothing about the behaviour they describe. These
tests assert the two steps separately: one compiles and is expected to carry
``--no-run``, and one executes and must not.

The assertions are over the parsed workflow document and the tokenized command,
not over the file's text, so re-indenting the workflow or renaming a nearby step
cannot break them and cannot hide a change either.
"""

from __future__ import annotations

import re
import shlex
import typing as typ

import pytest

from tests.make_contract_helpers import (
    objects,
    repo_root,
    sole_workflow_step,
    workflow_job,
)

_WORKFLOW: typ.Final = ".github/workflows/heavy-tests.yml"
_JOB: typ.Final = "heavy"
_COMPILE_STEP: typ.Final = "Compile Loom heavy tests"
_EXECUTE_STEP: typ.Final = "Run Loom models"

# `shlex.split` keeps an assignment prefix as one token, so the Loom
# configuration is asserted as the whole token rather than as two.
_LOOM_RUSTFLAGS: typ.Final = "RUSTFLAGS=--cfg loom"

# The name filter Cargo applies to the test binary. It must sit before the `--`
# separator, because everything after it belongs to the harness.
_FILTER: typ.Final = "loom_"

# The harness argument. `--include-ignored` rather than `--ignored`: the latter
# runs only tests marked `#[ignore]`, and five of the six models carry no such
# attribute, so it would select one model and report success.
_HARNESS_ARGUMENT: typ.Final = "--include-ignored"

# The checker the execution step runs Cargo under, and the list of models it
# requires a run to report as passed.
_RUNNER: typ.Final = "scripts/run_loom_models.py"
_MODEL_LIST: typ.Final = "rust_extension/tests/heavy/loom-models.txt"
_MODEL_DIRECTORY: typ.Final = "rust_extension/tests/heavy"

# A `#[test]` attribute followed by the function it marks.
_TEST_FUNCTION: typ.Final = re.compile(r"#\[test\]\s*fn\s+(?P<name>\w+)")

# The environment variable that bounds how much of the state space Loom
# explores. Without it a multi-threaded model does not fail, it fails to
# finish.
_PREEMPTION_BOUND: typ.Final = "LOOM_MAX_PREEMPTIONS"


def _step_tokens(step_name: str) -> tuple[str, ...]:
    """Return the tokenized ``run`` command of a named step in the heavy job."""
    step = sole_workflow_step(_WORKFLOW, _JOB, step_name)
    command = step.get("run")
    if not isinstance(command, str):
        msg = f"expected a string `run` command in the {step_name!r} step"
        raise TypeError(msg)
    # A backslash-newline is a line continuation to the shell, which `shlex`
    # would otherwise keep as a newline token.
    return tuple(shlex.split(command.replace("\\\n", " ")))


def _command_words(tokens: tuple[str, ...]) -> tuple[str, ...]:
    """Return the tokens after any leading ``NAME=value`` assignments.

    A shell command may be prefixed with environment assignments, and
    ``shlex.split`` keeps each quoted assignment as one token, so
    ``RUSTFLAGS="--cfg loom" cargo test`` tokenizes as
    ``("RUSTFLAGS=--cfg loom", "cargo", "test")``. Skipping the prefix is what
    lets the contract ask what program actually runs.

    Returns
    -------
    tuple[str, ...]
        The command and its arguments, with any assignment prefix removed.
        Empty when the command is nothing but assignments.
    """
    for index, token in enumerate(tokens):
        if "=" not in token.split(" ", 1)[0]:
            return tokens[index:]
    return ()


def _cargo_words(step_name: str) -> tuple[str, ...]:
    """Return the Cargo command a Loom step runs, without the checker around it.

    The execution step runs Cargo under ``scripts/run_loom_models.py``, after
    the checker's own ``--``; the compile step runs Cargo directly. Anything
    else is returned as it stands, so the assertions that it is ``cargo test``
    fail on it.

    Returns
    -------
    tuple[str, ...]
        The Cargo program and its arguments.
    """
    words = _command_words(_step_tokens(step_name))
    if words[:3] == ("uv", "run", "--script") and "--" in words:
        return words[words.index("--") + 1 :]
    return words


def _subsequence_at(tokens: tuple[str, ...], wanted: tuple[str, ...]) -> bool:
    """Return whether `wanted` appears in `tokens` as a contiguous run."""
    if not wanted:
        return True
    width = len(wanted)
    return any(
        tokens[start : start + width] == wanted
        for start in range(len(tokens) - width + 1)
    )


def test_the_execution_step_selects_the_loom_configuration() -> None:
    """Scenario: the lane is asked which configuration it runs the models under.

    Invariant: the execution step sets ``--cfg loom``. Without it the model
    modules are not compiled at all, so the command runs no models and the
    lane is green for the wrong reason.
    """
    tokens = _step_tokens(_EXECUTE_STEP)
    assert _LOOM_RUSTFLAGS in tokens, (
        f"the {_EXECUTE_STEP!r} step must set {_LOOM_RUSTFLAGS!r}, saw {tokens!r}"
    )


def test_the_execution_step_runs_the_models_rather_than_compiling_them() -> None:
    """Scenario: the lane is asked whether it executes the models.

    Invariant: the execution step carries no ``--no-run``. That flag is what
    made the previous lane compile the models and stop, which is the defect
    this contract exists to refuse.
    """
    tokens = _cargo_words(_EXECUTE_STEP)
    assert "--no-run" not in tokens, (
        f"the {_EXECUTE_STEP!r} step must execute the models, saw {tokens!r}"
    )


def test_the_execution_step_selects_every_model() -> None:
    """Scenario: the lane is asked which tests it selects.

    Invariant: the ``loom_`` filter reaches Cargo and ``--include-ignored``
    reaches the harness. ``--ignored`` alone would select only
    ``loom_stream_push_delivery``, the one model carrying that attribute, and
    report a passing run of one test where six are expected.

    The two are asserted by their side of the ``--`` separator rather than as
    one fixed sequence. Cargo accepts its own arguments in any order, so a
    contract pinning the whole spelling would refuse a command that is merely
    written differently, and a contract that reports a false positive gets
    switched off.
    """
    tokens = _cargo_words(_EXECUTE_STEP)
    assert "--" in tokens, (
        f"the {_EXECUTE_STEP!r} step must separate Cargo's arguments from the "
        f"harness's with `--`, saw {tokens!r}"
    )
    separator = tokens.index("--")
    cargo_arguments, harness_arguments = tokens[:separator], tokens[separator + 1 :]
    assert _FILTER in cargo_arguments, (
        f"the {_EXECUTE_STEP!r} step must pass the {_FILTER!r} filter to Cargo, "
        f"saw {tokens!r}"
    )
    assert _HARNESS_ARGUMENT in harness_arguments, (
        f"the {_EXECUTE_STEP!r} step must pass {_HARNESS_ARGUMENT!r} to the "
        f"harness, saw {tokens!r}"
    )


def test_the_execution_step_is_bounded() -> None:
    """Scenario: a model explores an interleaving on which a worker never answers.

    Invariant: the step carries a wall-clock timeout. Under the model the
    handlers' timed waits become unbounded, because Loom has no clock, so a
    liveness defect hangs the model rather than failing it. The job timeout is
    the outer bound that turns a hang into a reported failure.
    """
    step = sole_workflow_step(_WORKFLOW, _JOB, _EXECUTE_STEP)
    timeout = step.get("timeout-minutes")
    assert isinstance(timeout, int), (
        f"the {_EXECUTE_STEP!r} step must carry a whole-minute timeout-minutes, "
        f"saw {timeout!r}"
    )
    assert timeout > 0, (
        f"the {_EXECUTE_STEP!r} step's timeout-minutes must be positive, "
        f"saw {timeout!r}"
    )


def test_the_execution_step_runs_last() -> None:
    """Scenario: a model fails.

    Invariant: no step follows the execution step. A step that fails without
    ``continue-on-error`` skips every later step, so placing the models before
    the heavy suite, the clippy lanes and pytest would stop all of those
    running, and a model failure would hide everything else the lane exists to
    report.

    Asserted as the position of the step rather than as the name of whatever
    currently precedes it, so inserting a step anywhere earlier is free and
    inserting one after the models is refused.
    """
    job = workflow_job(_WORKFLOW, _JOB)
    steps = objects(job.get("steps"), subject=f"{_WORKFLOW} job {_JOB!r} steps")
    names = [step.get("name") for step in steps]
    assert names[-1] == _EXECUTE_STEP, (
        f"the {_EXECUTE_STEP!r} step must be the last step in the {_JOB!r} job, "
        f"saw {names!r}"
    )


def test_the_compile_step_still_compiles_only() -> None:
    """Scenario: the cheap compile check is asked what it is for.

    Invariant: it keeps ``--no-run``. Compiling the models is worth doing on
    its own, and keeping it separate is what lets the summary distinguish a
    compilation failure from a model failure. This is the one place
    ``--no-run`` belongs.
    """
    tokens = _step_tokens(_COMPILE_STEP)
    assert "--no-run" in tokens, (
        f"the {_COMPILE_STEP!r} step must compile without running, saw {tokens!r}"
    )
    assert _LOOM_RUSTFLAGS in tokens, (
        f"the {_COMPILE_STEP!r} step must set {_LOOM_RUSTFLAGS!r}, saw {tokens!r}"
    )


@pytest.mark.parametrize("step_name", [_COMPILE_STEP, _EXECUTE_STEP])
def test_each_loom_step_names_the_heavy_target(step_name: str) -> None:
    """Scenario: either Loom step is asked which Cargo target it drives.

    Invariant: both name the `heavy` integration target explicitly. The models
    live only there, and a command without the target would compile or run the
    whole suite under the Loom configuration.
    """
    tokens = _cargo_words(step_name)
    assert _subsequence_at(tokens, ("--test", "heavy")), (
        f"the {step_name!r} step must name the heavy target, saw {tokens!r}"
    )


@pytest.mark.parametrize("step_name", [_COMPILE_STEP, _EXECUTE_STEP])
def test_each_loom_step_actually_invokes_cargo_test(step_name: str) -> None:
    """Scenario: either Loom step is asked what program it runs.

    Invariant: the command is a real ``cargo test`` invocation, and not a
    wrapper, an ``echo``, a ``true``, or anything else that would satisfy every
    other assertion in this file while running no models at all.

    This is the gap the pre-merge table found, and it is the shape the estate
    keeps meeting: every other contract here reads an argument, and an argument
    list means nothing if the program in front of it changed. A step rewritten
    as ``echo cargo test --cfg loom ... -- --include-ignored`` would have passed
    the whole of the rest of this file.

    Environment assignments are skipped before the program is read, because the
    Loom configuration is passed as one, and the two words are asserted
    adjacent so a command merely *mentioning* ``test`` somewhere later does not
    satisfy it.
    """
    words = _cargo_words(step_name)
    assert words[:2] == ("cargo", "test"), (
        f"the {step_name!r} step must run `cargo test`, its command begins "
        f"{words[:3]!r}"
    )


def test_the_execution_step_bounds_loom_s_exploration() -> None:
    """Scenario: the lane is asked how much of the state space it explores.

    Invariant: the execution step sets ``LOOM_MAX_PREEMPTIONS`` in its own
    environment, to a positive whole number.

    Loom explores every interleaving it is allowed to, and the preemption bound
    is what makes that finite in practice. Without it a model with several
    threads does not fail, it simply does not finish: the step burns its
    wall-clock timeout and reports a timeout, which reads as an infrastructure
    problem rather than as the model saying anything. The bound is therefore
    part of what makes the step's result meaningful, not a performance tweak.

    Asserted on the step's own ``env`` rather than on the job's, because a job
    variable would be inherited by the compile step too, where it means
    nothing, and because a reader looking at the step should see the bound the
    step runs under.
    """
    step = sole_workflow_step(_WORKFLOW, _JOB, _EXECUTE_STEP)
    environment = step.get("env")
    assert isinstance(environment, dict), (
        f"the {_EXECUTE_STEP!r} step must carry its own `env` mapping, "
        f"saw {environment!r}"
    )
    bound = environment.get(_PREEMPTION_BOUND)
    assert bound is not None, (
        f"the {_EXECUTE_STEP!r} step must set {_PREEMPTION_BOUND}, "
        f"its environment is {environment!r}"
    )
    assert str(bound).isdigit(), (
        f"{_PREEMPTION_BOUND} must be a whole number, saw {bound!r}"
    )
    assert int(bound) > 0, f"{_PREEMPTION_BOUND} must be positive, saw {bound!r}"


def test_the_execution_step_runs_under_the_model_checker() -> None:
    """Scenario: the lane is asked how it knows the models ran.

    Invariant: Cargo runs under ``scripts/run_loom_models.py``, given the
    committed model list. ``cargo test`` passes with zero tests when the Loom
    configuration is dropped, when the filter matches nothing, and when the
    models are skipped; the checker fails each of those, because it requires
    every listed model to be reported as passed and nothing else.
    """
    words = _command_words(_step_tokens(_EXECUTE_STEP))
    assert words[:4] == ("uv", "run", "--script", _RUNNER), (
        f"the {_EXECUTE_STEP!r} step must run Cargo under {_RUNNER}, its "
        f"command begins {words[:4]!r}"
    )
    runner_arguments = words[4 : words.index("--")] if "--" in words else words[4:]
    assert _subsequence_at(runner_arguments, ("--expected", _MODEL_LIST)), (
        f"the checker must be given {_MODEL_LIST}, saw {runner_arguments!r}"
    )


def test_the_model_list_names_exactly_the_models() -> None:
    """Scenario: a model is added, renamed or deleted.

    Invariant: the list names every ``#[test]`` function in the Loom model
    modules, as libtest reports it (``module::function``), and nothing else.
    A model left off the list would run unchecked, and a stale entry would fail
    every run.
    """
    root = repo_root()
    defined = {
        f"{path.stem}::{match['name']}"
        for path in sorted((root / _MODEL_DIRECTORY).glob("loom_*.rs"))
        for match in _TEST_FUNCTION.finditer(path.read_text(encoding="utf-8"))
    }
    listed = {
        line.strip()
        for line in (root / _MODEL_LIST).read_text(encoding="utf-8").splitlines()
        if line.strip() and not line.strip().startswith("#")
    }
    assert defined, "the Loom model modules define no models"
    assert listed == defined, (
        f"listed but not defined: {sorted(listed - defined)}; "
        f"defined but not listed: {sorted(defined - listed)}"
    )
