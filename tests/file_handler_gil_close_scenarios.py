"""Scenario bodies for calls made while a file handler is closing.

``close()`` takes only a shared PyO3 borrow, so it can start while another
thread is blocked inside ``handle()`` or ``flush()``. It takes the shutdown
state and waits on the same worker-completion signal as every other closer, so
a concurrent close neither raises nor corrupts the handler. Calls that arrive
after the shutdown state has been taken observe a closed handler: ``handle()``
reports the closure, ``flush()`` returns ``False``, and a second ``close()``
shares the first one's completion wait.

:func:`tests.file_handler_gil_scenarios.render_scenario` joins each body to the
shared harness to produce a standalone child-interpreter program;
:mod:`tests.file_handler_gil_support` runs that program under a timeout; and
:mod:`tests.test_file_handler_gil_release` parametrizes it over the file,
rotating, and timed-rotating builders.

The probe uses :func:`sys.setswitchinterval` rather than a sleep. A one-second
switch interval keeps the closing thread on the interpreter lock from the
moment it signals that it is about to call ``close()`` into the binding itself.
The probing thread can only wake once that binding detaches, by which point the
shutdown state has been taken, so the observation does not depend on how
quickly the shared build host schedules either thread.
"""

from __future__ import annotations

#: A blocked ``handle()`` holds its own clone of the sender, so ``close()``
#: cannot end that wait. ``close()`` takes the shutdown state and waits on the
#: shared completion signal for at most one second, then returns without
#: raising; the blocked call still finishes once the worker drains.
CONCURRENT_CLOSE_BODY = """
def body():
    handler = build_handler({path!r}, OverflowPolicy.block())
    saturate(handler)
    blocked = start_blocked(lambda: handler.handle("core", "INFO", "third"))
    watchdog = start_watchdog_when_blocked(blocked)
    try:
        # ``handle`` is stuck on queue capacity, so the worker cannot complete.
        time.sleep({hold})

        # ``close`` must not raise: it takes the shutdown state and waits on
        # the shared completion signal, bounded by its one-second deadline.
        handler.close()
        try:
            handler.handle("core", "INFO", "after-close")
        except {error_type}:
            pass
        else:
            raise AssertionError("a closed handler accepted another record")
    finally:
        RELEASE_WORKER.set()

    join_blocked(blocked)
    assert_progress_during("handle", watchdog)
    handler.close()


guard(body)
"""

#: Closing takes the shutdown state before it waits, so a call that arrives
#: while the close is in progress observes a closed handler rather than a
#: borrow conflict. Each probe below asserts that contract for one operation.
CLOSE_BORROW_BODY = """
def body():
    handler = build_handler({path!r}, OverflowPolicy.block())
    saturate(handler)
    close_entered = threading.Event()
    close_failures = []

    def close_handler():
        close_entered.set()
        try:
            handler.close()
        except Exception as err:
            close_failures.append(err)

    # With a long switch interval the closing thread keeps the interpreter lock
    # from ``set()`` until its binding detaches, and the binding takes the
    # shutdown state before detaching. This thread is waiting on the event with
    # the lock released, so it can only resume once that state has been taken.
    # The probe therefore observes an in-progress close, not a close that has
    # merely been scheduled.
    sys.setswitchinterval(1.0)
    blocked = start_blocked(close_handler)
    assert close_entered.wait(CHILD_WAIT), "the close call never started"
    try:
{operation_call}
        assert blocked.is_alive(), "close returned before the worker was released"
    finally:
        RELEASE_WORKER.set()

    join_blocked(blocked)
    assert not close_failures, f"close() failed unexpectedly: {{close_failures!r}}"


guard(body)
"""

#: Each snippet asserts the closed-handler contract for one operation. The
#: error type is substituted per builder because the timed-rotating binding
#: maps the same enqueue failure to ``ValueError``.
CLOSE_BORROW_CALLS = {
    "close-handle": """
        try:
            handler.handle("core", "INFO", "during-close")
        except {error_type}:
            pass
        else:
            raise AssertionError("a closing handler accepted another record")
""",
    "close-flush": """
        assert handler.flush() is False, (
            "a flush during close must report that the worker is gone"
        )
""",
    # A second closer shares the first one's completion wait; it must return
    # without raising once the worker stops or its own deadline expires.
    "close-close": """
        handler.close()
""",
}
