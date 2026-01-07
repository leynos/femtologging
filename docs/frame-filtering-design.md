# Frame Filtering API Design

This document describes the frame filtering API for `exc_info` and `stack_info`
payloads in femtologging.

## Overview

The frame filtering API allows callers and formatters to exclude unwanted stack
frames from exception and stack trace payloads. Common use cases include:

- Removing logging infrastructure frames that add noise to stack traces
- Limiting stack depth for readability
- Excluding specific modules or functions from output

## Filter Flow

The following diagram shows the flow of the Python `filter_frames()` function,
which applies each configured filter in sequence to produce a filtered payload:

```mermaid
flowchart TD
    A[Python payload dict] --> B{is_exception_payload?}
    B -->|yes<br/>has type_name and message| C[filter_exception_payload]
    B -->|no| D[filter_stack_payload]

    C --> E[extract_frames]
    D --> E

    E --> F[apply_filters]

    F --> G{exclude_filenames provided?}
    G -->|yes| H[exclude_by_filename]
    G -->|no| I[skip filename filter]

    H --> J
    I --> J[frames]

    J --> K{exclude_functions provided?}
    K -->|yes| L[exclude_by_function]
    K -->|no| M[skip function filter]

    L --> N
    M --> N[frames]

    N --> O{exclude_logging?}
    O -->|yes| P[exclude_logging_infrastructure]
    O -->|no| Q[skip logging filter]

    P --> R
    Q --> R[frames]

    R --> S{max_depth provided?}
    S -->|yes| T[limit_frames]
    S -->|no| U[skip depth limit]

    T --> V[filtered_frames]
    U --> V

    V --> W[build new payload dict]
    W --> X[return filtered payload]
```

*Figure 1: Filter flow for the `filter_frames()` Python function. Each filter
is applied conditionally based on the provided parameters, with exclusion
filters applied before depth limiting.*

## Design Decisions

### Payload-level filtering

The filtering operates on payload dictionaries rather than modifying records
in-place. This approach:

- Aligns with the current design where `capture_*` functions return full frames
- Allows different formatters and handlers to filter differently
- Maintains testability and composability
- Keeps worker threads GIL-free (filtering operates on Rust-owned data)

### Filter order

Exclusion filters are applied before depth limiting. This ensures that:

1. Unwanted frames are removed first
2. The depth limit applies to the remaining useful frames
3. Users get the expected number of relevant frames

## Logging Infrastructure Patterns

Default patterns for `exclude_logging=True`:

- `"femtologging"` - this library
- `"_femtologging_rs"` - Rust extension
- `"logging/__init__"` - standard library logging
- `"logging/config"` - logging configuration
- `"<frozen importlib"` - import machinery

## API Summary

### Rust API

Methods on `StackTracePayload`:

- `filter()` - filter frames using a predicate
- `limit()` - keep at most N frames
- `exclude_filenames()` - exclude by filename patterns
- `exclude_functions()` - exclude by function patterns
- `exclude_logging_infrastructure()` - exclude common logging frames

Methods on `ExceptionPayload`:

- `filter_frames()` - recursive filtering (handles cause, context, groups)
- `limit_frames()` - recursive depth limiting
- `exclude_filenames()` - recursive filename exclusion
- `exclude_functions()` - recursive function exclusion
- `exclude_logging_infrastructure()` - recursive infrastructure exclusion

Helper functions in `frame_filter` module:

- `filter_frames()` - predicate-based filtering
- `limit_frames()` - depth limiting
- `exclude_by_filename()` - filename pattern matching
- `exclude_by_function()` - function name pattern matching
- `exclude_logging_infrastructure()` - infrastructure frame removal

### Python API

```python
filter_frames(
    payload: dict,
    *,
    exclude_filenames: list[str] | None = None,
    exclude_functions: list[str] | None = None,
    max_depth: int | None = None,
    exclude_logging: bool = False,
) -> dict
```

Returns a new payload dictionary with frames filtered according to the
specified parameters.

```python
get_logging_infrastructure_patterns() -> list[str]
```

Returns the default patterns used to identify logging infrastructure frames.

## Usage Example

```python
from femtologging import filter_frames

class MyHandler:
    def handle_record(self, record: dict) -> None:
        if exc := record.get("exc_info"):
            filtered = filter_frames(
                exc,
                exclude_logging=True,
                max_depth=10,
            )
            # Use filtered payload for output
```
