"""Shared checks for the re-entrant finalizer scenario scripts.

The scripts run in a child interpreter, but pytest's ``--doctest-modules`` also
imports them. A sibling-module import keeps the check in one place and works in
both contexts because the repository root is on ``sys.path`` either way.

Examples
--------
>>> require(True, "a satisfied check stays silent")
>>> try:
...     require(False, "expected failure")
... except AssertionError as error:
...     print(error)
expected failure
"""

from __future__ import annotations


def require(condition: object, message: str) -> None:
    """Abort the scenario with *message* unless *condition* is true.

    ``assert`` is unusable here: ``python -O`` removes assertion statements, so
    a scenario that skipped its checks would still exit zero and print its
    completion marker. Raising explicitly keeps the failure visible whatever
    optimisation level runs the suite.

    Raises
    ------
    AssertionError
        When *condition* is false, carrying *message*.
    """
    if not condition:
        raise AssertionError(message)
