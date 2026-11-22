"""State-machine properties for the opaque attachment identity tokens.

The example tests in :mod:`tests.test_attachment_identity` pin individual
comparisons. The token contract also has to hold across generated sequences of
attach, detach and clear operations: earlier tokens stay equal to themselves,
tokens for distinct attachments stay unequal, a dropped attachment's identity
is never inherited by a later attachment while its token is alive, and handler
tokens never equal filter tokens.

These properties depend on the order of the operations, so the module runs a
Hypothesis state machine rather than a parameterized table.
"""

from __future__ import annotations

import typing as typ

import pytest

import femtologging
from femtologging import (
    ConfigBuilder,
    LevelFilterBuilder,
    LoggerConfigBuilder,
    StreamHandlerBuilder,
    get_logger,
)

hypothesis = pytest.importorskip("hypothesis")
stateful = pytest.importorskip("hypothesis.stateful")

pytestmark = pytest.mark.skipif(
    not femtologging._has_test_util,
    reason="attachment identity hooks require the test-util feature",
)


class _IdentityLogger(typ.Protocol):
    """The logger surface the attachment-identity rules drive."""

    def add_handler(self, handler: object) -> None:
        """Attach one handler."""

    def remove_handler(self, handler: object) -> bool:
        """Detach one handler, reporting whether it was attached."""

    def clear_handlers(self) -> None:
        """Detach every handler."""

    def handler_identities_for_test(self) -> list[object]:
        """Return one token per attached handler, in attachment order."""

    def filter_identities_for_test(self) -> list[object]:
        """Return one token per configured filter, in configuration order."""


class _RecordingHandler:
    """Minimal Python handler that accepts records without storing them."""

    def handle(self, logger: object, level: str, message: str) -> None:
        """Accept one record, ignoring its values."""

    def flush(self) -> bool:
        """Report a successful flush."""
        return True


class AttachmentIdentityMachine(stateful.RuleBasedStateMachine):
    """Generate attach, detach and clear sequences and check every token.

    The machine keeps every token it has seen reachable, so the weak reference
    inside a dropped attachment's token keeps that allocation reserved. That is
    what makes the no-inheritance assertion meaningful: without the live token,
    the allocator could legitimately reuse the address.
    """

    def __init__(self) -> None:
        """Start with no logger and no recorded tokens."""
        super().__init__()
        self._logger: _IdentityLogger | None = None
        self._filter_token: object | None = None
        self._live: list[tuple[_RecordingHandler, object, object]] = []
        self._dropped: list[tuple[object, object]] = []

    @property
    def _active_logger(self) -> _IdentityLogger:
        """The configured logger every rule operates on."""
        assert self._logger is not None, "init_logger must run before every rule"
        return self._logger

    @property
    def _active_filter_token(self) -> object:
        """The filter token recorded in :meth:`init_logger`."""
        assert self._filter_token is not None, "init_logger must record a filter"
        return self._filter_token

    @stateful.initialize()
    def init_logger(self) -> None:
        """Configure a logger with one filter and record its filter token."""
        femtologging.reset_manager()
        (
            ConfigBuilder()
            .with_handler("stderr", StreamHandlerBuilder.stderr())
            .with_filter("lvl", LevelFilterBuilder().with_max_level("DEBUG"))
            .with_root_logger(LoggerConfigBuilder().with_level("DEBUG"))
            .with_logger(
                "identity.state",
                LoggerConfigBuilder().with_level("DEBUG").with_filters(["lvl"]),
            )
            .build_and_init()
        )
        self._logger = get_logger("identity.state")
        filters = self._logger.filter_identities_for_test()
        assert len(filters) == 1, "the configured logger carries one filter"
        self._filter_token = filters[0]

    @stateful.rule()
    def attach_handler(self) -> None:
        """Attach a fresh handler and record the token issued for it."""
        handler = _RecordingHandler()
        self._active_logger.add_handler(handler)
        tokens = self._active_logger.handler_identities_for_test()
        twin = self._active_logger.handler_identities_for_test()[-1]
        self._live.append((handler, tokens[-1], twin))
        self._assert_invariants()

    @stateful.rule()
    def detach_handler(self) -> None:
        """Detach the most recent handler, keeping its token reachable."""
        if not self._live:
            return
        handler, token, twin = self._live.pop()
        assert self._active_logger.remove_handler(handler) is True, (
            "an attached handler must be removable"
        )
        self._dropped.append((token, twin))
        self._assert_invariants()

    @stateful.rule()
    def clear_handlers(self) -> None:
        """Remove every handler at once, keeping all their tokens reachable."""
        self._dropped.extend((token, twin) for _, token, twin in self._live)
        self._active_logger.clear_handlers()
        self._live.clear()
        self._assert_invariants()

    def _assert_invariants(self) -> None:
        """Check the token contract against the recorded attachment state."""
        tokens = self._active_logger.handler_identities_for_test()
        assert tokens == self._live_tokens(), (
            "reads must follow the attachment order the machine recorded"
        )
        self._assert_live_tokens()
        self._assert_dropped_tokens()
        self._assert_filter_separation()

    def _assert_live_tokens(self) -> None:
        """Check equality and distinctness among the live attachments."""
        for _, token, twin in self._live:
            assert token == twin, "two tokens for one attachment must compare equal"
        for index, (_, left, _) in enumerate(self._live):
            for _, right, _ in self._live[index + 1 :]:
                assert left != right, (
                    "tokens for distinct live attachments must compare unequal"
                )

    def _assert_dropped_tokens(self) -> None:
        """Check that dropped identities stay reserved and never recur."""
        for dropped, twin in self._dropped:
            assert dropped == twin, (
                "a dropped attachment's tokens keep their reserved identity"
            )
            assert dropped != self._active_filter_token, (
                "a dropped handler token must not equal a filter token"
            )
            for live_token in self._live_tokens():
                assert dropped != live_token, (
                    "a later attachment must not inherit a dropped identity"
                )

    def _assert_filter_separation(self) -> None:
        """Check that handler tokens never compare equal to the filter token."""
        for live_token in self._live_tokens():
            assert live_token != self._active_filter_token, (
                "a handler token must not equal a filter token"
            )
            assert self._active_filter_token != live_token, (
                "the filter token must not equal a handler token"
            )

    def _live_tokens(self) -> list[object]:
        """Return the token recorded for each live attachment."""
        return [token for _, token, _ in self._live]

    def teardown(self) -> None:
        """Reset the global logger manager after each generated example."""
        femtologging.reset_manager()


def test_attachment_identity_invariants() -> None:
    """Every generated attach, detach and clear sequence keeps the contract."""
    stateful.run_state_machine_as_test(
        AttachmentIdentityMachine,
        settings=hypothesis.settings(
            max_examples=20,
            stateful_step_count=8,
            deadline=None,
        ),
    )
