# Logger Hierarchy and Handler Relationships

This note outlines the work required to add flexible logger/handler wiring to
`femtologging`. The current prototype assumes each logger owns exactly one
handler and that handlers are not shared. To match the capabilities of CPython's
`logging`, the following features must be implemented:

- multiple handlers per logger
- multiple loggers targeting the same handler safely
- hierarchical logger configuration using dotted names with propagation

## Steps to Implement

1. **Introduce a Manager registry**

   - Maintain a global `Manager` struct storing all loggers by name.
   - Each logger records its parent based on dotted name segments.
   - The root logger is created on first use and acts as the top of the tree.

2. **Allow multiple handlers per logger**

   - Change `FemtoLogger` to hold a `Vec<Arc<dyn FemtoHandlerTrait>>`.
   - Expose `add_handler()` and `remove_handler()` APIs.
   - Update logging macros to dispatch each record to every configured handler.

3. **Support handler sharing**

   - Wrap handlers in `Arc` so they can be referenced by multiple loggers.
   - Ensure the internal MPSC sender is cloned for each logger.
   - Document that handlers must be `Send + Sync + 'static` and stored as
     `Arc<dyn FemtoHandlerTrait + Send + Sync + 'static>`.

4. **Implement propagation across the hierarchy**

   - Each logger gains a `propagate` flag (default `True`).
   - When a logger handles a record, it first passes it to its own handlers,
     then forwards it to the parent logger if `propagate` is enabled.
   - Effective log level is computed by walking up the hierarchy.

5. **Provide configuration helpers**

   - Extend the builder API to attach multiple handler IDs per logger.
   - Allow handlers defined once to be referenced from several loggers.
   - Implement a `get_logger(name)` function in Python that mirrors CPython's
     semantics and returns existing instances from the registry.

## Testing Considerations

- Unit tests should verify that records are emitted once per handler even when
  shared between loggers.
- Behavioural tests must cover propagation rules to ensure child loggers inherit
  levels and handlers from their parents unless explicitly overridden.
- Concurrency tests should create several loggers writing to the same file
  handler from multiple threads. They must assert that each log record is
  written exactly once with no data loss. Ordering between threads is not
  guaranteed, but records emitted by a single logger should appear in the order
  they were produced. Tests must also check for duplicate records when a handler
  is shared across threads.

## Architecture Diagrams

<!-- markdownlint-disable MD013 -->

```mermaid
erDiagram
    MANAGER ||--o{ LOGGER : manages
    LOGGER }o--o{ HANDLER : references
    LOGGER }o--|| LOGGER : parent
    HANDLER ||--o{ RECORD : receives
```

```mermaid
classDiagram
    class Manager {
        +loggers: HashMap<String, FemtoLogger>
        +get_logger(name: String): FemtoLogger
    }
    class FemtoLogger {
        +name: String
        +parent: FemtoLogger
        +handlers: Vec<Arc<FemtoHandlerTrait>>
        +propagate: bool
        +add_handler(handler: Arc<FemtoHandlerTrait>)
        +remove_handler(handler_id: HandlerId)
    }
    class FemtoHandlerTrait {
        <<trait>>
    }
    class FemtoStreamHandler {
        +sender: MpscSender<Record>
    }
    class FemtoFileHandler {
        +sender: MpscSender<Record>
    }
    Manager "1" -- "*" FemtoLogger : manages
    FemtoLogger "*" -- "*" FemtoHandlerTrait : references (Arc)
    FemtoStreamHandler ..|> FemtoHandlerTrait
    FemtoFileHandler ..|> FemtoHandlerTrait
    FemtoLogger "1" -- "0..1" FemtoLogger : parent
```

```mermaid
classDiagram
    class Frame {
        +u32 0
        +Debug
        +PartialEq
        +bincode::Encode
        +bincode::BorrowDecode
    }
    class Response {
        <<generic>>
        +Stream(Pin<Box<dyn Stream<Item=Result<T, E>>>>)
    }
    Frame <.. Response : used as generic
    class stream_response {
        +stream_response() Response<Frame, ()>
    }
    stream_response ..> Response : returns
    stream_response ..> Frame : yields
```

<!-- markdownlint-enable MD013 -->
