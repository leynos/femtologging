"""Trust-boundary policy helpers for ``dictConfig`` filter factories."""

from __future__ import annotations


def factory_disallowed_message(fid: str) -> str:
    """Describe why a factory entry was refused in declarative-only mode.

    Factory entries are trusted-code-equivalent: a dotted target imports a module
    during resolution and then invokes the resolved callable, while a direct
    callable object is invoked without that step. The message names the
    offending filter and the policy that rejected it so a control plane can
    report why lower-trust configuration was refused.

    Parameters
    ----------
    fid
        Identifier of the filter entry that used a ``"()"`` key.

    Returns
    -------
    str
        Message naming the filter identifier and the policy.

    Examples
    --------
    >>> from femtologging._config_policy import factory_disallowed_message
    >>> factory_disallowed_message("svc")
    "filter 'svc' uses a '()' factory, rejected by allow_factories=False"

    """
    return f"filter {fid!r} uses a '()' factory, rejected by allow_factories=False"


def validate_allow_factories(value: object) -> bool:
    """Validate the ``allow_factories`` policy before configuration processing.

    Parameters
    ----------
    value
        Candidate policy from the public ``dictConfig`` argument.

    Returns
    -------
    bool
        The validated policy.

    Raises
    ------
    TypeError
        If ``value`` is not a bool, so ``0``, ``1``, and ``"false"`` cannot be
        mistaken for a policy choice.

    Examples
    --------
    >>> from femtologging._config_policy import validate_allow_factories
    >>> validate_allow_factories(False)
    False
    >>> validate_allow_factories("false")
    Traceback (most recent call last):
    ...
    TypeError: allow_factories must be a bool

    """
    if not isinstance(value, bool):
        msg = "allow_factories must be a bool"
        raise TypeError(msg)
    return value
