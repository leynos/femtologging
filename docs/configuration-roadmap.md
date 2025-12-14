# Roadmap for Configuration Approach

The following tasks are derived from the overall `femtologging` roadmap and
expanded with specifics for the configuration design.

## Phase 1 – Core functionality & MVP (configuration-related tasks)

- [x] **Implement** `femtologging::config::ConfigBuilder` **in Rust.**

- [x] **Implement** `femtologging::config::LoggerConfigBuilder` **in Rust.**

- [x] **Implement** `femtologging::config::FormatterBuilder` **in Rust.**

- [x] **Implement** `femtologging::handlers::FileHandlerBuilder` **and**
  `femtologging::handlers::StreamHandlerBuilder` **in Rust, implementing**
  `HandlerBuilderTrait`**.**
  - `HandlerBuilderTrait` defines an associated `Handler` type and a
    `build_inner`/`build` pattern. Builders capture any environmental context
    via explicit fields; the previously proposed `ConfigContext` has been
    dropped.

- [x] **Enable multiple handler IDs to be attached to a single logger in the
  builder API.**

- [x] **Store handlers in an `Arc` so that several loggers can share one
  instance safely.**

- [x] **Introduce a `Manager` registry with dotted-name hierarchy support and a
  root logger configuration.**

- [x] **Expose a mirroring** `femtologging.ConfigBuilder` **in Python via**
  `PyO3` **bindings.**

- [x] **Expose mirroring Python builders via** `PyO3` **bindings.**
  - [x] `LoggerConfigBuilder`
  - [x] `FormatterBuilder`
  - [x] `FileHandlerBuilder`
  - [x] `StreamHandlerBuilder`

- [ ] Add compile‑time log level filtering via Cargo features.

- [x] Ensure all components satisfy `Send`/`Sync` requirements.

- [x] Establish a basic test suite covering unit and integration tests for the
  builder configuration system in both Rust and Python, including syrupy
  snapshot assertions.

## Phase 2 – Expanded handlers & core features (configuration-related tasks)

- [x] Support dynamic log level updates at runtime using atomic variables and
  expose `set_level()` on `FemtoLogger` in Python.

- [x] Implement the `log::Log` trait for compatibility with the `log` crate.

- [x] Implement `femtologging.basicConfig()` translating to the builder API
   (see [configuration design](./configuration-design.md#basicconfig) and
   [example](../examples/basic_config.py)).

- [x] **Implement** `femtologging.dictConfig()` **in Python:**

  - [x] **Implement logic to parse the** `dictConfig` **schema, resolving string
    class names to** `femtologging` **builders.**

  - [x] **Handle** `args` **and** `kwargs` **evaluation for handler
    constructors.**

  - [x] **Implement ordered processing of** `version`,
    `disable_existing_loggers`, **formatters, handlers, loggers, and root.**
    Filters are not supported yet and must raise `ValueError` if present.

  - [x] **Ensure proper error handling for invalid** `dictConfig` **structures
    or unsupported features like** `incremental=True`**.**

- [x] Define the `FemtoFilter` trait and implement common filter types (e.g.,
  `LevelFilter`, `NameFilter`), with builder API integration for filters.[^1]

- [x] Implement `SocketHandlerBuilder` exposing TCP, Unix domain socket, and TLS
  transports with configurable timeouts, frame limits, and backoff controls in
  both Rust and Python.

- [x] Expand test coverage and start benchmarking for the configuration system
  and `basicConfig`/`dictConfig` compatibility.

## Phase 3 – Advanced features & ecosystem integration (configuration-related tasks)

- [x] **Implement** `femtologging.fileConfig()` **in Python:**

  - [x] **Develop a Rust function (exposed via PyO3) to parse INI files using a
    suitable Rust INI parsing crate.**

  - [x] **Implement Python-side logic to convert the Rust-parsed INI data into
    a** `dictConfig`**-compatible dictionary, including safe evaluation of**
    `args` **and** `kwargs` **strings.**

  - [x] **Call** `femtologging.dictConfig()` **with the generated dictionary.**

- [ ] Improve structured logging in macros to make context propagation easier,
  considering how configuration might support this.

- [ ] Investigate and implement more sophisticated dynamic reconfiguration
  capabilities for handlers and filters at runtime via the builder API (as a
  V1.1 or V2 feature).

[^1]: Completed in PR [#198](https://github.com/leynos/femtologging/pull/198)
      on 6 September 2025.

- [ ] Provide a `tracing_subscriber::Layer` so `femtologging` handlers can
  process `tracing` spans and events.
