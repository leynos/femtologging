# Fearless Concurrency: A Developer's Guide to Multithreading in PyO3 v0.25.1

## Introduction

This guide provides an exhaustive analysis of multithreading within Python
extensions built with Rust and PyO3 version 0.25.1. It is intended for an
audience of experienced systems developers who are already proficient in Rust,
Python, and the low-level CPython C API. As such, it presupposes a working
knowledge of foundational concepts like the Global Interpreter Lock (GIL),
Python's object model, and the manual reference counting
(`Py_INCREF`/`Py_DECREF`) and GIL management
(`PyGILState_Ensure`/`PyGILState_Release`) inherent to the C API.

The central thesis of this report is that PyO3 represents a paradigm shift in
developing concurrent Python extensions. It moves beyond the runtime discipline
and manual bookkeeping required by the C API, instead leveraging Rust's
powerful type system—specifically its concepts of lifetimes and traits—to
provide compile-time safety guarantees for GIL management and object
handling.[^1] This "safety by construction" approach eliminates entire classes
of common, hard-to-debug errors like segmentation faults from improper GIL
handling and data races from unsynchronized access to shared state.

The scope of this document encompasses a deep dive into PyO3's GIL management
model, its mechanisms for thread-safe object and data handling, an examination
of practical parallelism patterns and their associated pitfalls, a look at
robust error propagation across threads, and forward-looking considerations for
the experimental free-threaded builds of Python. By the end of this guide, a
developer will be equipped not only with the "how" of writing multithreaded
PyO3 code but, more importantly, the "why" behind its design, enabling the
construction of robust, correct, and high-performance concurrent systems.

## The Bedrock of Concurrency: PyO3's GIL Management Model

At the heart of PyO3's concurrency story is its explicit and type-safe model
for the Python Global Interpreter Lock. Where the C API relies on the
developer's discipline to manually acquire and release the GIL at appropriate
times, PyO3 encodes the state of the GIL into the Rust type system itself,
transforming potential runtime crashes into compile-time errors.

### The `Python<'py>` Token: A Compile-Time Proof of GIL Acquisition

The cornerstone of PyO3's GIL safety model is the `Python<'py>` token.[^2] This
is a zero-cost, marker-like struct that serves as a tangible, compile-time
"proof" that the current thread holds the GIL. Its presence is required by any
PyO3 API function that needs to interact with the Python interpreter.

The true innovation lies in the associated lifetime parameter, `'py`. This
lifetime is bound to the duration for which the GIL is held. Any other PyO3
type that is parameterized by this `'py` lifetime, such as the `Bound<'py, T>`
smart pointer, is statically tied to the GIL's state.[^2] The Rust compiler,
through its borrow-checking rules, ensures that no such GIL-bound type can
escape the scope where its corresponding

`Python<'py>` token is valid.

This provides a profound safety advantage over the CPython C API. In C, a
developer can hold a `PyObject*` pointer, but the compiler has no mechanism to
verify that `PyGILState_Ensure()` has been called before that pointer is used.
An accidental omission of this call leads to a segmentation fault or other
undefined behaviour at runtime.[^5] PyO3 eradicates this entire class of bugs.
An attempt to use a GIL-bound type like

`Bound<'py, PyList>` without a valid `Python<'py>` token in scope is not a
runtime error; it is a compile-time error.[^1] This shifts the burden of
correctness from fallible human discipline to the infallible compiler.

### Acquiring the GIL: From Implicit Context to Explicit Control

PyO3 provides several ways to obtain the `Python<'py>` token, catering to
different scenarios.

**Implicit Acquisition:** When writing a function exposed to Python via
`#[pyfunction]` or a method with `#[pymethods]`, the most common and efficient
way to get the token is to simply declare it as a function argument:
`fn my_func(py: Python<'_>,...)`.[^6] Because this function is being invoked by
the Python interpreter, the GIL is already held. PyO3 recognizes this and
provides the token automatically, incurring no runtime overhead. This is the
preferred method for functions that are called from Python.

**Explicit Acquisition with** `Python::with_gil`**:** When Rust code needs to
initiate interaction with the Python interpreter—for example, from a
Rust-spawned thread or within a Rust binary that embeds Python—the
`Python::with_gil` function is the primary mechanism.[^2] This function handles
the logic of acquiring the GIL, executing a user-provided closure with the

