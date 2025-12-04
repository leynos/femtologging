from collections.abc import Callable
from re import Pattern
from typing import (
    Any,
    ContextManager,
    ParamSpec,
    TypeVar,
    overload,
)

P = ParamSpec("P")
R = TypeVar("R", bound=BaseException)

# Pytest exposes a decorator with many optional parameters. We mirror
# the real signature here for accurate type checking even though it
# exceeds the usual argument count threshold.

@overload
def fixture[**P, R: BaseException](func: Callable[P, R]) -> Callable[P, R]: ...
@overload
def fixture(
    *,
    scope: str | None = ...,
    autouse: bool | None = ...,
    params: list[Any] | None = ...,
    ids: list[str] | Callable[[Any], str] | None = ...,
    name: str | None = ...,
) -> Callable[[Callable[P, R]], Callable[P, R]]: ...
def raises[R: BaseException](
    exc: type[R],
    match: str | Pattern[str] | None = ...,
    *,
    msg: str | None = ...,
) -> ContextManager[R]: ...

class mark:
    @staticmethod
    def parametrize(
        argnames: str | list[str],
        argvalues: list[Any] | tuple[Any, ...] | list[tuple[Any, ...]],
        *,
        ids: list[str] | Callable[[Any], str] | None = ...,
        indirect: bool | list[str] = ...,
        **kwargs: Any,
    ) -> Callable[[Callable[..., Any]], Callable[..., Any]]: ...
