"""Properties of the heavy lane step's readers.

Each case here pins a boundary the feature-matrix contract depends on: which
``set`` forms disable exit-on-error, how a shell comment is tokenized, and how
GitHub's shell inheritance resolves. They live apart from
``test_feature_test_lane_contract.py`` so that module stays a list of
judgements about the workflow rather than a test of the readers themselves.
"""

from __future__ import annotations

import pytest

from tests.feature_lane_step_readers import (
    declared_shell,
    disables_errexit,
    effective_shell,
    tokenize,
)


@pytest.mark.parametrize(
    ("script", "expected"),
    [
        pytest.param("set +e", True, id="plus-e"),
        pytest.param("set +o errexit", True, id="plus-o-errexit"),
        pytest.param("set -e", False, id="dash-e"),
        pytest.param("set -o errexit", False, id="dash-o-errexit"),
        pytest.param("set -euo pipefail", False, id="combined-enable"),
        pytest.param("echo 'set +e'", False, id="quoted-argument"),
        pytest.param("cargo test", False, id="not-a-set-builtin"),
        pytest.param("set -x", False, id="unrelated-option"),
    ],
)
def test_the_errexit_reader_distinguishes_enabling_from_disabling(
    script: str, *, expected: bool
) -> None:
    """Scenario: the script holds any ``set`` form.

    Invariant: only a genuine disabling form is reported. A reader that
    matched the substring ``set`` would flag ``set -e``, which is the opposite
    of the property, and a reader that ignored quoting would flag an argument.
    """
    assert [bool(disables_errexit(command)) for command in tokenize(script)] == [
        expected
    ], script


@pytest.mark.parametrize(
    ("script", "expected"),
    [
        pytest.param(
            "# Check the driver's result\nuv run --script x.py",
            [["uv", "run", "--script", "x.py"]],
            id="comment-apostrophe",
        ),
        pytest.param(
            "uv run --script x.py  # trailing note",
            [["uv", "run", "--script", "x.py"]],
            id="trailing-comment",
        ),
        pytest.param(
            'echo "a # b"',
            [["echo", "a # b"]],
            id="quoted-hash",
        ),
        pytest.param("# only a comment", [], id="comment-only"),
    ],
)
def test_the_reader_ignores_shell_comments(
    script: str, expected: list[list[str]]
) -> None:
    """Scenario: the step gains a comment, with or without an apostrophe.

    Invariant: comments are not read as syntax, and a quoted ``#`` stays
    argument content. Without comment handling, ``shlex.split`` raises on the
    apostrophe and a comment-only workflow edit breaks the contract.
    """
    assert tokenize(script) == expected, script


def test_a_custom_job_shell_default_is_resolved() -> None:
    """Scenario: the job declares a shell template that does not fail fast.

    Invariant: the resolver reports the job's template rather than the absent
    step-level key, so ``bash {0}`` cannot pass as the safe built-in default.
    """
    assert effective_shell() is None, (
        "the committed workflow must run under the built-in fail-fast shell"
    )
    job = {"defaults": {"run": {"shell": "bash {0}"}}}
    assert declared_shell(job) == "bash {0}", "a job-level shell default must be read"
    workflow = {"defaults": {"run": {"shell": "python {0}"}}}
    assert declared_shell(workflow) == "python {0}", (
        "a workflow-level shell default must be read"
    )
