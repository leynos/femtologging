"""Readers that hold the split between PR coverage and main publication.

CV-005 keeps coverage measurement and its ratchet in the pull-request lane and
moves both persistent outputs, the CodeScene upload and the ratchet baseline,
to one default-branch publisher. These readers answer the questions that split
turns on: which workflows a pull request can reach, where a report is
generated, handed over and uploaded, what a build matrix lists, and whether
the two lanes measure the same population. The protected environment and the
token it admits have their own readers in
:mod:`tests.coverage_environment_helpers`. Each reader fails closed on a shape
it cannot prove rather than assuming the safe reading.
"""

from __future__ import annotations

import typing as typ

from tests.workflow_document_helpers import repository_workflows, require

if typ.TYPE_CHECKING:
    import collections.abc as cabc

# These name the step and its command: never a secret value.
GENERATE_MARKER: typ.Final[str] = "generate-coverage"
BASELINE_MARKER: typ.Final[str] = "publish-coverage-baseline"
UPLOAD_MARKER: typ.Final[str] = "upload-codescene-coverage"
TOKEN_STEP_NAME: typ.Final[str] = "Check CodeScene token"  # ruff: ignore[hardcoded-password-string] a step name, not a value
#: The canonical token-presence step: it writes a boolean output, nothing more.
TOKEN_COMMAND: typ.Final[str] = (
    # ruff: ignore[hardcoded-password-string] names the secret in an expression
    'echo "available=${{ secrets.CS_ACCESS_TOKEN != \'\' }}" >> "$GITHUB_OUTPUT"'
)
#: The upload must require both the token's presence and the permitted branch.
UPLOAD_CONDITION: typ.Final[str] = (
    "${{ steps.codescene-token.outputs.available == 'true' "
    "&& github.ref == 'refs/heads/main' }}"
)
#: Inputs that define what a coverage run measures; both lanes must agree on
#: every one, or the baseline describes another population than the ratchet's.
PARITY_INPUTS: typ.Final[tuple[str, ...]] = (
    "output-path",
    "format",
    "cargo-manifest",
    "with-default-features",
    "features",
    "pytest-workers",
)
#: Events that let pull-request content influence a run. `pull_request_target`
#: runs from the base branch but is triggered by a pull request, so it carries
#: the same risk to the protected environment.
PULL_REQUEST_EVENTS: typ.Final[frozenset[str]] = frozenset({
    "pull_request",
    "pull_request_target",
})
SELF_REPOSITORY: typ.Final[str] = "leynos/femtologging"


def triggers(document: dict[str, typ.Any]) -> dict[str, typ.Any]:
    """Return a workflow's trigger mapping, keyed by event name.

    PyYAML resolves a bare ``on`` key to the boolean ``True`` under YAML 1.1,
    so both the documented and the resolved spellings are accepted when only
    one is present. A document carrying both is refused: one of them would be
    read as the trigger and the other ignored.

    Parameters
    ----------
    document
        A parsed workflow document.

    Returns
    -------
    dict[str, typing.Any]
        One entry per trigger event. A scalar trigger maps to ``None``, a
        sequence to ``None`` per name, and a mapping keeps its settings.

    Raises
    ------
    AssertionError
        If both spellings appear, no trigger is declared, or the trigger is
        not a name, a sequence of names or a mapping.
    """
    documented = "on" in document
    resolved = True in document
    require(not (documented and resolved), "refuse both `on` spellings together")
    raw = document["on"] if documented else document.get(True)
    require(raw is not None, "the workflow must declare triggers")
    match raw:
        case str():
            return {raw: None}
        case list():
            require(
                all(isinstance(name, str) for name in raw),
                "trigger names must all be strings",
            )
            return dict.fromkeys(str(name) for name in raw)
        case dict():
            declared = typ.cast("dict[object, typ.Any]", raw)
            return {str(name): settings for name, settings in declared.items()}
        case _:
            message = "triggers must be a name, a sequence or a mapping"
            raise AssertionError(message)


def pushes_to(events: cabc.Mapping[str, typ.Any], branch: str) -> bool:
    """Report whether `events` runs on a push to `branch`.

    Parameters
    ----------
    events
        A trigger mapping, such as ``triggers(document)``.
    branch
        The branch name a push must name, such as ``main``.

    Returns
    -------
    bool
        Whether a push trigger accepts `branch`. A push trigger without a
        branch filter accepts every branch.
    """
    if "push" not in events:
        return False
    settings = events["push"]
    if settings is None:
        return True
    require(isinstance(settings, dict), "a push trigger must be a mapping")
    branches = settings.get("branches")
    if branches is None:
        return True
    require(isinstance(branches, list), "push branches must be a sequence")
    return branch in [str(name) for name in branches]