`Python<'py>` token, and ensuring the GIL is released when the closure exits,
even in the case of a panic.

```rust
use pyo3::prelude::*;

fn call_python_from_rust() -> PyResult<()> {
    Python::with_gil(|py| {
        // The 'py' token is valid only within this closure.
        let sys = py.import("sys")?;
        let version: String = sys.getattr("version")?.extract()?;
        println!("Python version: {}", version);
        Ok(())
    })
}
```

This pattern is the safe, idiomatic Rust equivalent of a
`PyGILState_STATE gstate = PyGILState_Ensure();...; PyGILState_Release(gstate);`
 block in C. It leverages Rust's RAII (Resource Acquisition Is Initialization)
pattern to guarantee the release of the GIL. Furthermore, if the
`auto-initialize` feature is enabled in `Cargo.toml`, `Python::with_gil` will
also handle the one-time initialization of the Python interpreter if it hasn't
been started yet.[^2]

### Unlocking Parallelism: Releasing the GIL with `py.allow_threads()`

The most critical function for achieving true parallelism in a PyO3 extension
is `py.allow_threads()`. This method takes a closure, releases the GIL before
executing it, and re-acquires the GIL upon its completion.[^7] This allows
other Python threads to run or, more importantly for CPU-bound tasks, allows
other Rust threads to acquire the GIL if they are waiting.[^9]

```rust
use pyo3::prelude::*;
use rayon::prelude::*;

#[pyfunction]
fn sum_list_parallel(py: Python<'_>, numbers: Vec<i64>) -> i64 {
    // Release the GIL to allow Rayon's thread pool to work without blocking Python.
    py.allow_threads(|| {
        numbers.par_iter().sum()
    })
}
```

The safety of `allow_threads` is enforced by a clever abstraction: the `Ungil`
trait. On stable Rust, where custom auto traits are not available, `Ungil` is
defined as a proxy for the standard `Send` trait
(`unsafe impl<T: Send> Ungil for T {}`).[^1] The closure passed to

`allow_threads` must satisfy this `Ungil` bound, which effectively means it
must be `Send`. This has a powerful consequence: any non-`Send` types are
forbidden from being captured by the closure. Since GIL-bound types like
`Python<'py>` and `Bound<'py, T>` are inherently non-`Send`, the compiler
statically prevents them from "leaking" into a context where the GIL is not
held.

On nightly Rust, with the `nightly` feature flag enabled, this abstraction is
even more precise. `Ungil` is defined as a true auto trait, with explicit
negative implementations (`impl!Ungil for...`) for types like `Python<'_>` and
raw FFI pointers.[^1] This provides more robust and accurate safety guarantees
without relying on the broader

`Send` trait as a proxy.

### The Deadlock Pitfall: Navigating Interactions with Rust Synchronization Primitives

While PyO3 provides strong safety guarantees around the GIL itself, a classic
and dangerous pitfall emerges when mixing GIL management with other
synchronization primitives, such as `std::sync::Mutex`. The documentation
details a common deadlock scenario 6:

1. A Rust thread (Thread A) acquires the GIL.

2. Thread A then locks a standard Rust `Mutex`.

3. Thread A proceeds to call a Python function (e.g., `py.import()`, or any
   method on a Python object) which, under the hood, might temporarily release
   and then attempt to re-acquire the GIL.

4. Another thread (Thread B) seizes the opportunity to acquire the now-free GIL.

5. Thread B then attempts to lock the same `Mutex` that Thread A is holding,
   causing Thread B to block.

6. Meanwhile, the Python operation in Thread A completes and tries to re-acquire
   the GIL, but it is held by the blocked Thread B. Both threads are now
   waiting on a resource held by the other, resulting in a deadlock.

The solution to this is a golden rule that must be treated as inviolable in
concurrent PyO3 development: **Always release the GIL *before* acquiring other
locks or blocking on other long-running, non-Python operations.**

The correct pattern is to wrap the lock acquisition inside the `allow_threads`
closure:

```rust
// WRONG: Deadlock potential
// my_mutex.lock().unwrap();
// py.call_method0(...)?;

// CORRECT: Release GIL before locking
// py.allow_threads(|| {
//     my_mutex.lock().unwrap();
//     // do work with the lock...
// });
```

