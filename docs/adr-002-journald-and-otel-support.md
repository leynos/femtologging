# ADR 002: Introduce OpenTelemetry and journald handler support in femtologging

## Status

Accepted – plan recorded on 23 December 2025 to guide the integration of
OpenTelemetry and Journald logging support.

## Context and Problem Statement

Femtologging’s current implementation lacks direct support for two important
logging destinations:

- **OpenTelemetry** (OT) for exporting log records into distributed
  observability pipelines.

- **systemd Journald** for writing logs to the system journal on Linux.

As of now, `FemtoLogger` only forwards logs as formatted strings, with any
structured context either omitted or flattened[^1][^2]. This means there is no
built-in way to attach key–value data (e.g. span IDs, user identifiers) or
propagate trace context alongside log messages. The
absence of structured log record support and context propagation poses a design
challenge: both OpenTelemetry and Journald thrive on structured, contextual
data (e.g. OT log attributes, journald fields).

Multiple approaches have been proposed for each integration, leading to open
questions:

- **OpenTelemetry integration:** Should the implementation use a bespoke
  `FemtoOtelHandler` (directly using the OpenTelemetry SDK to send logs), or
  via a `tracing_subscriber::Layer` that bridges `femtologging` with the Rust
  `tracing` ecosystem (and thereby to OpenTelemetry)? These approaches differ
  in complexity and how they align with the architecture.

- **Journald integration:** Should the Journald handler communicate via the
  journald socket (using the native journal protocol) or link against
  `libsystemd` (through an FFI, e.g. using the `systemd` crate) to send
  entries? This decision affects portability, safety, and the ease of including
  structured fields.

- **Timing and prerequisites:** Can either handler be meaningfully implemented
  *before* femtologging supports rich structured records and context
  propagation? In other words, is there value in adding them now (with only
  message strings and basic metadata), or must the core logging data model
  first be enhanced?

These questions must be resolved in the context of femtologging’s design goals.
The architecture document emphasizes using safe Rust and avoiding unnecessary
`unsafe` FFI calls, as well as employing the producer–consumer model where each
handler runs in its own thread to avoid blocking producers. It also highlights
a future roadmap of ecosystem integration: compatibility with Rust’s `tracing`
was explicitly planned, and alignment with standard log schemas (like the
OpenTelemetry Log Data Model) is a stated goal. At the same time, the project
is focused on the most impactful features (“80/20” rule) and defers niche
capabilities unless clear demand is shown. OpenTelemetry and Journald support
are now emerging as clear needs for observability and deployment on Linux,
respectively.

In summary, an architectural decision is needed that balances immediate
usefulness with long-term design coherence: enabling OpenTelemetry and Journald
outputs in a way that fits femtologging's architecture, acknowledging current
limitations (no structured logging yet), and setting the stage for richer
integration once those limitations are lifted.

## Decision Outcome (Summary)

- **Add a `FemtoJournaldHandler`** (Linux-only) **that writes to the systemd
  journal via the native socket protocol**, rather than linking against
  `libsystemd`. This handler will run on its own consumer thread (preserving
  the non-blocking design) and transmit log entries as structured key–value
  pairs to journald when possible. In the short term, it will at least send the
  log message, level (mapped to a syslog/journald priority), and basic
  metadata. Using the journald socket avoids introducing new `unsafe` FFI
  dependencies, in line with the safety stance, and is sufficient to preserve
  structured data (the `tracing-journald` crate demonstrates a pure-Rust
  approach that retains structured fields[^3]). `libsystemd` will not be used
  unless future requirements (e.g. performance or specific journald features)
  prove the socket approach inadequate, deferring that consideration to a later
  date if needed.

- **Provide OpenTelemetry log integration via a `tracing_subscriber::Layer`**
  rather than an immediate bespoke handler. A `FemtoTracingLayer` will be
  developed that allows femtologging to consume or emit `tracing` events and
  spans, bridging the handlers with the broader Rust tracing ecosystem. This
  decision leverages the existing OpenTelemetry integration in the tracing
  ecosystem (through `tracing-opentelemetry`) as an interoperable solution. In
  practice, applications that use Rust's `tracing` (or instrument Python code
  via OpenTelemetry's Python SDK) can attach femtologging's layer alongside an
  OpenTelemetry subscriber, sending logs to both femtologging handlers and an
  OT collector. This approach avoids duplicating complex OT export logic and
  fits the roadmap of ecosystem integration to drive adoption. A custom
  `FemtoOtelHandler` (directly using the OpenTelemetry SDK) is **not** chosen
  at this time; it will be revisited once femtologging supports full structured
  records and trace context, since a bespoke handler would add significant
  complexity yet still be limited without those features.

