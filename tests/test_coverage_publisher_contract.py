"""Contracts for the publisher's topology and its baseline publication.

CV-005 keeps coverage generation, and its ratchet, in the pull-request lane and
moves both persistent outputs to one default-branch publisher. These contracts
hold the publisher's place in the workflow graph: one workflow pushes to
``main``, measures and publishes a baseline; it is unreachable from a pull
request; its single remote call stays the recorded one; and the pull-request
lane measures the same population without publishing anything.

The publisher also owns one baseline job per runner provider, because a cache
entry written on one provider is not read by the other and a fork or
Dependabot pull request falls back to the hosted pool. The publication
mechanics are held here too: both providers advance from one hand-over, a
failed lane withdraws only its own publication, and the upload reads the
measured commit. The upload guard and the environment readers have contracts
of their own in :mod:`tests.test_coverage_upload_guard_contract` and
:mod:`tests.test_coverage_environment_contract`.
"""

from __future__ import annotations

import typing as typ

import pytest

from tests.coverage_contract_helpers import (
    BASELINE_MARKER,
    CI_WORKFLOW,
    GENERATE_MARKER,
    LANE_HANDOVER_CONDITION,
    LANE_PROBE_CONDITION,
    PARTIAL_FAILURE_CONDITION,
    PUBLISHER_WORKFLOW,
    UPLOAD_MARKER,
    coverage_inputs,
    jobs_named,
    matrix_versions,
    repository_documents,
    steps_using,
    triggers,
)
from tests.coverage_environment_helpers import (
    CODESCENE_ENVIRONMENT,
    TOKEN_NAME,
    environment_name,
)
from tests.coverage_push_filter_helpers import pushes_to
from tests.coverage_reach_helpers import pull_request_reach
from tests.runner_placement_judgement import selected_runner

#: The one reusable workflow a pull-request event reaches that lives in
#: another repository. It was read at the pinned commit named in its
#: reference: no job there declares an environment. A new remote call, or a
#: moved pin, must be re-verified rather than silently allowed.
KNOWN_REMOTE_CALLS: typ.Final[frozenset[str]] = frozenset({
    (
        "leynos/shared-actions/.github/workflows/dependabot-automerge.yml"
        "@14c325b60788814cd50298eb8060228daee3b842"
    ),
})


def test_one_publisher_owns_the_default_branch_upload() -> None:
    """Exactly one workflow pushes to main, measures, and publishes a baseline."""
    documents = repository_documents()
    publishers = sorted(
        file_name
        for file_name, document in documents.items()
        if pushes_to(triggers(document), "main")
        and steps_using({file_name: document}, GENERATE_MARKER)
        and steps_using({file_name: document}, BASELINE_MARKER)
    )
    assert publishers == [PUBLISHER_WORKFLOW], publishers


def test_the_publisher_runs_only_on_a_push_to_the_trunk() -> None:
    """The publisher declares one trigger, a push restricted to `main`.

    `pushes_to` answers whether a push to main is accepted, which a widened
    filter still satisfies. The exact mapping is asserted here so an added
    trigger, or a `feature/*` branch, fails the contract.
    """
    document = repository_documents()[PUBLISHER_WORKFLOW]
    assert triggers(document) == {"push": {"branches": ["main"]}}, document.get("on")


def test_the_only_codescene_upload_is_the_publishers() -> None:
    """No workflow other than the publisher uploads to CodeScene.

    A second uploader would carry the report to the protected service from a
    route the reachability contract never inspects.
    """
    uploaders = [
        (file_name, job_id)
        for file_name, job_id, _ in steps_using(repository_documents(), UPLOAD_MARKER)
    ]
    assert [file_name for file_name, _ in uploaders] == [PUBLISHER_WORKFLOW], uploaders


def test_the_publisher_is_not_reachable_from_a_pull_request() -> None:
    """A pull request can reach neither the publisher nor its environment."""
    documents = repository_documents()
    reached, _ = pull_request_reach(documents)
    assert PUBLISHER_WORKFLOW not in reached, reached
    offenders = [
        (file_name, job_id)
        for file_name in reached
        for _, job_id, job in jobs_named({file_name: documents[file_name]})
        if environment_name(job) == CODESCENE_ENVIRONMENT
    ]
    assert not offenders, offenders


def test_the_only_remote_pull_request_call_is_the_recorded_one() -> None:
    """A new call into another repository must be verified, not assumed safe."""
    _, remote = pull_request_reach(repository_documents())
    assert remote == KNOWN_REMOTE_CALLS, remote


def test_the_publisher_never_cancels_and_keys_on_the_ref() -> None:
    """A cancelled publisher abandons its upload and its baseline write."""
    concurrency = repository_documents()[PUBLISHER_WORKFLOW].get("concurrency")
    assert isinstance(concurrency, dict), concurrency
    assert concurrency.get("group") == "${{ github.workflow }}-${{ github.ref }}", (
        concurrency
    )
    assert concurrency.get("cancel-in-progress") is False, concurrency


