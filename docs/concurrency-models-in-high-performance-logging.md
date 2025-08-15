# Concurrency models in high-performance logging: an architectural analysis

This document synthesizes an analysis of concurrency mechanisms within logging
libraries, using Microsoft's `picologging` as a primary case study. It examines
the library's existing model, which blends fine-grained locks with reliance on
Python's Global Interpreter Lock (GIL). It then explores how a language with a
sophisticated compile-time ownership model, such as Rust, could offer enhanced
safety guarantees. Finally, it delves into asynchronous architectures that
decouple log creation from I/O-bound emission, a critical pattern for high-
throughput applications.

## 1. The picologging concurrency model: a hybrid approach

`picologging` achieves its performance by implementing critical paths in C++
and making deliberate trade-offs in its concurrency strategy. It eschews the
single, global module lock found in CPython's standard `logging` library in
favour of a more granular approach.

### Fine-grained locking at the handler level

The most performance-critical aspect of concurrent logging is ensuring that
multiple threads can write log messages without corrupting the output stream
(e.g., a file or `stderr`). `picologging` addresses this with per-handler
locking.

- **Mechanism**: Each `Handler` instance contains its own C++
  `std::recursive_mutex`. This lock is acquired at the beginning of the
  `handle` method and released upon completion.

- **Implementation**: The `Handler` struct definition in
  `src/picologging/handler.hxx` includes the `lock` member. The
  `Handler_handle` function in `src/picologging/handler.cxx` orchestrates the
  lock acquisition and release around the call to the `emit` method.

- **Rationale**: This ensures that the process of formatting a log record and
  writing it to its destination is an atomic operation for each handler. By
  keeping the locks fine-grained (one per handler), threads writing to
  different destinations (e.g., one to a file, another to the console) do not
  contend with each other.

A crucial detail of this implementation is the sequence of operations within
the `handle` method: filtering occurs *before* the lock is acquired. This is an
important optimization, as it prevents the cost of filtering out a message from
contributing to lock contention. However, the work of formatting the message
via the `Formatter` occurs *inside* the locked region, as it is part of the
`emit` call chain.

### Reliance on the GIL for global state

For managing the global state—specifically the hierarchy of loggers stored in
the `Manager`'s `loggerDict`—`picologging` forgoes an explicit global lock.
Instead, it relies on the protection afforded by Python's **Global Interpreter
Lock (GIL)**.

- **Mechanism**: Operations that modify the shared logger hierarchy, such as
  `getLogger` creating a new logger instance or `_fixupParents` linking it into
  the tree, are not guarded by a specific lock within `picologging`. Their
  thread safety is a consequence of the GIL ensuring that only one thread can
  execute Python bytecode at any given time.

- **Rationale**: This design decision is predicated on the assumption that
  logger configuration is a relatively infrequent operation, often performed
  during application start-up in a single-threaded context. By avoiding an
  explicit global lock, `picologging` eliminates a potential point of
  contention for the far more frequent operation of emitting log messages,
  thereby prioritizing runtime performance[^1].

## 2. A Rust implementation: the power of compile-time safety

Translating this architecture to Rust highlights the profound difference
between runtime concurrency checks (mutexes, GIL) and compile-time guarantees
provided by an ownership model and borrow checker.

### Guaranteed handler safety with Mutex

In a Rust version, the handler's shared mutable state (the output stream) would
be wrapped in `std::sync::Mutex` and shared across threads using an `Arc` (an
atomically reference-counted pointer).

```rust
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

trait Handler: Send + Sync {
    fn emit(&self, record: &LogRecord);
}

struct StreamHandler {
    stream: Arc<Mutex<Box<dyn Write + Send>>>,
}

impl Handler for StreamHandler {
    fn emit(&self, record: &LogRecord) {
        // .lock() returns a MutexGuard; the lock is held for its lifetime.
        let mut stream_guard = self.stream.lock().unwrap();
        // The borrow checker now grants us exclusive mutable access.
        writeln!(*stream_guard, "{}", record.get_message()).unwrap();
    }
} // <- lock is automatically released here as stream_guard goes out of scope.

```

