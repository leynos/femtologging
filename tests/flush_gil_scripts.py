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
# only another Python thread can give. A second thread releases the formatter,
# but only once the flush reports that it has the interpreter detached: a wait
# marks itself in flight for exactly the span of its detach, and the extension
# records an observation only while one is in flight (``gil_release_test``).
# The release therefore cannot land in front of the wait -- not even in the
# detached section ``flush_handlers()`` keeps around the whole call -- and a
# wait that held the GIL would both block the release and record no
# observations at all.
#
# Because the count lives on the Rust side and is only advanced while a wait is
# detached, no part of it can come from before the wait or from the handover
# after the call returns; there is no baseline in Python to race.
FLUSH_FORMATTER_SCRIPT = """
import pathlib
import sys
import threading
import time

from femtologging import FileHandlerBuilder, get_logger
from femtologging._femtologging_rs import (
    gil_release_observations_for_test,
    note_python_thread_ran_for_test,
    reset_gil_release_observations_for_test,
    wait_for_wait_in_flight_for_test,
)

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

TICK_SECONDS = 0.005
# Half the handler's own one-second flush deadline, so a released formatter
# still lets the flush report success.
RELEASE_DELAY_SECONDS = 0.5
FORMATTER_TIMEOUT_SECONDS = 5.0
IN_FLIGHT_TIMEOUT_MS = 10_000


def pause_formatter(record):
    formatter_entered.set()
    if not release_formatter.wait(timeout=FORMATTER_TIMEOUT_SECONDS):
        raise TimeoutError("no other thread could release the formatter")
    return f"formatted {record['message']}"


def count_ticks():
    ticker_started.set()
    while not is_finished.is_set():
        # Counts only while the measured call has the interpreter detached.
        note_python_thread_ran_for_test()
        time.sleep(TICK_SECONDS)


handler = FileHandlerBuilder(str(path)).with_formatter(pause_formatter).build()
logger = get_logger("gil.flush.formatter")
logger.add_handler(handler)


def release_once_the_wait_begins():
    # A wait is in flight exactly while it has the interpreter detached, so
    # returning from this call means the native flush has given the GIL up,
    # whatever ran in front of it. A flush that kept the GIL never gets here,
    # and its formatter times out.
    if not wait_for_wait_in_flight_for_test(IN_FLIGHT_TIMEOUT_MS):
        return
    # Keep the formatter blocked a while longer, so the call has a window in
    # which only a released GIL lets the ticker run.
    time.sleep(RELEASE_DELAY_SECONDS)
    release_formatter.set()


ticker = threading.Thread(target=count_ticks, daemon=True)
ticker.start()
assert ticker_started.wait(timeout=5), "the ticker thread never started"

if mode == "direct":
    handler.handle("gil.flush.formatter", "INFO", "paused")
else:
    logger.info("paused")
assert formatter_entered.wait(timeout=5), "the handler's worker never formatted"

# Forget observations from anything that ran before the measured call.
reset_gil_release_observations_for_test()
release = threading.Thread(target=release_once_the_wait_begins, daemon=True)
release.start()
started = time.monotonic()
flushed = logger.flush_handlers()
elapsed = time.monotonic() - started
observed = gil_release_observations_for_test()
is_finished.set()
ticker.join()

assert flushed, "flush_handlers() stalled behind the GIL"
assert elapsed >= RELEASE_DELAY_SECONDS * 0.9, (
    f"flush_handlers() returned after {elapsed:.3f}s, so the blocked formatter "
    "was not what the flush waited for and this test proved nothing"
)
assert observed >= 10, (
    f"another Python thread ran only {observed} times during {elapsed:.3f}s of "
    "flush_handlers(), so the wait held the GIL"
)
# Read before close(): the record must be on disk because the flush waited for
# the handler, not because close() later drained the worker. If PyHandler's
# flush returned without delegating, the formatter would still be blocked here
# and this read would find an empty file.
assert path.read_text(encoding="utf-8") == "formatted paused\\n", (
    "flush_handlers() returned before the record reached the file"
)
handler.close()
"""
