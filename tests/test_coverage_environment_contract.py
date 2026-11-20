"""Contracts for the protected environment and the jobs allowed to declare it.

The CodeScene token lives in the ``codescene`` environment, whose deployment
policy admits ``main`` alone. GitHub matches environment names
case-insensitively and accepts an expression in place of a literal, so the
reader folds case and refuses a name it cannot resolve rather than reporting a
job safe. These contracts hold both behaviours, the single job that declares
the environment, and the absence of the secret from every workflow's ``env``
blocks under GitHub's case-insensitive secret matching.
"""

from __future__ import annotations

import typing as typ

import pytest

from tests.coverage_contract_helpers import (
    PUBLISHER_WORKFLOW,
    UPLOAD_MARKER,
    jobs_named,
    repository_documents,
    steps_using,
)
from tests.coverage_environment_helpers import (
    CODESCENE_ENVIRONMENT,
    environment_name,
    secret_in_env,
)


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


def test_the_protected_environment_is_declared_by_the_upload_job_alone() -> None:
    """The publisher declares the environment once, on the job that uploads."""
    document = repository_documents()[PUBLISHER_WORKFLOW]
    holders = [
        job_id
        for _, job_id, job in jobs_named({PUBLISHER_WORKFLOW: document})
        if environment_name(job) == CODESCENE_ENVIRONMENT
    ]
    uploads = steps_using({PUBLISHER_WORKFLOW: document}, UPLOAD_MARKER)
    assert len(holders) == 1, holders
    assert uploads, uploads
    assert all(job_id == holders[0] for _, job_id, _ in uploads), uploads


def test_no_workflow_puts_the_token_in_an_environment() -> None:
    """The uploader is composite and would hand a step env to its nested steps."""
    offenders = [
        (file_name, location)
        for file_name, document in repository_documents().items()
        for location in secret_in_env(document)
    ]
    assert not offenders, offenders


@pytest.mark.parametrize(
    ("document", "refusal"),
    [
        ({"env": "${{ secrets }}"}, "an env expression is not verifiable"),
        ({"jobs": {"build": {"env": ["A=B"]}}}, "an env expression is not verifiable"),
    ],
    ids=["expression-valued-env", "list-valued-env"],
)
def test_a_non_mapping_env_declaration_is_refused(
    document: dict[str, typ.Any], refusal: str
) -> None:
    """An ``env`` the reader cannot read is refused, not skipped.

    The runner expands an expression-valued ``env`` into a mapping before the
    job starts, so a reader that skipped it would report the workflow safe
    while a step could still be handed the token. Refusing it makes the
    repository-wide check fail instead of passing on an unread declaration.
    """
    with pytest.raises(AssertionError, match=refusal):
        secret_in_env(document)


@pytest.mark.parametrize(
    ("value", "expected"),
    [
        ("${{ secrets.CS_ACCESS_TOKEN }}", True),
        ("${{ secrets.cs_access_token }}", True),
        ("${{ secrets.Cs_Access_Token }}", True),
        ("${{ toJSON(secrets) }}", True),
        ("${{ format('}}{0}', toJSON(secrets)) }}", True),
        ("${{ format('it''s }}', toJSON(secrets)) }}", True),
        ("${{ toJSON(secrets)", True),
        ("${{ secrets[env.SECRET_NAME] }}", True),
        ("${{ secrets.GITHUB_TOKEN }}", False),
        ("the secrets are safe here", False),
        ("an unrelated value", False),
    ],
    ids=[
        "upper",
        "lower",
        "mixed",
        "aggregate-secrets",
        "quoted-closing-delimiter",
        "doubled-quote",
        "unterminated-expression",
        "indexed-secret",
        "other-secret",
        "plain-text",
        "no-secret",
    ],
)
def test_a_secret_reference_is_matched_without_case(
    value: str, *, expected: bool
) -> None:
    """GitHub folds secret names, so the scan must fold them too.

    A case-sensitive scan would pass ``secrets.cs_access_token`` as safe and
    miss the prohibited placement it is looking for. An expression that reads
    the whole context, or indexes it by a name this reader cannot evaluate,
    can reach the token without naming it, so it is reported as offending
    while a direct reference to another secret stays accepted.
    """
    document: dict[str, typ.Any] = {"env": {"VALUE": value}}
    assert bool(secret_in_env(document)) is expected, value
