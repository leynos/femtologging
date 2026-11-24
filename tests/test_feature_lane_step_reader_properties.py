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
    shell_fails_fast,
    tokenize,
    tokenize_statements,
)


@pytest.mark.parametrize(
    ("script", "expected"),
    [
        pytest.param("set +e", True, id="plus-e"),
        pytest.param("set +o errexit", True, id="plus-o-errexit"),
        pytest.param("set -e", False, id="dash-e"),
        pytest.param("set -o errexit", False, id="dash-o-errexit"),
        pytest.param("set -euo pipefail", False, id="combined-enable"),
        pytest.param("set +ex", True, id="combined-disabling-e"),
        pytest.param("set +xe", True, id="combined-disabling-e-last"),
        pytest.param("set -e +e", True, id="disabling-wins-when-later"),
        pytest.param("set +e -e", False, id="enabling-wins-when-later"),
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
    A long option such as ``--noprofile`` is one word, so its ``e`` is not the
    flag.
    """
    assert [
        bool(disables_errexit(statement)) for statement in tokenize_statements(script)
    ] == [expected], script


@pytest.mark.parametrize(
    ("script", "expected"),
    [
        pytest.param("echo ready; set +e", True, id="after-semicolon"),
        pytest.param("echo ready && set +e", True, id="after-and"),
        pytest.param("echo ready || set +e", True, id="after-or"),
        pytest.param("echo ready | set +e", True, id="after-pipe"),
        pytest.param("echo 'a; b'", False, id="quoted-semicolon"),
        pytest.param("echo a > out; set -e", False, id="after-redirection"),
    ],
)
def test_the_statement_reader_finds_a_set_after_a_separator(
    script: str, *, expected: bool
) -> None:
    """Scenario: ``set +e`` follows another command on the same line.

    Invariant: the second command is inspected, so a line whose first word is
    harmless cannot hide the option change. A separator inside quotes stays
    part of its word.
    """
    assert (
        any(disables_errexit(statement) for statement in tokenize_statements(script))
        is expected
    ), script


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


@pytest.mark.parametrize(
    ("shell", "expected"),
    [
        pytest.param(None, True, id="built-in-default"),
        pytest.param("bash", True, id="built-in-bash"),
        pytest.param("sh", True, id="built-in-sh"),
        pytest.param("bash -e {0}", True, id="explicit-fail-fast"),
        pytest.param("bash -euo pipefail {0}", True, id="grouped-fail-fast"),
        pytest.param("bash --noprofile -e {0}", True, id="long-option-then-errexit"),
        pytest.param("bash {0}", False, id="template-without-errexit"),
        pytest.param("bash --noprofile {0}", False, id="long-option-only"),
        pytest.param("bash -e +e {0}", False, id="later-disabling-wins"),
        pytest.param("bash {0} -e", False, id="option-after-operand"),
        pytest.param("python {0}", False, id="no-shell-option"),
        pytest.param("bash -e", False, id="no-placeholder"),
    ],
)
def test_the_shell_template_must_leave_errexit_enabled(
    shell: object, *, expected: bool
) -> None:
    """Scenario: a custom ``defaults.run.shell`` template is used.

    Invariant: only the built-in defaults and templates whose final
    ``errexit`` assignment enables it are accepted. Counting the presence of
    ``-e`` would accept ``bash -e +e {0}``, where the later option wins.
    """
    assert shell_fails_fast(shell) is expected, shell
