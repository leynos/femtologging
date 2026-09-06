"""Raise a nested exception and expose its parts with ``__traceback__`` cleared.

Loaded by ``traceback_capture_tests`` via ``include_str!``. Running this module
leaves ``exc_type``, ``exc_value`` and ``exc_tb`` in the supplied globals, with
``exc_value.__traceback__`` set to ``None`` so that tests can check that a
traceback passed explicitly in a ``(type, value, traceback)`` tuple is still
used for frame extraction.
"""

from __future__ import annotations

from typing import NoReturn


def inner() -> NoReturn:
    """Raise the fixture's inner exception.

    Raises
    ------
    ValueError
        Always raised with the fixture message.
    """
    raise ValueError("test error")


def outer() -> NoReturn:
    """Raise the nested fixture exception.

    Raises
    ------
    ValueError
        Propagated from :func:`inner` after adding an outer traceback frame.
    """
    inner()


try:
    outer()
except ValueError as e:
    exc_type = type(e)
    exc_value = e
    exc_tb = e.__traceback__
    # Clear the exception's __traceback__ to simulate the case where
    # it has been garbage collected or explicitly cleared
    e.__traceback__ = None
