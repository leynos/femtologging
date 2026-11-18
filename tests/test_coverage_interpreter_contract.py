"""Contracts for the interpreter each coverage lane measures on.

The ratchet baseline is keyed by the interpreter's major.minor, so the lane a
job measures on and the key its figures are stored under must agree. These
contracts hold that agreement across the two workflows: the publisher's lanes
are exactly the interpreters a pull request must pass, the interpreter reaches
the baseline key from the build matrix rather than a literal, and both lanes
resolve the same matrix value. They depend on the matrix and step readers in
:mod:`tests.coverage_contract_helpers`.
"""

from __future__ import annotations

import typing as typ

from tests.coverage_contract_helpers import (
    BASELINE_MARKER,
    GENERATE_MARKER,
    matrix_includes,
    matrix_versions,
    repository_documents,
    steps_using,
)

CI_WORKFLOW: typ.Final[str] = "ci.yml"
PUBLISHER: typ.Final[str] = "coverage-main.yml"

LANE: typ.Final[str] = "${{ matrix.python-version }}"


def _job(document: dict[str, typ.Any], job_id: str) -> dict[str, typ.Any]:
    """Return one job of `document` by id."""
    jobs = typ.cast("dict[str, typ.Any]", document["jobs"])
    return typ.cast("dict[str, typ.Any]", jobs[job_id])


def _required_ci_lanes(documents: dict[str, dict[str, typ.Any]]) -> list[str]:
    """Return the interpreters `ci.yml` requires rather than allows to fail."""
    build = _job(documents[CI_WORKFLOW], "build-test")
    return [
        str(entry.get("python-version"))
        for entry in matrix_includes(build)
        if not entry.get("experimental")
    ]


def test_the_publisher_measures_every_required_interpreter() -> None:
    """The baseline lanes are exactly the interpreters a pull request must pass.

    A lane the pull request requires but the publisher skips would ratchet
    against a baseline nobody advances, and a lane the pull request allows to
    fail must not gate the trunk.
    """
    documents = repository_documents()
    publisher = documents[PUBLISHER]
    required = matrix_versions(_job(publisher, "coverage-measure"), "python-version")
    assert required == ["3.12", "3.13", "3.14"], required
    published = matrix_versions(_job(publisher, "coverage-baseline"), "python-version")
    assert published == required, published
    assert _required_ci_lanes(documents) == required, documents


def test_each_lane_keys_its_baseline_by_the_matrix_interpreter() -> None:
    """The interpreter reaches the baseline key from the matrix, not a literal.

    A literal would publish one interpreter's figures under another's key, and
    the pull request measuring on that interpreter would ratchet against the
    wrong population.
    """
    publisher = repository_documents()[PUBLISHER]
    generated = steps_using({PUBLISHER: publisher}, GENERATE_MARKER)[0][2]
    shared = typ.cast("dict[str, typ.Any]", generated.get("with") or {})
    handover = f"coverage-baseline-handover-{LANE}"
    assert shared.get("python-version") == LANE, shared
    assert shared.get("baseline-artefact-name") == handover, shared
    published = steps_using({PUBLISHER: publisher}, BASELINE_MARKER)[0][2]
    written = typ.cast("dict[str, typ.Any]", published.get("with") or {})
    assert written.get("artefact-name") == handover, written


def test_both_lanes_resolve_the_same_interpreter_lane() -> None:
    """Each lane pins its interpreter through the matrix value.

    `python-version` is asserted here rather than added to `PARITY_INPUTS`: the
    pull-request step pins the interpreter through `UV_PYTHON`, which the
    action reads second, so the two steps spell the same lane differently and
    an input-by-input comparison would be false.
    """
    documents = repository_documents()
    publisher = typ.cast(
        "dict[str, typ.Any]", _job(documents[PUBLISHER], "coverage-measure").get("env")
    )
    pull_request = typ.cast(
        "dict[str, typ.Any]", _job(documents[CI_WORKFLOW], "build-test").get("env")
    )
    assert publisher.get("UV_PYTHON") == LANE, publisher
    assert pull_request.get("UV_PYTHON") == LANE, pull_request
