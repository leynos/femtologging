"""femtologging package."""

from __future__ import annotations

# Import the Rust extension packaged under this module's namespace first
# to keep imports at the top for linters.
from . import _femtologging_rs as rust  # type: ignore[attr-defined]
from .overflow_policy import OverflowPolicy
import ast
import logging
import sys
from dataclasses import dataclass
from typing import (
    Iterable,
    Mapping,
    MutableMapping,
    Sequence,
    TextIO,
    cast,
    overload,
)

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


# Mapping of string class names to handler builder classes. This mirrors the
# ``logging.config.dictConfig`` expectation of string references while steering
# users toward ``femtologging``'s builders.
_HANDLER_CLASS_MAP: dict[str, type[StreamHandlerBuilder] | type[FileHandlerBuilder]] = {
    "logging.StreamHandler": StreamHandlerBuilder,
    "femtologging.StreamHandler": StreamHandlerBuilder,
    "logging.FileHandler": FileHandlerBuilder,
    "femtologging.FileHandler": FileHandlerBuilder,
}


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
        Logging level.
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


def _coerce_args(args: object) -> list[object]:
    """Convert ``args`` into a list for handler construction.

    ``dictConfig`` accepts either a sequence or a string representing a Python
    literal. The string form is evaluated safely using ``ast.literal_eval`` so
    configuration loaded from text formats (e.g. JSON) can express tuples.
    """
    if isinstance(args, str):
        try:
            args = ast.literal_eval(args)
        except (ValueError, SyntaxError) as exc:
            raise ValueError(f"invalid args: {args}") from exc
    if args is None:
        return []
    if not isinstance(args, Sequence):
        raise ValueError("args must be a sequence")
    return list(args)


def _coerce_kwargs(kwargs: object) -> dict[str, object]:
    """Convert ``kwargs`` into a dictionary for handler construction."""
    if isinstance(kwargs, str):
        try:
            kwargs = ast.literal_eval(kwargs)
        except (ValueError, SyntaxError) as exc:
            raise ValueError(f"invalid kwargs: {kwargs}") from exc
    if kwargs is None:
        return {}
    if not isinstance(kwargs, Mapping):
        raise ValueError("kwargs must be a mapping")
    return dict(kwargs)


def _resolve_handler_class(
    name: str,
) -> type[StreamHandlerBuilder] | type[FileHandlerBuilder]:
    """Return the builder class for ``name`` or raise ``ValueError``."""
    cls = _HANDLER_CLASS_MAP.get(name)
    if cls is None:
        raise ValueError(f"unsupported handler class {name!r}")
    return cls


def _build_handler_from_dict(
    hid: str, data: MutableMapping[str, object]
) -> FileHandlerBuilder | StreamHandlerBuilder:
    """Create a handler builder from ``dictConfig`` handler data."""
    cls_name = data.get("class")
    if not isinstance(cls_name, str):
        raise ValueError(f"handler {hid!r} missing class")
    builder_cls = _resolve_handler_class(cls_name)
    args = _coerce_args(data.get("args"))
    kwargs = _coerce_kwargs(data.get("kwargs"))
    try:
        builder = builder_cls(*args, **kwargs)
    except Exception as exc:  # pragma: no cover - constructor errors propagated
        raise ValueError(f"failed to construct handler {hid!r}: {exc}") from exc
    if data.get("level") is not None:
        raise ValueError("handler level is not supported")
    if data.get("filters"):
        raise ValueError("handler filters are not supported")
    if fmt := data.get("formatter"):
        builder = builder.with_formatter(fmt)
    return builder


def _build_logger_from_dict(
    name: str, data: MutableMapping[str, object]
) -> LoggerConfigBuilder:
    """Create a ``LoggerConfigBuilder`` from ``dictConfig`` logger data."""
    if data.get("filters"):
        raise ValueError("filters are not supported")
    builder = LoggerConfigBuilder()
    if lvl := data.get("level"):
        builder = builder.with_level(lvl)
    if handlers := data.get("handlers"):
        builder = builder.with_handlers(handlers)
    if prop := data.get("propagate"):
        builder = builder.with_propagate(bool(prop))
    return builder


def dictConfig(config: Mapping[str, object]) -> None:
    """Configure logging using a ``dictConfig``-style dictionary."""
    if config.get("incremental"):
        raise ValueError("incremental configuration is not supported")
    version = int(config.get("version", 1))
    if version != 1:
        raise ValueError(f"unsupported configuration version {version}")
    builder = ConfigBuilder().with_version(version)
    if config.get("disable_existing_loggers"):
        builder.with_disable_existing_loggers(True)

    formatters = config.get("formatters", {})
    if not isinstance(formatters, Mapping):
        raise ValueError("formatters must be a mapping")
    for fid, fcfg in formatters.items():
        if not isinstance(fcfg, Mapping):
            raise ValueError("formatter config must be a mapping")
        fb = FormatterBuilder()
        if fmt := fcfg.get("format"):
            fb = fb.with_format(fmt)
        if datefmt := fcfg.get("datefmt"):
            fb = fb.with_datefmt(datefmt)
        builder.with_formatter(fid, fb)

    if config.get("filters"):
        raise ValueError("filters are not supported")

    handlers = config.get("handlers", {})
    if not isinstance(handlers, Mapping):
        raise ValueError("handlers must be a mapping")
    for hid, hcfg in handlers.items():
        if not isinstance(hcfg, MutableMapping):
            raise ValueError("handler config must be a mapping")
        builder.with_handler(hid, _build_handler_from_dict(hid, hcfg))

    loggers = config.get("loggers", {})
    if not isinstance(loggers, Mapping):
        raise ValueError("loggers must be a mapping")
    for name, lcfg in loggers.items():
        if not isinstance(lcfg, MutableMapping):
            raise ValueError("logger config must be a mapping")
        builder.with_logger(name, _build_logger_from_dict(name, lcfg))

    root = config.get("root")
    if not isinstance(root, MutableMapping):
        raise ValueError("root logger configuration is required")
    builder.with_root_logger(_build_logger_from_dict("root", root))

    builder.build_and_init()


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
    "BasicConfig",
    "basicConfig",
    "dictConfig",
    "hello",
]
