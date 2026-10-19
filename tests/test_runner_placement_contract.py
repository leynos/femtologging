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

import typing as typ

import pytest
import yaml

from tests.make_contract_helpers import repo_root
from tests.runner_placement_judgement import (
    FORK_CONDITION,
    HOSTED_CONDITION,
    PLACEMENTS,
    Origin,
    estate_expression,
    placement_faults,
    selected_runner,
)

#: A parsed workflow document. YAML is dynamically shaped, so `typing.Any` is
#: the honest value type at this boundary: every reader below narrows each
#: value with `isinstance` before using it, and nothing beyond this module sees
#: the unnarrowed form.
type Document = dict[str, typ.Any]

WORKFLOWS_DIRECTORY: typ.Final = ".github/workflows"


class WorkflowReadError(AssertionError):
    """Report a workflow the contract could not read, naming the file."""


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
        ("dependabot", "ubuntu-latest"),
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
        ("ubuntu-latest", 4),
        ("ubicloud-standard-2", 4),
        (
            (
                f"${{{{ {HOSTED_CONDITION} && 'ubicloud-standard-2' "
                "|| 'ubuntu-latest' }}"
            ),
            4,
        ),
        (estate_expression("ubicloud-standard-4"), 2),
        (
            (
                "${{ (github.event_name == 'pull_request') && 'ubuntu-latest' "
                "|| 'ubicloud-standard-2' }}"
            ),
            4,
        ),
        (
            (
                f"${{{{ {HOSTED_CONDITION} && 'ubicloud-standard-2' "
                "|| 'ubicloud-standard-2' }}"
            ),
            2,
        ),
        (["ubicloud-standard-2"], 4),
        (None, 4),
        (
            (
                f"${{{{ {FORK_CONDITION} && 'ubuntu-latest' "
                "|| 'ubicloud-standard-2' }}"
            ),
            4,
        ),
        (
            (
                f"${{{{ ({FORK_CONDITION} || github.actor == 'dependabot[bot]') "
                "&& 'ubuntu-latest' || 'ubicloud-standard-2' }}"
            ),
            4,
        ),
    ],
    ids=[
        "estate",
        "always-hosted",
        "always-ubicloud",
        "inverted-arms",
        "another-label",
        "another-condition",
        "hosted-origins-kept-on-ubicloud",
        "sequence",
        "not-a-string",
        "dependabot-arm-missing",
        "actor-instead-of-author",
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
