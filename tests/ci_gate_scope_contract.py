"""Rules for a gate's matrix tolerance and its effective shell.

A pull-request or scheduled gate is only a gate if its failure is not
tolerated on every leg, and if the shell GitHub runs actually executes the
command. These predicates hold both facts for the workflow contracts.

The matrix rule accepts a bare `matrix.<property>` selector only when the
property resolves, after exclusions, to at least one falsy leg. Anything the
helper cannot enumerate is refused rather than assumed safe.
"""

from __future__ import annotations

import re
import typing as typ

_MATRIX_SELECTOR: typ.Final[re.Pattern[str]] = re.compile(
    r"^\$\{\{\s*matrix\.(?P<property>[A-Za-z_][A-Za-z0-9_]*)\s*\}\}$"
)


def _matrix_values(job: dict[str, object], property_name: str) -> list[object] | None:
    """Return every value the executing matrix legs give ``property_name``.

    Both shapes a matrix may take are read: an axis list and `include` rows.
    An ``exclude`` entry removes a leg from the expansion, so its values are
    subtracted; otherwise ``[false, true]`` with the ``false`` leg excluded
    would read as still having a required leg. Only flat scalar axes are
    evaluated: a matrix built from an ``${{ }}`` expression, or from axes this
    helper cannot enumerate, returns ``None``, and the caller refuses it rather
    than guessing that a required leg survives.

    Returns
    -------
    list[object] | None
        The property's surviving values, or ``None`` when the matrix shape
        cannot be evaluated.
    """
    matrix = _flat_matrix(job)
    if matrix is None:
        return None
    values = _included_values(matrix, property_name)
    if values is None:
        return None
    excluded = matrix.get("exclude", [])
    if not isinstance(excluded, list):
        return None
    for row in excluded:
        if isinstance(row, dict) and row.get(property_name) in values:
            values.remove(row[property_name])
    return values or None


def _flat_matrix(job: dict[str, object]) -> dict[str, object] | None:
    """Return the job's matrix when every axis is a flat scalar list.

    Returns
    -------
    dict[str, object] | None
        The matrix, or ``None`` when it is absent or one axis holds a
        structured or expression value this helper cannot enumerate.
    """
    strategy = job.get("strategy")
    if not isinstance(strategy, dict):
        return None
    matrix = strategy.get("matrix")
    if not isinstance(matrix, dict):
        return None
    for axis, axis_values in matrix.items():
        if axis in {"include", "exclude"}:
            continue
        if not isinstance(axis_values, list) or any(
            isinstance(value, (dict, list)) for value in axis_values
        ):
            return None
    return matrix


def _included_values(
    matrix: dict[str, object], property_name: str
) -> list[object] | None:
    """Return the property's values before any exclusion is applied.

    Returns
    -------
    list[object] | None
        The axis and `include` values, or ``None`` when `include` is not a
        list of mappings.
    """
    axis = matrix.get(property_name, [])
    values: list[object] = list(axis) if isinstance(axis, list) else []
    include = matrix.get("include", [])
    if not isinstance(include, list):
        return None
    values.extend(
        row[property_name]
        for row in include
        if isinstance(row, dict) and property_name in row
    )
    return values


def job_tolerance_is_acceptable(job: dict[str, object]) -> bool:
    """Report whether a job's failure tolerance is acceptable for a gate.

    An allow-list of shapes, not a deny-list of spellings: a deny-list would
    have to enumerate every constant-true expression. A job may omit the key,
    set an explicit `false`, or select a leg with a bare matrix property read,
    which is how an experimental leg is singled out.

    A string containing ``matrix.`` is not enough. GitHub evaluates
    ``${{ true || matrix.experimental }}`` as logical OR, so it is true on
    every leg and would tolerate failure everywhere; the whole value must be
    the bare selector. The selector alone is not enough either: a matrix whose
    every row sets the property true has no required leg left to fail, so at
    least one of the property's values must be falsy.

    Returns
    -------
    bool
        True when the job's failure would still fail at least one leg.
    """
    if "continue-on-error" not in job:
        return True
    tolerance = job["continue-on-error"]
    if tolerance is False:
        return True
    match = _MATRIX_SELECTOR.match(tolerance) if isinstance(tolerance, str) else None
    if match is None:
        return False
    values = _matrix_values(job, match.group("property"))
    return values is not None and any(not value for value in values)


def step_tolerance_is_acceptable(step: dict[str, object]) -> bool:
    """Report whether a step's failure tolerance is acceptable for a gate.

    Stricter than the job rule. A matrix expression on a job says which leg is
    experimental; on a required step it would say this gate may fail.

    Returns
    -------
    bool
        True when the step's failure would still fail its job.
    """
    return "continue-on-error" not in step or step["continue-on-error"] is False
