"""Readers for the protected environment and the token it admits.

The CodeScene token is a deployment secret, so two questions decide whether a
pull request can reach it: which environment a job declares, and whether any
``env`` block names the token at all. GitHub matches environment names
case-insensitively and accepts an expression in place of a literal, so a
reader that returned the declaration verbatim could report no protected
environment for a job that declares one. Both readers therefore fail closed:
an expression they cannot resolve is refused rather than reported as safe.
The ``env`` scan fails closed the same way. A declaration that is not a
mapping is refused, because an expression there can expand to a mapping the
reader never sees. A reference that reaches the whole ``secrets`` context,
through ``toJSON(secrets)`` or an index this reader cannot evaluate, is
reported as carrying the token even though it never spells the name.
"""

from __future__ import annotations

import re
import typing as typ

from tests.workflow_document_helpers import require

if typ.TYPE_CHECKING:
    import collections.abc as cabc

CODESCENE_ENVIRONMENT: typ.Final[str] = "codescene"
#: The secret's name, never its value. It is matched inside ``env`` blocks.
TOKEN_NAME: typ.Final[str] = "CS_ACCESS_TOKEN"  # ruff: ignore[hardcoded-password-string] the secret's name, not its value
#: The characters that open one ``${{ ... }}`` expression wherever it sits in a
#: value. Secret access is only meaningful inside one, so text that merely
#: mentions the word cannot be mistaken for an expression.
_EXPRESSION_OPENING: typ.Final[str] = "${{"
#: The characters that close an expression, once no quoted string is open.
_EXPRESSION_CLOSING: typ.Final[str] = "}}"
#: A reference to the `secrets` context, with the name it selects if it names
#: one. The name group is absent for an aggregate access such as
#: ``toJSON(secrets)`` or an index such as ``secrets[env.NAME]``, either of
#: which can reach the token without spelling it.
_SECRETS_ACCESS: typ.Final[re.Pattern[str]] = re.compile(
    r"\bsecrets\b(?:\.([a-z_][a-z0-9_]*))?"
)


def _quoted_end(value: str, start: int) -> int | None:
    """Return the index just past the single-quoted literal at `start`.

    The Actions expression grammar spells a literal in single quotes and an
    embedded apostrophe as two adjacent quotes. Scanning for the closing quote
    therefore has to consume ``''`` as one escaped character; treating the
    first quote of the pair as the end would put the rest of the literal, and
    any real access after it, outside the expression.

    Parameters
    ----------
    value
        Text that may contain the literal.
    start
        Index of the opening quote.

    Returns
    -------
    int | None
        The index just past the closing quote, or ``None`` when the literal is
        never terminated.
    """
    index = start + 1
    while index < len(value):
        if value[index] != "'":
            index += 1
            continue
        if value.startswith("''", index):
            index += 2
            continue
        return index + 1
    return None


def _expression_end(value: str, start: int) -> int | None:
    """Return the index of the ``}}`` that closes the body starting at `start`.

    A closing delimiter inside a quoted literal does not end the expression.
    The Actions runner skips quoted strings when it looks for the end, so a
    reader that stopped at the first ``}}`` would truncate a body such as
    ``format('}}{0}', toJSON(secrets))`` and miss the access inside it.

    Returns
    -------
    int | None
        The index where the closing delimiter starts, or ``None`` when the
        expression, or a literal within it, is never terminated.
    """
    index = start
    while index < len(value):
        if value[index] == "'":
            literal_end = _quoted_end(value, index)
            if literal_end is None:
                return None
            index = literal_end
            continue
        if value.startswith(_EXPRESSION_CLOSING, index):
            return index
        index += 1
    return None


