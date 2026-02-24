"""Protocol definitions for femtologging configuration builders."""

from __future__ import annotations

import typing as typ


class _ConfigBuilder(typ.Protocol):
    """Protocol describing the builder interface used by ``dictConfig``."""

    def with_filter(self, fid: str, builder: object) -> typ.Self: ...

    def with_formatter(self, fid: str, builder: object) -> typ.Self: ...

    def with_handler(self, hid: str, builder: object) -> typ.Self: ...

    def with_logger(self, lname: str, builder: object) -> typ.Self: ...

    def with_root_logger(self, builder: object) -> typ.Self: ...

    def build_and_init(self) -> None: ...
