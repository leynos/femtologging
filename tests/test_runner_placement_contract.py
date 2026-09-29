"""Hold the workflows' Ubicloud placement to the files: expression and ceiling.

Ubicloud's cache proxy is scoped by ref, and a pull request from a fork cannot
obtain an Ubicloud runner at all. A lane that names an Ubicloud runner
therefore selects it by a runner-selection expression that falls back to the
hosted pool for a fork, and states its own ceiling, because an Ubicloud runner
is a self-hosted just-in-time runner that GitHub's six-hour cap for hosted jobs
does not bound.

The judgement is driven over constructed expressions in both directions before
the real files are asserted, because a check over this repository's own
correct workflows passes whether or not it discriminates anything.
"""

from __future__ import annotations

import re
import typing as typ

import pytest
import yaml

from tests.make_contract_helpers import repo_root

#: A parsed workflow document. YAML is dynamically shaped, so `typing.Any` is
#: the honest value type at this boundary: every reader below narrows each
#: value with `isinstance` before using it, and nothing beyond this module sees
#: the unnarrowed form.
type Document = dict[str, typ.Any]

WORKFLOWS_DIRECTORY: typ.Final = ".github/workflows"
HOSTED_LABEL: typ.Final = "ubuntu-latest"
FORK_CONDITION: typ.Final = "github.event.pull_request.head.repo.fork"

#: Every job that can land on Ubicloud: workflow, job, runner class and the
#: ceiling it states in minutes. The inventory is exact, so a new Ubicloud lane
#: without a ceiling, or a class or ceiling changed, fails until reviewed.
PLACEMENTS: typ.Final = (("ci.yml", "build-test", "ubicloud-standard-4", 45),)

#: The runner-selection shape: a condition, a quoted hosted arm and a quoted
#: other arm.
_SHAPE: typ.Final = re.compile(
    r"\$\{\{\s*(?P<condition>[^&|]+?)\s*&&\s*'(?P<hosted>[^']*)'"
    r"\s*\|\|\s*'(?P<other>[^']*)'\s*\}\}"
)

type Origin = typ.Literal["push", "same-repository", "fork"]
ORIGINS: typ.Final[tuple[Origin, ...]] = ("push", "same-repository", "fork")


class WorkflowReadError(AssertionError):
    """Report a workflow the contract could not read, naming the file."""


def estate_expression(label: str) -> str:
    """Return the runner-selection expression for an Ubicloud `label`.

    Parameters
    ----------
    label
        The Ubicloud runner class a non-fork run selects.

    Returns
    -------
    str
        A `${{ ... }}` expression selecting `label` unless the run is a pull
        request from a fork, which falls back to the hosted pool.
    """
    return f"${{{{ {FORK_CONDITION} && '{HOSTED_LABEL}' || '{label}' }}}}"


def selected_runner(runs_on: object, origin: Origin) -> str | None:
    """Return the label a `runs-on` value selects for a run.

    Parameters
    ----------
    runs_on
        The job's `runs-on` value as parsed.
    origin
        The kind of run: a push or dispatch (no pull request), a pull request
        from this repository, or a pull request from a fork.

    Returns
    -------
    str | None
        The selected label, or None when the value is not the runner-selection
        shape. A literal label is not the shape: a lane that never falls back
        cannot serve a fork.
    """
    shape = _SHAPE.fullmatch(runs_on.strip()) if isinstance(runs_on, str) else None
    if shape is None or shape["condition"] != FORK_CONDITION:
        return None
    return shape["hosted" if origin == "fork" else "other"]


def placement_faults(runs_on: object, label: str) -> list[str]:
    """Return one entry per kind of run the expression places wrongly.

    Parameters
    ----------
    runs_on
        The job's `runs-on` value as parsed.
    label
        The Ubicloud runner class every non-fork run must select.

    Returns
    -------
    list[str]
        Empty when a fork falls back to hosted and every other run is on
        `label`.
    """
    wanted = {"push": label, "same-repository": label, "fork": HOSTED_LABEL}
    return [
        f"{origin} selects {selected_runner(runs_on, origin)}, wanted {wanted[origin]}"
        for origin in ORIGINS
        if selected_runner(runs_on, origin) != wanted[origin]
    ]


