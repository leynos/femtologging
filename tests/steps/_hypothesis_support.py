"""Optional Hypothesis support for traceback normalization tests."""

from __future__ import annotations

import importlib
import importlib.util
import typing as typ

import pytest

from tests.steps.conftest import _SYSTEM_EXIT_PYTEST_LINES

_PropertyDecorator = typ.Callable[
    [typ.Callable[..., object]], typ.Callable[..., object]
]


class _HypothesisModule(typ.Protocol):
    """Subset of Hypothesis used to decorate this optional property test."""

    def given(self, **kwargs: object) -> _PropertyDecorator:
        """Return a property-test decorator for the supplied strategies."""


class _StrategiesModule(typ.Protocol):
    """Subset of Hypothesis strategies used by the traceback property test."""

    def characters(self, *, exclude_characters: str) -> object:
        """Return a character-generation strategy."""

    def integers(self, *, min_value: int, max_value: int) -> object:
        """Return an integer-generation strategy."""

    def sampled_from(self, elements: list[str]) -> object:
        """Return a strategy that samples from ``elements``."""

    def text(self, *, alphabet: object, max_size: int) -> object:
        """Return a text-generation strategy."""


def _entrypoint_property_cases() -> _PropertyDecorator:
    """Return the Hypothesis decorator, or skip when Hypothesis is unavailable."""
    if importlib.util.find_spec("hypothesis") is None:
        return pytest.mark.skip(
            reason=(
                "Hypothesis has no CPython 3.15 distribution yet; "
                "remove this skip with "
                "https://github.com/leynos/femtologging/issues/385"
            )
        )

    hypothesis = typ.cast(
        "_HypothesisModule",
        importlib.import_module("hypothesis"),
    )
    strategies = typ.cast(
        "_StrategiesModule",
        importlib.import_module("hypothesis.strategies"),
    )

    return hypothesis.given(
        segment=strategies.text(
            alphabet=strategies.characters(
                exclude_characters='"\n\r\v\f\x1c\x1d\x1e\x85\u2028\u2029'
            ),
            max_size=120,
        ),
        line_no=strategies.integers(min_value=1, max_value=999_999),
        entrypoint_line=strategies.sampled_from(sorted(_SYSTEM_EXIT_PYTEST_LINES)),
    )


_ENTRYPOINT_PROPERTY_CASES = _entrypoint_property_cases()
"""Decorator for property cases, or a tracked skip when Hypothesis is absent."""