This pattern is so fundamental that PyO3 provides specialized tools to handle
common variations correctly. The `GILOnceCell` type offers a deadlock-safe
alternative to `std::sync::OnceLock` for one-time global initialization, a
common source of this deadlock pattern.[^10] Additionally, for cases where a
lock must be held while interacting with Python, the

`MutexExt` trait provides a `lock_py_attached` method for `std::sync::Mutex`.
This specialized lock function is aware of PyO3's internals and helps prevent
deadlocks with the GIL or other global interpreter synchronization events.[^12]

<!-- markdownlint-disable MD013 MD033 MD056 -->
| PyO3 API                         | CPython C API Equivalent                                                | Core Function                           | Safety Guarantees in PyO3                                     |
| -------------------------------- | ----------------------------------------------------------------------- | --------------------------------------- | ------------------------------------------------------------- |
| `Python::with_gil`               | `PyGILState_STATE g = PyGILState_Ensure(); …; PyGILState_Release(g);`   | Acquire the GIL and run a closure       | RAII ensures the GIL is released even if the closure panics   |
| `py.allow_threads`               | `Py_BEGIN_ALLOW_THREADS`…`Py_END_ALLOW_THREADS`                         | Run blocking code without the GIL       | Allows other Python threads to run while the closure executes |
| `py` in `#[pyfunction]` argument | Implicit context                                                        | Access the GIL in callbacks from Python | Zero-cost token proving the GIL is held                       |

<!-- markdownlint-enable MD013 MD033 MD056 -->

## Managing State Across Threads: Object Lifetimes and Safety

Effective multithreading requires not just managing execution via the GIL, but
also safely handling the data and objects that are shared between threads. PyO3
provides a robust framework for this, centered on a distinction between
GIL-dependent and GIL-independent object handles and strict, compile-time
requirements for custom types.

### Thread-Safe Python Object Handles: `Py<T>` vs. `Bound<'py, T>`

PyO3 offers two primary smart pointers for Python objects, each with a distinct
role in a concurrent application.[^14] Understanding their difference is
crucial for writing correct multithreaded code.

`Py<T>` **and** `PyObject`**:** This is the GIL-independent, owned handle to a
Python object. Its most important characteristic is that it implements the
`Send` and `Sync` traits.[^15] This makes it the designated vehicle for sharing
references to Python objects across thread boundaries. If a Rust struct needs
to hold a Python object, or if a Python object reference needs to be sent from
one thread to another (e.g., via a channel), it must be stored as a

`Py<T>` (or its common alias, `PyObject`, which is `Py<PyAny>`).[^14] While it
can be safely moved between threads, operating on the object it points to
almost always requires acquiring the GIL and obtaining a temporary

`Bound` handle.

`Bound<'py, T>`**:** This is the GIL-dependent, "active" handle. It is
conceptually equivalent to a tuple of `(Python<'py>, Py<T>)`, meaning it
bundles an object handle with the proof that the GIL is held.[^14] Because it
is bound to the

`'py` lifetime, it is *not* `Send` and cannot be moved out of a GIL-protected
scope. Its advantage is that it provides the most complete and efficient API
for interacting with the Python object, as the GIL is guaranteed to be held.[^4]

This distinction defines a clear and safe workflow for cross-thread object
manipulation:

1. **Acquire GIL:** In the source thread, acquire the GIL (e.g., within a
   `Python::with_gil` block).

2. **Create/Obtain Object:** Create a new Python object or receive one from
   Python code. At this point, it is represented by a `Bound<'py, T>`.

3. **Unbind:** Before sending the reference to another thread, call `.unbind()`
   on the `Bound` handle. This consumes the `Bound` and returns a `Py<T>`,
   stripping away the GIL lifetime and producing a `Send`-able handle.

4. **Send:** Move the `Py<T>` to the destination thread (e.g., via a channel or
   by storing it in a shared `Arc<Mutex<...>>`).

5. **Re-bind:** In the destination thread, acquire the GIL (again, with
   `Python::with_gil`) to get a new `Python<'py>` token. Call `.bind(py)` on
   the received `Py<T>` to create a new, temporary `Bound<'py, T>` handle.

6. **Operate:** Use this temporary `Bound` handle to safely call methods on the
   Python object.

This bind-unbind-rebind cycle ensures that all interactions with Python objects
happen with the GIL held, while allowing the references themselves to be safely
transported across thread boundaries where the GIL is not held.

### The `Send + Sync` Mandate for `#[pyclass]`