def parse_document(name: str, text: str) -> Document:
    """Parse one workflow's text into a mapping.

    Parameters
    ----------
    name
        The file name, used to say which workflow failed.
    text
        The workflow's source.

    Returns
    -------
    dict[str, typing.Any]
        The parsed document.

    Raises
    ------
    WorkflowReadError
        If the text is not YAML or does not parse to a mapping, so a broken
        workflow fails the contract loudly instead of dropping out of it.
    """
    try:
        document = yaml.safe_load(text)
    except yaml.YAMLError as error:
        message = f"{name} is not valid YAML: {error}"
        raise WorkflowReadError(message) from error
    if not isinstance(document, dict):
        message = f"{name} does not parse to a mapping"
        raise WorkflowReadError(message)
    return document


def load_documents(
    directory: str = WORKFLOWS_DIRECTORY,
) -> dict[str, Document]:
    """Read and parse every workflow under a directory, keyed by file name.

    Parameters
    ----------
    directory
        The workflow directory, relative to the repository root.

    Returns
    -------
    dict[str, dict[str, typing.Any]]
        File name to parsed document.

    Raises
    ------
    WorkflowReadError
        If the directory or a file cannot be read, a file is not a workflow
        mapping, or no workflow is found at all.
    """
    root = repo_root() / directory
    try:
        found = {
            path.name: parse_document(path.name, path.read_text(encoding="utf-8"))
            for path in sorted(root.iterdir())
            if path.suffix.lower() in {".yml", ".yaml"}
        }
    except OSError as error:
        message = f"cannot read workflows under {root}: {error}"
        raise WorkflowReadError(message) from error
    if not found:
        message = f"no workflow parsed under {root}"
        raise WorkflowReadError(message)
    return found


def names_ubicloud(job: Document) -> bool:
    """Return whether a job can run on an Ubicloud runner, however it says so.

    Parameters
    ----------
    job
        A parsed job mapping.

    Returns
    -------
    bool
        True when its `runs-on` names Ubicloud, or reads a matrix value
        (`${{ matrix.runner }}`) while its `strategy` names Ubicloud, so an
        indirect placement is inventoried and then rejected by the judgement
        rather than skipped.
    """
    runs_on = str(job.get("runs-on", ""))
    return "ubicloud" in runs_on or (
        "matrix." in runs_on and "ubicloud" in str(job.get("strategy", ""))
    )


def placed_jobs(
    documents: dict[str, Document],
) -> list[tuple[str, str, object, object]]:
    """Return every job whose `runs-on` names Ubicloud.

    Parameters
    ----------
    documents
        Parsed workflows keyed by file name.

    Returns
    -------
    list[tuple[str, str, object, object]]
        Workflow, job, `runs-on` value and the `timeout-minutes` it states
        (None when it states none), in file and job order.
    """
    return [
        (name, job_id, job.get("runs-on"), job.get("timeout-minutes"))
        for name, document in sorted(documents.items())
        for job_id, job in (document.get("jobs") or {}).items()
        if isinstance(job, dict) and names_ubicloud(job)
    ]


@pytest.mark.parametrize(
    ("origin", "wanted"),
    [
        ("push", "ubicloud-standard-2"),
        ("same-repository", "ubicloud-standard-2"),
        ("fork", "ubuntu-latest"),
    ],
)
def test_the_expression_places_each_run(origin: Origin, wanted: str) -> None:
    """Place a push, a dispatch and a same-repository pull request on Ubicloud.

    Parameters
    ----------
    origin
        The kind of run.
    wanted
        The label the run must select; only a fork's pull request is hosted.
    """
    selected = selected_runner(estate_expression("ubicloud-standard-2"), origin)
    assert selected == wanted, f"{origin} selected {selected}, wanted {wanted}"


