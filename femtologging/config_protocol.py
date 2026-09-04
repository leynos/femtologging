"""Protocol definitions for femtologging configuration builders."""

from __future__ import annotations

import typing as typ


class _ConfigBuilder(typ.Protocol):
    """Protocol describing the builder interface used by ``dictConfig``."""

    def with_filter(self, fid: str, builder: object) -> typ.Self:
        """Register a filter under ``fid``, returning ``self`` for chaining."""

    def with_formatter(self, fid: str, builder: object) -> typ.Self:
        """Register a formatter under ``fid``, returning ``self`` for chaining."""

    def with_handler(self, hid: str, builder: object) -> typ.Self:
        """Register a handler under ``hid``, returning ``self`` for chaining."""

    def with_logger(self, lname: str, builder: object) -> typ.Self:
        """Register a named logger's configuration, returning ``self`` for chaining."""

    def with_root_logger(self, builder: object) -> typ.Self:
        """Register the root logger's configuration, returning ``self`` for chaining."""

    def build_and_init(self) -> None:
        """Materialise all registered components and install them globally.

        Implementors must apply the accumulated configuration atomically:
        callers rely on there being no partially-initialised logging state
        if construction fails partway through.
        """


class _LoggerMutationBuilder(typ.Protocol):
    """Protocol matching the concrete ``LoggerMutationBuilder`` API."""

    def with_level(self, level: object) -> typ.Self:
        """Set the logger's effective level, returning ``self`` for chaining."""

    # ruff: ignore[boolean-type-hint-positional-argument] the protocol mirrors
    # the public builder API's positional boolean setter.
    def with_propagate(self, propagate: bool) -> typ.Self:
        """Set whether records propagate to ancestor loggers, returning ``self``."""

    def replace_handlers(self, ids: list[str]) -> typ.Self:
        """Replace the logger's entire handler set with ``ids``, returning ``self``.

        Unlike :meth:`append_handlers`, existing handler associations are
        discarded rather than merged.
        """

    def append_handlers(self, ids: list[str]) -> typ.Self:
        """Add ``ids`` to the logger's existing handler set, returning ``self``."""

    def remove_handlers(self, ids: list[str]) -> typ.Self:
        """Remove ``ids`` from the logger's handler set, returning ``self``."""

    def clear_handlers(self) -> typ.Self:
        """Detach all handlers from the logger, returning ``self`` for chaining."""

    def replace_filters(self, ids: list[str]) -> typ.Self:
        """Replace the logger's entire filter set with ``ids``, returning ``self``.

        Unlike :meth:`append_filters`, existing filter associations are
        discarded rather than merged.
        """

    def append_filters(self, ids: list[str]) -> typ.Self:
        """Add ``ids`` to the logger's existing filter set, returning ``self``."""

    def remove_filters(self, ids: list[str]) -> typ.Self:
        """Remove ``ids`` from the logger's filter set, returning ``self``."""

    def clear_filters(self) -> typ.Self:
        """Detach all filters from the logger, returning ``self`` for chaining."""

    def as_dict(self) -> dict[str, object]:
        """Return the accumulated mutation as a ``dictConfig``-style mapping."""


class _RuntimeConfigBuilder(typ.Protocol):
    """Protocol describing the runtime mutation builder interface."""

    def with_filter(self, fid: str, builder: object) -> typ.Self:
        """Register a filter mutation under ``fid``, returning ``self``."""

    def with_handler(self, hid: str, builder: object) -> typ.Self:
        """Register a handler mutation under ``hid``, returning ``self``."""

    def with_logger(
        self,
        lname: str,
        builder: _LoggerMutationBuilder,
    ) -> typ.Self:
        """Register a mutation for the named logger, returning ``self``."""

    def with_root_logger(self, builder: _LoggerMutationBuilder) -> typ.Self:
        """Register a mutation for the root logger, returning ``self``."""

    def apply(self) -> None:
        """Apply all registered mutations to the live logging configuration.

        Implementors must apply mutations atomically: a failure partway
        through must not leave the runtime configuration in a mixed state
        of applied and unapplied changes.
        """

    def as_dict(self) -> dict[str, object]:
        """Return the accumulated mutation as a ``dictConfig``-style mapping."""
