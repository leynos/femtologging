"""Readers for the protected environment and the token it admits.

The CodeScene token is a deployment secret, so two questions decide whether a
pull request can reach it: which environment a job declares, and whether any
``env`` block names the token at all. GitHub matches environment names
case-insensitively and accepts an expression in place of a literal, so a
reader that returned the declaration verbatim could report no protected
environment for a job that declares one. Both readers therefore fail closed:
an expression they cannot resolve is refused rather than reported as safe.
"""

from __future__ import annotations

import typing as typ

from tests.workflow_document_helpers import require

if typ.TYPE_CHECKING:
    import collections.abc as cabc

CODESCENE_ENVIRONMENT: typ.Final[str] = "codescene"
#: The secret's name, never its value. It is matched inside ``env`` blocks.
TOKEN_NAME: typ.Final[str] = "CS_ACCESS_TOKEN"  # ruff: ignore[hardcoded-password-string] the secret's name, not its value


def _literal_environment(name: str) -> str:
    """Return `name` normalised for comparison, refusing an unresolvable one.

    GitHub matches environment names case-insensitively, so the comparison
    that decides whether a job reaches the protected environment must fold
    case or it reports no environment for a job that declares one. A name
    written as an expression cannot be proved equal to anything from here, so
    it is refused rather than returned verbatim: the caller's decision would
    otherwise be a guess presented as a proof.

    Parameters
    ----------
    name
        The declared environment name.

    Returns
    -------
    str
        The name folded to lower case.
    """
    require("${{" not in name, f"an environment expression is not verifiable: {name}")
    return name.lower()


def environment_name(job: dict[str, typ.Any]) -> str | None:
    """Return the environment a job deploys to, if it declares one.

    Parameters
    ----------
    job
        A workflow job document.

    Returns
    -------
    str | None
        The environment name, from either the scalar form or the mapping form
        that also carries a deployment URL, folded to lower case so the
        comparison with a protected environment is case-insensitive as GitHub's
        own matching is. ``None`` when no environment is declared. A
        declaration that is neither a name nor a mapping with one, and a name
        that is an expression this reader cannot resolve, are refused rather
        than guessed at.
    """
    raw = job.get("environment")
    if raw is None:
        return None
    if isinstance(raw, str):
        return _literal_environment(raw)
    require(isinstance(raw, dict), "an environment must be a name or a mapping")
    name = raw.get("name")
    require(isinstance(name, str), "an environment mapping must declare a name")
    return _literal_environment(name)


def _strings(node: object) -> cabc.Iterator[str]:
    """Yield every string nested in `node`, mapping keys included."""
    match node:
        case str():
            yield node
        case dict():
            for key, value in typ.cast("dict[object, object]", node).items():
                yield str(key)
                yield from _strings(value)
        case list():
            for item in typ.cast("list[object]", node):
                yield from _strings(item)


def env_blocks(
    node: object, path: str = ""
) -> cabc.Iterator[tuple[str, dict[str, typ.Any]]]:
    """Yield every ``env`` mapping nested in `node` with its location.

    Parameters
    ----------
    node
        A parsed workflow document or a part of one.
    path
        Location prefix used to report where the mapping was found.

    Yields
    ------
    tuple[str, dict[str, typing.Any]]
        The location and the ``env`` mapping declared there. Workflow-, job-
        and step-level ``env`` blocks are all found.
    """
    match node:
        case dict():
            for key, value in typ.cast("dict[object, object]", node).items():
                if key == "env" and isinstance(value, dict):
                    yield f"{path}/env", typ.cast("dict[str, typ.Any]", value)
                yield from env_blocks(value, f"{path}/{key}")
        case list():
            for at, item in enumerate(typ.cast("list[object]", node)):
                yield from env_blocks(item, f"{path}/{at}")


def _names_token(value: str) -> bool:
    """Report whether `value` names the CodeScene secret, folded for case.

    GitHub resolves secret references without case sensitivity, so
    ``secrets.cs_access_token`` reads the same secret as the uppercase
    spelling. A case-sensitive scan would pass that ``env`` entry as safe and
    miss the prohibited placement it is looking for.

    Parameters
    ----------
    value
        One string nested in an ``env`` mapping.

    Returns
    -------
    bool
        Whether the string names the secret in any case.

    Examples
    --------
    >>> _names_token("${{ secrets.cs_access_token }}")
    True
    >>> _names_token("${{ secrets.GITHUB_TOKEN }}")
    False
    """
    return TOKEN_NAME.casefold() in value.casefold()


def secret_in_env(document: dict[str, typ.Any]) -> list[str]:
    """Return the locations whose ``env`` names the CodeScene token.

    Parameters
    ----------
    document
        A parsed workflow document.

    Returns
    -------
    list[str]
        One location per offending ``env`` mapping. The upload action receives
        the token as an input, never through the environment: it is composite
        and would hand a step's environment to its nested steps.
    """
    return [
        location
        for location, mapping in env_blocks(document)
        if any(_names_token(value) for value in _strings(mapping))
    ]
