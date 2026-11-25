"""Rules for a heavy-job step's compiler environment.

Several heavy-job steps select Loom with an inline ``RUSTFLAGS`` assignment.
Cargo gives ``CARGO_ENCODED_RUSTFLAGS`` precedence over it, and reads the
encoded variable from the step, the job or the workflow. A value at any of
those scopes therefore makes the step compile without `--cfg loom` while the
command text still names it, so each gate refuses every such declaration.

The rule lives here rather than beside either caller because the compile gate
and the formatter gate share it, and both modules are held to the repository's
file-length limit.
"""

from __future__ import annotations

import typing as typ

if typ.TYPE_CHECKING:
    import collections.abc as cabc

from tests.heavy_workflow_document import ENCODED_FLAGS

__all__ = ["declares_environment", "encoded_flags_problems"]


def declares_environment(container: cabc.Mapping[str, object], name: str) -> bool:
    """Return whether a container's environment declares `name`.

    The key's presence is what matters: a null declaration still exports an
    empty value, which would displace the inline flags just as a value does.

    Returns
    -------
    bool
        True when the container's `env` mapping defines `name`.
    """
    environment = container.get("env")
    return isinstance(environment, dict) and name in environment


def encoded_flags_problems(
    subject: str,
    step: dict[str, object],
    found: cabc.Mapping[str, object],
    document: cabc.Mapping[str, object],
) -> list[str]:
    """Return a complaint per scope that could displace a step's inline flags.

    Parameters
    ----------
    subject : str
        The step's name, used in the complaints.
    step : dict[str, object]
        The step whose environment is the narrowest scope checked.
    found : cabc.Mapping[str, object]
        The step's job, the next scope out.
    document : cabc.Mapping[str, object]
        The workflow, the widest scope.

    Returns
    -------
    list[str]
        One complaint per scope whose environment declares
        ``CARGO_ENCODED_RUSTFLAGS``.
    """
    scopes = (
        ("step", step),
        ("job", found),
        ("workflow", document),
    )
    return [
        f"the {subject!r} step's {scope} must not set {ENCODED_FLAGS}"
        for scope, container in scopes
        if declares_environment(container, ENCODED_FLAGS)
    ]