@pytest.mark.parametrize(
    ("runs_on", "expected"),
    [
        (estate_expression("ubicloud-standard-2"), 0),
        ("ubuntu-latest", 3),
        ("ubicloud-standard-2", 3),
        (
            f"${{{{ {FORK_CONDITION} && 'ubicloud-standard-2' || 'ubuntu-latest' }}}}",
            3,
        ),
        (estate_expression("ubicloud-standard-4"), 2),
        (
            (
                "${{ github.event_name == 'pull_request' && 'ubuntu-latest' "
                "|| 'ubicloud-standard-2' }}"
            ),
            3,
        ),
        (
            (
                f"${{{{ {FORK_CONDITION} && 'ubicloud-standard-2' "
                "|| 'ubicloud-standard-2' }}"
            ),
            1,
        ),
        (["ubicloud-standard-2"], 3),
        (None, 3),
    ],
    ids=[
        "estate",
        "always-hosted",
        "always-ubicloud",
        "inverted-arms",
        "another-label",
        "another-condition",
        "fork-kept-on-ubicloud",
        "sequence",
        "not-a-string",
    ],
)
def test_a_misplaced_lane_is_reported(runs_on: object, expected: int) -> None:
    """Report each careless edit, and not the runner-selection expression.

    Parameters
    ----------
    runs_on
        A `runs-on` value to judge.
    expected
        How many kinds of run it places wrongly.
    """
    faults = placement_faults(runs_on, "ubicloud-standard-2")
    assert len(faults) == expected, f"expected {expected}, saw {faults}"


@pytest.mark.parametrize(
    ("key", "expected"),
    [
        ("    timeout-minutes: 30\n", 30),
        ("", None),
        ("    timeout-minutes: thirty\n", "thirty"),
    ],
    ids=["stated", "missing", "a-string"],
)
def test_a_ceiling_is_read_as_the_file_states_it(key: str, expected: object) -> None:
    """Read the ceiling verbatim, so a wrong one cannot pass the inventory.

    Parameters
    ----------
    key
        The `timeout-minutes` line a constructed job carries, or nothing.
    expected
        What the inventory must report for it.
    """
    expression = estate_expression("ubicloud-standard-2")
    text = f"jobs:\n  lane:\n    runs-on: {expression}\n{key}"
    placed = placed_jobs({"x.yml": parse_document("x.yml", text)})
    assert placed == [("x.yml", "lane", expression, expected)], f"read {placed}"


def test_a_hosted_job_is_not_inventoried() -> None:
    """Leave a hosted lane outside the inventory, since it needs no ceiling."""
    document = parse_document("x.yml", "jobs:\n  lane:\n    runs-on: ubuntu-latest\n")
    placed = placed_jobs({"x.yml": document})
    assert placed == [], f"a hosted job was inventoried: {placed}"


@pytest.mark.parametrize(
    ("text", "reason"),
    [
        ("on: [push\n", "not valid YAML"),
        ("- on: push\n", "does not parse to a mapping"),
    ],
    ids=["not-yaml", "not-a-mapping"],
)
def test_an_unreadable_workflow_is_refused_by_name(text: str, reason: str) -> None:
    """Refuse a broken workflow, naming it, rather than skip it.

    Parameters
    ----------
    text
        A workflow source that cannot be a mapping.
    reason
        The reason the error must give.
    """
    with pytest.raises(WorkflowReadError, match=reason):
        parse_document("broken.yml", text)


def test_every_ubicloud_lane_is_placed_by_the_expression_and_states_a_ceiling() -> None:
    """Find exactly the inventoried jobs, each placed and each with a ceiling."""
    placed = placed_jobs(load_documents())
    found = [(name, job, ceiling) for name, job, _, ceiling in placed]
    expected = [(name, job, ceiling) for name, job, _, ceiling in PLACEMENTS]
    assert found == expected, f"found {found}, expected {expected}"
    for (name, job, runs_on, _), (_, _, label, _) in zip(
        placed, PLACEMENTS, strict=True
    ):
        assert not placement_faults(runs_on, label), f"{name}: {job} is misplaced"


def test_an_indirect_ubicloud_runner_is_inventoried_and_rejected() -> None:
    """Inventory a runner named through the matrix, and refuse it.

    An indirect `runs-on: ${{ matrix.runner }}` whose matrix names Ubicloud
    would otherwise escape the inventory, and with it the fork fallback and
    the ceiling. Only the runner-selection expression places a lane.
    """
    text = (
        "jobs:\n  lane:\n    runs-on: ${{ matrix.runner }}\n"
        "    strategy:\n      matrix:\n        runner: [ubicloud-standard-2]\n"
    )
    placed = placed_jobs({"x.yml": parse_document("x.yml", text)})
    assert len(placed) == 1, f"the matrix runner was not inventoried: {placed}"
    assert placement_faults(placed[0][2], "ubicloud-standard-2"), "not rejected"
