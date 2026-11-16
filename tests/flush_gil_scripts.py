"""Scenario scripts the flush GIL tests run in a fresh interpreter.

Each script runs through ``python -c`` so it owns its dispatch shards and its
handler workers: dispatch shards are shared across a process, so a record can be
refused when earlier tests left its shard busy, and a shared worker could let a
flush succeed for a reason the script did not arrange. The comments inside each
script carry the argument its assertions make.
"""

# Checks the drop counter first, so a refused record cannot pass for a flush
# that returned too early.
FLUSH_HANDLERS_SCRIPT = """
import pathlib
import sys

from femtologging import FemtoFileHandler, get_logger

path = pathlib.Path(sys.argv[1])
handler = FemtoFileHandler(str(path))
logger = get_logger("gil.flush")
logger.add_handler(handler)

logger.info("delivered")
assert logger.flush_handlers(), "flush_handlers stalled behind the GIL"
assert logger.get_dropped() == 0, "the record was refused before dispatch"
assert path.read_text(encoding="utf-8") == "gil.flush [INFO] delivered\\n", (
    "flush_handlers() returned before the record reached the file"
)
handler.close()
"""

# The worker is told to flush only every third record, so the one record logged
# here stays in its buffer. That makes delivery proof of delegation rather than
# of timing: no thread, clock, or scheduler can write the file, so a
# `PyHandler::flush()` that reported success without asking the handler to
# flush would leave the file empty. The empty read before the call is the
# precondition that proves the record really was still buffered.
BUFFERED_RECORDS_SCRIPT = """
import pathlib
import sys

from femtologging import FileHandlerBuilder, get_logger

path = pathlib.Path(sys.argv[1])
handler = FileHandlerBuilder(str(path)).with_flush_after_records(3).build()
logger = get_logger("gil.flush.buffered")
logger.add_handler(handler)

logger.info("held")
assert path.read_text(encoding="utf-8") == "", (
    "the record reached the file before flush_handlers(), so this test cannot "
    "show that the flush is what delivered it"
)
assert logger.flush_handlers(), "flush_handlers did not report the flush"
assert logger.get_dropped() == 0, "the record was refused before dispatch"
assert path.read_text(encoding="utf-8") == "gil.flush.buffered [INFO] held\\n", (
    "flush_handlers() returned without flushing the buffered record"
)
handler.close()
"""

