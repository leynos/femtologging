"""femtologging package."""

from __future__ import annotations

# Import the Rust extension packaged under this module's namespace first
# to keep imports at the top for linters.
from . import _femtologging_rs as rust
from .overflow_policy import OverflowPolicy
from .config import dictConfig
import logging
import sys
from dataclasses import dataclass
from typing import (
    Callable,
    Iterable,
    TextIO,
    cast,
    overload,
)


hello = rust.hello
FemtoLogger = rust.FemtoLogger
get_logger = rust.get_logger
reset_manager = rust.reset_manager_py
FemtoHandler = rust.FemtoHandler
FemtoStreamHandler = rust.FemtoStreamHandler
FemtoFileHandler = rust.FemtoFileHandler
FemtoRotatingFileHandler = rust.FemtoRotatingFileHandler
HandlerOptions = rust.HandlerOptions
ROTATION_VALIDATION_MSG = rust.ROTATION_VALIDATION_MSG
StreamHandlerBuilder = rust.StreamHandlerBuilder
FileHandlerBuilder = rust.FileHandlerBuilder
RotatingFileHandlerBuilder = rust.RotatingFileHandlerBuilder
ConfigBuilder = rust.ConfigBuilder
LoggerConfigBuilder = rust.LoggerConfigBuilder
FormatterBuilder = rust.FormatterBuilder
LevelFilterBuilder = rust.LevelFilterBuilder
NameFilterBuilder = rust.NameFilterBuilder
FilterBuildError = rust.FilterBuildError
HandlerConfigError = rust.HandlerConfigError
HandlerIOError = rust.HandlerIOError
_force_rotating_fresh_failure = getattr(
    rust, "force_rotating_fresh_failure_for_test", None
)
_clear_rotating_fresh_failure = getattr(
    rust, "clear_rotating_fresh_failure_for_test", None
)

if callable(_force_rotating_fresh_failure) and callable(_clear_rotating_fresh_failure):
    _force_rotating_fresh_failure_for_test = cast(
        Callable[[int, str | None], None],
        _force_rotating_fresh_failure,
    )
    _clear_rotating_fresh_failure_for_test = cast(
        Callable[[], None],
        _clear_rotating_fresh_failure,
    )
else:
    # Feature disabled: expose no-ops that fail loudly when invoked.

    def _force_rotating_fresh_failure_for_test(
        count: int, reason: str | None = None
    ) -> None:
        raise RuntimeError(
            "rotating fresh-failure hook requires the extension built with the 'python' feature"
        )

    def _clear_rotating_fresh_failure_for_test() -> None:
        return


@dataclass
class BasicConfig:
    """Configuration parameters for basicConfig()."""

    level: str | int | None = None
    filename: str | None = None
    stream: TextIO | None = None
    force: bool = False
    handlers: Iterable[FemtoHandler] | None = None


@overload
def basicConfig(config: BasicConfig, /) -> None: ...


@overload
def basicConfig(**kwargs: object) -> None: ...


def basicConfig(config: BasicConfig | None = None, /, **kwargs: object) -> None:
    """Configure the root logger using the builder API.

        Parameters mirror ``logging.basicConfig`` but currently only a subset is
        supported. ``config`` may be a :class:`BasicConfig` instance; if provided,
        its values take precedence over individual parameters. ``level`` may be a
        string or numeric value understood by the standard :mod:`logging` module.
        ``filename`` configures a :class:`FemtoFileHandler`; otherwise a
        :class:`FemtoStreamHandler` targeting ``stderr`` is installed. ``stream``
        may be ``sys.stdout`` to redirect output. ``force`` removes any existing
        handlers from the root logger before applying the new configuration.
        ``handlers`` allows attaching pre‑constructed handlers directly.

        Parameters
        ----------
        config : BasicConfig, optional
            Configuration dataclass providing parameters for ``basicConfig``.

        Other Parameters
        ----------------
        level : str or int, optional
            Logging level. Accepts case-insensitive "TRACE", "DEBUG", "INFO", "WARN",
    "WARNING", "ERROR", and "CRITICAL". "WARN" and "WARNING" are equivalent.
        filename : str, optional
            File to write logs to.
        stream : TextIO, optional
            ``sys.stdout`` or ``sys.stderr``.
        force : bool, default False
            Remove any existing handlers before configuring.
        handlers : Iterable[FemtoHandler], optional
            Pre‑constructed handlers to attach.

        Examples
        --------
        Using a dataclass::

            cfg = BasicConfig(level="INFO")
            basicConfig(cfg)

        Using individual parameters::

            basicConfig(level="INFO")

        Notes
        -----
        ``format`` and ``datefmt`` are intentionally unsupported until formatter
        customisation is implemented.
    """
    allowed = {"level", "filename", "stream", "force", "handlers"}
    unknown = set(kwargs) - allowed
    if unknown:
        name = next(iter(unknown))
        raise TypeError(f"basicConfig() got an unexpected keyword argument {name!r}")

    level: str | int | None
    filename: str | None
    stream: TextIO | None
    force: bool
    handlers: Iterable[FemtoHandler] | None
    if config is not None:
        level = (
            config.level
            if config.level is not None
            else cast(str | int | None, kwargs.get("level"))
        )
        filename = (
            config.filename
            if config.filename is not None
            else cast(str | None, kwargs.get("filename"))
        )
        stream = (
            config.stream
            if config.stream is not None
            else cast(TextIO | None, kwargs.get("stream"))
        )
        force = bool(config.force)
        handlers = (
            config.handlers
            if config.handlers is not None
            else cast(Iterable[FemtoHandler] | None, kwargs.get("handlers"))
        )
    else:
        level = cast(str | int | None, kwargs.get("level"))
        filename = cast(str | None, kwargs.get("filename"))
        stream = cast(TextIO | None, kwargs.get("stream"))
        force = bool(kwargs.get("force", False))
        handlers = cast(Iterable[FemtoHandler] | None, kwargs.get("handlers"))

    _validate_basic_config_params(filename, stream, handlers)

    root = get_logger("root")
    if force:
        root.clear_handlers()

    _configure_handlers(root, handlers, filename, stream)

    _set_logger_level(root, level)


