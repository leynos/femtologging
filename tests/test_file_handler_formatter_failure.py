"""Close-time behaviour when a Python formatter fails.

A Python-callable formatter runs on the handler's worker thread, so an
exception it raises cannot reach the calling thread. The worker prints the
failure and substitutes a placeholder for the record instead. These tests pin
that shutdown does not depend on the formatter succeeding, so a broken
formatter cannot leave a handler that refuses to close.
"""

from __future__ import annotations

import typing as typ

import pytest

from femtologging import FileHandlerBuilder

if typ.TYPE_CHECKING:
    import collections.abc as cabc
    from pathlib import Path


class FormatterRefusedError(ValueError):
    """Raised by a test formatter to prove worker failures stay contained."""

    def __init__(self) -> None:
        """Carry the refusal reason that the worker prints."""
        super().__init__("formatter refused every record")


def test_close_completes_when_the_formatter_raises(tmp_path: Path) -> None:
    """A raising formatter must not stop the handler from closing.

    The formatter runs on the worker thread, so its exception is printed there
    and replaced with a placeholder rather than propagated. ``close()`` still
    waits for the worker and the handler reports itself closed afterwards.
    """
    path = tmp_path / "raising.log"

    def raising_formatter(_record: cabc.Mapping[str, object]) -> str:
        """Fail every record the worker tries to format."""
        raise FormatterRefusedError

    handler = (
        FileHandlerBuilder(str(path))
        .with_capacity(2)
        .with_flush_after_records(1)
        .with_formatter(raising_formatter)
        .build()
    )
    handler.handle("core", "INFO", "one")

    handler.close()

    with pytest.raises(RuntimeError, match="Handler error: handler is closed"):
        handler.handle("core", "INFO", "after close")
    assert handler.flush() is False, "a closed handler must not acknowledge a flush"