- **Structured logging and context propagation are recognized as prerequisites
  for full-featured integration,** but all progress on these handlers will
  *not* be blocked until those capabilities land. Instead, a **phased
  integration** is planned:

- In the **short term**, implement the Journald handler and tracing layer with
  the current log record structure (message + basic metadata). They will
  function with unstructured messages initially – e.g. journald entries will
  contain the formatted log message and metadata like level and logger name,
  and the OpenTelemetry path will propagate log events without per-log
  key–value fields or trace IDs.

- In the **long term** (once femtologging’s macros and `FemtoLogRecord` support
  key–value fields and capturing trace identifiers), enhance both handlers to
  utilize this information. The Journald handler will map structured fields
  into journald journal fields (e.g. custom KEY=VALUE entries, or standard
  fields like `CODE_FILE`, `CODE_LINE` for source info). The OpenTelemetry
  integration will be able to include log attributes and span context
  (trace-id, span-id) with each record, aligning with the OpenTelemetry Log
  Data Model. Both handlers are being designed with this evolution in mind, so
  they can be upgraded when the core library’s structured logging improvements
  (scheduled as part of Phase 3) are in place.

- **Feature gating and platform considerations:** Both integrations will be
  introduced behind optional Cargo features to avoid impacting users who don’t
  need them. For example, a `"journald"` feature will enable the Journald
  handler (only available on Unix targets), and a `"tracing"` or
  `"otel-integration"` feature will enable the tracing subscriber layer and any
  OpenTelemetry dependencies. This keeps the default build lean and free of
  platform-specific code or heavy telemetry libraries unless explicitly
  requested. The tracing layer feature is anticipated to be enabled by default
  (similar to how `log` compatibility is enabled by default) to encourage
  ecosystem uptake, whereas the Journald feature may remain opt-in or
  auto-enabled on Linux builds. In all cases, if the features are disabled, the
  new handlers add zero overhead.

- **Consistency with architectural principles:** The chosen solutions maintain
  femtologging’s core principles. Each new handler will follow the
  producer–consumer model (dedicated thread per handler) so that logging to OT
  or Journald will not stall application threads. By using the journald socket
  and Rust's tracing APIs, the implementation remains within safe, idiomatic
  Rust – avoiding the need for any `unsafe` foreign function interface (FFI)
  calls to C libraries unless absolutely necessary. This ensures compile-time
  safety and portability are upheld (the code will compile and run on platforms
  without systemd by simply not enabling the journald feature).

## Goals and Non-Goals

**Goals:**

- **Systemd Journald Integration:** Enable applications using femtologging to
  log directly to the local systemd journal on Linux, with minimal
  configuration. Logs sent to journald should include appropriate severity
  levels and, in future, structured metadata fields for optimal integration
  with `journalctl` and monitoring tools.

- **OpenTelemetry Compatibility:** Provide a pathway for femtologging logs to
  enter OpenTelemetry pipelines. This includes allowing femtologging to accept
  logs from code using the `tracing` crate (bridging `tracing` events into
  femtologging’s handlers) and, eventually, exporting femtologging’s own logs
  to an OTLP collector with full context. The design should align with
  OpenTelemetry’s log data model and not preclude correlating logs with traces
  (when trace context is available).

- **Preserve Performance and Non-Blocking Architecture:** The addition of these
  handlers should not compromise the performance of the logging hot path.
  Application threads logging to a Journald or OT handler should remain as fast
  as logging to a file or console, thanks to the asynchronous design. The
  consumer threads for these handlers must efficiently handle I/O (socket
  writes or OT SDK calls) and use batching or buffering if appropriate, to
  avoid undue overhead.

- **Optional and Configurable:** The solution should be flexible – projects
  that need these integrations can enable them, while others can exclude them
  to avoid extra dependencies or overhead. Configuration of these handlers
  (e.g. journal specifics or OT endpoint/credentials) should be exposed via the
  builder API and Python bindings, consistent with how other handlers are
  configured.

