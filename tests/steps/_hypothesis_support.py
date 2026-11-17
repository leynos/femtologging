"""Optional Hypothesis support for traceback normalization tests."""

from __future__ import annotations

import collections.abc as cabc
import importlib
import importlib.util
import typing as typ

import pytest

from tests.steps.traceback_normalization import SYSTEM_EXIT_PYTEST_LINES

type _PropertyDecorator = cabc.Callable[
    [cabc.Callable[..., object]], cabc.Callable[..., object]
]


class _HypothesisModule(typ.Protocol):
    """Subset of Hypothesis used to decorate this optional property test."""

    def given(self, **kwargs: object) -> _PropertyDecorator:
        """Return a property-test decorator for the supplied strategies."""


class _StrategiesModule(typ.Protocol):
    """Subset of Hypothesis strategies used by the generated-input tests."""

    def booleans(self) -> object:
        """Return a boolean-generation strategy."""

    def characters(
        self,
        *,
        categories: tuple[str, ...] | None = None,
        exclude_characters: str = "",
    ) -> object:
        """Return a character-generation strategy."""

    def integers(self, *, min_value: int, max_value: int) -> object:
        """Return an integer-generation strategy."""

    def lists(self, elements: object, *, max_size: int) -> object:
        """Return a strategy of lists drawn from ``elements``."""

    def sampled_from(self, elements: list[str]) -> object:
        """Return a strategy that samples from ``elements``."""

    def text(self, *, alphabet: object, max_size: int) -> object:
        """Return a text-generation strategy."""


_HYPOTHESIS_MISSING_REASON = (
    "Hypothesis has no CPython 3.15 distribution yet; "
    "remove this skip with femtologging issue #385"
)


def _hypothesis_module() -> _HypothesisModule | None:
    """Return the Hypothesis module, or ``None`` when it is not installed."""
    if importlib.util.find_spec("hypothesis") is None:
        return None
    return typ.cast("_HypothesisModule", importlib.import_module("hypothesis"))


def _strategies_module() -> _StrategiesModule | None:
    """Return the Hypothesis strategies module, or ``None`` when it is absent."""
    if importlib.util.find_spec("hypothesis") is None:
        return None
    return typ.cast(
        "_StrategiesModule",
        importlib.import_module("hypothesis.strategies"),
    )


def _entrypoint_property_cases() -> _PropertyDecorator:
    """Return the Hypothesis decorator, or skip when Hypothesis is unavailable."""
    hypothesis = _hypothesis_module()
    strategies = _strategies_module()
    if hypothesis is None or strategies is None:
        return pytest.mark.skip(reason=_HYPOTHESIS_MISSING_REASON)

    return hypothesis.given(
        segment=strategies.text(
            alphabet=strategies.characters(
                exclude_characters='"\n\r\v\f\x1c\x1d\x1e\x85\u2028\u2029'
            ),
            max_size=120,
        ),
        line_no=strategies.integers(min_value=1, max_value=999_999),
        entrypoint_line=strategies.sampled_from(sorted(SYSTEM_EXIT_PYTEST_LINES)),
    )


def _line_property_cases() -> _PropertyDecorator:
    """Return the generator for line text, terminator and final-newline cases."""
    hypothesis = _hypothesis_module()
    strategies = _strategies_module()
    if hypothesis is None or strategies is None:
        return pytest.mark.skip(reason=_HYPOTHESIS_MISSING_REASON)

    line_alphabet = strategies.characters(
        exclude_characters="\r\n\v\f\x1c\x1d\x1e\x85\u2028\u2029"
    )
    return hypothesis.given(
        bodies=strategies.lists(
            strategies.text(alphabet=line_alphabet, max_size=40),
            max_size=6,
        ),
        ending=strategies.sampled_from(["\n", "\r\n"]),
        ends_with_terminator=strategies.booleans(),
    )


def _frame_path_property_cases() -> _PropertyDecorator:
    """Return the generator for path segments that are safe inside a frame line."""
    hypothesis = _hypothesis_module()
    strategies = _strategies_module()
    if hypothesis is None or strategies is None:
        return pytest.mark.skip(reason=_HYPOTHESIS_MISSING_REASON)

    segment_alphabet = strategies.characters(
        categories=("L", "N", "P", "S", "Zs"),
        exclude_characters='"/\\<>:|?*.\x00',
    )
    return hypothesis.given(
        segment=strategies.text(alphabet=segment_alphabet, max_size=40),
    )


_ENTRYPOINT_PROPERTY_CASES = _entrypoint_property_cases()
"""Decorator for entrypoint property cases, or a tracked skip."""

_LINE_PROPERTY_CASES = _line_property_cases()
"""Decorator for line-content property cases, or a tracked skip."""

_FRAME_PATH_PROPERTY_CASES = _frame_path_property_cases()
"""Decorator for frame-path property cases, or a tracked skip."""
