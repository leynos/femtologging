"""Readers that hold the split between PR coverage and main publication.

CV-005 keeps coverage measurement and its ratchet in the pull-request lane and
moves both persistent outputs, the CodeScene upload and the ratchet baseline,
to one default-branch publisher. These readers answer the questions that split
turns on: which workflows a pull request can reach, which jobs declare the
protected environment, where a report is generated, handed over and uploaded,
and whether the two lanes measure the same population. Each reader fails closed
on a shape it cannot prove rather than assuming the safe reading.
"""

from __future__ import annotations

import typing as typ
from pathlib import Path

from tests.workflow_document_helpers import repository_workflows, require

if typ.TYPE_CHECKING:
    import collections.abc as cabc

CODESCENE_ENVIRONMENT: typ.Final[str] = "codescene"
# These name the secret, its step and its command: never a secret value.
TOKEN_NAME: typ.Final[str] = "CS_ACCESS_TOKEN"  # ruff: ignore[hardcoded-password-string] the secret's name, not its value
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


def environment_name(job: dict[str, typ.Any]) -> str | None:
    """Return the environment a job deploys to, if it declares one.

    Parameters
    ----------
    job
        A workflow job document.

    Returns
    -------
    str | None
        The environment name, from either the scalar form or the mapping form
        that also carries a deployment URL, or ``None`` when no environment is
        declared.
    """
    raw = job.get("environment")
    if raw is None:
        return None
    if isinstance(raw, str):
        return raw
    require(isinstance(raw, dict), "an environment must be a name or a mapping")
    name = raw.get("name")
    require(isinstance(name, str), "an environment mapping must declare a name")
    return name


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


def _strings(node: object) -> cabc.Iterator[str]:
    """Yield every string nested in `node`, mapping keys included."""
    match node:
        case str():
            yield node
        case dict():
            for key, value in typ.cast("dict[object, object]", node).items():
                yield str(key)
                yield from _strings(value)
        case list():
            for item in typ.cast("list[object]", node):
                yield from _strings(item)


def env_blocks(
    node: object, path: str = ""
) -> cabc.Iterator[tuple[str, dict[str, typ.Any]]]:
    """Yield every ``env`` mapping nested in `node` with its location.

    Parameters
    ----------
    node
        A parsed workflow document or a part of one.
    path
        Location prefix used to report where the mapping was found.

    Yields
    ------
    tuple[str, dict[str, typing.Any]]
        The location and the ``env`` mapping declared there. Workflow-, job-
        and step-level ``env`` blocks are all found.
    """
    match node:
        case dict():
            for key, value in typ.cast("dict[object, object]", node).items():
                if key == "env" and isinstance(value, dict):
                    yield f"{path}/env", typ.cast("dict[str, typ.Any]", value)
                yield from env_blocks(value, f"{path}/{key}")
        case list():
            for at, item in enumerate(typ.cast("list[object]", node)):
                yield from env_blocks(item, f"{path}/{at}")


def secret_in_env(document: dict[str, typ.Any]) -> list[str]:
    """Return the locations whose ``env`` names the CodeScene token.

    Parameters
    ----------
    document
        A parsed workflow document.

    Returns
    -------
    list[str]
        One location per offending ``env`` mapping. The upload action receives
        the token as an input, never through the environment: it is composite
        and would hand a step's environment to its nested steps.
    """
    return [
        location
        for location, mapping in env_blocks(document)
        if any(TOKEN_NAME in value for value in _strings(mapping))
    ]


def _local_target(reference: str, documents: cabc.Mapping[str, object]) -> str | None:
    """Return the local workflow a job reference names, or ``None``.

    Parameters
    ----------
    reference
        A job's ``uses`` value.
    documents
        The repository's workflows, keyed by file name.

    Returns
    -------
    str | None
        The referenced workflow's file name, or ``None`` for a workflow in
        another repository, whose jobs this repository cannot inspect.
    """
    if reference.startswith("./"):
        file_name = Path(reference).name
        require(file_name in documents, f"a job calls a missing workflow: {reference}")
        return file_name
    require(
        not reference.startswith(f"{SELF_REPOSITORY}/"),
        f"a qualified self-call cannot be verified: {reference}",
    )
    return None


