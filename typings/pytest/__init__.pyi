from typing import (
    Any,
    Callable,
    ContextManager,
    Pattern,
    Type,
    TypeVar,
    overload,
    ParamSpec,
)

P = ParamSpec("P")
R = TypeVar("R", bound=BaseException)

@overload
def fixture(func: Callable[P, R]) -> Callable[P, R]: ...
@overload
def fixture(
    *,
    scope: str | None = ...,
    autouse: bool | None = ...,
    params: list[Any] | None = ...,
    ids: list[str] | Callable[[Any], str] | None = ...,
    name: str | None = ...,
) -> Callable[[Callable[P, R]], Callable[P, R]]: ...
def raises(
    exc: Type[R],
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
