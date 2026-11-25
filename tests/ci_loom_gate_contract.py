"""Rules for the pull-request gate that reproduces the scheduled Loom build.

The scheduled ``heavy-tests`` job compiles every target under ``--cfg loom``
before it runs any model. That configuration is a day late by design, so a
library test that refers to a helper compiled out under ``--cfg loom`` can sit
on main overnight. These rules hold the same configuration on every pull
request, as one whole step command, and refuse the shapes that would satisfy a
text search while running nothing: a comment, an ``echo``, a condition, a
tolerated failure or a masked exit status.

Each rule returns one complaint per violation, so the contract test can prove
every judgement against a deliberately broken copy as well as the real file.
The document accessors live in :mod:`tests.make_contract_helpers`.
"""

from __future__ import annotations

import shlex
import typing as typ

from tests.ci_gate_scope_contract import (
    job_tolerance_is_acceptable,
    step_tolerance_is_acceptable,
)
from tests.make_contract_helpers import (
    command_words,
    defaults_shell,
    mapping,
    repo_root,
    strict_safe_load,
)

type Tokens = tuple[str, ...]
type Workflow = dict[str, object]

#: The workflow and job that own the gate.
CI_WORKFLOW: typ.Final = ".github/workflows/ci.yml"
CI_JOB: typ.Final = "build-test"

#: The gate's step name and the scheduled command it must run verbatim.
CI_LOOM_CLIPPY_STEP: typ.Final = "Loom all-targets Clippy"
LOOM_CLIPPY_COMMAND: typ.Final = (
    'RUSTFLAGS="--cfg loom -C link-arg=-fuse-ld=mold" cargo clippy '
    "--manifest-path rust_extension/Cargo.toml --no-default-features "
    "--all-targets -- -D warnings"
)

#: `--cfg loom` changes every crate's fingerprint, so the gate builds apart
#: from the ordinary lanes rather than invalidating them.
LOOM_CLIPPY_TARGET_DIR: typ.Final = "target/loom"

#: Displaces an inline `RUSTFLAGS`, so no scope of the gate may set it.
ENCODED_FLAGS: typ.Final = "CARGO_ENCODED_RUSTFLAGS"

#: The step the gate must follow, because it resolves dependencies first.
FETCH_STEP: typ.Final = "Fetch locked dependencies"

#: Shell fragments that turn a gate into something other than the command it
#: names, or that mask its exit status. The whole-command comparison already
#: rules each out; naming them makes the refusal legible to a reader.
MASK_FRAGMENTS: typ.Final[Tokens] = ("||", "&&", ";", "echo ", "#")


def load_ci_workflow() -> Workflow:
    """Return the parsed pull-request workflow.

    PyYAML's default loader silently keeps the last value for a repeated
    mapping key, so a duplicated `run`, `env` or `defaults` key would replace
    the entry the contract needs to inspect. The loader refuses a duplicate
    before any value is constructed.

    Returns
    -------
    Workflow
        The workflow mapping, suitable for a deep-copied mutation; the strict
        loader raises ``ConstructorError`` when a key is repeated.
    """
    return mapping(
        strict_safe_load((repo_root() / CI_WORKFLOW).read_text(encoding="utf-8")),
        subject=CI_WORKFLOW,
    )


def ci_steps(document: Workflow) -> list[dict[str, object]]:
    """Return the job's own step list, so mutations reach the document.

    :func:`tests.make_contract_helpers.objects` returns a new list of the same
    step mappings, which is enough for reading and silently loses a removal,
    so this accessor returns the document's own list instead.

    Returns
    -------
    list[dict[str, object]]
        The job's step mappings, in document order.

    Raises
    ------
    TypeError
        If the job's ``steps`` entry is not a list.
    """
    jobs = mapping(document.get("jobs"), subject=f"{CI_WORKFLOW} jobs")
    build_test = mapping(jobs.get(CI_JOB), subject=f"{CI_JOB} job")
    found_steps = build_test.get("steps")
    if not isinstance(found_steps, list):
        msg = f"{CI_JOB} steps must be a list"
        raise TypeError(msg)
    return typ.cast("list[dict[str, object]]", found_steps)