- **Align with Future Structured Logging:** Design the integration such that
  when femtologging introduces richer structured logging (key–value pairs in
  macros, context propagation), the handlers can seamlessly incorporate that
  data. For instance, plan how a trace ID or user-defined key in a log record
  would map to a journald field or an OpenTelemetry log attribute, even if that
  mapping will not be implemented until later. This foresight will prevent
  throw-away work and ensure continuity from unstructured to structured logging
  support.

**Non-Goals:**

- **Implementing Full Distributed Tracing:** This ADR does *not* introduce
  capturing or emitting distributed trace spans itself – it is focused on logs.
  While trace *context* for correlating logs is considered, setting up tracing
  spans or metrics in an application is outside the logging framework's scope
  (those would be handled by the application or other libraries). The concern
  is only to carry and output trace identifiers in logs when available, not to
  manage traces.

- **Supporting Every Platform's Native Logger:** Windows Event Log or generic
  syslog via UDP are not addressed in this decision. The Journald handler is
  inherently Linux/systemd-specific. Other platform-specific logging
  integrations (e.g. Windows `EventLogHandler` or UNIX syslog protocol) are out
  of scope here and can be decided separately if needed. The focus is on
  Journald due to its prevalence in modern Linux deployments and clear user
  demand, whereas others will follow only if a strong need arises (respecting
  the 80/20 focus).

- **Immediate Structured Logging Overhaul:** The full structured logging
  feature will not be designed in this ADR (that is already planned in the
  roadmap). It is acknowledged that the current handler implementations will be
  somewhat limited until that feature is delivered. This ADR's migration plan
  will defer certain capabilities (like including arbitrary key–value data or
  span IDs in outputs) to that future work, rather than try to solve it all
  now.

- **Mandating New Dependencies Unconditionally:** Any solution that forces all
  users to depend on heavy external crates or system libraries will be avoided.
  For example, linking against `libsystemd` is not included in the plan, nor is
  making OpenTelemetry a required dependency for femtologging core. Any new
  dependencies introduced for these features will be optional and isolated
  behind feature flags, ensuring that users can opt out completely.

## Migration Plan

The integration will proceed in phases to incrementally deliver functionality
while accommodating the project’s ongoing development of structured logging.
Each phase is designed to provide a useful increment in capability, with later
phases enhancing or adjusting earlier work once the core library supports more
features.

### Phase 0 – Foundation and Feature Gating

- **Define Cargo Features:** Introduce two new Cargo feature flags, e.g.
  `"journald"` and `"tracing-bridge"` (name tentative). The Journald handler
  code will be compiled only when the `"journald"` feature is enabled (and on
  target_os = "linux"), and the tracing subscriber layer will be included with
  the `"tracing-bridge"` feature. For Python packaging, optional extras may be
  added or these may simply be included in the default build on supported
  platforms – this detail will be decided based on whether they should be
  enabled by default. Initially, `"tracing-bridge"` might be enabled by default
  (since it has no effect unless used, but aids integration), and `"journald"`
  left off by default to avoid issues on non-Linux platforms.

- **Scaffolding:** Set up stub classes and configuration hooks for
  `FemtoJournaldHandler` and the tracing layer. This includes:

- Creating a `FemtoJournaldHandler` struct (implementing `FemtoHandlerTrait`)
  with minimal fields (e.g. socket path or handle) and a placeholder
  `handle(record)` implementation that will later be filled in.

- Adding a method in the builder API (Rust and Python) to allow users to add a
  Journald handler to a logger (e.g. `LoggerBuilder.journald(level, ...)`
  analogous to file or socket handlers).

- Preparing a `tracing_subscriber::Layer` implementation (or a function to
  create one) within the crate. This might be behind a module like
  `femtologging::tracing` enabled only with feature. The layer will implement
  the `on_event` and `on_span` callbacks to forward events to femtologging. At
  this phase it can be a stub that simply prints or counts events, just to get
  the plumbing in place.

- Ensure that adding these stubs does not break existing tests; guard any new
  code paths until fully implemented.

- **Documentation:** Document the existence of the new features (in README or
  docs) as “experimental integrations” so early adopters can try them
  knowingly. Clearly mark that structured data support is limited until a
  future phase.

### Phase 1 – Basic Journald Handler Implementation