def _workflow_run_names(events: cabc.Mapping[str, typ.Any]) -> list[str]:
    """Return the workflows a ``workflow_run`` trigger listens for."""
    settings = events.get("workflow_run")
    if settings is None:
        return []
    require(isinstance(settings, dict), "a workflow_run trigger must be a mapping")
    names = settings.get("workflows")
    require(isinstance(names, list), "workflow_run must name its workflows")
    return [str(name) for name in names]


def _job_references(document: dict[str, typ.Any]) -> list[str]:
    """Return the reusable workflows the document's jobs call."""
    jobs = document.get("jobs")
    require(isinstance(jobs, dict), "the workflow must declare a jobs mapping")
    return [
        job["uses"]
        for job in typ.cast("dict[str, object]", jobs).values()
        if isinstance(job, dict) and isinstance(job.get("uses"), str)
    ]


def _display_name_index(
    documents: cabc.Mapping[str, dict[str, typ.Any]],
) -> dict[str, str]:
    """Map each workflow's display name to the file that declares it."""
    return {
        str(document["name"]): file_name
        for file_name, document in documents.items()
        if "name" in document
    }


def _pull_request_documents(
    documents: cabc.Mapping[str, dict[str, typ.Any]],
) -> set[str]:
    """Return the workflows a pull-request event triggers directly."""
    return {
        file_name
        for file_name, document in documents.items()
        if PULL_REQUEST_EVENTS & set(triggers(document))
    }


def _document_targets(
    file_name: str,
    document: dict[str, typ.Any],
    documents: cabc.Mapping[str, dict[str, typ.Any]],
    by_display: cabc.Mapping[str, str],
) -> tuple[list[str], list[str]]:
    """Return the workflows one document can reach, and its remote calls.

    A reusable-workflow call into another repository is returned separately
    rather than followed: its jobs live elsewhere, so its environment
    declarations cannot be proved from this repository.

    Returns
    -------
    tuple[list[str], list[str]]
        The workflows this document reaches, and the remote reusable-workflow
        references it calls.
    """
    calls = [(ref, _local_target(ref, documents)) for ref in _job_references(document)]
    remote = [ref for ref, target in calls if target is None]
    targets = [target for _, target in calls if target is not None]
    for display in _workflow_run_names(triggers(document)):
        require(display in by_display, f"{file_name} listens for unknown: {display}")
        targets.append(by_display[display])
    return targets, remote


def _enqueue_unreached(target: str, reached: set[str], frontier: list[str]) -> None:
    """Add `target` to the frontier unless it has already been reached."""
    if target not in reached:
        reached.add(target)
        frontier.append(target)


def pull_request_reach(
    documents: cabc.Mapping[str, dict[str, typ.Any]],
) -> tuple[set[str], set[str]]:
    """Return the workflows a pull request can reach, and its remote calls.

    Reachability follows the three routes the workflow language offers: a
    pull-request trigger, a local reusable-workflow call, and a ``workflow_run``
    trigger naming a workflow already reached. A call into another repository
    is returned separately rather than followed: its jobs are not in this
    repository, so its environment declarations cannot be proved from here.

    Parameters
    ----------
    documents
        The repository's workflows, keyed by file name.

    Returns
    -------
    tuple[set[str], set[str]]
        The reachable file names and the remote reusable-workflow references
        those workflows call, which the caller must account for explicitly.
    """
    by_display = _display_name_index(documents)
    reached = _pull_request_documents(documents)
    remote: set[str] = set()
    frontier = sorted(reached)
    while frontier:
        file_name = frontier.pop()
        document = documents[file_name]
        targets, remote_calls = _document_targets(
            file_name, document, documents, by_display
        )
        remote.update(remote_calls)
        for target in targets:
            _enqueue_unreached(target, reached, frontier)
    return reached, remote


def repository_documents() -> dict[str, dict[str, typ.Any]]:
    """Return every repository workflow, keyed by file name."""
    return dict(repository_workflows())


def coverage_inputs(step: dict[str, typ.Any]) -> dict[str, typ.Any]:
    """Return the parity inputs a coverage step declares."""
    shared = step.get("with") or {}
    require(isinstance(shared, dict), "a coverage step's `with` must be a mapping")
    return {name: shared.get(name) for name in PARITY_INPUTS}