def _expressions(value: str) -> cabc.Iterator[str]:
    """Yield the body of each ``${{ ... }}`` expression in `value`.

    An unterminated expression yields the rest of the value as its body. The
    runner would reject such a workflow, and this reader fails closed: scanning
    the remainder can still report a secret access, while dropping the
    expression would report the placement safe.

    Parameters
    ----------
    value
        One string nested in an ``env`` mapping.

    Yields
    ------
    str
        The body of one expression, without its delimiters.

    Examples
    --------
    >>> list(_expressions("${{ secrets.A }} and ${{ secrets.B }}"))
    [' secrets.A ', ' secrets.B ']
    >>> list(_expressions("${{ format('}}{0}', toJSON(secrets)) }}"))
    [" format('}}{0}', toJSON(secrets)) "]
    >>> list(_expressions("${{ toJSON(secrets)"))
    [' toJSON(secrets)']
    """
    cursor = 0
    while (start := value.find(_EXPRESSION_OPENING, cursor)) != -1:
        body_start = start + len(_EXPRESSION_OPENING)
        end = _expression_end(value, body_start)
        if end is None:
            yield value[body_start:]
            return
        yield value[body_start:end]
        cursor = end + len(_EXPRESSION_CLOSING)


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

    An ``env`` declaration that is not a mapping is refused, not skipped: the
    runner expands an expression there into a mapping before the job starts, so
    this reader cannot see what the job would receive, and passing over it
    would report a workflow safe that may hand the token to a step.

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
                if key == "env":
                    require(
                        isinstance(value, dict),
                        f"an env expression is not verifiable: {path}/env",
                    )
                    yield f"{path}/env", typ.cast("dict[str, typ.Any]", value)
                yield from env_blocks(value, f"{path}/{key}")
        case list():
            for at, item in enumerate(typ.cast("list[object]", node)):
                yield from env_blocks(item, f"{path}/{at}")


def _reaches_whole_secrets(value: str) -> bool:
    """Report whether an expression in `value` reads more than one secret.

    ``toJSON(secrets)`` serialises every available secret, and
    ``secrets[env.NAME]`` selects one by a name this reader cannot evaluate;
    either can hand out the CodeScene token without ever spelling it. A
    direct ``secrets.OTHER_NAME`` reference still names exactly one secret, so
    it stays acceptable. Only expression bodies are scanned: the word
    "secrets" in ordinary text is not a reference to anything, and a quoted
    literal does not end the expression that contains it.

    Parameters
    ----------
    value
        One string nested in an ``env`` mapping.

    Returns
    -------
    bool
        Whether an expression reaches the whole ``secrets`` context or
        indexes it with a name the reader cannot resolve.

    Examples
    --------
    >>> _reaches_whole_secrets("${{ toJSON(secrets) }}")
    True
    >>> _reaches_whole_secrets("${{ format('}}{0}', toJSON(secrets)) }}")
    True
    >>> _reaches_whole_secrets("${{ secrets[env.NAME] }}")
    True
    >>> _reaches_whole_secrets("${{ secrets.GITHUB_TOKEN }}")
    False
    >>> _reaches_whole_secrets("the secrets are safe here")
    False
    """
    return any(
        access.group(1) is None
        for expression in _expressions(value)
        for access in _SECRETS_ACCESS.finditer(expression.casefold())
    )


def _names_token(value: str) -> bool:
    """Report whether `value` names the CodeScene secret, folded for case.

    GitHub resolves secret references without case sensitivity, so
    ``secrets.cs_access_token`` reads the same secret as the uppercase
    spelling. A case-sensitive scan would pass that ``env`` entry as safe and
    miss the prohibited placement it is looking for. Naming the token is not
    the only way to reach it, so a value that reads the whole secret context
    is reported as carrying the token too.

    Parameters
    ----------
    value
        One string nested in an ``env`` mapping.

    Returns
    -------
    bool
        Whether the string names the secret in any case, or reaches the whole
        ``secrets`` context.

    Examples
    --------
    >>> _names_token("${{ secrets.cs_access_token }}")
    True
    >>> _names_token("${{ secrets.GITHUB_TOKEN }}")
    False
    >>> _names_token("${{ toJSON(secrets) }}")
    True
    """
    return TOKEN_NAME.casefold() in value.casefold() or _reaches_whole_secrets(value)


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