def test_pull_requests_measure_and_ratchet_without_publishing() -> None:
    """The pull-request lane ratchets locally and publishes nothing."""
    document = repository_documents()[CI_WORKFLOW]
    matches = steps_using({CI_WORKFLOW: document}, GENERATE_MARKER)
    assert len(matches) == 1, matches
    _, job_id, step = matches[0]
    shared = step.get("with") or {}
    assert shared.get("with-ratchet") == "true", shared
    assert shared.get("publish-artefact") == "false", shared
    job = typ.cast("dict[str, typ.Any]", document["jobs"][job_id])
    assert environment_name(job) is None, (
        "a pull request must not reach the environment"
    )
    assert TOKEN_NAME not in str(step), "the pull-request lane must not name the token"


def test_both_lanes_measure_the_same_population() -> None:
    """Every input that defines the measured population is identical."""
    documents = repository_documents()
    pull_request = steps_using({CI_WORKFLOW: documents[CI_WORKFLOW]}, GENERATE_MARKER)
    publisher = steps_using(
        {PUBLISHER_WORKFLOW: documents[PUBLISHER_WORKFLOW]}, GENERATE_MARKER
    )
    assert len(pull_request) == 1, pull_request
    assert len(publisher) == 1, publisher
    assert coverage_inputs(pull_request[0][2]) == coverage_inputs(publisher[0][2]), (
        pull_request,
        publisher,
    )


def test_the_baseline_is_published_from_the_runner_the_pull_requests_use() -> None:
    """The hand-over keeps the measurement off the cache store it cannot share."""
    document = repository_documents()[PUBLISHER_WORKFLOW]
    measures = steps_using({PUBLISHER_WORKFLOW: document}, GENERATE_MARKER)[0]
    publishes = steps_using({PUBLISHER_WORKFLOW: document}, BASELINE_MARKER)[0]
    shared = measures[2].get("with") or {}
    assert shared.get("with-ratchet") == "true", shared
    assert shared.get("publish-baseline") == "never", shared
    artefact = shared.get("baseline-artefact-name")
    assert isinstance(artefact, str), shared
    assert artefact, shared
    jobs = typ.cast("dict[str, typ.Any]", document["jobs"])
    baseline_job = typ.cast("dict[str, typ.Any]", jobs[publishes[1]])
    assert baseline_job.get("needs") == measures[1], baseline_job
    # A push has no pull request, so the placement expression resolves to the
    # Ubicloud store the pull-request lanes restore from.
    assert (
        selected_runner(baseline_job.get("runs-on"), "push") == "ubicloud-standard-2"
    ), baseline_job
    handover = publishes[2].get("with") or {}
    assert handover.get("artefact-name") == artefact, handover


def _publisher_jobs(
    document: dict[str, typ.Any],
) -> list[tuple[str, dict[str, typ.Any]]]:
    """Return the jobs that publish a baseline, in document order."""
    return [
        (job_id, job)
        for _, job_id, job in jobs_named({PUBLISHER_WORKFLOW: document})
        if any(
            BASELINE_MARKER in str(step.get("uses", ""))
            for step in typ.cast("list[dict[str, typ.Any]]", job.get("steps") or [])
        )
    ]


def _runner_label(job: dict[str, typ.Any]) -> str:
    """Return the runner label a push selects, reading the raw value if needed."""
    runs_on = job.get("runs-on")
    return selected_runner(runs_on, "push") or str(runs_on)


def test_both_runner_providers_receive_a_baseline() -> None:
    """Each provider's cache store gets its own publisher.

    A fork or Dependabot pull request runs on the hosted pool, and a cache
    entry written on Ubicloud is not visible there. Without a hosted
    publisher, that lane would restore nothing, start its baseline at zero and
    so never fail its ratchet.
    """
    jobs = _publisher_jobs(repository_documents()[PUBLISHER_WORKFLOW])
    assert len(jobs) >= 1, jobs
    labels = {_runner_label(job) for _, job in jobs}
    assert labels == {"ubicloud-standard-2", "ubuntu-latest"}, labels


