"""Scenario bodies for calls made while a file handler is closing.

Two different states involve ``close()``, and they have different contracts. A
blocked ``handle()`` keeps its shared borrow for its whole wait, so a
concurrent ``close()`` cannot take the exclusive borrow it needs and is
rejected. A ``close()`` that has started, by contrast, gives its Python borrow
back before it waits for the worker: calls made during that wait reach a
handler that is already closed instead of being rejected.

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

#: A second ``close()`` is a no-op: the first call already took the shutdown
#: state, so there is nothing left for it to wait on.
CLOSE_CLOSE_PROBE = """
        handler.close()
        assert blocked.is_alive(), "close returned before the worker was released"
"""

#: Scenario bodies keyed by the operation they perform during an in-progress
#: close.
CLOSE_DURING_BODIES = {
    "close-handle": _CLOSE_PROLOGUE + CLOSE_HANDLE_PROBE + _CLOSE_EPILOGUE,
    "close-flush": _CLOSE_PROLOGUE + CLOSE_FLUSH_PROBE + _CLOSE_EPILOGUE,
    "close-close": _CLOSE_PROLOGUE + CLOSE_CLOSE_PROBE + _CLOSE_EPILOGUE,
}
