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
accident. The rules live in ``heavy_workflow_contract_helpers``; this module
supplies the mutations and the tests.
"""

from __future__ import annotations

import copy
import typing as typ

import pytest

from tests.heavy_workflow_contract_helpers import workflow_problems
from tests.heavy_workflow_document import (
    BUILD_STEP,
    HEAVY_JOB,
    NOTIFY_JOB,
    SYNC_STEP,
    env,
    job,
    load_workflow,
    named_step,
    raw_steps,
    replace_run,
    run_text,
    steps,
)

if typ.TYPE_CHECKING:
    import collections.abc as cabc

    from tests.heavy_workflow_document import Document


def _unfrozen_sync(document: Document) -> None:
    """Remove ``--frozen`` from the sync."""
    replace_run(document, SYNC_STEP, "uv sync --group dev")


def _pip_install_returns(document: Document) -> None:
    """Reintroduce an unpinned pip install."""
    found = job(document, HEAVY_JOB)
    raw_steps(found, subject=HEAVY_JOB).insert(
        0, {"name": "Install tools", "run": "pip install pytest"}
    )


def _sync_after_build(document: Document) -> None:
    """Move the locked sync after the editable build."""
    found_steps = raw_steps(job(document, HEAVY_JOB), subject=HEAVY_JOB)
    sync_at = next(
        at for at, step in enumerate(found_steps) if step.get("name") == SYNC_STEP
    )
    build_at = next(
        at for at, step in enumerate(found_steps) if step.get("name") == BUILD_STEP
    )
    found_steps[sync_at], found_steps[build_at] = (
        found_steps[build_at],
        found_steps[sync_at],
    )


def _sync_without_test_util_config(document: Document) -> None:
    """Drop the coverage configuration from the sync step."""
    found = job(document, HEAVY_JOB)
    env(named_step(found, SYNC_STEP, subject=HEAVY_JOB)).pop("UV_CONFIG_FILE", None)


def _build_without_test_util(document: Document) -> None:
    """Build the editable extension without the test hooks."""
    replace_run(
        document,
        BUILD_STEP,
        "source .venv/bin/activate\n"
        "maturin develop --manifest-path rust_extension/Cargo.toml "
        "--features python",
    )


def _notification_without_needs(document: Document) -> None:
    """Remove the notification job's dependency on the heavy job."""
    job(document, NOTIFY_JOB).pop("needs", None)


def _notification_always(document: Document) -> None:
    """Run the notification job on every outcome."""
    job(document, NOTIFY_JOB)["if"] = "always()"


def _notification_or_condition(document: Document) -> None:
    """Accept either a failure or a scheduled run, rather than both."""
    job(document, NOTIFY_JOB)["if"] = "failure() || github.event_name == 'schedule'"


def _notification_inverted_condition(document: Document) -> None:
    """Fail on every event except a scheduled one."""
    job(document, NOTIFY_JOB)["if"] = "failure() && github.event_name != 'schedule'"


def _notification_disabled_condition(document: Document) -> None:
    """Keep both required terms but add a conjunct that disables the job."""
    job(document, NOTIFY_JOB)["if"] = (
        "failure() && github.event_name == 'schedule' && false"
    )


def _notification_without_issue_write(document: Document) -> None:
    """Withhold the permission the notification job needs."""
    job(document, NOTIFY_JOB)["permissions"] = {"contents": "read"}


def _notification_without_actions_read(document: Document) -> None:
    """Withhold the scope needed to read the failed run's creation time."""
    job(document, NOTIFY_JOB)["permissions"] = {
        "contents": "read",
        "issues": "write",
    }


def _notification_without_run_creation_time(document: Document) -> None:
    """Ask the run for a field other than its creation time."""
    found = job(document, NOTIFY_JOB)
    step = steps(found, subject=NOTIFY_JOB)[0]
    step["run"] = run_text(step).replace("--json createdAt", "--json databaseId")


def _notification_without_run_url(document: Document) -> None:
    """Drop the run URL from the notification."""
    env(job(document, NOTIFY_JOB)).pop("RUN_URL", None)


def _notification_finds_a_label(document: Document) -> None:
    """Find the tracking issue by a label instead of its exact title."""
    found = job(document, NOTIFY_JOB)
    step = steps(found, subject=NOTIFY_JOB)[0]
    step["run"] = run_text(step).replace(
        "gh issue list --repo", "gh issue list --label nightly --repo"
    )


def _notification_searches_bodies(document: Document) -> None:
    """Search issue bodies rather than titles, so a title match is missed."""
    found = job(document, NOTIFY_JOB)
    step = steps(found, subject=NOTIFY_JOB)[0]
    step["run"] = run_text(step).replace("in:title", "in:body")


def _loom_step_no_longer_last(document: Document) -> None:
    """Insert a step after the Loom models."""
    found = job(document, HEAVY_JOB)
    raw_steps(found, subject=HEAVY_JOB).append({"name": "After", "run": "true"})


_MUTATIONS = (
    pytest.param(_unfrozen_sync, id="unfrozen-sync"),
    pytest.param(_pip_install_returns, id="pip-install-returns"),
    pytest.param(_sync_after_build, id="sync-after-build"),
    pytest.param(_sync_without_test_util_config, id="sync-without-test-util"),
    pytest.param(_build_without_test_util, id="build-without-test-util"),
    pytest.param(_notification_without_needs, id="notification-without-needs"),
    pytest.param(_notification_always, id="notification-always"),
    pytest.param(_notification_or_condition, id="notification-or-condition"),
    pytest.param(
        _notification_inverted_condition,
        id="notification-inverted-condition",
    ),
    pytest.param(
        _notification_disabled_condition,
        id="notification-disabled-condition",
    ),
    pytest.param(_notification_without_issue_write, id="notification-read-only"),
    pytest.param(_notification_without_actions_read, id="notification-no-actions"),
    pytest.param(
        _notification_without_run_creation_time,
        id="notification-wrong-run-field",
    ),
    pytest.param(_notification_without_run_url, id="notification-without-run-url"),
    pytest.param(_notification_finds_a_label, id="notification-by-label"),
    pytest.param(_notification_searches_bodies, id="notification-searches-bodies"),
    pytest.param(_loom_step_no_longer_last, id="loom-not-last"),
)


def test_the_real_heavy_workflow_is_accepted() -> None:
    """The committed workflow satisfies every rule, so the mutations prove something."""
    problems = workflow_problems(load_workflow())
    assert not problems, problems


@pytest.mark.parametrize("mutate", _MUTATIONS)
def test_a_broken_heavy_workflow_is_reported(
    mutate: cabc.Callable[[Document], None],
) -> None:
    """Each deliberate break is reported."""
    document = copy.deepcopy(load_workflow())
    mutate(document)
    assert workflow_problems(document), "the mutated workflow must be reported"
