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

Each body pins the state it probes with :func:`sys.setswitchinterval` rather
than a sleep. A one-second switch interval keeps the thread that starts the
call on the interpreter lock from the moment it signals until its binding
either blocks or detaches, so the probing thread can only resume once the
handler is provably in the state under test. That makes the verdict
independent of how quickly the shared build host schedules either thread.
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

#: ``flush()`` also holds its shared borrow for its whole wait -- here on the
#: worker's acknowledgement -- so a concurrent ``close()`` is rejected just as
#: it is while ``handle()`` waits on queue capacity. A capacity of 2 leaves a
#: command slot free for the flush itself, and the worker is released well
#: inside the flush deadline, so the wait under test is the acknowledgement
#: rather than the one-second enqueue timeout.
CONCURRENT_CLOSE_FLUSH_BODY = """
def body():
    handler = build_handler({path!r}, OverflowPolicy.block(), capacity=2)
    results = []
    saturate(handler)
    blocked = start_blocked(lambda: results.append(handler.flush()))
    watchdog = start_watchdog_when_blocked(blocked)
    try:
        # ``flush`` is still waiting for the worker's acknowledgement, so its
        # shared borrow is active when ``close`` asks for the exclusive one.
        time.sleep({close_hold})

        try:
            handler.close()
        except RuntimeError:
            pass
        else:
            raise AssertionError(
                "close() took the handler while flush() was in flight"
            )
    finally:
        RELEASE_WORKER.set()

    join_blocked(blocked)
    assert_progress_during("flush", watchdog)
    assert results == [True], f"the flush unexpectedly reported {{results!r}}"
    handler.close()


guard(body)
"""


#: Shared prologue for the three "operation during close" bodies. The closing
#: thread takes the shutdown state and then detaches; with a long switch
#: interval the probing thread can only resume after that, so the operation it
#: performs is genuinely concurrent with an in-progress close.
_CLOSE_PROLOGUE = """
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

    sys.setswitchinterval(1.0)
    blocked = start_blocked(close_handler)
    assert close_entered.wait(CHILD_WAIT), "the close call never started"
    try:
"""

#: Shared epilogue for the same bodies: release the worker, wait for the
#: in-progress close to finish, and report a close that failed on its own.
_CLOSE_EPILOGUE = """
    finally:
        RELEASE_WORKER.set()

    join_blocked(blocked)
    assert not close_failures, f"close() failed unexpectedly: {{close_failures!r}}"


guard(body)
"""

#: ``handle()`` reaches a closed handler and reports it, rather than being
#: rejected by the borrow rules. The timed-rotating binding reports handler
#: errors as ``ValueError``, matching its overflow-timeout mapping, so both
#: exception types name the closed handler.
CLOSE_HANDLE_PROBE = """
        try:
            handler.handle("core", "INFO", "during-close")
        except (RuntimeError, ValueError) as err:
            assert "closed" in str(err), f"unexpected rejection: {{err!r}}"
        else:
            raise AssertionError(
                "handle() succeeded after close() took the shutdown state"
            )
        assert blocked.is_alive(), "close returned before the worker was released"
"""

#: ``flush()`` reports the closed handler by returning ``False``.
CLOSE_FLUSH_PROBE = """
        results = [handler.flush()]
        assert results == [False], (
            f"flush() during close must report False, got {{results!r}}"
        )
"""

#: A second ``close()`` shares the first one's completion wait. It must return
#: without raising once the worker stops or its own deadline expires; it does
#: not start new shutdown work.
CLOSE_CLOSE_PROBE = """
        handler.close()
"""

#: Scenario bodies keyed by the operation they perform during an in-progress
#: close.
CLOSE_DURING_BODIES = {
    "close-handle": _CLOSE_PROLOGUE + CLOSE_HANDLE_PROBE + _CLOSE_EPILOGUE,
    "close-flush": _CLOSE_PROLOGUE + CLOSE_FLUSH_PROBE + _CLOSE_EPILOGUE,
    "close-close": _CLOSE_PROLOGUE + CLOSE_CLOSE_PROBE + _CLOSE_EPILOGUE,
}
