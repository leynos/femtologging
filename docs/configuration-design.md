# Femtologging Configuration Design Document

## 1. Core Configuration Model: The Builder Pattern

The primary and recommended method for configuring `femtologging` in new
applications will be a fluent builder API. This pattern promotes clarity, type
safety (especially in Rust), and discoverability.

### 1.1. Rust Builder API Design

The Rust configuration will expose a `ConfigBuilder` struct, allowing for a
programmatic and type-safe setup of the logging system.

```rust
// In femtologging::config::ConfigBuilder
pub struct ConfigBuilder {
    // Internal state to hold configuration parts
    version: u8,
    disable_existing_loggers: bool,
    default_level: Option<Level>,
    formatters: BTreeMap<String, FormatterBuilder>,
    filters: BTreeMap<String, FilterBuilder>, // Future: FilterBuilder
    handlers: BTreeMap<String, Box<dyn HandlerBuilderTrait>>, // Boxed trait object for handlers
    loggers: BTreeMap<String, LoggerConfigBuilder>,
    root_logger: Option<LoggerConfigBuilder>,
}

impl ConfigBuilder {
    /// Creates a new, empty `ConfigBuilder`.
    pub fn new() -> Self { /* ... */ }

    /// Sets the configuration schema version. Currently, only 1 is supported.
    pub fn with_version(mut self, version: u8) -> Self { /* ... */ }

    /// Sets whether existing loggers should be disabled upon configuration.
    pub fn with_disable_existing_loggers(mut self, disable: bool) -> Self { /* ... */ }

    /// Sets the default log level for loggers that do not have an explicit level configured.
    pub fn with_default_level(mut self, level: Level) -> Self { /* ... */ }

    /// Adds a formatter configuration by its unique ID, replacing any existing entry.
    pub fn with_formatter(mut self, id: impl Into<String>, builder: FormatterBuilder) -> Self {
        /* ... */
    }

    /// Adds a filter configuration by its unique ID.
    // pub fn with_filter(mut self, id: impl Into<String>, builder: FilterBuilder) -> Self {
    //     /* ... */
    // } // Future

    /// Adds a handler configuration by its unique ID.
    /// Requires a boxed trait object for the specific handler builder.
    pub fn with_handler(
        mut self,
        id: impl Into<String>,
        builder: Box<dyn HandlerBuilderTrait>,
    ) -> Self {
        /* ... */
    }

    /// Adds a logger configuration by its name, replacing any existing entry.
    pub fn with_logger(
        mut self,
        name: impl Into<String>,
        builder: LoggerConfigBuilder,
    ) -> Self {
        /* ... */
    }

    /// Sets the configuration for the root logger. Calling this multiple times
    /// replaces the previous root logger.
    pub fn with_root_logger(
        mut self,
        builder: LoggerConfigBuilder,
    ) -> Self {
        /* ... */
    }

    /// Finalizes the configuration and initializes the global logging system.
    pub fn build_and_init(self) -> Result<(), ConfigError> { /* ... */ }
}

// In femtologging::config::LoggerConfigBuilder
pub struct LoggerConfigBuilder {
    level: Option<Level>,
    propagate: Option<bool>,
    filters: Vec<String>,
    handlers: Vec<String>,
}

impl LoggerConfigBuilder {
    /// Creates a new `LoggerConfigBuilder`.
    pub fn new() -> Self { /* ... */ }

    /// Sets the log level for this logger.
    pub fn with_level(mut self, level: Level) -> Self { /* ... */ }

    /// Sets whether messages should propagate to parent loggers.
    pub fn with_propagate(mut self, propagate: bool) -> Self { /* ... */ }

    /// Sets the filter identifiers, replacing any previously configured filters.
    pub fn with_filters<I, S>(mut self, filter_ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        /* ... */
    }

    /// Sets the handler identifiers, replacing any previously configured handlers.
    pub fn with_handlers<I, S>(mut self, handler_ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        /* ... */
    }
}

// In femtologging::config::FormatterBuilder
pub struct FormatterBuilder {
    format: Option<String>,
    datefmt: Option<String>,
    // style: Option<FormatterStyle>, // Future: For %, {, $ styles
}

impl FormatterBuilder {
    /// Creates a new `FormatterBuilder`.
    pub fn new() -> Self { /* ... */ }

    /// Sets the format string for the formatter.
    pub fn with_format(mut self, format_str: impl Into<String>) -> Self { /* ... */ }

    /// Sets the date format string for the formatter.
    pub fn with_datefmt(mut self, date_format_str: impl Into<String>) -> Self { /* ... */ }
}

// In femtologging::handlers::HandlerBuilderTrait (and specific builders)
pub trait HandlerBuilderTrait: Send + Sync {
    type Handler: FemtoHandlerTrait;

    fn build_inner(&self) -> Result<Self::Handler, HandlerBuildError>;

    fn build(&self) -> Result<Box<dyn FemtoHandlerTrait>, HandlerBuildError> {
        let handler = self.build_inner()?;
        Ok(Box::new(handler))
    }
}

// Builders capture any required context directly. The earlier
// `build_handler(&ConfigContext)` design has been dropped; shared state is
// injected through builder fields instead of a dedicated context object.

// Example: In femtologging::handlers::FileHandlerBuilder
pub struct FileHandlerBuilder {
    path: String,
    mode: Option<String>,
    encoding: Option<String>,
    level: Option<Level>,
    formatter_id: Option<String>,
    filters: Vec<String>,
    capacity: Option<usize>,
    flush_record_interval: Option<usize>, // records
}

impl FileHandlerBuilder {
    /// Creates a new `FileHandlerBuilder` for a given path.
    pub fn new(path: impl Into<String>) -> Self { /* ... */ }

    /// Sets the file opening mode (e.g., "a", "w").
    pub fn mode(mut self, mode: impl Into<String>) -> Self { /* ... */ }

    /// Sets the encoding for the file.
    pub fn encoding(mut self, encoding: impl Into<String>) -> Self { /* ... */ }

    /// Sets the log level for this handler.
    pub fn with_level(mut self, level: Level) -> Self { /* ... */ }

    /// Sets the ID of the formatter to be used by this handler.
    pub fn with_formatter(mut self, formatter_id: impl Into<String>) -> Self { /* ... */ }

    /// Sets the filter identifiers, replacing any previously configured filters.
    pub fn with_filters<I, S>(mut self, filter_ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        /* ... */
    }

    /// Sets the internal channel capacity for the handler.
    pub fn with_capacity(mut self, capacity: usize) -> Self { /* ... */ }

    /// Sets how often the worker thread flushes the file. Measured in
    /// records and must be greater than zero so periodic flushing always
    /// occurs.
    pub fn flush_record_interval(mut self, interval: usize) -> Self { /* ... */ }
}

impl HandlerBuilderTrait for FileHandlerBuilder { /* ... */ }


// Example: In femtologging::handlers::StreamHandlerBuilder
pub struct StreamHandlerBuilder {
    stream_target: String, // "stdout", "stderr", or "ext://sys.stdout", "ext://sys.stderr"
    level: Option<Level>,
    formatter_id: Option<String>,
    filters: Vec<String>,
    capacity: Option<usize>,
    flush_timeout_ms: Option<i64>, // milliseconds
}

impl StreamHandlerBuilder {
    /// Creates a new `StreamHandlerBuilder` writing to stdout.
    pub fn stdout() -> Self { /* ... */ }

    /// Creates a new `StreamHandlerBuilder` writing to stderr.
    pub fn stderr() -> Self { /* ... */ }

    /// Sets the target stream (e.g., "stdout", "stderr").
    pub fn stream_target(mut self, target: impl Into<String>) -> Self { /* ... */ }

    /// Sets the log level for this handler.
    pub fn with_level(mut self, level: Level) -> Self { /* ... */ }

    /// Sets the ID of the formatter to be used by this handler.
    pub fn with_formatter(mut self, formatter_id: impl Into<String>) -> Self { /* ... */ }

    /// Sets the filter identifiers, replacing any previously configured filters.
    pub fn with_filters<I, S>(mut self, filter_ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        /* ... */
    }

    /// Sets the internal channel capacity for the handler.
    pub fn with_capacity(mut self, capacity: usize) -> Self { /* ... */ }

    /// Sets the flush timeout in milliseconds. Must be greater than zero.
    pub fn with_flush_timeout_ms(mut self, timeout_ms: i64) -> Self { /* ... */ }
}

impl HandlerBuilderTrait for StreamHandlerBuilder { /* ... */ }

```