*Goal:* Enable logging to systemd journal with unstructured messages and basic
metadata, using a safe pure-Rust approach.

- **Socket Communication:** Implement the `FemtoJournaldHandler.handle()`
  method to send log records to the journal via the UNIX socket. The
  implementation will likely use the *datagram* socket provided by journald
  (`/run/systemd/journal/socket`) to send structured records. The handler's
  thread will open this socket once and reuse it for all log writes. Each log
  record will be converted into the journald native format:

- Map femtologging’s log level to a numeric Priority (e.g. Info→6, Error→3,
  etc., following syslog/journal conventions).

- Construct the message payload. At minimum, include `PRIORITY=<N>` and
  `MESSAGE=<formatted log message>`. The implementation also includes
  `LOGGER=<logger name>` and perhaps `THREAD_NAME`/`THREAD_ID` if available
  from metadata. Source file/line could be included as `CODE_FILE` and
  `CODE_LINE` (journald recognizes these fields).

- Write the payload as newline-separated `KEY=value` lines terminated by an
  extra newline (which is the journald framing for datagrams). This can be done
  by formatting into a byte buffer and calling `sendto()` via Rust’s std
  `UdpSocket` or `std::os::unix::net::UnixDatagram`.

- Handle errors gracefully: if the socket write fails (e.g. journald not
  running or buffer full), the handler should emit a one-time warning (perhaps
  via stderr or an internal metric) and then drop subsequent messages or
  back off. It must **not** block the producing threads – any such error
  handling stays within the consumer thread.

- **No libsystemd dependency:** Confirm that the above implementation works for
  typical cases. Integration testing on a Linux environment with journald will
  be relied upon: logs emitted via `FemtoJournaldHandler` should appear in
  `journalctl` with correct priority and message content. The pure socket
  method should cover this (as evidence, other Rust libraries successfully log
  to journald without FFI[^3]).

- **Testing:** Write unit tests for the mapping logic (e.g. level to priority).
  Integration tests can be tricky (would require running on Linux with
  journald); a conditional test might be employed that writes to
  `/run/systemd/journal/socket` and then reads back via the journal API, or
  simply verifies that no errors occur. At minimum, manual testing in a Linux
  virtual machine (VM) or container will verify end-to-end behaviour. Any
  manual steps will be documented.

- **Documentation and Examples:** Update user-facing docs to show how to use
  the Journald handler (e.g. in Python:
  `femtologging.basicConfig(handlers=[femtologging.JournaldHandler()])`). Also
  mention that on non-Linux platforms it’s a no-op or unavailable. If feasible,
  detect at runtime if journald is not present and log a warning when
  attempting to use it.

- **Limitations Acknowledged:** In documentation, clearly state that at this
  stage the Journald handler will only log the message and fixed metadata.
  Structured key–value data passed to `femtologging.log()` is not yet forwarded
  as distinct journal fields (it will be supported in a future phase).
  Likewise, there’s no automatic trace correlation yet. This sets user
  expectations appropriately.

### Phase 2 – Tracing Layer for OpenTelemetry (and More)

*Goal:* Allow femtologging to participate in the `tracing` ecosystem, enabling
OpenTelemetry export via existing `tracing` layers, and also capturing
`tracing` events in femtologging handlers.

- **Implement `FemtoTracingLayer`:** Using the scaffold from Phase 0, flesh out
  a `tracing_subscriber::Layer` implementation. This layer will be capable of
  intercepting `tracing::Event`s and `tracing::Span`s. For this purpose, the
  layer will focus on events (which correspond to log records). Key steps:

- In `on_event`, format or convert the `tracing::Event` into a
  `FemtoLogRecord`. The `tracing` metadata can be leveraged: the event's
  message, level, target, file/line, and any key–value fields (via the `fields`
  in the event). Since femtologging's current API for logging from Rust is
  through the `log::Log` trait (already implemented[^4][^5]), one approach is
  to call `FemtoLogAdapter.log()` internally. However, a more direct
  construction of `FemtoLogRecord` might be done to include structured fields:
  the implementation can iterate over the event's fields and insert them into
  the `FemtoLogRecord.metadata.key_values` map (as strings) for future use.

- Dispatch the constructed `FemtoLogRecord` to the appropriate femtologging
  logger/handlers. The event's target might be used or the user might be
  explicitly required to specify which logger to use (perhaps a global logger
  or mapping by target name). A simple strategy is to send all tracing events
  to the root logger or a special "tracing" logger in femtologging.