def loom_step(document: Workflow) -> dict[str, object]:
    """Return the sole Loom Clippy step of a mutable workflow copy.

    Returns
    -------
    dict[str, object]
        The step mapping.
    """
    return next(
        step for step in ci_steps(document) if step.get("name") == CI_LOOM_CLIPPY_STEP
    )


def _normalize_run(run: str) -> str:
    """Return a shell run value without line continuations or layout.

    Boundary whitespace is layout: a YAML ``run: |`` scalar keeps its final
    newline, and that newline does not separate a second command. An internal
    unescaped newline does: collapsing it to a space would let a two-line
    script — an assignment on one line and the command on the next — read as
    the single required command, while the shell runs the assignment in a
    child of its own and Cargo never sees ``--cfg loom``. Such a value is
    returned with the newline intact, so the whole-command comparison refuses
    it.

    Returns
    -------
    str
        The normalised command, or the boundary-stripped value when it
        contains an internal unescaped newline.
    """
    stripped = run.strip()
    if "\n" in stripped.replace("\\\n", ""):
        return stripped
    return " ".join(stripped.replace("\\\n", " ").split())


def _shell_problems(
    document: Workflow,
    build_test: dict[str, object],
    step: dict[str, object],
) -> list[str]:
    """Return complaints about overrides of how the gate's command runs.

    A ``shell`` template such as ``bash -n {0}`` makes GitHub parse the script
    without executing it, so the gate passes while Clippy never runs. The
    effective shell is the step's own setting, else the job's
    ``defaults.run.shell``, else the workflow's; all three are held to the
    default executor by refusing an override at any of them.

    Returns
    -------
    list[str]
        One complaint per scope that overrides the shell.
    """
    problems: list[str] = []
    if "shell" in step:
        problems.append(f"the {CI_LOOM_CLIPPY_STEP!r} step must not override the shell")
    if defaults_shell(build_test) is not None:
        problems.append(f"the {CI_JOB} job must not set defaults.run.shell")
    if defaults_shell(document) is not None:
        problems.append(f"{CI_WORKFLOW} must not set defaults.run.shell")
    return problems


def _run_problems(step: dict[str, object]) -> list[str]:
    """Return the complaints about what the gate step executes.

    The whole ``run`` value is compared, after normalising line continuations
    and layout, so a comment, an ``echo``, a trailing status mask, or a second
    newline-separated command cannot satisfy the contract by leaving the
    command text in place.

    Returns
    -------
    list[str]
        One complaint per masking, whole-command or program failure.
    """
    run = step.get("run")
    normalized = _normalize_run(run) if isinstance(run, str) else ""
    problems = [
        f"the {CI_LOOM_CLIPPY_STEP!r} step must not contain {fragment!r}"
        for fragment in MASK_FRAGMENTS
        if fragment in normalized
    ]
    if normalized != LOOM_CLIPPY_COMMAND:
        problems.append(
            f"the {CI_LOOM_CLIPPY_STEP!r} step must run exactly the scheduled "
            f"Loom Clippy command, saw {run!r}"
        )
        return problems
    words = command_words(tuple(shlex.split(normalized)))
    if words[:2] != ("cargo", "clippy"):
        problems.append(
            f"the {CI_LOOM_CLIPPY_STEP!r} step must invoke cargo clippy, its "
            f"command begins {words[:3]!r}"
        )
    return problems


def _gate_problems(build_test: dict[str, object], step: dict[str, object]) -> list[str]:
    """Return the complaints about conditions and failure tolerance.

    Returns
    -------
    list[str]
        One complaint per condition or tolerated failure on the job or step.
    """
    problems: list[str] = []
    if "if" in step:
        problems.append(f"the {CI_LOOM_CLIPPY_STEP!r} step must carry no condition")
    if not step_tolerance_is_acceptable(step):
        problems.append(
            f"the {CI_LOOM_CLIPPY_STEP!r} step must not tolerate its own failure"
        )
    if "if" in build_test:
        problems.append(f"the {CI_JOB} job must carry no condition")
    if not job_tolerance_is_acceptable(build_test):
        problems.append(
            f"the {CI_JOB} job must not tolerate failure outside a matrix leg"
        )
    return problems