# The handler's worker formats the record in Python and waits for a release
# only another Python thread can give. A second thread releases the formatter
# but only once it can see the flush running on this thread's stack, which the
# interpreter reports while the C call is detached. Seeing the frame is not
# enough on its own, because a thread can be switched out between entering the
# helper and calling: the observer also requires the frame's instruction offset
# to be the call instruction, so the release cannot land in that gap either.
# If the flush holds the GIL, no other Python thread runs between the call being
# entered and its timeout, so the release the formatter waits for cannot
# arrive, the formatter times out, and the flush fails. The ticker counts the
# interpreter time the flush lets other Python code have, measured from the
# moment the call was seen running, so time before the call cannot be counted.
# ``flush_handlers()`` detaches for the whole call, so the release is also
# delayed past a measured bound on everything in front of the native wait; a
# release issued while only the dispatch drain was running could otherwise
# release a flush that had not reached its wait yet.
FLUSH_FORMATTER_SCRIPT = """
import dis
import pathlib
import sys
import threading
import time

from femtologging import FileHandlerBuilder, get_logger

path = pathlib.Path(sys.argv[1])
# "dispatched" queues the record through the logger, so ``flush_handlers()``
# drains the dispatch shard before it flushes. "direct" queues it on the handler
# itself, leaving the shard empty, so the drain returns at once and the only
# wait left is the native one.
mode = sys.argv[2]
formatter_entered = threading.Event()
release_formatter = threading.Event()
ticker_started = threading.Event()
is_finished = threading.Event()
ticks = [0]
ticks_at_call = [None]

TICK_SECONDS = 0.005
# Long enough that the ticker must get the GIL many times if the flush really
# waits, several times longer than everything that runs before the native wait
# (measured below), and half the handler's own one-second flush deadline so a
# released formatter still lets the flush report success.
RELEASE_DELAY_SECONDS = 0.5
FORMATTER_TIMEOUT_SECONDS = 5.0
CALL_TIMEOUT_SECONDS = 10.0


def pause_formatter(record):
    formatter_entered.set()
    if not release_formatter.wait(timeout=FORMATTER_TIMEOUT_SECONDS):
        raise TimeoutError("no other thread could release the formatter")
    return f"formatted {record['message']}"


def count_ticks():
    ticker_started.set()
    while not is_finished.is_set():
        ticks[0] += 1
        time.sleep(TICK_SECONDS)


handler = FileHandlerBuilder(str(path)).with_formatter(pause_formatter).build()
logger = get_logger("gil.flush.formatter")
logger.add_handler(handler)

# ``flush_handlers()`` detaches for the whole call, so the GIL is free from the
# moment the call instruction is reached until the handler is flushed. Measure
# how long that pre-wait part lasts with the same call and nothing blocked: the
# release below is delayed several times longer, so it cannot be issued while
# only the dispatch drain is running, and the ticks it gates can only come from
# the native wait. Without this bound the release could land in the detached
# window in front of the wait, and a wait that held the GIL would still look
# released.
control_handler = FileHandlerBuilder(str(path.with_name("control.log"))).build()
# A sibling of the measured logger, not a child: a child would inherit the
# blocking handler and its record would reach the file under test.
control_logger = get_logger("gil.flush.control")
control_logger.add_handler(control_handler)
if mode == "direct":
    control_handler.handle("gil.flush.control", "INFO", "control")
else:
    control_logger.info("control")
control_started = time.monotonic()
assert control_logger.flush_handlers(), "the control flush failed"
control_elapsed = time.monotonic() - control_started
control_handler.close()
assert control_elapsed < RELEASE_DELAY_SECONDS / 5, (
    f"the path in front of the native wait took {control_elapsed:.3f}s, so the "
    "release could be issued before the wait starts"
)

call = logger.flush_handlers
call_name = "flush_handlers()"
caller_ident = threading.get_ident()


def invoke_call():
    # The only statement is the call, so this frame is on the caller's stack
    # exactly while the flush runs and has not returned.
    return call()


# The call instruction's byte offsets. A frame reports the instruction it was
# last executing, so accepting only these offsets keeps an observation out of
# the window between the frame being created and the call starting.
CALL_OFFSETS = frozenset(
    instruction.offset
    for instruction in dis.get_instructions(invoke_call)
    if instruction.opname.startswith("CALL")
)
# The call is the helper's only statement, so its only call instruction is the
# one under test. A second one would let the release land while the helper ran
# something else, which is the window this guards against.
assert len(CALL_OFFSETS) == 1, "invoke_call must make exactly one call"


def release_once_the_call_is_running():
    # Wait for the interpreter to report the flush frame, running its call
    # instruction, on the calling thread. The interpreter reports that frame
    # while the call holds it detached, and the offset shows the call itself is
    # in progress, so a release can never land before the call starts.
    deadline = time.monotonic() + CALL_TIMEOUT_SECONDS
    while time.monotonic() < deadline:
        frame = sys._current_frames().get(caller_ident)
        if (
            frame is not None
            and frame.f_code is invoke_call.__code__
            and frame.f_lasti in CALL_OFFSETS
        ):
            ticks_at_call[0] = ticks[0]
            # Keep the formatter blocked a while longer, so the call has a
            # window in which only a released GIL lets the ticker run.
            time.sleep(RELEASE_DELAY_SECONDS)
            release_formatter.set()
            return
        time.sleep(TICK_SECONDS)


ticker = threading.Thread(target=count_ticks, daemon=True)
ticker.start()
assert ticker_started.wait(timeout=5), "the ticker thread never started"

if mode == "direct":
    handler.handle("gil.flush.formatter", "INFO", "paused")
else:
    logger.info("paused")
assert formatter_entered.wait(timeout=5), "the handler's worker never formatted"

release = threading.Thread(target=release_once_the_call_is_running, daemon=True)
release.start()
started = time.monotonic()
flushed = invoke_call()
elapsed = time.monotonic() - started
# Read the count while the GIL is still held, so no tick after the call can be
# mistaken for one the call let through.
baseline = ticks_at_call[0]
observed = 0 if baseline is None else ticks[0] - baseline
is_finished.set()
ticker.join()

assert flushed, f"{call_name} stalled behind the GIL"
assert baseline is not None, (
    f"no other thread ever saw {call_name} running, so the wait held the GIL"
)
assert elapsed >= 0.2, (
    f"{call_name} returned after {elapsed:.3f}s, so the blocked formatter "
    "was not what the flush waited for and this test proved nothing"
)
assert observed >= 10, (
    f"another Python thread ran only {observed} times during {elapsed:.3f}s of "
    f"{call_name}, so the wait held the GIL"
)
# Read before close(): the record must be on disk because the flush waited for
# the handler, not because close() later drained the worker. If PyHandler's
# flush returned without delegating, the formatter would still be blocked here
# and this read would find an empty file.
assert path.read_text(encoding="utf-8") == "formatted paused\\n", (
    f"{call_name} returned before the record reached the file"
)
handler.close()
"""