@pytest.mark.parametrize("job_id", ["coverage-baseline", "coverage-baseline-hosted"])
def test_each_publisher_survives_a_failed_sibling_and_publishes_its_own_lane(
    job_id: str,
) -> None:
    """A failed lane withdraws its own baseline and no sibling's.

    Measurement keeps running its other lanes, but a failed dependency would
    still skip every publisher. Each publisher therefore runs on
    `!cancelled()`, probes its own hand-over only when the matrix failed, and
    publishes only when that hand-over or the whole matrix succeeded.
    """
    document = repository_documents()[PUBLISHER_WORKFLOW]
    job = typ.cast("dict[str, typ.Any]", document["jobs"][job_id])
    assert job.get("needs") == "coverage-measure", job.get("needs")
    assert job.get("if") == PARTIAL_FAILURE_CONDITION, job.get("if")
    steps = {
        str(step.get("name")): step
        for step in typ.cast("list[dict[str, typ.Any]]", job.get("steps") or [])
    }
    probe = steps.get("Check for this lane's hand-over")
    assert probe is not None, sorted(steps)
    assert probe.get("if") == LANE_PROBE_CONDITION, probe.get("if")
    assert probe.get("continue-on-error") is True, probe
    publish = steps.get("Publish the ratchet baseline")
    assert publish is not None, sorted(steps)
    assert publish.get("if") == LANE_HANDOVER_CONDITION, publish.get("if")
    shared = publish.get("with") or {}
    assert shared.get("artefact-name") == (
        "coverage-baseline-handover-${{ matrix.python-version }}"
    ), shared


def test_both_publishers_read_the_same_hand_over() -> None:
    """The two stores advance from one figure, not two measurements."""
    document = repository_documents()[PUBLISHER_WORKFLOW]
    measures = steps_using({PUBLISHER_WORKFLOW: document}, GENERATE_MARKER)[0]
    handed_over = (measures[2].get("with") or {}).get("baseline-artefact-name")
    published = [
        (step.get("with") or {}).get("artefact-name")
        for _, _, step in steps_using({PUBLISHER_WORKFLOW: document}, BASELINE_MARKER)
    ]
    assert published == [handed_over, handed_over], published


def test_each_hand_over_is_keyed_by_the_interpreter_that_measured_it() -> None:
    """A lane's artefact and its measurement name the same interpreter.

    The baseline cache key carries the interpreter's major.minor, so an
    artefact named after one lane while the action measured another would
    publish a figure under the wrong key. Both sides are bound to
    `matrix.python-version`, and every publisher measures the same lane list as
    the measurement matrix.
    """
    document = repository_documents()[PUBLISHER_WORKFLOW]
    measure_job = typ.cast("dict[str, typ.Any]", document["jobs"]["coverage-measure"])
    measures = steps_using({PUBLISHER_WORKFLOW: document}, GENERATE_MARKER)[0]
    shared = measures[2].get("with") or {}
    bound = "${{ matrix.python-version }}"
    assert shared.get("python-version") == bound, shared
    assert shared.get("baseline-artefact-name") == (
        f"coverage-baseline-handover-{bound}"
    ), shared
    env = typ.cast("dict[str, typ.Any]", measure_job.get("env") or {})
    assert env.get("UV_PYTHON") == bound, env
    lanes = matrix_versions(measure_job, "python-version")
    assert lanes == ["3.12", "3.13", "3.14"], lanes
    for job_id, job in _publisher_jobs(document):
        assert matrix_versions(job, "python-version") == lanes, job_id


def test_a_failed_sibling_lane_does_not_withdraw_the_upload() -> None:
    """Only the 3.14 lane's report is needed, so its failure is the only bar.

    The job runs after a failed dependency, and its report fetch stays
    unguarded: a missing 3.14 report then fails the job instead of being read
    as "nothing to upload", so the workflow cannot report success without
    publishing.
    """
    document = repository_documents()[PUBLISHER_WORKFLOW]
    uploads = steps_using({PUBLISHER_WORKFLOW: document}, UPLOAD_MARKER)
    assert len(uploads) == 1, uploads
    job = typ.cast("dict[str, typ.Any]", document["jobs"][uploads[0][1]])
    assert job.get("if") == PARTIAL_FAILURE_CONDITION, job.get("if")
    steps = {
        str(step.get("name")): step
        for step in typ.cast("list[dict[str, typ.Any]]", job.get("steps") or [])
    }
    fetch = steps.get("Fetch the measured report")
    assert fetch is not None, sorted(steps)
    assert "continue-on-error" not in fetch, fetch


def test_the_upload_reads_the_measured_commit() -> None:
    """The uploader attributes the report from a clone, so it holds one.

    A fresh workspace holds no repository metadata, and the report alone does
    not say which branch and commit it describes. The job therefore checks out
    the measured revision, with the token left unpersisted.
    """
    document = repository_documents()[PUBLISHER_WORKFLOW]
    uploads = steps_using({PUBLISHER_WORKFLOW: document}, UPLOAD_MARKER)
    job = typ.cast("dict[str, typ.Any]", document["jobs"][uploads[0][1]])
    assert job.get("permissions") == {"contents": "read"}, job.get("permissions")
    steps = typ.cast("list[dict[str, typ.Any]]", job.get("steps") or [])
    checkouts = [
        step for step in steps if "actions/checkout@" in str(step.get("uses", ""))
    ]
    assert len(checkouts) == 1, checkouts
    shared = checkouts[0].get("with") or {}
    assert shared.get("ref") == "${{ github.sha }}", shared
    assert shared.get("persist-credentials") is False, shared