def _target_problems(step: dict[str, object]) -> list[str]:
    """Return the complaint when the gate shares the ordinary target directory.

    Returns
    -------
    list[str]
        An empty list when the step sets its own ``target/loom`` directory.
    """
    environment = step.get("env")
    if isinstance(environment, dict) and (
        environment.get("CARGO_TARGET_DIR") == LOOM_CLIPPY_TARGET_DIR
    ):
        return []
    return [
        f"the {CI_LOOM_CLIPPY_STEP!r} step must build into {LOOM_CLIPPY_TARGET_DIR}"
    ]


def _order_problems(found_steps: list[dict[str, object]]) -> list[str]:
    """Return the complaints about the gate's position after the fetch.

    Returns
    -------
    list[str]
        An empty list when the gate follows the locked dependency fetch.
    """
    names = [found.get("name") for found in found_steps]
    if FETCH_STEP not in names:
        return [f"{CI_JOB} must retain its {FETCH_STEP!r} step"]
    if names.index(CI_LOOM_CLIPPY_STEP) <= names.index(FETCH_STEP):
        return [f"the {CI_LOOM_CLIPPY_STEP!r} step must run after {FETCH_STEP!r}"]
    return []


def loom_clippy_problems(document: Workflow) -> list[str]:
    """Return every complaint about the pull-request Loom Clippy gate.

    The pull request must run the exact command the scheduled heavy job runs,
    as a whole step command. A gate rewritten as a comment, an ``echo``, a
    conditional, or a command whose status is masked would satisfy a check that
    only searched the step text, so the comparison is over the whole value.

    Parameters
    ----------
    document : Workflow
        The parsed pull-request workflow.

    Returns
    -------
    list[str]
        One complaint per missing, conditional, masked or misplaced
        requirement. An empty list means the gate is real.
    """
    jobs = mapping(document.get("jobs"), subject=f"{CI_WORKFLOW} jobs")
    build_test = mapping(jobs.get(CI_JOB), subject=f"{CI_JOB} job")
    found_steps = ci_steps(document)
    matches = [step for step in found_steps if step.get("name") == CI_LOOM_CLIPPY_STEP]
    if len(matches) != 1:
        return [f"{CI_JOB} must have exactly one {CI_LOOM_CLIPPY_STEP!r} step"]
    step = matches[0]
    return [
        *_run_problems(step),
        *_gate_problems(build_test, step),
        *_target_problems(step),
        *_order_problems(found_steps),
        *_shell_problems(document, build_test, step),
        *_encoded_flags_problems(document, build_test, step),
    ]


def _encoded_flags_problems(
    document: Workflow,
    build_test: dict[str, object],
    step: dict[str, object],
) -> list[str]:
    """Return complaints about encoded compiler flags that displace the command's.

    Cargo gives `CARGO_ENCODED_RUSTFLAGS` precedence over an inline
    `RUSTFLAGS`, so a value set at step, job or workflow scope would remove
    `--cfg loom` while the command text kept it. The gate names its flags
    inline, so no scope may carry the encoded variable.

    Returns
    -------
    list[str]
        One complaint per scope that sets the variable.
    """
    problems: list[str] = []
    if _environment_has(step, ENCODED_FLAGS):
        problems.append(
            f"the {CI_LOOM_CLIPPY_STEP!r} step must not set {ENCODED_FLAGS}"
        )
    if _environment_has(build_test, ENCODED_FLAGS):
        problems.append(f"the {CI_JOB} job must not set {ENCODED_FLAGS}")
    if _environment_has(document, ENCODED_FLAGS):
        problems.append(f"{CI_WORKFLOW} must not set {ENCODED_FLAGS}")
    return problems


def _environment_has(container: dict[str, object], name: str) -> bool:
    """Return whether a workflow, job or step environment sets `name`.

    Returns
    -------
    bool
        True when the container's `env` mapping defines `name`.
    """
    environment = container.get("env")
    return isinstance(environment, dict) and name in environment