The file builder uses a `flush_record_interval` measured in records, while the
stream builder's `flush_timeout_ms` is a duration in milliseconds. These
semantics intentionally differ: file handlers flush after a set number of
records, whereas stream handlers flush after a period of inactivity. Their
dictionary representations mirror these names to avoid ambiguity.

### 1.2. Python Builder API Design (Congruent with Rust and Python Schemas)

The Python API will mirror the Rust builder's semantics, providing a familiar
and idiomatic Python interface. This will involve exposing builder classes and
methods via `PyO3` bindings. Type hints will be used for clarity.

```python
# In femtologging.config
from typing import List, Optional, Union
from .levels import Level  # Assuming an enum or similar for levels

class ConfigBuilder:
    def __init__(self) -> None: ...
    def with_version(self, version: int) -> "ConfigBuilder": ...
    def with_disable_existing_loggers(self, disable: bool) -> "ConfigBuilder": ...
    def with_default_level(self, level: Union[str, Level]) -> "ConfigBuilder": ...
    def with_formatter(self, id: str, builder: "FormatterBuilder") -> "ConfigBuilder": ...  # replaces existing formatter
    def with_filter(self, id: str, builder: "FilterBuilder") -> "ConfigBuilder": ... # Future
    def with_handler(self, id: str, builder: "HandlerBuilder") -> "ConfigBuilder": ... # Union of specific handler builders
    def with_logger(self, name: str, builder: "LoggerConfigBuilder") -> "ConfigBuilder": ...  # replaces existing logger
    def with_root_logger(self, builder: "LoggerConfigBuilder") -> "ConfigBuilder": ...  # replaces previous root logger
    def build_and_init(self) -> None: ...

class LoggerConfigBuilder:
    def __init__(self) -> None: ...
    def with_level(self, level: Union[str, Level]) -> "LoggerConfigBuilder": ...
    def with_propagate(self, propagate: bool) -> "LoggerConfigBuilder": ...
    def with_filters(self, filter_ids: List[str]) -> "LoggerConfigBuilder": ...  # replaces existing filters
    def with_handlers(self, handler_ids: List[str]) -> "LoggerConfigBuilder": ...  # replaces existing handlers

class FormatterBuilder:
    def __init__(self) -> None: ...
    def with_format(self, format_str: str) -> "FormatterBuilder": ...
    def with_datefmt(self, date_format_str: str) -> "FormatterBuilder": ...
    # def style(self, style: str) -> "FormatterBuilder": ... # Future

# In femtologging.handlers
class HandlerBuilder: # Abstract base class or conceptual union
    # Common methods
    def with_level(self, level: Union[str, Level]) -> "HandlerBuilder": ...
    def with_formatter(self, formatter_id: str) -> "HandlerBuilder": ...
    def with_filters(self, filter_ids: List[str]) -> "HandlerBuilder": ...  # replaces existing filters
    def with_capacity(self, capacity: int) -> "HandlerBuilder": ... # Common for queue-based handlers

class FileHandlerBuilder(HandlerBuilder):
    def __init__(self, path: str) -> None: ...
    def mode(self, mode: str) -> "FileHandlerBuilder": ...
    def encoding(self, encoding: str) -> "FileHandlerBuilder": ...
    def flush_record_interval(self, interval: int) -> "FileHandlerBuilder": ...

class StreamHandlerBuilder(HandlerBuilder):
    @classmethod
    def stdout(cls) -> "StreamHandlerBuilder": ...
    @classmethod
    def stderr(cls) -> "StreamHandlerBuilder": ...
    def stream_target(self, target: str) -> "StreamHandlerBuilder": ... # "stdout", "stderr", "ext://sys.stdout", "ext://sys.stderr"

# ... Other handler builders (RotatingFileHandlerBuilder, SocketHandlerBuilder etc.)
```

