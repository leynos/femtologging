"""Contracts for the split between pull-request coverage and main publication.

CV-005 keeps coverage generation, and its ratchet, in the pull-request lane,
and moves both persistent outputs to one default-branch publisher. These tests
read the workflows as documents and hold that split: one publisher on a push
to ``main``, one protected environment used by that publisher alone, a token
that travels only as the uploader's input, an upload guarded by both token
presence and branch, and two lanes that measure the same population.
"""

from __future__ import annotations

import typing as typ

import pytest

from tests.coverage_contract_helpers import (
    BASELINE_MARKER,
    GENERATE_MARKER,
    TOKEN_COMMAND,
    TOKEN_STEP_NAME,
    UPLOAD_CONDITION,
    UPLOAD_MARKER,
    coverage_inputs,
    jobs_named,
    pushes_to,
    repository_documents,
    steps_using,
    triggers,
)
from tests.coverage_environment_helpers import (
    CODESCENE_ENVIRONMENT,
    TOKEN_NAME,
    environment_name,
    secret_in_env,
)
from tests.coverage_reach_helpers import pull_request_reach
from tests.runner_placement_judgement import selected_runner

CI_WORKFLOW: typ.Final[str] = "ci.yml"
PUBLISHER: typ.Final[str] = "coverage-main.yml"
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


def _token_steps(document: dict[str, typ.Any]) -> list[tuple[str, dict[str, typ.Any]]]:
    """Return the token-presence steps of `document` with their job ids."""
    return [
        (job_id, step)
        for _, job_id, job in jobs_named({PUBLISHER: document})
        for step in job.get("steps") or []
        if step.get("name") == TOKEN_STEP_NAME
    ]


def _token_guard_problems(
    document: dict[str, typ.Any],
    uploads: list[tuple[str, str, dict[str, typ.Any]]],
) -> list[str]:
    """Return one complaint per way the token-presence step could be weakened."""
    tokens = _token_steps(document)
    if len(tokens) != 1:
        return [f"expected one token-presence step, found {len(tokens)}"]
    job_id, token_step = tokens[0]
    problems: list[str] = []
    if token_step.get("id") != "codescene-token":
        problems.append("the token step must expose the `codescene-token` id")
    if token_step.get("run") != TOKEN_COMMAND:
        problems.append("the token step must run the canonical command alone")
    if not any(upload_job == job_id for _, upload_job, _ in uploads):
        problems.append("the upload must run beside the token-presence step")
    return problems


def _upload_guard_problems(
    uploads: list[tuple[str, str, dict[str, typ.Any]]],
) -> list[str]:
    """Return one complaint per way the upload step could be weakened."""
    if len(uploads) != 1:
        return [f"expected one upload step, found {len(uploads)}"]
    upload = uploads[0][2]
    problems: list[str] = []
    if upload.get("if") != UPLOAD_CONDITION:
        problems.append("the upload must require the token and the trunk")
    shared = upload.get("with") or {}
    if shared.get("access-token") != "${{ secrets.CS_ACCESS_TOKEN }}":
        problems.append("the token must travel as the uploader's input")
    return problems


def _guard_problems(document: dict[str, typ.Any]) -> list[str]:
    """Return one complaint per way the upload guard could be weakened."""
    uploads = steps_using({PUBLISHER: document}, UPLOAD_MARKER)
    problems = _token_guard_problems(document, uploads)
    problems.extend(_upload_guard_problems(uploads))
    problems.extend(
        f"the token is in {location}" for location in secret_in_env(document)
    )
    return problems


def _publisher_document(
    *,
    token_run: str = TOKEN_COMMAND,
    upload_if: str = UPLOAD_CONDITION,
    job_env: dict[str, str] | None = None,
) -> dict[str, typ.Any]:
    """Return a synthetic publisher document carrying `token_run` and `upload_if`."""
    upload_job: dict[str, typ.Any] = {
        "runs-on": "ubuntu-latest",
        "environment": CODESCENE_ENVIRONMENT,
        "steps": [
            {"name": TOKEN_STEP_NAME, "id": "codescene-token", "run": token_run},
            {
                "name": "Upload coverage data to CodeScene",
                "if": upload_if,
                "uses": f"example/{UPLOAD_MARKER}@sha",
                "with": {"access-token": "${{ secrets.CS_ACCESS_TOKEN }}"},
            },
        ],
    }
    if job_env is not None:
        upload_job["env"] = job_env
    return {
        "name": "Coverage (main)",
        "on": {"push": {"branches": ["main"]}},
        "jobs": {
            "coverage-upload": upload_job,
            "coverage-baseline": {
                "needs": "coverage-upload",
                "runs-on": "ubicloud-standard-2",
                "steps": [{"uses": f"example/{BASELINE_MARKER}@sha"}],
            },
        },
    }


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
    assert publishers == [PUBLISHER], publishers


def test_the_publisher_runs_only_on_a_push_to_the_trunk() -> None:
    """The publisher declares one trigger, a push restricted to `main`.

    `pushes_to` answers whether a push to main is accepted, which a widened
    filter still satisfies. The exact mapping is asserted here so an added
    trigger, or a `feature/*` branch, fails the contract.
    """
    document = repository_documents()[PUBLISHER]
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
    assert [file_name for file_name, _ in uploaders] == [PUBLISHER], uploaders


def test_an_environment_name_is_matched_case_insensitively() -> None:
    """GitHub folds environment names, so the reader must fold them too.

    Returning the declaration verbatim would report no protected environment
    for a job that declares one in another case.
    """
    scalar = environment_name({"environment": "CodeScene"})
    mapping = {"name": "CODESCENE", "url": "${{ github.ref }}"}
    mapped = environment_name({"environment": mapping})
    assert scalar == CODESCENE_ENVIRONMENT, scalar
    assert mapped == CODESCENE_ENVIRONMENT, mapped


