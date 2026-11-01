"""Scenario bodies for calls made while a file handler is closing.

These bodies describe the two borrow states a close can produce. A blocked
``handle()`` keeps its shared borrow, so a concurrent ``close()`` cannot take
the exclusive borrow it needs. A waiting ``close()`` holds that exclusive
borrow, so a concurrent ``handle()``, ``flush()``, or ``close()`` is rejected.

:func:`tests.file_handler_gil_scenarios.render_scenario` joins each body to the
shared harness to produce a standalone child-interpreter program;
:mod:`tests.file_handler_gil_support` runs that program under a timeout; and
:mod:`tests.test_file_handler_gil_release` parametrizes it over the file,
rotating, and timed-rotating builders.

The probe uses :func:`sys.setswitchinterval` rather than a sleep. A one-second
switch interval keeps the closing thread on the interpreter lock from the
moment it signals that it is about to call ``close()`` into the binding itself.
The probing thread can only wake once that binding detaches, by which point the
exclusive borrow is held, so the rejection does not depend on how quickly the
shared build host schedules either thread.
"""

from __future__ import annotations

#: A blocked ``handle()`` keeps its shared borrow, so ``close()`` cannot take
#: the exclusive borrow until the producer returns.
CONCURRENT_CLOSE_BODY = """
def body():
    handler = build_handler({path!r}, OverflowPolicy.block())
    saturate(handler)
    blocked = start_blocked(lambda: handler.handle("core", "INFO", "third"))
    watchdog = start_watchdog_when_blocked(blocked)
    try:
        # ``handle`` is stuck on queue capacity, so its borrow remains active.
        time.sleep({hold})

        try:
            handler.close()
        except RuntimeError:
            pass
        else:
            raise AssertionError(
                "close() took the handler while handle() was blocked in it"
            )
    finally:
        RELEASE_WORKER.set()

    join_blocked(blocked)
    assert_progress_during("handle", watchdog)
    handler.close()


guard(body)
"""

#: Closing holds the exclusive PyO3 borrow while it waits with the GIL
#: detached. Calls that need a borrow of the same handler must fail promptly.
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
    # exclusive borrow before detaching. This thread is waiting on the event
    # with the lock released, so it can only resume once that borrow is held.
    # The probe therefore observes the borrow of an in-progress close, not a
    # close that has merely been scheduled.
    sys.setswitchinterval(1.0)
    blocked = start_blocked(close_handler)
    assert close_entered.wait(CHILD_WAIT), "the close call never started"
    try:
        try:
            {operation_call}
        except RuntimeError:
            pass
        else:
            raise AssertionError(
                "a handler operation did not reject the exclusive close borrow"
            )
        assert blocked.is_alive(), "close returned before the worker was released"
    finally:
        RELEASE_WORKER.set()

    join_blocked(blocked)
    assert not close_failures, f"close() failed unexpectedly: {{close_failures!r}}"


guard(body)
"""

CLOSE_BORROW_CALLS = {
    "close-handle": 'handler.handle("core", "INFO", "during-close")',
    "close-flush": "handler.flush()",
    "close-close": "handler.close()",
}
