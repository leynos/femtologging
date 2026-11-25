# Repository layout

This document is the canonical map of where important code lives and what each
path is responsible for. Design rationale belongs in the design documents and
the ADRs; this page answers "where do I change this?".

## Top level

- `femtologging/` — the Python package and the public configuration surface.
- `rust_extension/` — the Rust extension crate that implements logging.
- `tests/` — behavioural, contract, and property tests for both languages.
- `scripts/` — repository tooling invoked by the Makefile and CI.
- `docs/` — user, developer, design, and migration documentation.
- `ansible/` — host provisioning for the development environment.

## Python configuration modules

The `dictConfig` implementation is split by responsibility so that a change in
one concern does not reopen the others.

- `femtologging/config.py` — the public `dictConfig` entry point and the
  builder-level section parsers. It validates the policy and section shapes,
  then hands the sections to `config_sections.py`. It also re-exports the
  shared shape validators for existing consumers.
- `femtologging/_config_policy.py` — the filter-factory trust-boundary policy.
  `validate_allow_factories` rejects a non-bool policy before any configuration
  is read; `factory_disallowed_message` is the single source of the rejection
  text. The module has no state and performs no I/O.
- `femtologging/_config_validation.py` — the shared shape validators
  (`_validate_mapping_type`, `_validate_string_keys`, `_validate_string_list`).
  New modules should import these directly; `config.py` re-exports them only so
  that `config_sections.py` and `config_socket_opts.py` keep their existing
  import path.
- `femtologging/config_sections.py` — walks the configuration sections through
  `_SectionSpec`, `_SECTION_SPECS`, and `_attach_section`, with
  `_collect_filters` and `_process_sections` as the entry points.
- `femtologging/_config_filters.py` — builds one filter from one entry.
  `build_filter_from_dict` turns a declarative entry into a native filter
  builder and an enabled `"()"` entry into a resolved callback filter.
- `femtologging/_filter_factory.py` — resolves a dotted factory path to a
  Python object by importing modules and reading attributes.
- `femtologging/config_socket.py` and `config_socket_opts.py` — socket handler
  construction, including transport, TLS, and backoff parsing.
- `femtologging/file_config.py` — the `fileConfig` compatibility entry point.
- `femtologging/_basic_config.py` — `basicConfig` and its `BasicConfig` record.

## Where to add things

- A new filter form: add the builder in `femtologging/_config_filters.py` and
  the declarative key handling there, not in `config_sections.py`.
- A new configuration section: add a `_SectionSpec` in
  `femtologging/config_sections.py` and a parser in `femtologging/config.py`.
- A new shared shape check: add it to
  `femtologging/_config_validation.py`.
- A new trust-boundary rule: add it to `femtologging/_config_policy.py` so the
  message and the validation stay together.
- A new public option on `dictConfig`: document it in the users' guide, the
  migration guide, and the `dictConfig` docstring in the same change.

## Related documents

- [Configuration design](configuration-design.md) records the `dictConfig`
  behaviour and ordering.
- [Developers guide](developers-guide.md) records the validation, lint, test,
  and transaction contracts.
- [Users guide](users-guide.md) is the user-facing reference.
