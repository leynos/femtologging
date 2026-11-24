"""Rules for a gate's matrix tolerance and its effective shell.

A pull-request or scheduled gate is only a gate if its failure is not
tolerated on every leg, and if the shell GitHub runs actually executes the
command. These predicates hold both facts for the workflow contracts.

The matrix rule accepts a bare `matrix.<property>` selector only when the
property resolves, after exclusions, to at least one falsy leg. Anything the
helper cannot enumerate is refused rather than assumed safe.
"""

from __future__ import annotations

import itertools
import re
import typing as typ

if typ.TYPE_CHECKING:
    import collections.abc as cabc

_MATRIX_SELECTOR: typ.Final[re.Pattern[str]] = re.compile(
    r"^\$\{\{\s*matrix\.(?P<property>[A-Za-z_][A-Za-z0-9_]*)\s*\}\}$"
)


def _flat_matrix(job: dict[str, object]) -> dict[str, object] | None:
    """Return the job's matrix mapping, if it has one.

    Returns
    -------
    dict[str, object] | None
        The matrix, or ``None`` when the job has no matrix this helper can
        read.
    """
    strategy = job.get("strategy")
    if not isinstance(strategy, dict):
        return None
    matrix = strategy.get("matrix")
    return matrix if isinstance(matrix, dict) else None


def _matrix_combinations(job: dict[str, object]) -> list[dict[str, object]] | None:
    """Return the matrix combinations GitHub actually executes.

    The expansion follows GitHub's order: the cartesian product of the axes,
    then the ``exclude`` rows remove complete combinations, then each
    ``include`` row is merged into every combination whose original axis values
    it does not overwrite, or appended as a combination of its own when it
    merges into none. Applying those steps in this order matters: an exclusion
    can remove the only required leg, and a later `include` row can overwrite
    an earlier row's added value on the same surviving leg.

    Only flat scalar axes are expanded. A matrix built from an ``${{ }}``
    expression, or an axis this helper cannot enumerate, returns ``None`` and
    the caller refuses it rather than assuming a required leg survives.

    Returns
    -------
    list[dict[str, object]] | None
        The executing combinations, or ``None`` when the shape cannot be
        evaluated.
    """
    matrix = _flat_matrix(job)
    if matrix is None:
        return None
    axes = _flat_axes(matrix)
    if axes is None:
        return None
    combinations = _axis_combinations(axes)
    combinations = _exclude_combinations(combinations, matrix.get("exclude", []))
    if combinations is None:
        return None
    return _include_combinations(combinations, axes, matrix.get("include", []))


def _flat_axes(matrix: dict[str, object]) -> dict[str, list[object]] | None:
    """Return the matrix axes, or ``None`` for a shape this helper cannot read.

    Returns
    -------
    dict[str, list[object]] | None
        Every axis and its scalar values, or ``None`` when an axis holds a
        structured or expression value.
    """
    axes: dict[str, list[object]] = {}
    for axis, axis_values in matrix.items():
        if axis in {"include", "exclude"}:
            continue
        if not isinstance(axis_values, list) or any(
            isinstance(value, (dict, list)) for value in axis_values
        ):
            return None
        axes[axis] = axis_values
    return axes


def _axis_combinations(axes: dict[str, list[object]]) -> list[dict[str, object]]:
    """Return the cartesian product of the axes.

    Returns
    -------
    list[dict[str, object]]
        One mapping per axis combination. An include-only matrix has no axes,
        so it starts empty and each `include` row becomes a leg.
    """
    if not axes:
        return []
    return [
        dict(zip(axes.keys(), values, strict=True))
        for values in itertools.product(*axes.values())
    ]


def _exclude_combinations(
    combinations: list[dict[str, object]], excluded: object
) -> list[dict[str, object]] | None:
    """Remove every combination an `exclude` row matches completely.

    Returns
    -------
    list[dict[str, object]] | None
        The surviving combinations, or ``None`` when `exclude` is not a list
        of mappings.
    """
    if not isinstance(excluded, list):
        return None
    for row in excluded:
        if not isinstance(row, dict):
            return None
        combinations = [
            combination
            for combination in combinations
            if not _matches(combination, row)
        ]
    return combinations


def _include_combinations(
    combinations: list[dict[str, object]],
    axes: dict[str, list[object]],
    included: object,
) -> list[dict[str, object]] | None:
    """Merge each `include` row into the combinations it does not overwrite.

    Returns
    -------
    list[dict[str, object]] | None
        The final combinations, or ``None`` when `include` is not a list of
        mappings.
    """
    if not isinstance(included, list):
        return None
    for row in included:
        if not isinstance(row, dict):
            return None
        if not axes:
            # An include-only matrix has no base combinations to merge into,
            # so every row is a leg of its own; merging them onto one empty
            # mapping would collapse them into a single overwritten leg.
            combinations = [*combinations, dict(row)]
            continue
        merged_into = [
            combination
            for combination in combinations
            if _matches(combination, row, keys=axes)
        ]
        if not merged_into:
            combinations = [*combinations, dict(row)]
            continue
        for combination in merged_into:
            combination.update(row)
    return combinations


def _matches(
    combination: dict[str, object],
    row: dict[str, object],
    *,
    keys: cabc.Mapping[str, object] | None = None,
) -> bool:
    """Return whether `row` matches `combination` on `keys` (default: all).

    An `exclude` row names only the axis values it removes, so it matches on
    every key it carries. An `include` row may only merge into a combination
    whose original axis values it leaves alone, so it matches on the axis keys.

    Returns
    -------
    bool
        Whether every key under comparison has the same value in both.
    """
    checked = row if keys is None else {key: row[key] for key in row if key in keys}
    return all(combination.get(key) == value for key, value in checked.items())


def _matrix_values(job: dict[str, object], property_name: str) -> list[object] | None:
    """Return the property's value on every executing matrix combination.

    Returns
    -------
    list[object] | None
        The surviving values, or ``None`` when the matrix shape cannot be
        evaluated or no leg sets the property.
    """
    combinations = _matrix_combinations(job)
    if combinations is None:
        return None
    values = [
        combination[property_name]
        for combination in combinations
        if property_name in combination
    ]
    return values or None


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
