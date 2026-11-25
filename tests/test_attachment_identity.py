"""Tests for the opaque attachment identities exposed to test builds.

The test-only hooks compare logger attachments without reporting native
addresses: they return tokens that compare by allocation identity. These tests
pin that contract. Repeated reads agree, distinct attachments differ, foreign
values fall back to Python's own comparison rules, the representation never
reveals an address, and the address-returning hooks are gone.
"""

from __future__ import annotations

import re

import pytest

import femtologging
from femtologging import get_logger

pytestmark = pytest.mark.skipif(
    not femtologging._has_test_util,
    reason="attachment identity hooks require the test-util feature",
)

_ADDRESS_PATTERN = re.compile(r"0x[0-9a-fA-F]+")


class _RecordingHandler:
    """Minimal Python handler that accepts records without storing them."""

    def handle(self, logger: object, level: str, message: str) -> None:
        """Accept one record, ignoring its values."""

    def flush(self) -> bool:
        """Report a successful flush."""
        return True


class _AlwaysEqual:
    """Foreign operand whose reflected equality accepts anything.

    A token can only reach this reflected check when its own ``__eq__``
    returned ``NotImplemented``; returning ``False`` would resolve the
    comparison before Python tried this operand.
    """

    def __eq__(self, other: object) -> bool:
        """Accept *other* unconditionally."""
        return True


def test_identities_are_stable_and_address_free(reset_logger_state: None) -> None:
    """Repeated reads return equal tokens whose repr reveals no address."""
    logger = get_logger("identity.stable")
    logger.add_handler(_RecordingHandler())

    first = logger.handler_identities_for_test()
    second = logger.handler_identities_for_test()

    assert len(first) == 1, "one attached handler should yield one identity"
    assert first == second, "repeated reads must agree on the same attachment"
    assert first[0] == second[0], "one attachment must compare equal to itself"
    assert (first[0] != second[0]) is False, (
        "an equal token pair must not compare unequal"
    )
    assert repr(first[0]) == "<femtologging.AttachmentIdentity>", (
        "the repr should name the token type"
    )
    assert _ADDRESS_PATTERN.search(repr(first[0])) is None, (
        "the repr must not leak a native address"
    )


def test_appending_a_handler_adds_a_distinct_identity(
    reset_logger_state: None,
) -> None:
    """An appended handler keeps earlier tokens and gains a new one."""
    logger = get_logger("identity.append")
    logger.add_handler(_RecordingHandler())
    before = logger.handler_identities_for_test()

    logger.add_handler(_RecordingHandler())
    after = logger.handler_identities_for_test()

    assert after[: len(before)] == before, (
        "appending must leave earlier tokens unchanged"
    )
    assert len(after) == len(before) + 1, "one appended handler adds one token"
    assert after[-1] not in before, "the new token must differ from earlier ones"
    assert after[0] != after[1], "two distinct handlers must compare unequal"
    assert (after[0] != after[1]) is True, (
        "unequal token pairs must report unequal through !="
    )


def test_a_reattached_handler_never_inherits_a_dropped_identity(
    reset_logger_state: None,
) -> None:
    """A dropped attachment keeps its identity reserved for its token."""
    logger = get_logger("identity.lifetime")
    first = _RecordingHandler()
    logger.add_handler(first)
    dropped = logger.handler_identities_for_test()[0]
    twin = logger.handler_identities_for_test()[0]

    assert logger.remove_handler(first) is True, "the attached handler must be found"
    del first

    logger.add_handler(_RecordingHandler())
    reattached = logger.handler_identities_for_test()[0]

    assert dropped == twin, (
        "two tokens for one attachment must stay equal after the attachment drops"
    )
    assert reattached != dropped, (
        "a later attachment must not inherit a dropped attachment's identity"
    )
    assert dropped != reattached, (
        "a dropped attachment's token must not match a later attachment"
    )


def test_removing_an_unattached_handler_returns_false(
    reset_logger_state: None,
) -> None:
    """``remove_handler`` reports whether anything was actually detached.

    The return value is the only signal a caller has that the handler it named
    was attached. Returning ``True`` for a handler the logger never held would
    make a successful detach and a misspelt one indistinguishable, and a caller
    that logs the outcome would report a removal that never happened.
    """
    logger = get_logger("identity.remove")
    attached = _RecordingHandler()
    stranger = _RecordingHandler()
    logger.add_handler(attached)

    assert logger.remove_handler(stranger) is False, (
        "a handler the logger never held must not report a removal"
    )
    assert len(logger.handler_identities_for_test()) == 1, (
        "a missed removal must leave the attachment in place"
    )

    assert logger.remove_handler(attached) is True, (
        "the attached handler must still be removable afterwards"
    )
    assert logger.handler_identities_for_test() == [], (
        "the attached handler must actually be gone"
    )
    assert logger.remove_handler(attached) is False, (
        "a second removal of the same handler must report nothing was detached"
    )


def test_filter_identities_follow_the_configured_filters(core_config: None) -> None:
    """Filter tokens are ordered, stable, and distinct from handler tokens."""
    core = get_logger("core")

    filters = core.filter_identities_for_test()
    handlers = core.handler_identities_for_test()

    assert len(filters) == 1, "the core logger should carry one configured filter"
    assert filters == core.filter_identities_for_test(), (
        "repeated reads must agree on the configured filter"
    )
    assert get_logger("root").filter_identities_for_test() == [], (
        "the root logger should carry no filters"
    )
    assert handlers[0] != filters[0], (
        "a handler token must differ from a filter token on the same logger"
    )
    assert filters[0] != handlers[0], (
        "the filter token must differ from the handler token too"
    )


def test_identities_defer_foreign_comparisons(reset_logger_state: None) -> None:
    """Foreign operands and ordering fall back to Python's comparison rules."""
    logger = get_logger("identity.foreign")
    logger.add_handler(_RecordingHandler())
    token = logger.handler_identities_for_test()[0]

    assert token == _AlwaysEqual(), (
        "foreign equality must defer to the other operand's reflected check"
    )
    assert token != object(), "a token must not equal an unrelated object"
    assert (token == object()) is False, "foreign equality must resolve to False"
    assert token != [], "a token must not equal an empty list"
    with pytest.raises(TypeError):
        _ = token < object()
    with pytest.raises(TypeError):
        int(token)


def test_native_address_hooks_are_not_exposed(reset_logger_state: None) -> None:
    """The extension no longer offers address-returning attachment hooks."""
    logger = get_logger("identity.addresses")

    assert not hasattr(logger, "handler_ptrs_for_test"), (
        "handler addresses must not reach Python"
    )
    assert not hasattr(logger, "filter_ptrs_for_test"), (
        "filter addresses must not reach Python"
    )
    assert not hasattr(femtologging.rust, "AttachmentIdentity"), (
        "the token class must stay unregistered in the public module"
    )
