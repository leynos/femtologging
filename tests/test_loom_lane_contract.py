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

import shlex
import typing as typ

import pytest

from tests.make_contract_helpers import objects, sole_workflow_step, workflow_job

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


def _step_tokens(step_name: str) -> tuple[str, ...]:
    """Return the tokenized ``run`` command of a named step in the heavy job."""
    step = sole_workflow_step(_WORKFLOW, _JOB, step_name)
    command = step.get("run")
    if not isinstance(command, str):
        msg = f"expected a string `run` command in the {step_name!r} step"
        raise TypeError(msg)
    return tuple(shlex.split(command))


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
    tokens = _step_tokens(_EXECUTE_STEP)
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
    tokens = _step_tokens(_EXECUTE_STEP)
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
    """Scenario: the execution step fails, as it is expected to until the seam lands.

    Invariant: no step follows it. A step that fails without
    ``continue-on-error`` skips every later step, so placing the models before
    the heavy suite, the clippy lanes and pytest would stop all of those
    running. The lane would then report the Loom gap by suppressing everything
    else it exists to report, which is a worse lane than the silent one this
    milestone replaces.

    Asserted as the position of the step rather than as the name of whatever
    currently precedes it, so inserting a step anywhere earlier is free and
    inserting one after the models is refused.
    """
    job = workflow_job(_WORKFLOW, _JOB)
    steps = objects(job.get("steps"), subject=f"{_WORKFLOW} job {_JOB!r} steps")
    names = [step.get("name") for step in steps]
    assert names[-1] == _EXECUTE_STEP, (
        f"the {_EXECUTE_STEP!r} step must be the last step in the {_JOB!r} job "
        f"while it is knowingly failing, saw {names!r}"
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
    tokens = _step_tokens(step_name)
    assert _subsequence_at(tokens, ("--test", "heavy")), (
        f"the {step_name!r} step must name the heavy target, saw {tokens!r}"
    )
