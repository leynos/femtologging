"""State-machine properties for the file-handler shutdown lifecycle.

The subprocess scenarios in :mod:`tests.test_file_handler_gil_release` pin one
interleaving each: they park a worker, start a call, and prove whether the GIL
was released. Those cases cannot enumerate the *order* of ``handle()``,
``flush()``, and ``close()`` calls, so they say nothing about what happens when
a caller flushes twice, closes before handling anything, or closes, handles,
and closes again.

This module generates bounded sequences of those calls for every file-handler
builders across queue capacities and overflow policies. It asserts the
lifecycle invariants that must hold whatever the order:

- ``close()`` never raises, however many times it runs, so shutdown is
  idempotent;
- once a close has begun, ``handle()`` reports the closure rather than
  accepting the record, and it keeps reporting it for every later call;
- once a close has begun, ``flush()`` reports ``False``;
- a call made before the close either completes or raises the documented
  overflow error for its policy, never an unrelated failure.

The machine runs in-process: it exercises the real binding and its worker
thread, but it does not attempt to control scheduling. Interleaving evidence
stays with the subprocess scenarios, which is why both suites are kept.
"""

from __future__ import annotations

import functools
import typing as typ

import pytest

from femtologging import (
    FileHandlerBuilder,
    OverflowPolicy,
    RotatingFileHandlerBuilder,
    TimedRotatingFileHandlerBuilder,
)

hypothesis = pytest.importorskip("hypothesis")
stateful = pytest.importorskip("hypothesis.stateful")
strategies = pytest.importorskip("hypothesis.strategies")

if typ.TYPE_CHECKING:
    import collections.abc as cabc
    from pathlib import Path

    from tests.test_wait_diagnostics import Handler, HandlerBuilder


#: Every builder funnels ``handle()`` into the same core enqueue, so all three
#: must share one lifecycle contract. The rotating thresholds are pushed out of
#: reach: rotation is not what these properties are about.
_BUILDERS: dict[str, cabc.Callable[[str], HandlerBuilder]] = {
    "file": FileHandlerBuilder,
    "rotating": lambda path: (
        RotatingFileHandlerBuilder(path).with_max_bytes(1 << 40).with_backup_count(1)
    ),
    "timed-rotating": lambda path: (
        TimedRotatingFileHandlerBuilder(path).with_when("H").with_interval(1 << 20)
    ),
}

#: The exception each binding raises when an enqueue is refused. The file and
#: rotating bindings use ``RuntimeError``; the timed-rotating binding maps the
#: same condition to ``ValueError``. Pinning the type per builder keeps that
#: documented Python surface distinct instead of accepting either exception.
_ENQUEUE_EXCEPTIONS: dict[str, type[Exception]] = {
    "file": RuntimeError,
    "rotating": RuntimeError,
    "timed-rotating": ValueError,
}

#: The failure each dropping policy may report for an open handler. ``block``
#: waits instead of refusing, so it permits no failure at all. A disconnected
#: worker reports "handler is closed" and must fail the example rather than
#: being mistaken for the documented overflow outcome.
_OVERFLOW_FAILURES: dict[str, tuple[str, ...]] = {
    "drop": ("queue full",),
    "block": (),
    "timeout": ("handler send timed out",),
}

#: The policies the generated sequences may run under. ``block`` cannot be left
#: out: it is the only policy that changes whether ``handle()`` can wait, and
#: the lifecycle must hold for it just as it does for the dropping policies.
_POLICIES = ("drop", "block", "timeout")

#: The policies that take no parameter. Indexing by name keeps a mistyped
#: policy loud (``KeyError``) instead of silently falling back to ``drop``.
_SIMPLE_POLICIES: dict[str, cabc.Callable[[], OverflowPolicy]] = {
    "drop": OverflowPolicy.drop,
    "block": OverflowPolicy.block,
}


def _make_policy(name: str, timeout_ms: int) -> OverflowPolicy:
    """Return the overflow policy named by a generated example.

    Parameters
    ----------
    name : str
        One of ``"drop"``, ``"block"``, or ``"timeout"``.
    timeout_ms : int
        Deadline for the ``"timeout"`` policy; ignored otherwise.

    Returns
    -------
    OverflowPolicy
        Policy object accepted by ``with_overflow_policy``.
    """
    if name == "timeout":
        return OverflowPolicy.timeout(timeout_ms)
    return _SIMPLE_POLICIES[name]()