When a Rust struct is exposed to Python using the `#[pyclass]` attribute, PyO3
imposes a critical restriction: the struct must implement both the `Send` and
`Sync` marker traits.[^12] This is not an arbitrary limitation; it is a direct
and necessary consequence of Python's own threading model. From the Python
interpreter's perspective, any object can be passed to any thread, and multiple
threads can hold references to and call methods on the same object
simultaneously.[^12]

- `Send` **is required** because Python makes no guarantee about which thread
  will ultimately be responsible for dropping an object. The object must be
  safe to be moved to and deallocated on a different thread from where it was
  created.

- `Sync` **is required** because multiple Python threads could concurrently call
  methods on the object, meaning they would be accessing its underlying Rust
  data by reference from multiple threads at the same time.

PyO3 enforces this requirement at compile time. If a struct marked with
`#[pyclass]` does not satisfy these bounds, the code will fail to compile. This
is a powerful safety feature that prevents entire categories of data races that
are trivial to introduce accidentally in C extensions, where no such
compile-time check exists.[^3] For the rare case of a strictly single-threaded
application, this check can be bypassed with

`#[pyclass(unsendable)]`, but this is strongly discouraged as it trades
compile-time safety for the potential of runtime errors if threads are ever
introduced.[^12]

### Interior Mutability and Concurrency Control in `#[pyclass]`

Rust's strict aliasing rules (one mutable reference *or* multiple immutable
references) are at odds with Python's model of shared mutability. A method on a
Python class can mutate `self` even when multiple references to the object
exist. To bridge this gap, PyO3 does not allow `#[pymethods]` to take
`&mut self` in the traditional Rust sense.

Instead, PyO3 employs an interior mutability pattern for all `#[pyclass]`
objects, analogous to `std::cell::RefCell`.[^17] To access the underlying Rust
data, methods must use runtime borrow checking. From a method on a

`#[pyclass]`, one would call `self.borrow(py)` to get an immutable reference or
`self.borrow_mut(py)` to get a mutable reference to the inner data.[^15] These
calls perform a runtime check to ensure Rust's aliasing rules are not violated.

In a multithreaded context, this runtime borrow check becomes a concurrency
control mechanism. If two Python threads simultaneously call methods that both
attempt to get a mutable borrow on the same Rust object, the second thread's
call to `borrow_mut()` will panic or return a `PyBorrowMutError`.[^3] This
effectively serializes mutable access and prevents data races on the

`#[pyclass]`'s internal state. While this provides a baseline level of safety,
relying on it can lead to unexpected runtime exceptions under contention. For
robust applications, it is often better to use more explicit concurrency
controls.

### Advanced Thread-Safety Patterns for `#[pyclass]` Data

To satisfy the `Send + Sync` requirement for complex types and to build more
robust concurrent applications, developers should move beyond the default
interior mutability and adopt explicit thread-safety patterns.

- Pattern 1: Atomics and #[pyclass(frozen)]

  For simple fields like counters, flags, or configuration settings, using
  Rust's atomic types (std::sync::atomic::{AtomicI32, AtomicBool, etc.}) is the
  most performant option.[^18] Atomic operations are lock-free and provide
  guaranteed thread-safe access. This pattern works best when the

  `#[pyclass]` is also marked as `#[pyclass(frozen)]`, which prevents
  attributes from being changed from Python, simplifying the reasoning about
  the object's state.[^13]

  ```rust
  use std::sync::atomic::{AtomicUsize, Ordering};
  use pyo3::prelude::*;

  #[pyclass(frozen)]
  struct SharedCounter {
      value: AtomicUsize,
  }

  #[pymethods]
  impl SharedCounter {
      #[new]
      fn new() -> Self {
          SharedCounter { value: AtomicUsize::new(0) }
      }

      fn increment(&self) {
          self.value.fetch_add(1, Ordering::Relaxed);
      }

      fn get(&self) -> usize {
          self.value.load(Ordering::Relaxed)
      }
  }

  ```

- Pattern 2: Mutexes

  For more complex data that cannot be represented by atomics or needs to be
  updated transactionally, the standard approach is to wrap the data in a
  std::sync::Mutex.[^18] Each method that needs to access the data must first
  lock the mutex. This ensures that only one thread can access the inner data
  at a time, preventing data races at the cost of potential blocking.[^13] When
  using this pattern, the deadlock rule from Section 1.4 is paramount: if a
  Python API call needs to be made while the lock is held, special care is
  required, potentially using

  `MutexExt::lock_py_attached`.

