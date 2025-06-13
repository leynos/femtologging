# Picologging Core Features

This document summarizes the key features of the `picologging` library.
Items are ordered by priority for the Rust port.

## 1. Logging API Compatibility

- Drop‑in replacement for the standard `logging` module
- Functions include `getLogger`, `basicConfig`, and common level helpers
- Supports standard log levels and hierarchical names

## 2. Core Data Structures

- **Logger** manages levels and handlers
- **LogRecord** represents a log event
- **Handler** defines an output destination
- **Formatter** and **FormatStyle** shape messages
- **Filterer** performs basic record filtering
- **Manager** maintains the logger tree

## 3. Built‑in Handlers

- **StreamHandler** and **FileHandler** for basic output
- **RotatingFileHandler** and **TimedRotatingFileHandler** for rotation
- **WatchedFileHandler** for logrotate‑style changes
- **QueueHandler** and **QueueListener** for async logging
- **BufferingHandler** and **MemoryHandler** for in‑memory buffering
- **SocketHandler** for network logging

## 4. Configuration Helpers

- `basicConfig` convenience function
- `dictConfig` (without the `incremental` option)
- Formatting styles: percent, `str.format`, and `string.Template`

## 5. Limitations

- Custom log levels and log record factories are not supported
- `LogRecord` always captures process and thread IDs
- Some advanced features are omitted to boost speed

## Prioritization for Rust Port

1. **Logging API Compatibility** – essential drop‑in behavior
1. **Core Data Structures** – backbone of the system
1. **StreamHandler** and **FileHandler** – basic output paths
1. **Formatter** and styles – human‑readable logs
1. **Rotating** and **TimedRotating** handlers – common operations
1. **Queue** and **Buffering** handlers – high‑throughput support
1. **SocketHandler** and **WatchedFileHandler** – lower priority
