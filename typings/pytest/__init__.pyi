import collections.abc as cabc
import contextlib as ctxlib
import typing as typ
from re import Pattern

Any = typ.Any
ParamSpec = typ.ParamSpec
TypeVar = typ.TypeVar
overload = typ.overload
AbstractContextManager = ctxlib.AbstractContextManager
Callable = cabc.Callable

P = ParamSpec("P")
R = TypeVar("R")

# Pytest exposes a decorator with many optional parameters. We mirror
# the real signature here for accurate type checking even though it
# exceeds the usual argument count threshold.
def fixture(
    func: Callable[P, R] | None = ...,
    *,
    scope: str | None = ...,
    autouse: bool | None = ...,
    params: list[Any] | None = ...,
    ids: list[str] | Callable[[Any], str] | None = ...,
    name: str | None = ...,
) -> Callable[P, R] | Callable[[Callable[P, R]], Callable[P, R]]: ...
def raises[E: BaseException](
    exc: type[E],
    match: str | Pattern[str] | None = ...,
    *,
    msg: str | None = ...,
) -> AbstractContextManager[E]: ...

class mark:  # noqa: N801 - pytest exports lowercase marker namespace
    @staticmethod
    def parametrize(
        argnames: str | list[str],
        argvalues: list[Any] | tuple[Any, ...] | list[tuple[Any, ...]],
        *,
        ids: list[str] | Callable[[Any], str] | None = ...,
        indirect: bool | list[str] = ...,
        **kwargs: Any,
    ) -> Callable[[Callable[..., Any]], Callable[..., Any]]: ...