The borrow checker provides two unshakable guarantees:

1. **Exclusive Access**: It is a compile-time error to attempt to access the
   stream data without first acquiring the lock via `.lock()`.

2. **No Deadlocks from Forgetfulness**: The RAII (Resource Acquisition Is
   Initialization) pattern, where the lock is tied to the lifetime of the
   `MutexGuard`, makes it impossible to forget to release the lock.

This transforms a runtime convention into a compile-time proof of correctness,
eliminating an entire class of potential data race bugs.

### Granular registry safety with RwLock

For the global logger registry, Rust's `std::sync::RwLock` (Read-Write Lock)
provides a more granular and explicit alternative to relying on the GIL. Since
looking up existing loggers is far more common than creating new ones, an
`RwLock` is ideal.

```rust
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

lazy_static::lazy_static! {
    static ref LOGGER_REGISTRY: RwLock<HashMap<String, Arc<Logger>>> =
        RwLock::new(HashMap::new());
}

fn get_logger(name: &str) -> Arc<Logger> {
    // Acquire a read lock first; multiple threads can do this concurrently.
    let readers = LOGGER_REGISTRY.read().unwrap();
    if let Some(logger) = readers.get(name) {
        return Arc::clone(logger);
    }
    drop(readers); // Release read lock

    // If not found, acquire an exclusive write lock.
    let mut writers = LOGGER_REGISTRY.write().unwrap();
    // Double-check in case another thread created it while we waited for the lock.
    if let Some(logger) = writers.get(name) {
        return Arc::clone(logger);
    }
    // ... create and insert new logger ...
}

```

Here, the borrow checker enforces that you can only get immutable access with a
read lock and mutable access with a write lock, again preventing data races at
compile time.

## 3. Asynchronous architecture: decoupling for maximum throughput

For applications where logging latency is absolutely critical, the optimal
solution is to decouple the fast, thread-local work of record creation from the
slow, I/O-bound work of emission.

This is achieved with a producer-consumer pattern, where application threads
("producers") place log records onto a queue, and a dedicated background thread
("consumer") processes them.

### picologging's asynchronous support

`picologging` natively supports this architecture via its `QueueHandler` and
`QueueListener` classes[^2].

1. `QueueHandler`: Configured on the primary loggers, its only job is to take a
   `LogRecord` and place it on a `queue.Queue`. This is a very low-latency
   operation.

2. `QueueListener`: Running in its own thread, it watches the queue, dequeues
   records, and passes them to its *own* set of downstream, blocking handlers
   (e.g., a `FileHandler`).

This effectively moves all blocking I/O off the application's critical path,
ensuring log calls have minimal impact on performance.

### The idiomatic Rust pattern: MPSC channels

In Rust, this asynchronous model is best implemented not with a
`Mutex<VecDeque>` (which still involves a shared lock), but with a **Multi-
Producer, Single-Consumer (MPSC) channel** from the standard library.

This pattern is the definitive solution for this use case:

- **Multiple Producers (**`Sender`**)**: Each application thread holds a
  lightweight, cloneable `Sender`. Sending a message is a highly optimized,
  lock-free (or very low contention) operation.

- **Single Consumer (**`Receiver`**)**: A single background thread owns the
  `Receiver`. Because it is the sole owner, it can call the final I/O-bound
  handlers **without any locks whatsoever**. The channel's design guarantees
  serial, thread-safe delivery to this single consumer.

This architecture achieves the ultimate goal: the hot path (log creation) is
parallel and minimally contentious. Records are queued on bounded MPSC
channels, forming a producer–consumer pipeline. Each builder sets the channel
capacity and overflow policy, providing predictable back-pressure. The cold
path (I/O) is handled serially by a dedicated worker, eliminating lock
contention where it is most expensive.[^1][^2]

[^1]: See
      [logging-cpython-picologging-comparison.md](logging-cpython-picologging-comparison.md)

[^2]: Source:
      <!-- markdownlint-disable-next-line MD013 -->
      [`handlers.py`](https://github.com/microsoft/picologging/blob/dc110b52c9f2e209f97a6fe80d286afb73a8437e/src/picologging/handlers.py)