- Ensure thread-safety and performance: this callback is invoked in the context
  of the tracing event (possibly on an application thread). Care must be taken
  to minimize overhead. The layer should quickly enqueue the record to
  femtologging's channel (still respecting the design that the heavy lifting is
  done on consumer threads). This effectively means the tracing layer will act
  as a producer into the femtologging system.

- In `on_span` callbacks, spans may simply be ignored or used for contextual
  info. (Span metadata like trace IDs might be attached to log records, but
  since femtologging doesn't yet propagate context, a full span handling can be
  minimal to start.)

- **OpenTelemetry Export via Tracing:** Once the above layer is in place, users
  can create a `tracing_subscriber::Registry` and add both the
  `FemtoTracingLayer` and an OpenTelemetry layer (from the
  `tracing-opentelemetry` crate) to it. This means any log event instrumented
  via `tracing` macros will simultaneously go to femtologging and to an
  OpenTelemetry backend. Documentation or examples demonstrating this setup
  will be provided. For instance, if an application uses
  `tracing::info!(key=value, "message")`, the layer will route it into
  femtologging (so it can be handled by any configured femtologging handlers
  like file or journald), and the OpenTelemetry layer will convert it into an
  OT span event or log record for export. This achieves OT integration without
  femtologging itself having to implement the OT protocol.

- **Backpressure and Filtering:** Care is needed to avoid double logging or
  infinite loops – e.g. if femtologging internally uses `log::warn!` and the
  layer catches it. Documentation will state that the tracing layer is intended
  to capture events from the application, not logs emitted by femtologging
  itself (which typically uses its own internal logger or the standard log for
  warnings). A filter might be added in the layer to ignore events originating
  from femtologging's modules to avoid feedback loops.

- **Testing:** Create tests where a simple function is instrumented with
  `tracing` events, the layer is attached, and corresponding `FemtoLogRecord`s
  are verified to arrive in a femtologging handler. A simple handler (maybe a
  vector-collecting handler) can be used in tests to gather records. Also test
  interoperability: attach a `tracing_subscriber::fmt` layer (which logs to
  stdout) alongside the femtologging layer to ensure no conflict, and if
  possible attach a dummy OT layer (if the dev dependency for
  `tracing-opentelemetry` is available) to ensure it composes correctly.

- **Documentation:** Expand the user guide with a section “Using femtologging
  with tracing and OpenTelemetry”. Show how to enable the feature and
  initialize the tracing subscriber. Clarify that this is mostly useful for
  Rust code instrumentation. For Python-only users, note that this doesn’t
  automatically send Python logs to OT, but it is a building block for future
  features (and useful if the application mixes Python and Rust components and
  wants unified tracing).

*By the end of Phase 2, femtologging will support:*

- Logging to Journald (message-level).

- Receiving logs from `tracing` instrumentation and thereby allowing
  OpenTelemetry export (but still lacking native Python-to-OT log export).

These capabilities are functional but not yet fully leveraging structured data.
Phases 3 and 4 will address those gaps.

### Phase 3 – Core Enhancements for Structured Logging

This phase is largely outside the scope of this ADR’s implementation details,
as it involves improving femtologging’s core, but it is a *precondition* for
the final integration steps:

- **Structured Record Support:** Implement the planned improvements to capture
  key–value pairs in logging macros and store them in `FemtoLogRecord`. After
  this, a log call like `femtologging::info!("User logged in", user_id = 42)`
  in Rust (or the Python equivalent) will populate `record.metadata.key_values`
  with `{"user_id": "42"}` (as strings or possibly as `serde::Value`). These
  values should then be accessible to handlers and formatters.

- **Context Propagation:** Introduce a way to attach contextual data (like a
  trace ID or span ID) to log records. This might be via an explicit API or
  implicitly by reading from `tracing::Span` context when the
  `FemtoTracingLayer` is active. For example, the decision might be made that
  if a `tracing::Span` is current, the logging macros capture its trace ID into
  the record metadata. Alternatively, a Python integration might allow setting
  a "global context" that femtologging will include in each record. The exact
  mechanism will be designed in this phase, but the outcome should be that a
  `FemtoLogRecord` can carry a field like `trace_id`.

