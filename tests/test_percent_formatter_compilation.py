"""Behaviour of percent format strings compiled once at construction.

The formatter compiles its format string when it is built, not while a record
is rendered. Unsupported directives therefore survive as literal text rather
than failing configuration, and the work each record performs depends only on
the number of compiled segments. These tests pin the user-visible half of that
contract through the public builders.
"""

from __future__ import annotations

import typing as typ

from femtologging import (
    ConfigBuilder,
    FileHandlerBuilder,
    FormatterBuilder,
    LoggerConfigBuilder,
    get_logger,
    reset_manager,
)
from tests.helpers import poll_file_for_text

if typ.TYPE_CHECKING:
    from pathlib import Path


def test_formatter_preserves_malformed_directives_as_literal_text(
    tmp_path: Path,
) -> None:
    """Unsupported directives render literally while valid fields still resolve."""
    reset_manager()
    path = tmp_path / "malformed-formatter.log"
    handler = (
        FileHandlerBuilder(str(path))
        .with_flush_after_records(1)
        .with_formatter(FormatterBuilder().with_format("%(bad)q %x 100%% %(message)s"))
    )
    config = (
        ConfigBuilder()
        .with_version(1)
        .with_handler("output", handler)
        .with_root_logger(
            LoggerConfigBuilder().with_level("INFO").with_handlers(["output"])
        )
    )

    try:
        config.build_and_init()
        get_logger("app").info("kept")
        poll_file_for_text(path, "kept", timeout=1.0)
    finally:
        reset_manager()

    assert path.read_text().strip() == "%(bad)q %x 100% kept", (
        "malformed directives and lone percent signs must survive as literals"
    )


def test_repeated_records_render_through_one_compiled_format(tmp_path: Path) -> None:
    """A compiled format renders every record, including later directives."""
    reset_manager()
    path = tmp_path / "repeated-records.log"
    handler = (
        FileHandlerBuilder(str(path))
        .with_flush_after_records(3)
        .with_formatter(FormatterBuilder().with_format("%(levelname)s %(message)s"))
    )
    config = (
        ConfigBuilder()
        .with_version(1)
        .with_handler("output", handler)
        .with_root_logger(
            LoggerConfigBuilder().with_level("INFO").with_handlers(["output"])
        )
    )

    try:
        config.build_and_init()
        logger = get_logger("app")
        logger.info("first")
        logger.info("second")
        logger.info("third")
        poll_file_for_text(path, "third", timeout=1.0)
    finally:
        reset_manager()

    assert path.read_text().splitlines() == [
        "INFO first",
        "INFO second",
        "INFO third",
    ], "each record must render from the same compiled segments"
