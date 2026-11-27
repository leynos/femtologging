# Changelog

## [0.2.0-beta1] - 2026-11-28

_First pre-release of 0.2.0. Applications upgrading from 0.1 should follow the
[0.2.0 migration guide](docs/v0-2-0-migration-guide.md). Known issue: a second
thread's `flush()` or `close()` on a socket or HTTP handler raises
`RuntimeError` while another thread's `close()` is waiting._

### Changed

- **Breaking:** limit logger names to 255 UTF-8 bytes and 32 dot-separated
  segments, reject control characters, and cap the registry at 10,000 named
  loggers ([#544](https://github.com/leynos/femtologging/pull/544)).
- **Breaking:** replace per-logger worker threads with a fixed pool of one to
  four shared dispatch workers, so `threading.local` state in a handler may be
  shared between loggers
  ([#544](https://github.com/leynos/femtologging/pull/544)).
- **Breaking:** disable loggers omitted by a `disable_existing_loggers`
  configuration, so they reject records instead of still propagating to
  ancestors ([#547](https://github.com/leynos/femtologging/pull/547)).
- **Breaking:** honour the wrapped handler's `level` in `StdlibHandlerAdapter`
  ([#546](https://github.com/leynos/femtologging/pull/546)).
- **Breaking:** reject runtime handler and filter definitions that no logger
  references, and release definitions a change leaves unreferenced
  ([#548](https://github.com/leynos/femtologging/pull/548)).
- **Breaking:** build wheels and the sdist without the `test-util` feature,
  which removes the timed-rotation test hooks from them
  ([#552](https://github.com/leynos/femtologging/pull/552)).
- **Breaking:** remove every test-only Python and Rust hook, including the
  handler and filter address helpers, from builds without `test-util`
  ([#645](https://github.com/leynos/femtologging/pull/645)).
- **Breaking:** reject structured field names that belong to the record
  itself, such as `message` or `levelname`, in `extra` and `log_context()`
  ([#601](https://github.com/leynos/femtologging/pull/601)).
- **Breaking:** escape line breaks in the message in the default rendering, so
  a message cannot forge another record
  ([#641](https://github.com/leynos/femtologging/pull/641)).
- **Breaking:** reject invalid HTTP endpoints and headers when the handler is
  built, follow at most four redirects without credentials, and bound
  connecting by the write timeout
  ([#599](https://github.com/leynos/femtologging/pull/599)).
- **Breaking:** validate the version of every nested exception payload, and
  make `SchemaVersionError` `#[non_exhaustive]` with new `Nested`,
  `TreeTooDeep`, and `TreeTooLarge` variants
  ([#640](https://github.com/leynos/femtologging/pull/640)).
- **Breaking:** flush custom Rust handlers from `flush_handlers()` only when
  they implement `FemtoHandlerTrait::flush_until()`
  ([#531](https://github.com/leynos/femtologging/pull/531)).
- **Breaking:** return `PyResult` from the Rust `FemtoLogger::new()` and
  `Result` from `FemtoLogger::with_parent()`
  ([#544](https://github.com/leynos/femtologging/pull/544)).
- **Breaking:** declare `BasicConfig` as a slotted dataclass, so assigning an
  attribute other than its five fields raises `AttributeError`
  ([#428](https://github.com/leynos/femtologging/pull/428)).
- **Breaking:** apply `ConfigBuilder.build_and_init()`, `dictConfig()`, and
  `fileConfig()` all or nothing, leaving the previous configuration in place
  when a call raises ([#428](https://github.com/leynos/femtologging/pull/428)).
- **Breaking:** run Python handlers, including `StdlibHandlerAdapter`, inside a
  copy of the caller's `contextvars` context
  ([#425](https://github.com/leynos/femtologging/pull/425)).
- **Breaking:** bound `FemtoSocketHandler.close()` and
  `FemtoHTTPHandler.close()` by the handler's write timeout, giving each queued
  record one attempt and discarding, with a warning, any still queued at the
  deadline
  ([#484](https://github.com/leynos/femtologging/pull/484), [#549](https://github.com/leynos/femtologging/pull/549)).
- Bound `FemtoSocketHandler.flush()` by one write timeout instead of two
  ([#549](https://github.com/leynos/femtologging/pull/549)).
- **Breaking:** release the GIL while explicit `handle()`, `flush()`, and
  `close()` calls on file, rotating, and timed-rotating handlers wait, so those
  calls no longer keep other Python threads out, and bound their `close()` to
  one second ([#550](https://github.com/leynos/femtologging/pull/550)).
- **Breaking:** call a Python handler's `flush()` from `flush_handlers()`, on
  the dispatch worker, and return `False` when it raises or returns a falsy
  value other than `None`
  ([#555](https://github.com/leynos/femtologging/pull/555), [#592](https://github.com/leynos/femtologging/pull/592)).
- Bound the whole of `flush_handlers()` by one two-second deadline
  ([#531](https://github.com/leynos/femtologging/pull/531)).
- Count every drop since the previous report in drop warnings, and report
  held-back counts at flush, close, and destruction
  ([#613](https://github.com/leynos/femtologging/pull/613)).
- Bound `FemtoStreamHandler.flush()` by one flush timeout instead of two, and
  let `close()` return at that timeout instead of joining the worker thread
  ([#594](https://github.com/leynos/femtologging/pull/594)).
- **Breaking:** return `Result<(), HandlerError>` from the Rust
  `FemtoLogger::add_handler()` and reject Python-backed handlers that lack
  contextual dispatch with `HandlerError::MissingContextDispatch`
  ([#425](https://github.com/leynos/femtologging/pull/425)).
- Report HTTP transport failures as fixed categories that never include the
  URL, header values, or redirect targets
  ([#639](https://github.com/leynos/femtologging/pull/639)).
- Compile `%`-style formats once, when the formatter is built
  ([#636](https://github.com/leynos/femtologging/pull/636)).
- Decide Rust `log::log_enabled!` per target from femtologging's current levels
  ([#545](https://github.com/leynos/femtologging/pull/545)).
- Mark the package as `Development Status :: 4 - Beta` and publish its home
  page, repository, and documentation URLs
  ([#543](https://github.com/leynos/femtologging/pull/543)).

### Added

- Add `with_filters()` to handler builders, and handler `filters` in
  `dictConfig()`, for filters that also run on propagated records
  ([#426](https://github.com/leynos/femtologging/pull/426)).
- Implement `FormatterBuilder` with `%`-style format strings, usable directly or
  by name through `ConfigBuilder` and `dictConfig()`
  ([#426](https://github.com/leynos/femtologging/pull/426)).
- Add `FemtoLogger.disabled` and `get_disabled_drops()`
  ([#547](https://github.com/leynos/femtologging/pull/547)).
- Add the `max_exception_depth` and `max_exception_nodes` limits to
  `filter_frames()`, which now rejects cyclic payloads
  ([#553](https://github.com/leynos/femtologging/pull/553)).
- Publish an attested sdist and native wheels to PyPI and GitHub Releases
  ([#552](https://github.com/leynos/femtologging/pull/552)).
- Accept a keyword-only `extra` mapping of structured fields on every logging
  method except `exception()`
  ([#361](https://github.com/leynos/femtologging/pull/361), [#424](https://github.com/leynos/femtologging/pull/424)).
- Add `log_context()` for scoped fields that follow `asyncio` tasks
  ([#424](https://github.com/leynos/femtologging/pull/424), [#454](https://github.com/leynos/femtologging/pull/454)).
- Add `PythonCallbackFilterBuilder` for callable and standard-library-style
  filters, with `"()"` factory support in `dictConfig()`
  ([#366](https://github.com/leynos/femtologging/pull/366)).
- Add `FemtoTimedRotatingFileHandler` and `TimedRotatingFileHandlerBuilder`
  for time-based rotation, including `dictConfig()` and `fileConfig()` support
  ([#362](https://github.com/leynos/femtologging/pull/362)).
- Add `RuntimeConfigBuilder` and `LoggerMutationBuilder` to change handlers,
  filters, levels, and propagation on running loggers in one transactional step
  ([#363](https://github.com/leynos/femtologging/pull/363)).
- Add `wait_diagnostics()` to the file, rotating, and timed-rotating handlers,
  with matching `femtologging::internal` trace events under `tracing-compat`
  ([#550](https://github.com/leynos/femtologging/pull/550)).
- Add `get_flush_diagnostics()` for counters that explain `flush_handlers()`
  results ([#531](https://github.com/leynos/femtologging/pull/531)).
- Add `allow_factories=False` to `dictConfig()` to reject `"()"` filter
  factories in untrusted configuration
  ([#644](https://github.com/leynos/femtologging/pull/644)).
- Add the `get_context_dropped()` and `get_context_capture_failures()` logger
  counters
  ([#425](https://github.com/leynos/femtologging/pull/425), [#454](https://github.com/leynos/femtologging/pull/454)).
- Rewrite the users' guide as a path from a first message to production, and
  test every example in the README and guides
  ([#543](https://github.com/leynos/femtologging/pull/543)).

### Fixed

- Release the GIL while `flush_handlers()` and each built-in handler's `flush()`
  and `close()` wait on worker threads, which previously stalled, could lose
  queued records, and froze other Python threads
  ([#546](https://github.com/leynos/femtologging/pull/546), [#549](https://github.com/leynos/femtologging/pull/549)).
- Release the GIL while file handlers wait in `handle()`, and while stream
  handlers wait in `flush()` and `close()`
  ([#550](https://github.com/leynos/femtologging/pull/550), [#594](https://github.com/leynos/femtologging/pull/594)).
- Keep the GIL released while `flush_handlers()` flushes each handler, so a
  Python handler or formatter can run instead of deadlocking
  ([#555](https://github.com/leynos/femtologging/pull/555)).
- Warn, at a limited rate, when a logger fails to flush through the Rust `log`
  bridge, instead of discarding the failure
  ([#555](https://github.com/leynos/femtologging/pull/555)).
- Flush Python handlers registered through the Python wrapper, which
  `flush_handlers()` previously skipped
  ([#592](https://github.com/leynos/femtologging/pull/592)).
- Stop an HTTP handler from sending the HTTP client's own diagnostics to itself
  ([#626](https://github.com/leynos/femtologging/pull/626)).
- Stop `remove_handler()`, `clear_handlers()`, `clear_filters()`, a
  configuration change, or `reset_manager()` from deadlocking when a removed
  handler's or filter's finalizer logs
  ([#646](https://github.com/leynos/femtologging/pull/646)).

[0.2.0-beta1]: https://github.com/leynos/femtologging/releases/tag/v0.2.0-beta1
