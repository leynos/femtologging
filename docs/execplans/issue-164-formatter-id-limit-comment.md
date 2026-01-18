# Add formatter_id limitation comment

This Execution Plan (ExecPlan) is a living document. The sections
`Constraints`, `Tolerances`, `Risks`, `Progress`, `Surprises & Discoveries`,
`Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work
proceeds.

Status: COMPLETE

## Purpose / big picture

Users of `FileHandlerBuilder` can call `with_formatter()` with any string
identifier, but only "default" is currently implemented. Non-default values
produce a `HandlerBuildError::InvalidConfig` error at build time with the
message "unknown formatter id: {other}". This change adds a module-level
comment clarifying that:

1. Only "default" formatter_id is currently supported.
2. A registry will be wired later to resolve non-default identifiers.

After this change, developers reading `file_builder.rs` will immediately
understand the limitation without needing to trace through `build_inner()`.

## Constraints

- The existing `with_formatter()` API must remain unchanged.
- No logic changes; documentation only.
- Comments must use en-GB-oxendict spelling per AGENTS.md.
- Module-level comments must use `//!` syntax per AGENTS.md Rust guidance.

## Tolerances (exception triggers)

- Scope: if implementation requires changes to more than 1 file, stop and
  escalate.
- Interface: if any public API signature must change, stop and escalate.
- Dependencies: no new dependencies permitted.
- Iterations: if `make test` fails after 2 attempts, stop and escalate.

## Risks

- Risk: Comment placement may not align with existing docstring style.
  Severity: low. Likelihood: low. Mitigation: Review existing module docstring
  and match style.

## Progress

- [x] (2026-01-18) Create `docs/execplans/` directory.
- [x] (2026-01-18) Write execplan to
  `docs/execplans/issue-164-formatter-id-limit-comment.md`.
- [x] (2026-01-18) Add limitation comment to
  `rust_extension/src/handlers/file_builder.rs`.
- [x] (2026-01-18) Run `make fmt` and verify no changes.
- [x] (2026-01-18) Run `make lint` and verify no warnings.
- [x] (2026-01-18) Run `make test` and verify all tests pass (212 Rust tests,
  286 Python tests).
- [x] (2026-01-18) Commit with message closing #164.

## Surprises & discoveries

(None yet)

## Decision log

(None yet)

## Outcomes & retrospective

Implementation complete. The module-level comment was added to
`rust_extension/src/handlers/file_builder.rs` clarifying the "default"
formatter_id limitation. All quality gates passed on the first attempt.

Lessons learned: None—this was a straightforward documentation-only change with
no surprises.

## Context and orientation

The file `rust_extension/src/handlers/file_builder.rs` provides a builder API
for constructing `FemtoFileHandler` instances. The builder accepts various
configuration options, including a `formatter_id` via `with_formatter()`.

The current module docstring (lines 1-7) reads:

    //! Builder for [`FemtoFileHandler`].
    //!
    //! Provides a fluent API for configuring a file-based logging handler.
    //! Only a subset of options are currently supported; additional
    //! parameters such as encoding and mode will be added as the project
    //! evolves. Flushing is driven by a `flush_record_interval`
    //! measured in records.

The `build_inner()` method (lines 163-180) handles formatter configuration:

- `FormatterId::Default` or `None`: uses `DefaultFormatter`
- `FormatterId::Custom(other)`: returns `HandlerBuildError::InvalidConfig`

## Plan of work

### Stage A: documentation (single edit)

Insert 2–3 lines after the existing module docstring (after line 7, before line
8) that explain the formatter_id limitation. The comment should:

1. State that only "default" formatter_id is currently supported.
2. Note that a formatter registry will be wired later for custom identifiers.
3. Clarify that this is a current limitation, not a configuration error.

### Stage B: validation

Run all quality gates to ensure the change meets repository standards.

## Concrete steps

Working directory: `/root/repo`

1. Create the execplans directory:

       mkdir -p docs/execplans

2. Edit `rust_extension/src/handlers/file_builder.rs`. Insert after line 7:

       //!
       //! **Note:** Only the "default" `formatter_id` is currently supported.
       //! Non-default identifiers will produce a build error. A formatter
       //! registry will be wired in future to resolve custom identifiers at
       //! build time.

3. Run formatting check:

       make fmt

   Expected: no changes (comment-only edit).

4. Run lint:

       make lint

   Expected: no warnings or errors.

5. Run tests:

       set -o pipefail && make test 2>&1 | tee /tmp/test.log
       echo "Exit code: $?"

   Expected: all tests pass.

6. Commit:

       git add rust_extension/src/handlers/file_builder.rs docs/execplans/
       git commit -m "Document formatter_id limitation in FileHandlerBuilder

       Add module-level comment clarifying that only the \"default\" formatter_id
       is currently supported, with a note that a registry will be wired later.

       closes #164"

## Validation and acceptance

Quality criteria:

- Tests: `make test` passes with no failures.
- Lint: `make lint` produces no warnings.
- Format: `make fmt` produces no changes.

Quality method:

- Run `make fmt && make lint && make test` and verify exit code 0.

Acceptance:

- The module docstring in `rust_extension/src/handlers/file_builder.rs`
  contains a note about the "default" formatter_id limitation.
- The comment mentions future registry integration.
- Existing `with_formatter()` API unchanged.

## Idempotence and recovery

This change is idempotent. If the comment already exists, the edit will be a
no-op. If the edit fails partway, simply retry from step 2.

## Artifacts and notes

Expected final module docstring (lines 1-12):

    //! Builder for [`FemtoFileHandler`].
    //!
    //! Provides a fluent API for configuring a file-based logging handler.
    //! Only a subset of options are currently supported; additional
    //! parameters such as encoding and mode will be added as the project
    //! evolves. Flushing is driven by a `flush_record_interval`
    //! measured in records.
    //!
    //! **Note:** Only the "default" `formatter_id` is currently supported.
    //! Non-default identifiers will produce a build error. A formatter
    //! registry will be wired in future to resolve custom identifiers at
    //! build time.

## Interfaces and dependencies

No new interfaces or dependencies. The change is documentation-only.