- **Formatter and API adjustments:** Update `FemtoFormatter` implementations to
  optionally output structured data. Possibly provide a `StructuredFormatter`
  that formats key–values as JSON or key=value text. Ensure that handlers can
  retrieve the raw record if they need to do custom output (the new handlers
  will use this to avoid reformatting to string when sending to structured
  sinks).

Phase 3 is largely an internal refactoring/improvement of femtologging. It lays
the groundwork such that in Phase 4 the OpenTelemetry and Journald handlers can
truly shine. It's worth noting that Phase 3 corresponds to the roadmap's
advanced features and is expected to land in an upcoming minor release of
femtologging (since it's needed for many reasons beyond just OT/Journald
integration).

### Phase 4 – Enhanced OpenTelemetry and Journald Integration (Structured)

*Goal:* Revisit the two handlers and upgrade them to fully utilize structured
logs and context, completing the integration.

- **Journald Handler – Structured Fields:** Modify `FemtoJournaldHandler` to
  include all available structured data from `FemtoLogRecord` in the journal
  entry:

- Each key–value pair in `record.metadata.key_values` can be sent as a separate
  journald field. Keys will need to be sanitized to conform to journald
  conventions (uppercase alphanumeric and underscores). For example,
  `user_id=42` might be sent as `USER_ID=42`. Custom fields should be prefixed
  or clearly delineated to avoid clashing with reserved journal fields.

- Include standard fields: if not already, add `CODE_FILE`, `CODE_LINE`, and
  `CODE_FUNCTION` from the record’s source information; add `TRACE_ID` and
  `SPAN_ID` if present in the context metadata (this assumes context
  propagation provided these).

- With possibly many fields, ensure the formatting to the socket is correct
  (each `KEY=value` as a separate line). The message size should also be
  considered – journald has a limit on message datagram size (currently 8MiB,
  which is unlikely to be hit with a few fields, but worth noting).

- Testing: Once implemented, verify that structured fields appear in the
  journal. One can use `journalctl -o json` to see the fields, or
  `journalctl -f` to see that the custom fields are present. A log with
  multiple key–values should be tested and the output verified.

- Performance: adding structured data is just string formatting, which on the
  consumer thread is acceptable. If a record has dozens of fields, the overhead
  increases, but that is an explicit trade-off when using structured logging.

- **Evaluating libsystemd Need:** At this stage, consider if using `libsystemd`
  (via the `systemd` crate or FFI) offers any advantage for journald logging.
  For instance, libsystemd's `sd_journal_send` can automatically capture the
  code location if called via a macro, and might do batching internally.
  However, if the Phase 4 implementation reliably sends all data and performs
  well, the conclusion may be that sticking with the socket approach is
  sufficient. This evaluation will be documented in case stakeholders inquire
  why one method was chosen. (The preference is to remain with the socket
  solution unless a clear performance or correctness issue emerges, in line
  with avoiding unnecessary `unsafe` code.)

- **OpenTelemetry Handler (if needed):** With structured logging in place, the
  decision will be made whether to implement a dedicated `FemtoOtelHandler`
  that exports log records via the OpenTelemetry Rust SDK. This could be
  valuable for pure-Python users who are not using `tracing` but still want to
  send logs to an OT collector. The handler would run in a background thread
  and batch/process log records into OpenTelemetry Protocol (OTLP) export
  calls:

- Use the OpenTelemetry crate's Logging API (if stable) or treat log records as
  span events on a dummy span. By Phase 4, the OpenTelemetry community may have
  solidified how logs are ingested (perhaps an official Rust log exporter
  exists). That would be leveraged instead of crafting a custom OTLP marshaling
  from scratch.

- If implementing, this handler would require configuration (endpoint,
  credentials, resource attributes for the service, etc.) likely via
  environment or builder. It would be made an optional component behind an
  `"otel-handler"` feature to avoid forcing OT dependencies.

- However, this will only be pursued if there is demand and if the tracing
  layer approach is insufficient. It is possible that the tracing integration
  already covers most needs (since an application can choose to use `tracing`
  for its logging and achieve the same result). User feedback will be gauged at
  this point. For this ADR, the decision is to **defer** a bespoke
  OpenTelemetry handler until structured logging is available; it is noted here
  as a potential Phase 4 task, not a committed plan.

