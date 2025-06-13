# Logging Class Overview

Below is a condensed inheritance / composition view of the `logging` package’s
class hierarchy in CPython (main branch, June 2025).\
It focuses on the public classes defined in `logging/__init__.py` and
`logging/handlers.py` and shows how the principal handler families relate to
`Logger`, `Filterer`, and `Formatter`.

```mermaid
classDiagram
    direction LR

    class Filterer
    class Filter

    class Logger
    class RootLogger
    class LoggerAdapter
    class Manager
    class PlaceHolder
    class LogRecord

    class Handler
    class StreamHandler
    class FileHandler
    class WatchedFileHandler
    class BaseRotatingHandler
    class RotatingFileHandler
    class TimedRotatingFileHandler
    class BufferingHandler
    class MemoryHandler
    class SocketHandler
    class DatagramHandler
    class SysLogHandler
    class SMTPHandler
    class HTTPHandler
    class NTEventLogHandler
    class QueueHandler
    class QueueListener
    class NullHandler

    class Formatter
    class PercentStyle
    class StrFormatStyle
    class StringTemplateStyle
    class BufferingFormatter

    %%-------------------  Inheritance  -------------------
    Filterer <|-- Logger
    Filterer <|-- Handler

    Logger     <|-- RootLogger
    Handler    <|-- StreamHandler
    Handler    <|-- FileHandler
    FileHandler <|-- WatchedFileHandler
    FileHandler <|-- BaseRotatingHandler
    BaseRotatingHandler <|-- RotatingFileHandler
    BaseRotatingHandler <|-- TimedRotatingFileHandler
    Handler    <|-- BufferingHandler
    BufferingHandler <|-- MemoryHandler
    Handler    <|-- SocketHandler
    SocketHandler <|-- DatagramHandler
    DatagramHandler <|-- SysLogHandler
    Handler    <|-- SMTPHandler
    Handler    <|-- HTTPHandler
    Handler    <|-- NTEventLogHandler
    Handler    <|-- NullHandler
    Handler    <|-- QueueHandler

    %%-------------------  Composition / “uses”  ----------
    Logger --> Manager           : manages
    Logger --> Handler           : dispatches to
    Logger --> LogRecord         : creates
    LoggerAdapter --> Logger     : wraps
    QueueHandler --> QueueListener : listener
    Filter -- Filterer           : attached-to
    Formatter --> PercentStyle         : «uses»
    Formatter --> StrFormatStyle       : «uses»
    Formatter --> StringTemplateStyle  : «uses»
    BufferingFormatter --> Formatter   : composes
```

## Notes & coverage

- `Filterer` is a mix-in providing support for `Filter` objects; both `Logger`
  and every `Handler` subclass inherit from it to gain
  `addFilter`/`removeFilter`, as seen in the core module definition
  ([github.com](https://github.com/python/cpython/raw/main/Lib/logging/__init__.py?plain=1)).

- Core logging classes (`Logger`, `Manager`, `PlaceHolder`, `LogRecord`,
  `Formatter`, …) live in `logging/__init__.py`.

- Most specialised handlers live in `logging/handlers.py`. The file defines the
  rotating‐file family, network-oriented handlers, queue helpers, etc.; for
  instance `BaseRotatingHandler` and its subclasses
  ([github.com](https://github.com/python/cpython/raw/main/Lib/logging/handlers.py?plain=1)),
  and buffering-based handlers
  ([github.com](https://github.com/python/cpython/raw/main/Lib/logging/handlers.py?plain=1)).

- Only inheritance and the most important *has-a* relations are shown; run-time
  wiring (e.g. a `Logger` holding multiple `Handler` instances) is depicted at a
  high level.

- Platform-specific handlers (such as `NTEventLogHandler`) are included even
  though they are only available on Windows builds.

- `logging.config` is outside the scope of the request, so configurator classes
  (`DictConfigurator`, `BaseConfigurator`, …) are omitted.

Drop the diagram straight into any Markdown viewer with Mermaid support (or
GitHub/GitLab’s preview) to render it.