### 1.3. Implemented handler builders

The initial implementation provides `FileHandlerBuilder` and
`StreamHandlerBuilder` as thin wrappers over the existing handler types.
`FileHandlerBuilder` supports capacity, flush interval, and overflow policy,
while `StreamHandlerBuilder` configures the stream target and capacity. Both
builders expose `build()` methods returning ready‑to‑use handlers. Advanced
options such as file encoding or custom writers are deferred until the
corresponding handler features are ported from picologging.

## 2. Backwards Compatibility APIs

`femtologging` will provide functions in the Python package to ensure backwards
compatibility with existing codebases that use standard `logging` configuration
methods. These functions will internally leverage the new builder API.

### 2.1. `basicConfig`

The `femtologging.basicConfig(**kwargs)` function will be provided, offering
the same interface as `logging.basicConfig`.

- **Functionality:** It will configure the root logger, typically adding a
  `StreamHandler` to `stderr` or `stdout` (or a `FileHandler` if `filename` is
  provided) and setting its level.

- **Internal Translation:** `basicConfig` will parse its `kwargs` (e.g.,
  `level`, `format`, `filename`, `filemode`, `encoding`, `datefmt`, `force`).

  - It will instantiate a `ConfigBuilder`.

  - If `filename` is provided, a `FileHandlerBuilder` will be created with
    `path=filename`, `mode=filemode` (default 'a'), and `encoding`.

  - Otherwise, a `StreamHandlerBuilder` will be created, writing to `stderr`
    (default).

  - A `FormatterBuilder` will be created using `with_format` and
    `with_datefmt` (if provided). This formatter will be added to the
    `ConfigBuilder` with a default ID (e.g.,
    `"default_basic_config_formatter"`) and associated with the handler.

  - The handler will be added to the `ConfigBuilder` with a default ID (e.g.,
    `"default_basic_config_handler"`).

  - The root logger's level will be set using `level`, and the default handler
    will be attached.

  - The `force` parameter will control whether to clear existing handlers on the
    root logger before applying the new configuration.

  - Finally, `build_and_init()` will be called on the constructed
    `ConfigBuilder`.

