"""Scenario bodies for calls made while a file handler is closing."""

from __future__ import annotations

#: A blocked ``handle()`` holds its own clone of the sender, so ``close()``
#: cannot end that wait. ``close()`` takes the shutdown state and waits on the
#: shared completion signal for at most one second, then returns without
#: raising; the blocked call still finishes once the worker drains.
CONCURRENT_CLOSE_BODY = """
def body():
    handler = build_handler({path!r}, OverflowPolicy.block())
    saturate(handler)
    watchdog = start_watchdog()
    blocked = start_blocked(lambda: handler.handle("core", "INFO", "third"))
    try:
        # ``handle`` is stuck on queue capacity, so the worker cannot complete.
        time.sleep({handoff})

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
#: borrow conflict. ``handle()`` reports the closure; ``flush()`` returns
#: ``False`` because there is no worker left to acknowledge it.
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
{operation_call}
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
}
