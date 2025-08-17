#!/usr/bin/env python3
# /// script
# requires-python = ">=3.12"
# dependencies = [
#     "femtologging @ {path = \"..\"}",
# ]
# ///
"""Demonstrate ``basicConfig`` and stream separation."""

from __future__ import annotations

from femtologging import basicConfig, get_logger


def main() -> None:
    """Configure logging and emit records at common levels.

    With ``basicConfig(level=\"INFO\")``, messages below ``INFO`` are
    suppressed. ``INFO``/``WARNING`` are written to standard output and
    ``ERROR`` to standard error, as routed by the default stream handler.
    """
    basicConfig(level="INFO")
    logger = get_logger("example")

    logger.debug("debug suppressed")
    logger.info("streamed to stdout")
    logger.warning("warning on stdout")
    logger.error("error goes to stderr")


if __name__ == "__main__":
    main()
