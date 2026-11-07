"""Scenario bodies for waits performed inside a file-handler call.

Each body names one wait a Python-exposed binding performs while the
interpreter lock is released: queue capacity for ``"block"``, the
``"timeout:N"`` deadline, the flush enqueue deadline, the flush
acknowledgement, and worker shutdown for ``close()``.

The bodies are plain strings with named placeholders.
:func:`tests.file_handler_gil_scenarios.render_scenario` joins the selected
body to the shared harness defined there, and
:mod:`tests.file_handler_gil_support` runs the result in a child interpreter.
"""

from __future__ import annotations

#: How long the ``"block"`` scenario waits before releasing the parked worker.
#: The call cannot return before then, so this is a guaranteed-blocked window.
BLOCK_HOLD_SECONDS = 1.0

#: The overflow timeout the scenarios configure. It mirrors the binding's own
#: unit, milliseconds, so the scenario and the wait it exercises agree.
TIMEOUT_MS = 2000

#: How long the close scenario keeps its worker parked before releasing it. The
#: release must land inside the binding's one-second shutdown wait so that the
#: close observes the worker's acknowledgement and joins it, while the whole
#: window stays comfortably above
#: :data:`~tests.file_handler_gil_scenarios.MIN_WAIT_SECONDS`. A close that
#: times out instead detaches its worker, and that detached worker may then
#: attach to the interpreter while the child finalizes -- a state CPython 3.12
#: does not guard against.
CLOSE_HOLD_SECONDS = 0.6

#: An unbounded ``"block"`` wait ends only when the worker is released, so the
#: window is guaranteed to span :data:`BLOCK_HOLD_SECONDS`.
_BLOCK_BODY = """
def body():
    handler = build_handler({path!r}, OverflowPolicy.block())
    saturate(handler)
    blocked = start_blocked(lambda: handler.handle("core", "INFO", "third"))
    watchdog = start_watchdog_when_blocked(blocked)
    try:
        # The call cannot return while the worker is parked and the queue is
        # full, so this window is a genuine blocked wait.
        time.sleep({hold})
    finally:
        RELEASE_WORKER.set()
    join_blocked(blocked)

    assert_progress_during("handle", watchdog)
    handler.close()


guard(body)
"""


#: The timeout policy ends its own wait, so progress must have been recorded
#: while the call was still outstanding.
_TIMEOUT_BODY = """
def body():
    handler = build_handler({path!r}, OverflowPolicy.timeout({timeout_ms}))
    failures = []

    def call():
        try:
            handler.handle("core", "INFO", "third")
        except {error_type} as err:
            failures.append(err)

    saturate(handler)
    blocked = start_blocked(call)
    watchdog = start_watchdog_when_blocked(blocked)
    join_blocked(blocked)

    assert_progress_during("handle", watchdog)
    assert len(failures) == 1, (
        "a timed-out handle() must raise exactly once, got "
        f"{{len(failures)}} failure(s)"
    )
    assert "timed out" in str(failures[0]), (
        "a saturated timeout policy must report the timeout, got "
        f"{{failures[0]!r}}"
    )
    RELEASE_WORKER.set()
    handler.close()


guard(body)
"""


#: With the queue full, ``flush()`` cannot even enqueue its command, so it
#: waits for a command slot until the one-second deadline expires. The flush
#: acknowledgement is not involved here; :data:`_FLUSH_ACK_BODY` covers it.
_FLUSH_BODY = """
def body():
    handler = build_handler({path!r}, OverflowPolicy.block())
    results = []
    saturate(handler)
    blocked = start_blocked(lambda: results.append(handler.flush()))
    watchdog = start_watchdog_when_blocked(blocked)
    join_blocked(blocked)

    # ``flush`` gives up on the full queue after one second, so the window this
    # asserts over is the binding's own enqueue deadline.
    assert_progress_during("flush", watchdog)
    assert results == [False], (
        "a flush that outlives its one-second deadline must report False, got "
        f"{{results!r}}"
    )
    RELEASE_WORKER.set()
    handler.close()


guard(body)
"""


#: A capacity of 2 leaves one command slot free after ``saturate``, so the
#: flush command is enqueued at once and the caller waits on the worker's
#: acknowledgement rather than on queue capacity. That is the second wait the
#: binding performs, and it must release the GIL too.
_FLUSH_ACK_BODY = """
def body():
    handler = build_handler({path!r}, OverflowPolicy.block(), capacity=2)
    results = []
    saturate(handler)
    blocked = start_blocked(lambda: results.append(handler.flush()))
    watchdog = start_watchdog_when_blocked(blocked)
    join_blocked(blocked)

    # The worker is parked and the queue has room, so the only wait available
    # is the acknowledgement receive, bounded by the same one-second deadline.
    assert_progress_during("flush", watchdog)
    assert results == [False], (
        "a flush whose acknowledgement outlives the deadline must report False, "
        f"got {{results!r}}"
    )
    RELEASE_WORKER.set()
    handler.close()


guard(body)
"""


#: ``close()`` waits for the worker to stop. The worker is released while the
#: call is still inside its one-second shutdown wait, so close sees the
#: acknowledgement and joins the worker. Letting the call time out instead
#: would detach a thread that can still attach to Python after the child
#: interpreter starts finalizing, which CPython 3.12 does not guard against.
_CLOSE_BODY = """
def body():
    handler = build_handler({path!r}, OverflowPolicy.block())
    saturate(handler)
    blocked = start_blocked(handler.close)
    watchdog = start_watchdog_when_blocked(blocked)
    # Hold the worker for most of, but not all of, the close deadline: the
    # window under test stays well above the minimum, and the remaining budget
    # still ends the call through the worker's acknowledgement rather than
    # through its deadline.
    time.sleep({close_hold})
    RELEASE_WORKER.set()
    join_blocked(blocked)

    assert_progress_during("close", watchdog)


guard(body)
"""


#: The wait scenarios, keyed by the label their failures report.
WAIT_BODIES: dict[str, str] = {
    "handle-block": _BLOCK_BODY,
    "handle-timeout": _TIMEOUT_BODY,
    "flush": _FLUSH_BODY,
    "flush-ack": _FLUSH_ACK_BODY,
    "close": _CLOSE_BODY,
}
