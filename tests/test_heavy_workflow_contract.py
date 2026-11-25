"""Contract tests for the scheduled heavy-tests workflow.

The nightly lane must install its Python test tools from ``uv.lock`` rather
than resolving them at run time. An unpinned ``pip install`` let the runner
resolve pluggy 1.7.0 while the lock pinned 1.6.0, and the ``stack_info``
snapshot recorded pluggy's own source text, so every scheduled run from
15 October 2026 failed. The lane must also make a red nightly visible: a
separate job comments on, or opens, an issue titled ``heavy-tests nightly
failure``.

Each judgement is driven over a deliberately broken copy of the workflow as
well as the real file, so a check that reports nothing cannot pass by
accident. The mutations live in ``heavy_workflow_mutations``; the rules live in
``heavy_workflow_contract_helpers``.
"""

from __future__ import annotations

import copy
import typing as typ

import pytest

from tests.heavy_workflow_contract_helpers import workflow_problems
from tests.heavy_workflow_document import load_workflow
from tests.heavy_workflow_mutations import MUTATIONS

if typ.TYPE_CHECKING:
    import collections.abc as cabc

    from tests.heavy_workflow_document import Document


def test_the_real_heavy_workflow_is_accepted() -> None:
    """The committed workflow satisfies every rule, so the mutations prove something."""
    problems = workflow_problems(load_workflow())
    assert not problems, problems


@pytest.mark.parametrize("mutate", MUTATIONS)
def test_a_broken_heavy_workflow_is_reported(
    mutate: cabc.Callable[[Document], None],
) -> None:
    """Each deliberate break is reported.

    The Loom mutations are the converse of prior-success gating. An earlier
    step is allowed to stop the models from running only by failing, which
    fails the job; GitHub otherwise treats a skipped step as a success. An
    explicit ``if`` on the heavy job or either Loom step, a ``continue-on-error``
    that absorbs a failure, a masked checker status, or a changed preemption
    bound would all restore the "compile passed, models skipped" state this
    lane exists to make impossible, so each is refused and each refusal is
    proved against a broken copy.
    """
    document = copy.deepcopy(load_workflow())
    mutate(document)
    assert workflow_problems(document), "the mutated workflow must be reported"