- Pattern 3: Manual unsafe Send/Sync Implementation

  In rare, advanced scenarios, a #[pyclass] might contain a type that is not
  Send or Sync (e.g., a raw pointer from a C library), but the developer can
  guarantee that access to it will be managed safely. In this case, it is
  possible to implement unsafe impl Send for MyClass {} and unsafe impl Sync
  for MyClass {}. This is an expert-level escape hatch that shifts the full
  responsibility for preventing data races onto the developer. It should only
  be used after a rigorous soundness review, as described in resources like the
  Rustonomicon.[^12]

<!-- markdownlint-disable MD013 MD033 MD056 -->

| Type                 | GIL-Bound?  | Send? | Sync? | Primary Use Case                                                              |
| -------------------- | ----------- | ----- | ----- | ----------------------------------------------------------------------------- |
| `Py<T>` / `PyObject` | No          | Yes   | Yes   | Storage and transport: storing in structs, sending between threads.           |
| `Bound<'py, T>`      | Yes (`'py`) | No    | No    | Operation: calling methods, accessing data. The "working" handle.             |
| `&Bound<'py, T>`     | Yes (`'py`) | Yes   | Yes   | Borrowing: passing as a non-owning function argument within a GIL-held scope. |

<!-- markdownlint-enable MD013 MD033 MD056 -->

## Practical Multithreading Patterns and Best Practices

With a firm grasp of GIL management and thread-safe object handling, it is
possible to construct powerful concurrent patterns. The choice of pattern
depends heavily on the nature of the task and the direction of data flow
between Python and Rust.

### The "Offload to Rust" Pattern: Maximizing CPU-Bound Performance

This is the canonical and most effective pattern for leveraging Rust's
performance to accelerate Python. It is ideal for CPU-bound computations like
numerical analysis, data processing, or simulations. The workflow is
straightforward 7:

1. A Python function calls a Rust function exposed via `#[pyfunction]`.

2. The arguments are converted from Python types (e.g., `PyList`) into native
   Rust types (e.g., `Vec<f64>`). This happens while the GIL is still held.

3. The Rust function calls `py.allow_threads()` to release the GIL.

4. Inside the `allow_threads` closure, the now GIL-free Rust code performs the
   heavy computation in parallel, typically using a library like Rayon.

5. Upon completion, the closure exits, `allow_threads` re-acquires the GIL, and
   the Rust result is converted back into a Python object to be returned to the
   caller.

An analysis of the canonical word-count example demonstrates this perfectly 7:

```rust
use pyo3::prelude::*;
use rayon::prelude::*;

fn count_words_in_line(line: &str, needle: &str) -> usize {
    line.split_whitespace().filter(|&word| word == needle).count()
}

#[pyfunction]
fn search_parallel(py: Python<'_>, contents: String, needle: String) -> usize {
    // Release the GIL and offload the parallel work to Rayon's thread pool.
    py.allow_threads(move |

| {
        contents
           .par_lines()
           .map(|line| count_words_in_line(line, &needle))
           .sum()
    })
}
```

This pattern effectively circumvents the GIL for the most intensive part of the
operation, enabling true multicore parallelism and delivering performance far
exceeding what is possible with Python's native `threading` module for
CPU-bound work.[^7] The main performance consideration is the cost of data
marshaling—the conversion between Python and Rust types at the function
boundary. For this pattern to be effective, the computational work done in Rust
should significantly outweigh this conversion overhead.

### The "Rust-Managed Threads" Pattern: Interacting with Python from Worker Threads

A more complex but sometimes necessary pattern involves Rust code spawning its
own threads which then need to communicate back with the Python interpreter.
This could be for tasks like reporting progress, logging, or manipulating
Python objects from a background worker.

The core rule for this pattern is that **any Rust-spawned thread must acquire
the GIL for itself** using `Python::with_gil` before it can safely interact
with any Python APIs.[^7] A

`Python<'py>` token cannot be shared or sent between threads.

This introduces a subtle but critical deadlock risk that is a common pitfall.
If a main thread holds the GIL while spawning worker threads (e.g., using a
Rayon thread pool) and then blocks waiting for those workers to complete, a
deadlock will occur if any of those workers attempt to acquire the GIL.[^7] The
main thread holds the GIL and is waiting for the workers, while the workers are
waiting for the GIL held by the main thread.