- **Trace Context in Tracing Layer:** Update `FemtoTracingLayer` to propagate
  span context into femtologging records. After Phase 3, femtologging will have
  a means to accept a trace ID. The layer can be modified such that when it
  creates a `FemtoLogRecord` from a tracing `Event`, it attaches the current
  span's trace ID (if any) into the record's metadata (e.g. as
  `"trace_id": "<hex-id>"`). This way, if femtologging later sends the record
  to journald or to a file, that trace_id is present. In particular, if sending
  to OpenTelemetry via a direct handler, the trace_id can be included to
  correlate log with trace. Similarly, journald logs could include `TRACE_ID`
  field.

- **End-to-End OpenTelemetry Logging:** At the conclusion of Phase 4, an
  application that uses femtologging (in Python or Rust) and has instrumented
  traces can have all three signals unified:

- If using the tracing layer + OT subscriber, trace and log correlation is
  automatic (the OT subscriber will know the trace IDs for events).

- If using the direct OT handler, it will include trace_id fields in OTLP
  exports, achieving correlation on the backend.

This will be verified by integration testing in a demo: e.g., log an event
while a trace span is active and confirm in an OT backend (or in the exported
JSON) that the log record has the same trace_id as the span.

- **Documentation & Guides:** Finally, update documentation to reflect the
  mature state:

- Show examples of structured logging (with key–value) and how they appear in
  journald (`journalctl` examples).

- Provide a guide for sending logs to OpenTelemetry: either via tracing or the
  direct handler if implemented. Include guidance on when to use which approach.

- Clearly describe any limitations or configurations (for instance, performance
  considerations, how to size batching for OT export, etc.).

Throughout these phases, feature flags and possibly temporary environment
toggles will be used to introduce the features gradually. For example, the
OpenTelemetry direct export might be marked as experimental even after Phase 4,
to get feedback before committing to API stability.

## Known Risks and Limitations

- **Incomplete Utility Before Phase 3:** Until structured logging and context
  propagation are implemented, the OpenTelemetry and Journald integrations will
  not realize their full potential. Users leveraging them early will only get
  unstructured text logs. For Journald this is acceptable (it’s equivalent to
  how many apps use journald), but for OpenTelemetry it’s of limited use (logs
  will lack trace correlation). This is mitigated by communicating clearly that
  these features are incremental. The risk is some may find the initial
  offering underwhelming – this trade-off is accepted to deliver iterative
  value and gather feedback sooner.

- **Complexity and Maintenance:** Introducing a journald path and a tracing
  layer adds complexity to the codebase. The journald integration needs careful
  handling of OS-specific code and testing on Linux. The tracing layer involves
  interfacing with an external ecosystem (`tracing` crate) and must be kept
  compatible as those libraries evolve. This is addressed by feature-gating and
  by writing thorough tests. The maintenance burden is justified by the value
  added, but it is noted that any breaking changes in the `tracing` or
  `opentelemetry` crates may require the integration layer to be updated
  accordingly.

- **Performance Overhead:** Writing to journald is typically fast (local UDS
  write), but journald itself can introduce latency under heavy load.
  Similarly, the tracing layer means every `tracing::event` triggers additional
  work (creating a FemtoLogRecord and channel send). If an application emits a
  very high volume of tracing events, this could strain femtologging's queues
  or consumers. To mitigate, these code paths will be benchmarked. The design
  already offloads work to the consumer threads and uses lock-free channels,
  which aligns with performance goals. Channel sizes may need to be tuned or
  backpressure mechanisms (e.g. drop policy) provided specifically for these
  handlers if they become bottlenecks.

- **Ordering and Duplication of Logs:** When using the tracing bridge, there is
  a possibility of logs appearing twice or out-of-order if an application also
  uses another subscriber. For example, if someone uses
  `tracing_subscriber::fmt` (to log to stdout) and femtologging simultaneously,
  the same event will be processed by both. This is generally intended, but
  users should be aware to avoid double-logging inadvertently. Documentation
  will recommend using femtologging as the primary output and not also adding a
  redundant `fmt` layer, or vice versa, unless that is explicitly wanted.

- **Platform Constraints:** The Journald handler will only function on
  systemd-based Linux systems. On other OSes, it will be disabled at compile
  time. If someone enables the feature on a non-Linux target, a compile-time
  error or no-op stub will be produced. This is a conscious decision.
  Documentation will mention that on Windows or macOS, journald logging is not
  available (and suggest alternative approaches, like logging to file or using
  syslog if needed for Unix without systemd).