### 2.2. `dictConfig`

`femtologging.dictConfig(config: dict)` will support dictionary-based
configuration, as specified by `logging.config.dictConfig`.

- **Functionality:** This will allow users to define their entire logging
  configuration (loggers, handlers, formatters, filters) using a Python
  dictionary, often loaded from JSON or YAML files.

- **Internal Translation and Challenges:**

  - The `dictConfig` function will parse the input dictionary and map its
    structure to calls on the `femtologging.ConfigBuilder`.

  - **Configuration Order:** The parsing will logically follow the established
    order:

    1. **Version Check:** Validate the `version` key (must be `1`).

    1. `disable_existing_loggers`: Directly map this boolean to
       `ConfigBuilder.with_disable_existing_loggers()`.

    1. **Formatters**: Iterate through the `formatters` dictionary. For each
       `id` and `formatter_config_dict`:

       - Resolve `class` (if specified, for custom formatters) or default to
         `femtologging.FormatterBuilder`.

       - Instantiate the appropriate `FormatterBuilder`.

    - Call `with_format` and `with_datefmt` on the builder.

    - Add the `FormatterBuilder` to the `ConfigBuilder` via
      `with_formatter()` using its `id`.

    1. **Filters**: (Future) Similar to formatters, resolve `class` and
       parameters, then add to `ConfigBuilder`.

    1. **Handlers**: Iterate through the `handlers` dictionary. For each `id`
       and `handler_config_dict`:

       - **Dynamic Class Resolution**: The `class` entry (e.g.,
         `"logging.StreamHandler"`, `"femtologging.handlers.FileHandler"`) will
         be critical. `femtologging` will maintain an internal Python-side
         registry (a dictionary mapping string class names to `femtologging`'s
         Python-exposed builder classes). This registry will be used to
         instantiate the correct builder (e.g., `StreamHandlerBuilder`,
         `FileHandlerBuilder`).

       - `args` **and** `kwargs` **Handling**: The `args` (list) and `kwargs`
         (dictionary) from the `dictConfig` entry will be directly passed to
         the builder's `__init__` and subsequent methods. This implies the
         `femtologging` builder methods must be designed to accept these
         parameters. For example, `FileHandlerBuilder("path", mode="a")` in
         Python. The Rust binding will then map these Python arguments to the
         appropriate Rust builder methods (`.path(...)`, `.mode(...)`).

       - Set `level`, `formatter` (by ID), and `filters` (by IDs) on the handler
         builder.

       - Set handler-specific parameters (e.g., `filename`, `maxBytes`,
         `backupCount` for `RotatingFileHandler`) by passing them as `kwargs`
         to the builder's constructor or dedicated methods.

       - Add the `HandlerBuilder` (or its concrete subclass instance) to the
         `ConfigBuilder` using its `id`.

    1. **Loggers**: Iterate through the `loggers` dictionary. For each `name`
       and `logger_config_dict`:

       - Instantiate `LoggerConfigBuilder`.

       - Set `level`, `propagate`, `filters`, `handlers` (all by IDs).

       - Add the `LoggerConfigBuilder` to the `ConfigBuilder` via
         `with_logger()` using the logger's `name`.

    1. **Root Logger**: Process the `root` dictionary if present, similar to
       named loggers, and set it via `ConfigBuilder.with_root_logger()`.

  - `incremental`: As with `picologging`, `femtologging` will **not** support
    the `incremental` option \[cite: 1.1, 2.5,
    uploaded:leynos/femtologging/femtologging-1f5b6d137cfb01ba5e55f41c583992a64998826/docs/core\_[features.md](http://features.md)\].
     If `incremental` is `True`, a `ValueError` will be raised.

  - **Error Handling:** Robust error handling will be crucial to provide clear
    and informative messages for invalid configurations, unknown class names,
    or malformed arguments.

### 2.3. `fileConfig`

`femtologging.fileConfig(fname: str, **kwargs)` will support INI-style
configuration files, as per `logging.config.fileConfig`.

- **Functionality:** This method reads configuration from a file in a format
  compatible with Python's `ConfigParser`.

- **Internal Translation:**

  - **Rust-backed INI Parsing:** The `fileConfig` function (in Python) will
    delegate the actual INI file parsing to a new Rust function exposed via
    PyO3. This Rust function will use an existing, robust Rust INI parsing
    crate (e.g., `ini` or `configparser`) to read the INI file into a
    structured representation (e.g., `HashMap<String, HashMap<String, String>>`
    representing sections and key-value pairs).

  - **Python-side Conversion to** `dictConfig` **Schema:** The Rust-parsed data
    will be returned to Python. The Python `fileConfig` function will then
    convert this INI-style data into a dictionary structure that strictly
    adheres to the `dictConfig` schema. This conversion involves:

    - Identifying `[loggers]`, `[handlers]`, `[formatters]` sections and their
      `keys` attributes.

    - For each component (logger, handler, formatter), extracting its specific
      configuration from sections like `[logger_<name>]`, `[handler_<name>]`,
      `[formatter_<name>]`.

    - **Parameter Evaluation:** Crucially, string values from INI (especially
      for `args` and `kwargs` entries in handler sections) will be treated as
      Python literal expressions. These strings will be safely evaluated using
      `ast.literal_eval` (or a similar secure method) to convert them into
      actual Python tuples, lists, numbers, or dictionaries, suitable for the
      `dictConfig` structure. This ensures compatibility with complex handler
      constructors that expect specific Python types.

    - The `defaults` dictionary passed to `fileConfig()` will be used to
      substitute `%(key)s` placeholders in the INI file.

  - **Delegation to** `dictConfig`**:** Finally, the fully formed
    `dictConfig`-compatible dictionary will be passed to
    `femtologging.dictConfig()`. This makes `fileConfig` a two-stage process:
    INI parsing (Rust) -> `dictConfig` dictionary conversion (Python) ->
    `dictConfig` processing (Python, calling Rust builders). This simplifies
    the overall implementation by centralizing the core configuration logic in
    `dictConfig` and its builder translation.

## 3. Runtime Reconfiguration

- **Dynamic Log Level Updates:** As outlined in the design document \[cite:
  uploaded:leynos/femtologging/femtologging-1f5b6d137cfb01ba5e55f41c583992a64985340c/docs/[rust-multithreaded-logging-framework-for-python-design.md](http://rust-multithreaded-logging-framework-for-python-design.md)\],
  Dynamic log-level changes for loggers will be a core feature, utilizing
  atomic operations in Rust for thread-safe updates. This will be exposed via
  methods on `FemtoLogger` instances (e.g., `logger.set_level()`).

- **Future Enhancements:** Dynamic changes to handlers, formatters, and filters
  (e.g., swapping out a file handler for a new one with a different path, or
  changing a formatter's string) will be considered for future versions (V1.1
  or V2) due to their complexity. This would require careful management of
  consumer threads and resource lifecycles in Rust, likely involving a
  `reload()` method on the `ConfigBuilder` or dedicated control plane for the
  logging system.

## 4. Integration with Rust Ecosystem

`femtologging` will integrate with the broader Rust logging ecosystem by
implementing the `log::Log` trait and providing a `tracing_subscriber::Layer`
\[cite:
uploaded:leynos/femtologging/femtologging-1f5b6d137cfb01ba5e55f41c583992a64985340c/docs/[rust-multithreaded-logging-framework-for-python-design.md](http://rust-multithreaded-logging-framework-for-python-design.md)\].
 This ensures that `femtologging` can serve as a high-performance backend for
applications already using these established facades, without requiring them to
switch their logging calls.

## 5. Implementation Notes

The initial implementation introduces `ConfigBuilder`, `LoggerConfigBuilder`,
`FormatterBuilder`, `FileHandlerBuilder`, and `StreamHandlerBuilder` with
fluent, chainable methods exposed to Python via `PyO3`. `build_and_init`
currently validates only the configuration version and does not yet wire the
builders into the runtime.