The solution, again, is `py.allow_threads`. The code that spawns the worker
threads and waits for their completion **must** be wrapped in an
`allow_threads` block. This releases the GIL from the main thread, allowing the
worker threads to acquire it as needed.[^7]

The following example demonstrates this pattern, where a Rayon thread pool
processes a list of `Py<UserID>` objects, with each worker thread acquiring the
GIL to access the object's data 7:

```rust
use pyo3::prelude::*;
use rayon::prelude::*;

#[pyclass]
struct UserID { id: i64 }

fn process_users_in_parallel(users: Vec<Py<UserID>>) -> PyResult<Vec<bool>> {
    Python::with_gil(|py| {
        // The spawning logic is wrapped in `allow_threads` to prevent deadlock.
        py.allow_threads(move |

| {
            let results: Vec<bool> = users
               .par_iter()
               .map(|user_obj| {
                    // Each worker thread must acquire the GIL for itself.
                    Python::with_gil(|inner_py| {
                        user_obj.borrow(inner_py).id > 5
                    })
                })
               .collect();
            results
        })
    })
}
```

This pattern is powerful but significantly more complex to reason about than
the "offload" pattern. It reintroduces the need to manage competition for a
shared lock (the GIL) and should be used judiciously. Whenever possible, it is
preferable to structure code to use the simpler "offload" pattern, which
minimizes GIL contention and keeps the Rust/Python interaction boundary
cleaner.[^21]

### Propagating Errors Across Threads: A Guide to `PyErr`

Robust concurrent programming requires a sound strategy for error handling. A
key design feature of PyO3 is that its error type, `PyErr`, is
`Send + Sync`.[^22] This was a deliberate change in past versions to facilitate
exactly these kinds of multithreaded use cases.

Because `PyErr` is thread-safe, an error can be created in one thread,
propagated across a channel or returned from a parallel iterator, and
ultimately handled or raised in another thread. The workflow is idiomatic Rust:

1. A worker thread (which may or may not hold the GIL) encounters an error. It
   creates a `PyErr` instance and returns it as the `Err` variant of a `Result`.

2. Because `PyErr` is `Send`, this `Result` can be safely sent back to the main
   thread that initiated the concurrent operation.

3. The main thread receives the `Result::Err(py_err)`.

4. If this main thread is in the context of a `#[pyfunction]`, it can simply
   return this `Result`. PyO3 will automatically catch the `Err` variant and
   raise the contained `PyErr` as a Python exception in the calling Python
   code.[^23]

A noteworthy feature is that `PyErr` supports lazy instantiation. When creating
an error with `PyErr::new::<PyTypeError, _>("message")`, the actual Python
`TypeError` object is not created immediately. Instead, a lightweight,
`Send + Sync` representation of the error is created.[^24] This allows errors
to be constructed in contexts where the GIL is not held (e.g., inside

`allow_threads`). The full Python exception object is only materialized later,
when it is needed—typically when `PyErr::restore` is called or when PyO3
prepares to raise the exception back to the interpreter.

## A Glimpse into the Future: Free-Threaded Python

The development of a "free-threaded" build of CPython (introduced
experimentally in Python 3.13 via PEP 703) promises a future without the Global
Interpreter Lock. PyO3 has been designed with this future in mind, and the
principles of safe concurrency learned in a GIL-enabled world translate
directly, making PyO3 extensions remarkably well-prepared for this shift.

### Understanding the Paradigm Shift: From GIL to Interpreter Attachment

In a free-threaded Python world, the core concurrency concept changes. The
question is no longer "Does my thread hold the GIL?" but rather "Is my thread
attached to the Python interpreter runtime?".[^3] Calling any C API function is
only legal if the thread has a valid thread state (

`PyThreadState`).

PyO3's existing API maps cleanly onto this new mental model 2:

- `Python::with_gil` conceptually becomes `Python::attach`, ensuring the current
  OS thread is registered with the interpreter.

- `py.allow_threads` becomes the mechanism to temporarily detach a thread from
  the runtime.

- The `Python<'py>` token's meaning evolves. It no longer signifies exclusive
  access to the interpreter but rather valid, concurrent access. It is still
  the required proof that the thread is attached and can safely call Python
  APIs.

