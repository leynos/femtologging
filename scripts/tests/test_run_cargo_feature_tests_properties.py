"""Property tests for the Cargo feature-matrix test driver.

The example tests in :mod:`test_run_cargo_feature_tests` pin a failing first
selection, a failing last selection and the all-pass case. The interesting
input is the position of the first failure, which no example list
enumerates, so these properties walk arbitrary selection lists and exit
codes. Run this file through ``make feature-tests-runner-test``.
"""

from __future__ import annotations

import dataclasses as dc
import typing as typ

import run_cargo_feature_tests
from hypothesis import given, settings
from hypothesis import strategies as st
from test_run_cargo_feature_tests import HARNESS_ARGS, manifest_path, selects

if typ.TYPE_CHECKING:
    import collections.abc as cabc


@dc.dataclass(slots=True)
class RecordingRunner:
    """A runner that replays fixed outcomes and records what it was asked to do."""

    outcomes: list[int]
    calls: list[tuple[str, ...]] = dc.field(default_factory=list)

    def __call__(self, argv: cabc.Sequence[str]) -> int:
        """Record one invocation and return its scripted outcome.

        Returns
        -------
            The next scripted exit code.
        """
        self.calls.append(tuple(argv))
        return self.outcomes[len(self.calls) - 1]


#: Feature-set names, including duplicates and the reserved name, since a
#: caller may repeat a selection and the driver must not care.
FEATURE_SET_CHOICES: typ.Final[tuple[str, ...]] = (
    "none",
    "python",
    "log-compat",
    "python,log-compat",
    "tracing-compat",
)
FEATURE_SET_NAMES = st.sampled_from(FEATURE_SET_CHOICES)

#: Exit codes a selection may report. 0 passes; the rest are the varied
#: non-zero codes Cargo actually uses.
EXIT_CODES = st.sampled_from([0, 1, 2, 101, 130, 255])


@settings(max_examples=200, deadline=None)
@given(
    feature_sets=st.lists(FEATURE_SET_NAMES, min_size=1, max_size=6),
    outcomes=st.lists(EXIT_CODES, min_size=6, max_size=6),
)
def test_the_run_stops_at_the_first_failing_feature_set(
    feature_sets: list[str], outcomes: list[int]
) -> None:
    """Scenario: any selection list, with failures at any positions.

    Invariant: the driver invokes Cargo once per selection up to and
    including the first failure, never afterwards, and returns that
    selection's exit code; with no failure it invokes every selection and
    returns 0.

    Stated as a property because the interesting input is the position of the
    first failure, which no example list enumerates: the example tests cover a
    failing first selection, a failing last selection and the all-pass case,
    and this covers the middle, repeated failures, duplicate names and exit
    codes other than 101.
    """
    runner = RecordingRunner(outcomes=list(outcomes))
    plan = run_cargo_feature_tests.FeatureTestPlan(
        manifest=manifest_path(),
        feature_sets=tuple(feature_sets),
        test_args=HARNESS_ARGS,
    )

    result = run_cargo_feature_tests.run_feature_sets(plan, runner)

    first_failure = next(
        (
            index
            for index, code in enumerate(outcomes[: len(feature_sets)])
            if code != 0
        ),
        None,
    )
    if first_failure is None:
        assert result == 0, "a run with no failing selection must succeed"
        assert len(runner.calls) == len(feature_sets), "every selection must be tested"
        return
    assert result == outcomes[first_failure], (
        "the run must return the first failing selection's exit code"
    )
    assert len(runner.calls) == first_failure + 1, (
        "the run must stop at the first failing selection"
    )
    for index, call in enumerate(runner.calls):
        assert selects(call, feature_sets[index]), (
            "each invocation must carry its own feature selection"
        )


@settings(max_examples=50, deadline=None)
@given(feature_sets=st.lists(FEATURE_SET_NAMES, min_size=1, max_size=4))
def test_a_run_of_passing_feature_sets_never_stops_early(
    feature_sets: list[str],
) -> None:
    """Scenario: every selection accepts the code.

    Invariant: the driver succeeds and invokes every selection exactly once,
    so a clean result can never come from skipping work.
    """
    runner = RecordingRunner(outcomes=[0] * len(feature_sets))
    plan = run_cargo_feature_tests.FeatureTestPlan(
        manifest=manifest_path(),
        feature_sets=tuple(feature_sets),
        test_args=HARNESS_ARGS,
    )

    assert run_cargo_feature_tests.run_feature_sets(plan, runner) == 0, (
        "every selection passing succeeds"
    )
    assert [call[-len(HARNESS_ARGS) :] for call in runner.calls] == [
        HARNESS_ARGS for _ in feature_sets
    ], "every selection must carry the harness arguments"
