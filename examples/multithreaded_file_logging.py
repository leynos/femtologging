#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.12"
# dependencies = [
#     "femtologging @ {path = \"..\"}",
# ]
# ///
"""Demonstrate femtologging's builder API with a file handler in many threads."""

from __future__ import annotations

from pathlib import Path
from random import randint
from threading import Thread

from femtologging import (
    ConfigBuilder,
    FileHandlerBuilder,
    FormatterBuilder,
    LoggerConfigBuilder,
    get_logger,
)


def configure(logging_path: Path) -> None:
    """Initialise femtologging with a file handler.

    Using the builder pattern ensures the configuration is explicit and easy to
    follow. The handler writes to ``logging_path`` and the root logger forwards
    all records to it.
    """

    fmt = FormatterBuilder().with_format("{asctime} {threadName} {levelname} {message}")

    handler = (
        FileHandlerBuilder(str(logging_path))
        .with_capacity(1024 * 1024)
        .with_formatter("fmt")
    )

    config = (
        ConfigBuilder()
        .with_formatter("fmt", fmt)
        .with_handler("file", handler)
        .with_root_logger(
            LoggerConfigBuilder().with_level("INFO").with_handlers(["file"])
        )
    )
    config.build_and_init()


def worker(thread_id: int) -> None:
    """Generate and log a random range of integers."""
    logger = get_logger("example")
    start = randint(0, 1000)
    stop = start + randint(10, 100)
    for value in range(start, stop):
        logger.info("thread %s produced %s", thread_id, value)


def main() -> None:
    """Configure logging and spawn worker threads."""
    log_path = Path(__file__).with_suffix(".log")
    configure(log_path)

    threads = [Thread(target=worker, args=(i,), name=f"worker-{i}") for i in range(64)]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()


if __name__ == "__main__":
    main()