class HandlerLifecycle(stateful.RuleBasedStateMachine):
    """Generate call sequences and assert the shutdown contract after each.

    The handler is built in :meth:`initialize` so one example can vary the
    policy, capacity, and timeout. ``close()`` may be called at any point,
    including before any record is handled, and the machine tracks that fact so
    later calls are checked against the closed contract rather than the open
    one.
    """

    def __init__(self, log_path: str, builder_name: str) -> None:
        """Bind the machine to one log path and one handler builder."""
        super().__init__()
        self._log_path = log_path
        self._builder_name = builder_name
        self._handler: Handler | None = None
        self._policy = ""
        self._closed = False

    @property
    def _active_handler(self) -> Handler:
        """The handler ``init_handler`` built for this example."""
        assert self._handler is not None, "init_handler must run before every rule"
        return self._handler

    @property
    def _enqueue_exception(self) -> type[Exception]:
        """The exception this example's binding raises for a refused enqueue."""
        return _ENQUEUE_EXCEPTIONS[self._builder_name]

    @stateful.initialize(
        policy=strategies.sampled_from(_POLICIES),
        capacity=strategies.integers(min_value=1, max_value=4),
        timeout_ms=strategies.integers(min_value=1, max_value=50),
    )
    def init_handler(self, policy: str, capacity: int, timeout_ms: int) -> None:
        """Build one handler with the drawn policy, capacity, and timeout."""
        self._policy = policy
        builder = _BUILDERS[self._builder_name](self._log_path)
        self._handler = (
            builder
            .with_capacity(capacity)
            .with_flush_after_records(10_000)
            .with_overflow_policy(_make_policy(policy, timeout_ms))
            .build()
        )

    @stateful.rule(message=strategies.text(max_size=16))
    def handle(self, message: str) -> None:
        """Send one record, or observe that the handler refuses it."""
        if self._closed:
            self._assert_closed_refusal(message)
        else:
            self._assert_open_outcome(message)

    def _assert_closed_refusal(self, message: str) -> None:
        """Check that a closed handler refuses the record and names the closure."""
        with pytest.raises(self._enqueue_exception) as excinfo:
            self._active_handler.handle("core", "INFO", message)
        assert "closed" in str(excinfo.value), (
            f"a closed handler must name the closure, got {excinfo.value!r}"
        )

    def _assert_open_outcome(self, message: str) -> None:
        """Check that an open handler accepts the record or refuses it as documented.

        An open handler may refuse the record only in the way its overflow
        policy documents: ``drop`` reports a full queue, ``timeout`` reports an
        expired deadline, and ``block`` waits instead of refusing. A
        disconnected worker or any other failure must fail the example. The
        refusal is captured rather than asserted inside the handler so the
        check stays a plain assertion instead of a ``pytest.raises`` call.
        """
        try:
            self._active_handler.handle("core", "INFO", message)
        except self._enqueue_exception as err:
            refusal = err
        else:
            refusal = None
        if refusal is not None:
            permitted = _OVERFLOW_FAILURES[self._policy]
            assert any(text in str(refusal) for text in permitted), (
                f"the {self._policy!r} policy raised an undocumented failure: "
                f"{refusal!r}"
            )

    @stateful.rule()
    def flush(self) -> None:
        """Flush the handler and check the result against its state."""
        result = self._active_handler.flush()
        if self._closed:
            assert result is False, (
                f"flush() after close() must report False, got {result!r}"
            )
        else:
            assert isinstance(result, bool), (
                f"flush() on an open handler must return a bool, got {result!r}"
            )

    @stateful.rule()
    def close(self) -> None:
        """Close the handler; repeating it must stay safe."""
        # ``close()`` returns ``None`` and waits at most one second for the
        # worker, so an unexpected raise fails the example rather than being
        # swallowed. The wait is bounded, so a stuck worker cannot hang the run.
        self._active_handler.close()
        self._closed = True

    def teardown(self) -> None:
        """Release the worker if the example ended before a generated close."""
        if self._handler is not None:
            self._handler.close()


@pytest.mark.parametrize("builder_name", sorted(_BUILDERS))
def test_handler_lifecycle_invariants(tmp_path: Path, builder_name: str) -> None:
    """Every generated call sequence must satisfy the shutdown contract."""
    stateful.run_state_machine_as_test(
        functools.partial(
            HandlerLifecycle,
            log_path=str(tmp_path / f"{builder_name}.log"),
            builder_name=builder_name,
        ),
        settings=hypothesis.settings(
            max_examples=20,
            stateful_step_count=6,
            deadline=None,
        ),
    )
