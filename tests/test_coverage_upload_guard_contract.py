"""Contracts for the CodeScene token guard and the upload it protects.

The upload is optional and guarded twice: a token-presence step writes a
boolean output, and the upload step runs only when that output is true and the
ref is ``main``. The token itself reaches the uploader as its ``access-token``
input and sits in no ``env``, because the upload action is composite and hands
a step's environment to the nested steps it runs. These contracts state the
canonical guard, pin seven ways to weaken it, and check that the guard reader
reports every one, so the real publisher can pass the same reader.
"""

from __future__ import annotations

import typing as typ

import pytest

from tests.coverage_contract_helpers import (
    BASELINE_MARKER,
    PUBLISHER_WORKFLOW,
    TOKEN_COMMAND,
    TOKEN_STEP_NAME,
    UPLOAD_CONDITION,
    UPLOAD_MARKER,
    jobs_named,
    repository_documents,
    steps_using,
)
from tests.coverage_environment_helpers import CODESCENE_ENVIRONMENT, secret_in_env


def _token_steps(document: dict[str, typ.Any]) -> list[tuple[str, dict[str, typ.Any]]]:
    """Return the token-presence steps of `document` with their job ids."""
    return [
        (job_id, step)
        for _, job_id, job in jobs_named({PUBLISHER_WORKFLOW: document})
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
    uploads = steps_using({PUBLISHER_WORKFLOW: document}, UPLOAD_MARKER)
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


def test_the_upload_is_guarded_by_token_presence_and_the_trunk() -> None:
    """The real publisher states the canonical guard and binds no secret."""
    problems = _guard_problems(repository_documents()[PUBLISHER_WORKFLOW])
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
