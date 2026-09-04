# v0.2.0 migration guide

This document describes breaking changes introduced in v0.2.0 and the steps
required to update calling code.

______________________________________________________________________

## `BasicConfig` is now a slotted dataclass

`BasicConfig` (exported from `femtologging`) is now declared with
`@dataclasses.dataclass(slots=True)`. Slots remove the per-instance
`__dict__`, so a `BasicConfig` instance can no longer hold attributes beyond
its five declared fields: `level`, `filename`, `stream`, `force`, and
`handlers`. The benefit is that a mistyped or aspirational attribute name now
fails loudly with an `AttributeError` at the point of assignment, instead of
silently creating a dead attribute that the rest of the code never reads.

### What breaks

Assigning any attribute not among the declared fields now raises
`AttributeError` instead of succeeding silently.

### Before

```python
cfg = BasicConfig(level="INFO")
cfg.filename_backup = "app.log.bak"  # silently created a dead attribute
```

### After

```python
cfg = BasicConfig(level="INFO")
# `filename_backup` is not a declared field, so this now raises:
# AttributeError: 'BasicConfig' object has no attribute 'filename_backup'
cfg.filename_backup = "app.log.bak"
```

### How to migrate

- Use one of the declared fields (`level`, `filename`, `stream`, `force`,
  `handlers`) if the value belongs in `basicConfig`'s configuration surface.
- Otherwise, keep auxiliary state in the caller's own structure — a separate
  variable, dataclass, or dictionary — rather than attaching it to the
  `BasicConfig` instance.

______________________________________________________________________

## Unchanged APIs

Reading and assigning the five declared fields (`level`, `filename`,
`stream`, `force`, `handlers`) is unchanged. `BasicConfig` construction,
`basicConfig()`, and every other public API are unaffected by this change.