def jobs_named(
    documents: cabc.Mapping[str, dict[str, typ.Any]],
) -> cabc.Iterator[tuple[str, str, dict[str, typ.Any]]]:
    """Yield every job of `documents` as its file, id and document.

    Parameters
    ----------
    documents
        File name to parsed workflow document.

    Yields
    ------
    tuple[str, str, dict[str, typing.Any]]
        The workflow file name, the job id and the job document.
    """
    for file_name, document in documents.items():
        jobs = document.get("jobs")
        require(isinstance(jobs, dict), f"{file_name} must declare a jobs mapping")
        for job_id, job in typ.cast("dict[str, object]", jobs).items():
            require(
                isinstance(job, dict), f"{file_name} job {job_id!r} must be a mapping"
            )
            yield file_name, str(job_id), typ.cast("dict[str, typ.Any]", job)


def steps_using(
    documents: cabc.Mapping[str, dict[str, typ.Any]], marker: str
) -> list[tuple[str, str, dict[str, typ.Any]]]:
    """Return every step whose ``uses`` names the action `marker`.

    Parameters
    ----------
    documents
        File name to parsed workflow document.
    marker
        Text identifying the action, such as ``generate-coverage``.

    Returns
    -------
    list[tuple[str, str, dict[str, typing.Any]]]
        The workflow file, job id and step for each match, in document order.
    """
    matches = []
    for file_name, job_id, job in jobs_named(documents):
        steps = job.get("steps") or []
        require(
            isinstance(steps, list), f"{file_name} job {job_id!r} needs a step list"
        )
        for step in steps:
            require(
                isinstance(step, dict), f"{file_name} job {job_id!r} has a bad step"
            )
            if marker in str(step.get("uses", "")):
                matches.append((
                    file_name,
                    job_id,
                    typ.cast("dict[str, typ.Any]", step),
                ))
    return matches


def repository_documents() -> dict[str, dict[str, typ.Any]]:
    """Return every repository workflow, keyed by file name.

    Reads the checked-in workflow files through
    :func:`tests.workflow_document_helpers.repository_workflows`, so this is
    the one reader here that touches the filesystem.

    Returns
    -------
    dict[str, dict[str, typing.Any]]
        Each workflow file name mapped to its parsed document.
    """
    return dict(repository_workflows())


def matrix_versions(job: dict[str, typ.Any], key: str) -> list[str]:
    """Return the values a job's build matrix lists for `key`.

    Parameters
    ----------
    job
        A workflow job document.
    key
        The matrix axis to read, such as ``python-version``.

    Returns
    -------
    list[str]
        The axis values in declared order.
    """
    strategy = job.get("strategy")
    require(isinstance(strategy, dict), "the job must declare a strategy matrix")
    matrix = strategy.get("matrix")
    require(isinstance(matrix, dict), "the job must declare a matrix mapping")
    values = matrix.get(key)
    require(isinstance(values, list), f"the matrix must list {key}")
    require(
        all(isinstance(value, str) for value in values),
        f"every {key} entry must be a string",
    )
    return typ.cast("list[str]", values)


def matrix_includes(job: dict[str, typ.Any]) -> list[dict[str, typ.Any]]:
    """Return the entries of a job's ``include`` matrix axis.

    Parameters
    ----------
    job
        A workflow job document.

    Returns
    -------
    list[dict[str, typing.Any]]
        Each included combination as its own mapping, in declared order.
    """
    strategy = job.get("strategy")
    require(isinstance(strategy, dict), "the job must declare a strategy matrix")
    matrix = strategy.get("matrix")
    require(isinstance(matrix, dict), "the job must declare a matrix mapping")
    includes = matrix.get("include")
    require(isinstance(includes, list), "the matrix must declare an include list")
    require(
        all(isinstance(entry, dict) for entry in includes),
        "every matrix include entry must be a mapping",
    )
    return typ.cast("list[dict[str, typ.Any]]", includes)


def coverage_inputs(step: dict[str, typ.Any]) -> dict[str, typ.Any]:
    """Return the parity inputs a coverage step declares.

    An input the step omits is represented as ``None`` rather than dropped, so
    two steps compare equal only when they name the same inputs with the same
    values. That is what makes an equality check a parity claim: a dropped
    input on one side would otherwise hide behind the other side's absence.

    Parameters
    ----------
    step
        A coverage-generation step document.

    Returns
    -------
    dict[str, typing.Any]
        One entry per :data:`PARITY_INPUTS` name, ``None`` where unset.
    """
    shared = step.get("with") or {}
    require(isinstance(shared, dict), "a coverage step's `with` must be a mapping")
    return {name: shared.get(name) for name in PARITY_INPUTS}
