"""Scenario bodies for calls made while a file handler is closing."""

from __future__ import annotations

#: A blocked ``handle()`` keeps its shared borrow, so ``close()`` cannot take
#: the exclusive borrow until the producer returns.
CONCURRENT_CLOSE_BODY = """
def body():
    handler = build_handler({path!r}, OverflowPolicy.block())
    saturate(handler)
    watchdog = start_watchdog()
    blocked = start_blocked(lambda: handler.handle("core", "INFO", "third"))
    try:
        # ``handle`` is stuck on queue capacity, so its borrow remains active.
        time.sleep({handoff})

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

    blocked = start_blocked(close_handler)
    assert close_entered.wait(CHILD_WAIT), "the close call never started"
    time.sleep(0.02)
    assert blocked.is_alive(), "close returned before the worker was released"
    try:
        try:
            {operation_call}
        except RuntimeError:
            pass
        else:
            raise AssertionError(
                "a handler operation did not reject the exclusive close borrow"
            )
    finally:
        RELEASE_WORKER.set()

    join_blocked(blocked)
    assert not close_failures, f"close() failed unexpectedly: {{close_failures!r}}"


guard(body)
"""

CLOSE_BORROW_CALLS = {
    "close-handle": 'handler.handle("core", "INFO", "during-close")',
    "close-flush": "handler.flush()",
}