- **Future Evolution of Standards:** OpenTelemetry logging is still an evolving
  standard. There is a risk that the APIs or recommended integration patterns
  change. By leaning on the `tracing-opentelemetry` bridge (which is maintained
  by OT community), the implementation is insulated from some of that churn. If
  a direct OTLP exporter is implemented, it will be done using the official SDK
  to stay aligned with the standard. Preparation remains to adapt if the OTLP
  log data model or configuration conventions change.

- **Security Considerations:** Logging to external systems (especially
  OpenTelemetry) means potentially transmitting sensitive data (log contents,
  contextual keys) over the network. The design will ensure that enabling an OT
  handler is an explicit action. Users will be advised to consider what they
  log (no PII in trace context keys unless needed, etc.). For journald, logs
  are local but end up in a system store; again users should be mindful of what
  they log at high severity levels as those might be broadly visible in
  syslogs. These concerns are not unique to the system but worth noting.

## Architectural Rationale

The chosen approaches were guided by femtologging’s overarching architectural
principles and the practical need to deliver features incrementally:

- Using a dedicated thread per handler and non-blocking channels aligns with
  the **producer-consumer model** for all handlers. Both the Journald and OT
  handlers conform to this, ensuring high throughput is maintained and any
  slowness is isolated (e.g., if the journald daemon is slow or the OT exporter
  backs up, only the handler's thread is affected, not the logging callers).

- Avoiding `libsystemd` FFI is consistent with the commitment to **avoid unsafe
  code and heavy platform-specific dependencies** without strong justification.
  It was determined that writing to the journald socket is sufficient for now;
  this keeps the implementation in safe Rust and avoids potential deployment
  issues (e.g. dealing with system library versions or linking errors).

- Leveraging the **existing Rust ecosystem (tracing)** for OpenTelemetry
  integration is a strategic choice. It would have been an attractive “pure
  Rust” accomplishment to implement an OTLP exporter from scratch, but doing so
  duplicatively would violate the DRY principle and likely lag behind the
  official SDK's capabilities. By integrating with `tracing`, femtologging
  becomes accessible to a wider community and gains OpenTelemetry support
  almost for free via `tracing-opentelemetry`. This design also future-proofs
  femtologging: if new `tracing` layers emerge (say for other backends or new
  telemetry standards), femtologging handlers can tap into them with minimal
  changes.

- The phased approach addresses the tension between **short-term deliverables
  and long-term completeness**. It explicitly acknowledges where interim
  solutions are not perfect (e.g., no structured fields yet) and defers those
  aspects until the core library is ready. This ensures that half-baked
  structured logging logic is not shoehorned into the handlers themselves. When
  Phase 3 lands, the handlers will naturally evolve, rather than requiring a
  redesign.

- Finally, this decision aligns with the project’s goal to **match and exceed
  common logging functionality**. CPython’s logging module doesn’t natively
  support OpenTelemetry or journald, but these integrations are increasingly
  expected in modern cloud and Linux environments. By adding them (carefully),
  femtologging differentiates itself as a forward-looking logging framework
  that bridges traditional logging with contemporary observability practices –
  all while maintaining the performance and safety promises that motivated the
  project in the first place.

[^1]: <https://github.com/leynos/femtologging/blob/f9a83c1e7f1cb6da4803ff56e8b0ab0967f31085/rust_extension/src/logger.rs#L159-L166>
[^2]: <https://github.com/leynos/femtologging/blob/f9a83c1e7f1cb6da4803ff56e8b0ab0967f31085/rust_extension/src/logger.rs#L96-L104>
[^3]: <https://docs.rs/tracing-journald/latest/tracing_journald/#:~:text=Support%20for%20logging%20tracing%20events,to%20journald%2C%20preserving%20structured%20information>
[^4]: <https://github.com/leynos/femtologging/blob/f9a83c1e7f1cb6da4803ff56e8b0ab0967f31085/docs/roadmap.md#L146-L155>
[^5]: <https://github.com/leynos/femtologging/blob/f9a83c1e7f1cb6da4803ff56e8b0ab0967f31085/docs/roadmap.md#L156-L164>