Even without a GIL, the interpreter still has global synchronization points,
for instance, during garbage collection or when instrumenting code for
profiling.[^3] A long-running Rust task that does not detach from the
interpreter could block these global events and hang the entire application.
Therefore, the practice of using

`allow_threads` for long-running, non-Python work remains a critical best
practice.

### Preparing Your Code for a GIL-less World

To signal that an extension module is safe for use in the free-threaded build,
it must be explicitly marked with `#[pymodule(gil_used = false)]`.[^3] If a
module is not marked, a free-threaded Python interpreter will re-enable the GIL
for the duration of its import and usage, issuing a

`RuntimeWarning` to the user.

The foresight of PyO3's design becomes apparent here. The strict `Send + Sync`
requirement for `#[pyclass]` means that any correctly written concurrent PyO3
extension is already using explicit synchronization mechanisms (like `Mutex` or
`Atomic*`) rather than implicitly relying on the GIL for thread safety.[^3]
Such extensions are largely ready for the free-threaded world. In contrast,
many C extensions that function correctly only because the GIL serializes
access to their internal state will break when the GIL is removed.

However, one area requires increased vigilance: the default runtime
borrow-checking mechanism in `#[pyclass]`. With true parallelism, the
likelihood of two Python threads simultaneously calling methods that require a
mutable borrow (`borrow_mut()`) on the same object increases dramatically. This
will lead to more frequent runtime panics or `PyBorrowMutError`s.[^3] This
underscores the importance of moving beyond the default behaviour and using
explicit, robust concurrency controls like mutexes for any

`#[pyclass]` intended for use in a high-contention, free-threaded environment.

<!-- markdownlint-disable MD013 MD033 MD056 -->

| Strategy                     | Mechanism                                | Pros                                         | Cons                                                                  | Ideal Use Case                                                           |
| ---------------------------- | ---------------------------------------- | -------------------------------------------- | --------------------------------------------------------------------- | ------------------------------------------------------------------------ |
| Default Interior Mutability  | Runtime borrow-checking (PyRef/PyRefMut) | Automatic for simple types.                  | Raises runtime errors/panics under contention.                        | Prototyping; low-contention scenarios; single-threaded applications.     |
| Atomics + #[pyclass(frozen)] | std::sync::atomic types                  | Lock-free, high performance.                 | Limited to simple data types (integers, bools).                       | Simple counters, flags, or state fields in high-contention environments. |
| Mutex/Locks                  | std::sync::Mutex wrapping data           | Supports complex, arbitrary data structures. | Can introduce blocking and potential deadlocks if not used carefully. | Complex, multi-field state that needs to be updated transactionally.     |

<!-- markdownlint-enable MD013 MD033 MD056 -->

## Conclusion: A Summary of Rules for Robust Multithreaded Extensions

Developing multithreaded Python extensions in Rust with PyO3 is a powerful
technique for building high-performance, concurrent applications. By embracing
Rust's type system, PyO3 provides a level of safety and developer ergonomics
that is a significant leap forward from the manual, error-prone discipline of
the CPython C API. Adhering to the patterns and principles outlined in this
guide will enable developers to harness this power effectively and safely. The
most critical rules for success can be summarized as follows:

- **Rule 1: Trust the Type System.** Leverage PyO3's core abstractions. The
  compiler's enforcement of the `'py` lifetime, the `Send`/`Sync` bounds on
  `#[pyclass]`, and the `Ungil` trait on `allow_threads` are your primary
  defense against concurrency errors. Design your code to satisfy these
  constraints, rather than seeking ways to circumvent them.

- **Rule 2: Release the GIL Before You Block.** The most common cause of
  deadlocks is holding the GIL while attempting to acquire another lock or
  perform a long-running, blocking operation. Make it a reflexive habit to wrap
  any such operation in a `py.allow_threads(||...)` block.

- **Rule 3: Use the Right Handle for the Job.** Use `Py<T>` for storage and for
  transporting Python object references across thread boundaries. Use
  `Bound<'py, T>` for all active operations on an object within a GIL-held
  scope. Master the bind-unbind-rebind workflow.

- **Rule 4: Manage** `#[pyclass]` **Concurrency Explicitly.** Do not rely on the
  default runtime borrow-checking as your primary concurrency strategy in
  high-contention environments. For robust, production-grade extensions,
  explicitly manage the thread safety of your `#[pyclass]` data using
  `std::sync::Mutex` for complex state or `std::sync::atomic` types for simple
  fields.

