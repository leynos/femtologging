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
# but only once the flush reports that it has released the GIL: the extension
# raises a test-only signal at each detach site, as the last thing it does with
# the GIL still held (``gil_release_test``). A thread can only observe that
# signal if the flush gave the GIL up, so the release cannot land in front of
# the wait -- not even in the detached section ``flush_handlers()`` keeps
# around the whole call. If the flush holds the GIL instead, no other Python
# thread runs between the call being entered and its timeout, so the signal is
# never observed, the release never arrives, the formatter times out, and the
# flush fails.
#
# To make that schedule forced rather than argued, a second handler registered
# ahead of the file handler pauses in its own flush for longer than the release
# delay. That pause runs in the detached section in front of the native wait,
# where a release timed from the start of the call would already have fired.
# The release here cannot fire there, because the signal it waits for is raised
# only at the native flush.
#
# The ticker counts the interpreter time the flush lets other Python code have,
# measured from the moment that signal was observed, so time before the wait
# cannot be counted.
FLUSH_FORMATTER_SCRIPT = """
import pathlib
import sys
import threading
import time

from femtologging import FileHandlerBuilder, get_logger
from femtologging._femtologging_rs import (
    drain_gil_release_signals_for_test,
    wait_for_gil_release_for_test,
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
ticks = [0]
ticks_at_release = [None]

TICK_SECONDS = 0.005
# Half the handler's own one-second flush deadline, so a released formatter
# still lets the flush report success.
RELEASE_DELAY_SECONDS = 0.5
# Longer than the release delay: a release timed from the start of the call
# would fire inside this pause, with the native wait still ahead of it.
PAUSE_BEFORE_NATIVE_SECONDS = 0.6
FORMATTER_TIMEOUT_SECONDS = 5.0
RELEASE_TIMEOUT_MS = 10_000


class PausingHandler:
    \"\"\"A handler that pauses in the detached section before the native wait.\"\"\"

    def handle(self, logger, level, message):
        \"\"\"Discard the record; this handler only contributes a flush pause.\"\"\"

    def flush(self):
        \"\"\"Spend longer than the release delay before reporting success.\"\"\"
        time.sleep(PAUSE_BEFORE_NATIVE_SECONDS)
        return True


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
# Registered first, so it is flushed before the file handler: its pause lands
# between the call and the native wait, which is exactly where a timed release
# would misfire.
logger.add_handler(PausingHandler())
logger.add_handler(handler)


def release_once_the_wait_begins():
    # The signal is raised while the flush still holds the GIL, immediately
    # before it detaches. Returning from this wait therefore proves the flush
    # has given up the GIL, whatever ran in front of the native flush.
    if not wait_for_gil_release_for_test(RELEASE_TIMEOUT_MS):
        return
    ticks_at_release[0] = ticks[0]
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

# Arm the gate on a clean channel: a signal left by any earlier flush belongs
# to that call, not this one.
drain_gil_release_signals_for_test()
release = threading.Thread(target=release_once_the_wait_begins, daemon=True)
release.start()
started = time.monotonic()
flushed = logger.flush_handlers()
elapsed = time.monotonic() - started
# Read the count while the GIL is still held, so no tick after the call can be
# mistaken for one the call let through.
baseline = ticks_at_release[0]
observed = 0 if baseline is None else ticks[0] - baseline
is_finished.set()
ticker.join()

assert flushed, "flush_handlers() stalled behind the GIL"
assert baseline is not None, (
    "flush_handlers() never reported that it had released the GIL, so the wait "
    "held it"
)
assert elapsed >= (PAUSE_BEFORE_NATIVE_SECONDS + RELEASE_DELAY_SECONDS) * 0.9, (
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