@pytest.mark.parametrize(
    ("declaration", "refusal"),
    [
        ("${{ inputs.environment }}", "an environment expression is not verifiable"),
        (
            {"name": "${{ inputs.environment }}"},
            "an environment expression is not verifiable",
        ),
        ({"url": "${{ x }}"}, "an environment mapping must declare a name"),
        (7, "an environment must be a name or a mapping"),
    ],
    ids=[
        "scalar-expression",
        "mapping-expression",
        "mapping-without-name",
        "scalar-int",
    ],
)
def test_an_unprovable_environment_declaration_is_refused(
    declaration: object, refusal: str
) -> None:
    """A name this repository cannot resolve, or cannot read, is not guessed."""
    with pytest.raises(AssertionError, match=refusal):
        environment_name({"environment": declaration})


def test_the_publisher_is_not_reachable_from_a_pull_request() -> None:
    """A pull request can reach neither the publisher nor its environment."""
    documents = repository_documents()
    reached, _ = pull_request_reach(documents)
    assert PUBLISHER not in reached, reached
    offenders = [
        (file_name, job_id)
        for file_name in reached
        for _, job_id, job in jobs_named({file_name: documents[file_name]})
        if environment_name(job) == CODESCENE_ENVIRONMENT
    ]
    assert not offenders, offenders


def test_the_protected_environment_is_declared_by_the_upload_job_alone() -> None:
    """The publisher declares the environment once, on the job that uploads."""
    document = repository_documents()[PUBLISHER]
    holders = [
        job_id
        for _, job_id, job in jobs_named({PUBLISHER: document})
        if environment_name(job) == CODESCENE_ENVIRONMENT
    ]
    uploads = steps_using({PUBLISHER: document}, UPLOAD_MARKER)
    assert len(holders) == 1, holders
    assert uploads, uploads
    assert all(job_id == holders[0] for _, job_id, _ in uploads), uploads


def test_the_only_remote_pull_request_call_is_the_recorded_one() -> None:
    """A new call into another repository must be verified, not assumed safe."""
    _, remote = pull_request_reach(repository_documents())
    assert remote == KNOWN_REMOTE_CALLS, remote


def test_the_publisher_never_cancels_and_keys_on_the_ref() -> None:
    """A cancelled publisher abandons its upload and its baseline write."""
    concurrency = repository_documents()[PUBLISHER].get("concurrency")
    assert isinstance(concurrency, dict), concurrency
    assert concurrency.get("group") == "${{ github.workflow }}-${{ github.ref }}", (
        concurrency
    )
    assert concurrency.get("cancel-in-progress") is False, concurrency


def test_the_upload_is_guarded_by_token_presence_and_the_trunk() -> None:
    """The real publisher states the canonical guard and binds no secret."""
    problems = _guard_problems(repository_documents()[PUBLISHER])
    assert not problems, problems


@pytest.mark.parametrize(
    ("token_run", "upload_if", "job_env"),
    [
        (TOKEN_COMMAND, UPLOAD_CONDITION, None),
        ("true", UPLOAD_CONDITION, None),
        (f"{TOKEN_COMMAND}\ntrue", UPLOAD_CONDITION, None),
        (TOKEN_COMMAND, "github.ref == 'refs/heads/main'", None),
        (TOKEN_COMMAND, "steps.codescene-token.outputs.available == 'true'", None),
        (TOKEN_COMMAND, "always()", None),
        (
            TOKEN_COMMAND,
            UPLOAD_CONDITION,
            {"CS_ACCESS_TOKEN": "${{ secrets.CS_ACCESS_TOKEN }}"},
        ),
    ],
    ids=[
        "canonical",
        "missing-step-output",
        "extra-command",
        "token-conjunct-dropped",
        "ref-conjunct-dropped",
        "always",
        "token-in-job-env",
    ],
)
def test_the_guard_reader_notices_each_way_the_upload_can_be_exposed(
    token_run: str, upload_if: str, job_env: dict[str, str] | None
) -> None:
    """Every weakening of the guard is reported, so the real one can pass."""
    problems = _guard_problems(
        _publisher_document(token_run=token_run, upload_if=upload_if, job_env=job_env)
    )
    canonical = (token_run, upload_if, job_env) == (
        TOKEN_COMMAND,
        UPLOAD_CONDITION,
        None,
    )
    assert bool(problems) is not canonical, problems


def test_no_workflow_puts_the_token_in_an_environment() -> None:
    """The uploader is composite and would hand a step env to its nested steps."""
    offenders = [
        (file_name, location)
        for file_name, document in repository_documents().items()
        for location in secret_in_env(document)
    ]
    assert not offenders, offenders


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
    publisher = steps_using({PUBLISHER: documents[PUBLISHER]}, GENERATE_MARKER)
    assert len(pull_request) == 1, pull_request
    assert len(publisher) == 1, publisher
    assert coverage_inputs(pull_request[0][2]) == coverage_inputs(publisher[0][2]), (
        pull_request,
        publisher,
    )


def test_the_baseline_is_published_from_the_runner_the_pull_requests_use() -> None:
    """The hand-over keeps the measurement off the cache store it cannot share."""
    document = repository_documents()[PUBLISHER]
    measures = steps_using({PUBLISHER: document}, GENERATE_MARKER)[0]
    publishes = steps_using({PUBLISHER: document}, BASELINE_MARKER)[0]
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
