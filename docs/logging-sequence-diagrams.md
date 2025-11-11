# Logging Sequence Diagrams

Below are five **Mermaid sequence diagrams** that trace the *happy-path*
control-flow inside CPython’s `logging` package (main branch, June 2025).\\

All call-stacks have been pared down to the routines and objects that perform
real work, using the current source code for reference
([`Lib/logging/__init__.py`](https://github.com/python/cpython/raw/main/Lib/logging/__init__.py)).

______________________________________________________________________

## 1 `getLogger(name)` – retrieve or create a logger

```mermaid
sequenceDiagram
    participant Client          as User code
    participant LoggingMod      as logging (module)
    participant Manager         as logging.Logger.manager
    participant Logger          as new/existing Logger

    Client  ->> LoggingMod: getLogger("my.app")
    LoggingMod  ->> Manager: getLogger("my.app")
    alt logger already exists
        Manager  --> LoggingMod: existing Logger
    else first request for name
        Manager  ->> Logger: __init__("my.app")
        Logger   -->> Manager: instance
        Manager  ->> Manager: loggerDict["my.app"] = Logger
        Manager  --> LoggingMod: Logger
    end
    LoggingMod  --> Client: Logger
```

## `Manager.get_logger()` – femtologging

```mermaid
sequenceDiagram
    participant PythonAPI as Python API
    participant Manager
    participant FemtoLogger

    PythonAPI->>Manager: get_logger(py, name)
    alt logger exists
        Manager-->>PythonAPI: return existing FemtoLogger
    else logger does not exist
        Manager->>Manager: determine parent (dotted-name logic)
        Manager->>FemtoLogger: FemtoLogger::with_parent(name, parent)
        Manager->>Manager: store logger in registry
        Manager-->>PythonAPI: return new FemtoLogger
    end
```

______________________________________________________________________

## 2 `basicConfig(...)` – one-shot root logger configuration

```mermaid
sequenceDiagram
    participant Client          as User code
    participant LoggingMod      as logging.basicConfig()
    participant RootLogger      as logging.root
    participant Handler         as StreamHandler | FileHandler
    participant Formatter       as logging.Formatter

    Client  ->> LoggingMod: basicConfig(level=INFO,…)
    LoggingMod  ->> RootLogger: check handlers[]
    alt handlers already present and force is False
        LoggingMod  --> Client: return (no-op)
    else do initial setup
        LoggingMod  ->> Handler: create()
        LoggingMod  ->> Formatter: create()
        Formatter   ->> Handler: setFormatter()
        LoggingMod  ->> RootLogger: addHandler(Handler)
        LoggingMod  ->> RootLogger: setLevel(INFO)
        LoggingMod  --> Client: return
    end
```

______________________________________________________________________

## 3 `logger.info()` when *effective level = WARNING*

```mermaid
sequenceDiagram
    participant Client as User code
    participant Logger as Logger(level=WARNING)

    Client  ->> Logger: info("hello")
    Logger  ->> Logger: isEnabledFor(INFO=20)
    note right of Logger: returns **False**
    Logger  --> Client: return (message dropped)
```

The `INFO` record never reaches `_log()` because the short-circuit guard fails.

______________________________________________________________________

## 4 `logger.warning()` when *effective level = WARNING*

```mermaid
sequenceDiagram
    participant Client   as User code
    participant Logger   as Logger(level=WARNING)
    participant LogRec   as LogRecord
    participant Handler  as e.g. StreamHandler

    Client  ->> Logger: warning("something odd")
    Logger  ->> Logger: isEnabledFor(WARNING=30) ✔
    Logger  ->> Logger: _log()
    Logger  ->> LogRec: makeRecord()
    Logger  ->> Handler: handle(record)
    Handler ->> Handler: emit(record)
    Handler --> Logger: done
    Logger  --> Client: return
```

The message propagates to every handler whose own level permits it; only one
generic handler is shown for brevity.

______________________________________________________________________

## 5 `shutdown()` – orderly shutdown at process exit

```mermaid
sequenceDiagram
    participant Client   as Application exit
    participant LoggingM as logging.shutdown()
    participant Handler  as Each handler (reverse order)

    Client  ->> LoggingM: shutdown()
    loop reversed(_handlerList)
        LoggingM ->> Handler: acquire()
        alt Handler.flushOnClose
            LoggingM ->> Handler: flush()
        end
        LoggingM ->> Handler: close()
        LoggingM ->> Handler: release()
    end
    LoggingM --> Client: return
```

`shutdown()` is automatically registered with `atexit`, so normal interpreter
termination flushes and closes all live handlers
([Lib/logging/\_\_init\_\_.py](https://github.com/python/cpython/raw/main/Lib/logging/__init__.py)).

______________________________________________________________________

## 6 `Logger.warning()` with Filter + Formatter

```mermaid
sequenceDiagram
    participant App           as User code
    participant Logger        as Logger
    participant LogFilter     as Logger-level Filter
    participant Handler       as Stream/File/etc. Handler
    participant HdlrFilter    as Handler-level Filter
    participant Formatter     as Formatter
    participant Output        as I/O destination

    %% entry
    App        ->> Logger: warning("msg", *args, **kw)
    Logger     ->> Logger: isEnabledFor(WARNING=30) ✔
    Logger     ->> Logger: _log()
    Logger     ->> Logger: makeRecord()
    Logger     ->> LogFilter: filter(record)
    alt filter returns False
        LogFilter  -->> Logger: False
        Logger     -->> App: return (dropped)
    else passes (possibly mutating)
        LogFilter  -->> Logger: record
        Logger     ->> Handler: handle(record)
        Handler    ->> HdlrFilter: filter(record)
        alt handler filter blocks
            HdlrFilter -->> Handler: False
            Handler  -->> Logger: return
        else allowed
            HdlrFilter -->> Handler: record
            Handler    ->> Handler: acquire() lock
            Handler    ->> Formatter: format(record)
            Formatter  -->> Handler: "text line"
            Handler    ->> Output: write("text line\\n")
            Handler    ->> Handler: release() lock
            Handler    -->> Logger: emitted
        end
        Logger     -->> App: return
    end
```

### Key code points for `Logger.warning()`

- `Logger.warning()` short-circuits on level then calls `_log()`.
- `Handler.handle()` applies its own filters, formats the record and calls
  `emit()` under a lock.

______________________________________________________________________

## 7 `logging.config.dictConfig()` – dictionary-driven configuration

```mermaid
sequenceDiagram
    participant App          as User code
    participant CfgMod       as logging.config
    participant DictConf     as DictConfigurator
    participant Formats      as _install_formatters()
    participant Filters      as _install_filters()
    participant Handlers     as _install_handlers()
    participant Loggers      as _install_loggers()
    participant Root         as root Logger

    App      ->> CfgMod: dictConfig(config_dict)
    CfgMod   ->> DictConf: __init__(config_dict)
    CfgMod   ->> DictConf: configure()
    DictConf ->> DictConf: validate & extract version, incremental, …
    DictConf ->> Formats: build Formatter objects
    Formats  -->> DictConf: dict of formatters
    DictConf ->> Filters: build Filter objects
    Filters  -->> DictConf: dict of filters
    DictConf ->> Handlers: build Handler objects\n(attach formatters/filters, set levels)
    Handlers -->> DictConf: dict of handlers
    DictConf ->> Loggers: configure named loggers (level, handlers, propagate)
    Loggers  -->> DictConf: configured
    DictConf ->> Root: configure root logger (level, handlers)
    DictConf ->> DictConf: _handle_existing_loggers()
    DictConf -->> CfgMod: return
    CfgMod   -->> App: logging configured
```

### Key code points for `dictConfig()`

- `dictConfig(config)` simply instantiates `DictConfigurator` and calls its
  `configure()` method.
- Inside `DictConfigurator.configure()` the helper routines
  `_install_formatters`, `_install_filters`, `_install_handlers`,
  `_install_loggers`, and `configure_root` are invoked in that order (see start
  of file for these helpers), before the clean-up call
  `_handle_existing_loggers`.

______________________________________________________________________

#### Reading the diagrams

- *Participants* are real objects or modules as they appear in the current
  source.

- `alt`/`else` blocks show mutually exclusive paths; `loop` indicates iteration.

- Only logically significant calls are shown—locks, internal helpers and error
  handling are omitted unless essential to the behaviour being described.