def _has_conflicting_handler_params(
    handlers: Iterable[FemtoHandler] | None,
    filename: str | None,
    stream: TextIO | None,
) -> bool:
    """Check if handlers conflict with filename or stream parameters."""
    return handlers is not None and (filename is not None or stream is not None)


def _validate_basic_config_params(
    filename: str | None,
    stream: TextIO | None,
    handlers: Iterable[FemtoHandler] | None,
) -> None:
    """Validate ``basicConfig`` parameters."""
    if filename and stream:
        raise ValueError("Cannot specify both `filename` and `stream`")

    if _has_conflicting_handler_params(handlers, filename, stream):
        msg = "Cannot specify `handlers` with `filename` or `stream`"
        raise ValueError(msg)

    if stream not in (None, sys.stdout, sys.stderr):
        raise ValueError(
            f"stream must be sys.stdout or sys.stderr, got {type(stream)!r}: {stream!r}"
        )


def _configure_handlers(
    root: FemtoLogger,
    handlers: Iterable[FemtoHandler] | None,
    filename: str | None,
    stream: TextIO | None,
) -> None:
    """Attach or build handlers for the root logger."""
    if handlers is not None:
        for h in handlers:
            root.add_handler(h)
    else:
        _build_and_configure_handler(filename, stream)


def _build_and_configure_handler(
    filename: str | None,
    stream: TextIO | None,
) -> None:
    """Build a handler via the builder API and install it."""
    builder = ConfigBuilder()

    handler_id = "basic_config_handler"
    handler = _create_handler_builder(filename, stream)
    builder.with_handler(handler_id, handler)

    logger_cfg = LoggerConfigBuilder().with_handlers([handler_id])
    builder.with_root_logger(logger_cfg)

    builder.build_and_init()


def _create_handler_builder(
    filename: str | None,
    stream: TextIO | None,
) -> FileHandlerBuilder | StreamHandlerBuilder:
    """Create a handler builder for ``basicConfig``."""
    if filename:
        return FileHandlerBuilder(filename)
    if stream is sys.stdout:
        return StreamHandlerBuilder.stdout()
    return StreamHandlerBuilder.stderr()


def _set_logger_level(root: FemtoLogger, level: str | int | None) -> None:
    """Set the root logger level if provided."""
    if level is not None:
        lvl = logging.getLevelName(level) if isinstance(level, int) else level
        root.set_level(lvl)


__all__ = [
    "FemtoHandler",
    "FemtoLogger",
    "get_logger",
    "reset_manager",
    "FemtoStreamHandler",
    "FemtoFileHandler",
    "FemtoRotatingFileHandler",
    "HandlerOptions",
    "ROTATION_VALIDATION_MSG",
    "StreamHandlerBuilder",
    "FileHandlerBuilder",
    "RotatingFileHandlerBuilder",
    "LevelFilterBuilder",
    "NameFilterBuilder",
    "FilterBuildError",
    "ConfigBuilder",
    "LoggerConfigBuilder",
    "FormatterBuilder",
    "HandlerConfigError",
    "HandlerIOError",
    "OverflowPolicy",
    "BasicConfig",
    "basicConfig",
    "dictConfig",
    "hello",
]