- **Rule 5: Prefer the "Offload to Rust" Pattern.** The simplest, most
  performant, and easiest-to-reason-about pattern is to keep the boundary
  between Python and Rust clean. Marshal data into Rust, release the GIL,
  perform parallel computation in pure Rust, re-acquire the GIL, and return the
  result. Minimize callbacks from Rust worker threads into Python whenever
  possible.

- **Rule 6: Prepare for the Future.** Write your concurrent code as if the GIL
  does not exist. Use explicit synchronization primitives. This will not only
  make your code more robust in the current GIL-enabled world but will also
  ensure it is correct and performant in future free-threaded versions of
  Python. Once verified, mark your module with `#[pymodule(gil_used = false)]`.

## Works cited

[^1]: pyo3::marker - Rust, accessed on July 14, 2025,
<https://pyo3.rs/main/doc/pyo3/marker/>

[^2]: pyo3 - Rust - [Docs.rs](http://Docs.rs), accessed on July 14, 2025,
<https://docs.rs/pyo3/0.25.1/pyo3/>

[^3]: Supporting Free-Threaded Python - PyO3 user guide, accessed on July 14,
2025, <https://pyo3.rs/main/free-threading>

[^4]: pyo3 - Rust - [Docs.rs](http://Docs.rs), accessed on July 14, 2025,
<https://docs.rs/pyo3/latest/pyo3/>

[^5]: How to implement multi-thread programs using Python C API? - Stack
Overflow, accessed on July 14, 2025,
<https://stackoverflow.com/questions/78180254/how-to-implement-multi-thread-programs-using-python-c-api>

[^6]: Python in pyo3::marker - Rust - [Docs.rs](http://Docs.rs), accessed on
July 14, 2025, <https://docs.rs/pyo3/latest/pyo3/marker/struct.Python.html>

[^7]: Parallelism - PyO3 user guide, accessed on July 14, 2025,
<https://pyo3.rs/v0.25.1/parallelism.html>

<https://pyo3.rs/v0.2.7/parallelism>

[^9]: Parallelism - PyO3 user guide, accessed on July 14, 2025,
<https://pyo3.rs/v0.11.0/parallelism>

[^10]: FAQ and troubleshooting - PyO3 user guide, accessed on July 14, 2025,
<https://pyo3.rs/latest/faq.html>

<https://pyo3.rs/main/doc/pyo3/sync/struct.giloncecell>

[^12]: Thread safety - PyO3 user guide, accessed on July 14, 2025,
<https://pyo3.rs/v0.25.1/class/thread-safety>

[^13]: Thread safety - PyO3 user guide, accessed on July 14, 2025,
<https://pyo3.rs/v0.24.0/class/thread-safety>

[^14]: Python object types - PyO3 user guide, accessed on July 14, 2025,
<https://pyo3.rs/v0.24.2/types.html>

[^15]: PyObject in pyo3 - Rust, accessed on July 14, 2025,
<https://pyo3.rs/main/doc/pyo3/type.pyobject>

2025, <https://pyo3.rs/v0.20.1/types>

[^17]: GIL, mutability and object types - PyO3 user guide, accessed on July 14,
2025, <https://pyo3.rs/v0.20.3/types>

[^18]: Thread safety - PyO3 user guide, accessed on July 14, 2025,
<https://pyo3.rs/v0.23.3/class/thread-safety.html>

<https://pyo3.rs/v0.23.3/class>

Technologies, accessed on July 14, 2025,
<https://configr.medium.com/boost-python-performance-with-cython-numba-and-pyo3-486d59d8c2c6>

[^21]: Rust multi-thread and pyo3 real world problem. - Reddit, accessed on July
14, 2025,
<https://www.reddit.com/r/rust/comments/1jcsncv/rust_multithread_and_pyo3_real_world_problem/>

[^22]: Appendix A: Migration Guide - PyO3 user guide, accessed on July 14, 2025,
<https://pyo3.rs/v0.12.0/migration>

[^23]: Error handling – PyO3 user guide, accessed on July 14, 2025,
<https://pyo3.rs/main/function/error-handling.html>

[^24]: PyErr in pyo3::err - Rust, accessed on July 14, 2025,
<https://pyo3.rs/internal/doc/pyo3/err/struct.pyerr>
