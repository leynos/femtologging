"""Readers that classify a workflow job's runner labels.

A job's runner may be a scalar, a sequence, a mapping or a matrix expression,
and the provisioning contract has to know whether the concrete runner is
Linux. Keeping this classification beside the document reader would let one
module grow past its size limit, and the two concerns are independent: the
document reader never asks about the platform.
"""

from __future__ import annotations

import typing as typ

if typ.TYPE_CHECKING:
    from tests.workflow_document_helpers import Job


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


def _matrix_runner_values(matrix: dict[str, typ.Any]) -> list[str]:
    """Return the runner labels a job matrix names.

    Parameters
    ----------
    matrix
        The job's ``strategy.matrix`` mapping.

    Returns
    -------
    list[str]
        Labels from ``include`` entries and from a direct ``os`` key.
    """
    included = matrix.get("include")
    entries = included if isinstance(included, list) else []
    from_include = [
        str(entry["os"])
        for entry in entries
        if isinstance(entry, dict) and "os" in entry
    ]
    return from_include + _flatten(matrix.get("os"))


def runner_values(job: Job) -> list[str]:
    """Return the runner labels and matrix operating systems of `job`."""
    values = _flatten(job.get("runs-on"))
    matrix = job.get("strategy", {}).get("matrix", {})
    if isinstance(matrix, dict):
        values.extend(_matrix_runner_values(matrix))
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
