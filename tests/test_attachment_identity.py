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


def test_identities_are_stable_and_address_free(reset_logger_state: None) -> None:
    """Repeated reads return equal tokens whose repr reveals no address."""
    logger = get_logger("identity.stable")
    logger.add_handler(_RecordingHandler())

    first = logger.handler_identities_for_test()
    second = logger.handler_identities_for_test()

    assert len(first) == 1, "one attached handler should yield one identity"
    assert first == second, "repeated reads must agree on the same attachment"
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
    assert handlers != filters, "handler tokens must not equal filter tokens"


def test_identities_defer_foreign_comparisons(reset_logger_state: None) -> None:
    """Foreign operands and ordering fall back to Python's comparison rules."""
    logger = get_logger("identity.foreign")
    logger.add_handler(_RecordingHandler())
    token = logger.handler_identities_for_test()[0]

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
