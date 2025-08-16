"""femtologging package."""

from __future__ import annotations

# Import the Rust extension packaged under this module's namespace first
# to keep imports at the top for linters.
from . import _femtologging_rs as rust  # type: ignore[attr-defined]
from .overflow_policy import OverflowPolicy
import logging
import sys

hello = rust.hello  # type: ignore[attr-defined]
FemtoLogger = rust.FemtoLogger  # type: ignore[attr-defined]
get_logger = rust.get_logger  # type: ignore[attr-defined]
reset_manager = rust.reset_manager_py  # type: ignore[attr-defined]
FemtoHandler = rust.FemtoHandler  # type: ignore[attr-defined]
FemtoStreamHandler = rust.FemtoStreamHandler  # type: ignore[attr-defined]
FemtoFileHandler = rust.FemtoFileHandler  # type: ignore[attr-defined]
StreamHandlerBuilder = rust.StreamHandlerBuilder  # type: ignore[attr-defined]
FileHandlerBuilder = rust.FileHandlerBuilder  # type: ignore[attr-defined]
ConfigBuilder = rust.ConfigBuilder  # type: ignore[attr-defined]
LoggerConfigBuilder = rust.LoggerConfigBuilder  # type: ignore[attr-defined]
FormatterBuilder = rust.FormatterBuilder  # type: ignore[attr-defined]
HandlerConfigError = rust.HandlerConfigError  # type: ignore[attr-defined]
HandlerIOError = rust.HandlerIOError  # type: ignore[attr-defined]


def basicConfig(
    *,
    level: str | int | None = None,
    filename: str | None = None,
    stream: object | None = None,
    force: bool = False,
    handlers: list | None = None,
) -> None:
    """Configure the root logger using the builder API.

    Parameters mirror ``logging.basicConfig`` but currently only a subset is
    supported. ``level`` may be a string or numeric value understood by the
    standard :mod:`logging` module. ``filename`` configures a
    :class:`FemtoFileHandler`; otherwise a :class:`FemtoStreamHandler`
    targeting ``stderr`` is installed. ``stream`` may be ``sys.stdout`` to
    redirect output. ``force`` removes any existing handlers from the root
    logger before applying the new configuration. ``handlers`` allows attaching
    pre‑constructed handlers directly.

    Examples
    --------
    Configure a simple stream handler::

        basicConfig(level="INFO")

    Notes
    -----
    ``format`` and ``datefmt`` are intentionally unsupported until formatter
    customisation is implemented.
    """
    if filename and stream:
        raise ValueError("Cannot specify both `filename` and `stream`")

    if handlers and (filename or stream):
        msg = "Cannot specify `handlers` with `filename` or `stream`"
        raise ValueError(msg)

    if force:
        get_logger("root").clear_handlers()

    root = get_logger("root")

    if handlers:
        for h in handlers:
            root.add_handler(h)
    else:
        builder = ConfigBuilder()

        handler_id = "basic_config_handler"
        if filename:
            handler = FileHandlerBuilder(filename)
        else:
            if stream is sys.stdout:
                handler = StreamHandlerBuilder.stdout()
            elif stream in (None, sys.stderr):
                handler = StreamHandlerBuilder.stderr()
            else:
                raise ValueError("stream must be sys.stdout or sys.stderr")
        builder.with_handler(handler_id, handler)

        logger_cfg = LoggerConfigBuilder().with_handlers([handler_id])
        builder.with_root_logger(logger_cfg)

        builder.build_and_init()

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
    "StreamHandlerBuilder",
    "FileHandlerBuilder",
    "ConfigBuilder",
    "LoggerConfigBuilder",
    "FormatterBuilder",
    "HandlerConfigError",
    "HandlerIOError",
    "OverflowPolicy",
    "basicConfig",
    "hello",
]
