from typing import Any, Callable, Type, ContextManager

def fixture(func: Callable[..., Any]) -> Callable[..., Any]: ...
def raises(exc: Type[BaseException]) -> ContextManager[BaseException]: ...

class mark:
    @staticmethod
    def parametrize(
        argnames: str, argvalues: list[tuple[Any, ...]]
    ) -> Callable[[Callable[..., Any]], Callable[..., Any]]: ...
