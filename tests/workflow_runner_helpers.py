"""Readers that classify a workflow job's runner labels.

A job's runner may be a scalar, a sequence, a mapping or a matrix expression,
and the provisioning contract has to know whether the concrete runner is
Linux. Keeping this classification beside the document reader would let one
module grow past its size limit, and the two concerns are independent: the
document reader never asks about the platform.
"""

from __future__ import annotations

import re
import typing as typ

if typ.TYPE_CHECKING:
    from tests.workflow_document_helpers import Job

#: A ``matrix.<key>`` reference in a ``runs-on`` expression names the matrix
#: value that supplies the concrete runner label.
_MATRIX_REFERENCE: typ.Final[re.Pattern[str]] = re.compile(
    r"matrix\.([A-Za-z_][A-Za-z0-9_]*)"
)


def _flatten(value: object) -> list[str]:
    """Return every scalar in a YAML value as text.

    Parameters
    ----------
    value
        A scalar, sequence or mapping from a workflow document.

    Returns
    -------
    list[str]
        The scalar values in document order.
    """
    match value:
        case str():
            return [value]
        case list() | tuple():
            return [text for item in value for text in _flatten(item)]
        case dict():
            return [text for item in value.values() for text in _flatten(item)]
        case _:
            return []


def _matrix_runner_keys(runs_on: list[str]) -> list[str]:
    """Return the matrix keys a job's ``runs-on`` values name, in order.

    Parameters
    ----------
    runs_on
        The scalar runner values flattened from a job's ``runs-on``.

    Returns
    -------
    list[str]
        The distinct ``matrix.<key>`` names, without repetition.
    """
    keys = [
        match.group(1)
        for value in runs_on
        for match in _MATRIX_REFERENCE.finditer(value)
    ]
    return list(dict.fromkeys(keys))


def _matrix_runner_values(matrix: dict[str, typ.Any], keys: list[str]) -> list[str]:
    """Return the runner labels a job matrix names for `keys`.

    Parameters
    ----------
    matrix
        The job's ``strategy.matrix`` mapping.
    keys
        The matrix keys the job's ``runs-on`` expressions reference; ``os``
        when it references none. Other fields, such as ``arch`` or ``target``,
        are not runner labels and are never read.

    Returns
    -------
    list[str]
        Labels from ``include`` entries and from the direct key mappings.
    """
    wanted = keys or ["os"]
    included = matrix.get("include")
    entries = included if isinstance(included, list) else []
    from_include = [
        str(entry[key])
        for entry in entries
        if isinstance(entry, dict)
        for key in wanted
        if key in entry
    ]
    return from_include + [text for key in wanted for text in _flatten(matrix.get(key))]


def runner_values(job: Job) -> list[str]:
    """Return the runner labels and matrix values of `job`."""
    values = _flatten(job.get("runs-on"))
    matrix = job.get("strategy", {}).get("matrix", {})
    if isinstance(matrix, dict):
        values.extend(_matrix_runner_values(matrix, _matrix_runner_keys(values)))
    return values


def is_linux_job(job: Job) -> bool:
    """Report whether `job` runs on Linux runners.

    A runner expression that no concrete label resolves (such as
    ``${{ matrix.os }}``) is left to the matrix values; an expression with no
    matrix detail is treated as Linux, because the repository's hosted suites
    run there. Non-Linux jobs are a legitimate exception: their platform
    linker replaces ``mold``.

    Returns
    -------
    bool
        Whether the job's concrete runners are Linux.
    """
    concrete = [value for value in runner_values(job) if "${{" not in value]
    if not concrete:
        return True
    joined = " ".join(concrete).lower()
    if any(word in joined for word in ("ubuntu", "linux", "ubicloud")):
        return True
    return not any(word in joined for word in ("windows", "macos"))
