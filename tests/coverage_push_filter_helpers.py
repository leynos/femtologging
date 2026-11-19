"""Read GitHub's ``push`` branch filters the way GitHub evaluates them.

A workflow's ``push`` trigger may carry ``branches`` or ``branches-ignore``,
each a list of glob patterns with an optional leading ``!`` exclusion. Deciding
whether a push to ``main`` is accepted therefore needs the filter's real
semantics, not a literal string comparison: an absent filter accepts every
branch, naming only a tag filter accepts none, and ``*`` stays within one path
segment while ``**`` spans them. The reader fails closed on a shape it cannot
translate — a character class, a malformed repetition, both halves of a filter
pair — rather than answer a question it cannot prove.

The trigger mapping these readers consume is built by
:func:`tests.coverage_contract_helpers.triggers`.
"""

from __future__ import annotations

import re
import typing as typ

from tests.workflow_document_helpers import require

if typ.TYPE_CHECKING:
    import collections.abc as cabc


def _filter_pattern(pattern: str) -> re.Pattern[str]:
    """Return the regular expression GitHub's branch filter reads for `pattern`.

    GitHub reads a branch filter as a glob rather than a literal name: `*`
    matches within one path segment, `**` spans segments, and `?` and `+`
    repeat the character before them (zero or one, and one or more). A
    character class is not translated here, so it is refused rather than read
    as a literal, which would silently answer the wrong question.

    Parameters
    ----------
    pattern
        One branch-filter entry, without its leading `!` exclusion marker.

    Returns
    -------
    re.Pattern[str]
        The pattern a branch name is matched against in full.

    Raises
    ------
    AssertionError
        If the pattern is empty, repeats something that is not a literal
        character, or uses a character class this reader cannot translate.
    """
    require(bool(pattern), "a branch filter must not name an empty pattern")
    parts: list[str] = []
    previous_is_literal = False
    index = 0
    while index < len(pattern):
        if pattern.startswith("**", index):
            parts.append(".*")
            previous_is_literal = False
            index += 2
        elif pattern[index] == "*":
            parts.append("[^/]*")
            previous_is_literal = False
            index += 1
        elif pattern[index] in "?+":
            require(
                previous_is_literal,
                "a branch filter `?` or `+` must repeat a literal character: "
                f"{pattern}",
            )
            parts.append(pattern[index])
            previous_is_literal = False
            index += 1
        elif pattern[index] == "[":
            message = f"a branch filter character class is not supported: {pattern}"
            raise AssertionError(message)
        else:
            parts.append(re.escape(pattern[index]))
            previous_is_literal = True
            index += 1
    return re.compile("".join(parts))


def _matches_filter(patterns: list[str], branch: str) -> bool:
    """Report whether `branch` matches `patterns`, honouring exclusions.

    A leading `!` excludes the pattern, and the last pattern that matches
    decides, as GitHub's own filter order does. A list that only excludes
    therefore accepts every branch it does not exclude.

    Parameters
    ----------
    patterns
        The branch-filter entries, in declaration order.
    branch
        The branch name a push names.

    Returns
    -------
    bool
        Whether the filter accepts `branch`.
    """
    matched = False
    for entry in patterns:
        negated = entry.startswith("!")
        pattern = entry[1:] if negated else entry
        if _filter_pattern(pattern).fullmatch(branch):
            matched = not negated
    return matched


def _branch_filter(settings: cabc.Mapping[str, typ.Any], key: str) -> list[str] | None:
    """Return one branch filter of a push trigger, or ``None`` if absent.

    Parameters
    ----------
    settings
        A push trigger's settings mapping.
    key
        Either ``branches`` or ``branches-ignore``.

    Returns
    -------
    list[str] | None
        The declared patterns in order, or ``None`` when the key is absent. A
        filter that is not a sequence of non-empty strings is refused rather
        than read.
    """
    raw = settings.get(key)
    if raw is None:
        return None
    require(isinstance(raw, list), f"push {key} must be a sequence")
    patterns = [str(name) for name in typ.cast("list[object]", raw)]
    require(patterns, f"push {key} must name at least one pattern")
    require(all(patterns), f"push {key} must not name an empty pattern")
    return patterns


def _declared_filters(
    declared: cabc.Mapping[str, typ.Any],
) -> tuple[list[str] | None, list[str] | None]:
    """Return a push trigger's branch filters, refusing a contradictory pair.

    Parameters
    ----------
    declared
        A push trigger's settings mapping.

    Returns
    -------
    tuple[list[str] | None, list[str] | None]
        The ``branches`` and ``branches-ignore`` patterns, each ``None`` when
        that half is absent.

    Raises
    ------
    AssertionError
        If both halves of one filter pair are declared, which GitHub imposes no
        order on, so neither reading could be called the source of truth.
    """
    included = _branch_filter(declared, "branches")
    excluded = _branch_filter(declared, "branches-ignore")
    # Raised explicitly, rather than through `require`, so the documented
    # `AssertionError` stays visible to the docstring linter.
    if included is not None and excluded is not None:
        message = "a push trigger cannot name both branches and branches-ignore"
        raise AssertionError(message)
    if "tags" in declared and "tags-ignore" in declared:
        message = "a push trigger cannot name both tags and tags-ignore"
        raise AssertionError(message)
    return included, excluded


def _accepts_branch(
    declared: cabc.Mapping[str, typ.Any],
    branch: str,
) -> bool:
    """Decide one push trigger's acceptance of `branch` from its filters.

    An absent branch filter accepts, a ``branches`` filter accepts the branches
    its patterns match, and a ``branches-ignore`` filter accepts every branch
    it does not exclude. Naming only a tag filter accepts no branch push at
    all: the workflow does not run for one, which is a definite "no" rather
    than an unreadable shape.

    Parameters
    ----------
    declared
        A push trigger's settings mapping.
    branch
        The branch name a push must name.

    Returns
    -------
    bool
        Whether the trigger accepts `branch`.
    """
    included, excluded = _declared_filters(declared)
    if included is not None:
        return _matches_filter(included, branch)
    if excluded is not None:
        return not _matches_filter(excluded, branch)
    return "tags" not in declared and "tags-ignore" not in declared


def pushes_to(events: cabc.Mapping[str, typ.Any], branch: str) -> bool:
    """Report whether `events` runs on a push to `branch`.

    A push trigger with no branch or tag filter accepts every branch. A
    `branches` filter accepts the branches its patterns match, honouring `!`
    exclusions. A `branches-ignore` filter accepts every branch it does not
    exclude. A trigger filtered only on tags accepts no branch push at all, and
    declaring both halves of one filter pair, or a filter form this reader
    cannot translate, is refused rather than guessed at.

    Parameters
    ----------
    events
        A trigger mapping, such as ``triggers(document)``.
    branch
        The branch name a push must name, such as ``main``.

    Returns
    -------
    bool
        Whether a push trigger accepts `branch`. A filter this reader cannot
        read, or a contradictory filter pair, is refused rather than answered.
    """
    if "push" not in events:
        return False
    settings = events["push"]
    if settings is None:
        return True
    require(isinstance(settings, dict), "a push trigger must be a mapping")
    return _accepts_branch(typ.cast("cabc.Mapping[str, typ.Any]", settings), branch)
