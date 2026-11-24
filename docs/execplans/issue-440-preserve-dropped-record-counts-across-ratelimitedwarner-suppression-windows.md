# Preserve dropped-record counts across `RateLimitedWarner` suppression windows

## Purpose / big picture

Issue #440 asks that a rate-limited drop warning never discard the drops a
suppression window accumulated. Upstream #531 (`c77e726e`) has landed that
preservation in `RateLimitedWarner`: a suppressed `warn_if_due` leaves the
counter untouched, `take_pending`/`flush` report it regardless of the interval,
and the flush barrier carries a taken count to the worker on its marker and
restores it if the marker never enters the queue. That half of the issue is
therefore implemented in `main` and is not reimplemented here.

What remained unimplemented is the consumer-facing half, which this plan now
covers:

- Every handler reports drops with one cumulative wording, "since the last
  report", instead of "in the last interval", because one report can cover
  several suppressed intervals.
- A warning that was triggered by a refusal names only that refusal's cause.
  The count may include earlier drops from other causes, so the message says
  "triggering cause", never "latest cause".
- An explicit flush reports a pending count inside the interval by carrying it
  on the worker's flush marker, together with the worker's own suppressed
  count; a marker that never enters a queue leaves the count pending. A close
  and destruction report a pending count directly, because on those paths no
  later warning will come and no caller deadline applies.
- Logger destruction reports pending dispatch-admission and context-capture
  drops with their own wordings. A logger flush adds no report of its own,
  because the barrier it runs under is deadline-bound.

An operator reading the warnings therefore sees the whole loss the handler can
still account for, and can tell which refusal asked for the report.

## Constraints

- `RateLimitedWarner`'s current API (`new`, `with_clock`, `record_drop`,
  `warn_if_due`, `flush`, `take_due`, `take_pending`, `restore_drops`, the
  `Clock` trait) is authoritative; this change does not alter it.
- No method signature changes; only message wording and the points at which a
  pending count is reported.
- The reports run through the process-wide `log` adapter. For HTTP and socket
  `close()` they run before the bounded shutdown wait and are not covered by
  `flush_timeout`.
- Existing tests that match the old strings are updated to the new wording.

## Tolerances (exception triggers)

- Scope: if a consumer contract cannot be expressed without changing the
  warner API, stop and record the boundary rather than extending the warner.
- Size: no code file over 400 lines; split a module rather than exceed it.
- Behaviour: if a report would need to block a caller beyond its deadline to
  preserve a count, discard-free reporting wins only where the existing
  contract already allows an unbounded `log` write; otherwise leave the count
  pending.

## Risks

- A count can still be unreported: no later due call, flush or close reaches
  it; a callback that panics before writing abandons it; a count pending at
  `fork` exists twice. The documentation states each case.
- Reporting at `close` before the deadline means a blocking sink can delay the
  close; the users guide and the Rust extension guide say so.

## Progress

- [x] (2026-11-21) Confirmed #531 landed the preservation core in `main` and
      that rebasing the old 63-commit series onto it is not viable: the series
      is built on `claim_interval`/`publish_report_time`, which `main` removed.
- [x] (2026-11-21) Started `port/issue-440-consumer-drop-reporting` from
      `c77e726e` and ported the consumer contract: HTTP, socket, stream, logger.
- [x] (2026-11-21) `cargo check --workspace --all-targets --all-features`
      passes on the ported tree.
- [x] (2026-11-21) Documentation updated in the users guide, developers guide,
      Rust extension guide and migration guide.
- [x] (2026-11-22) Review fixes for `71badd37`: the closed-handler branch of
      each `flush_until` path and the logger barrier no longer report drops
      synchronously; both workers report their own suppressed count before
      acknowledging; the context-capture fixture and the dispatch-admission
      fixture inject a fixed clock. Tests renamed or replaced accordingly, with
      worker-side regressions in `worker_flush_reporting_tests.rs` (HTTP and
      socket) and `worker_tests.rs`.
- [ ] (2026-11-21) Full deterministic gates on the ported candidate.
- [ ] (2026-11-22) Publish the candidate, obtain review, merge.

## Decision log

- Decision: rework the PR as a forward port instead of resolving the rebase.
  Rationale: `c77e726e` replaced the warner's claim/publication machinery with
  `take_due`/`take_pending`/`restore_drops`, so most of the old series
  describes code that no longer exists; the consumer contract is orthogonal and
  ports cleanly. The superseded series is preserved at
  `refs/backup/issue440-2ab341fd` for its evidence and review history.
  Date/Author: 2026-11-21, agent.
- Decision: keep `main`'s flush-marker transport for open handlers and add the
  synchronous report only on `close` and `Drop`, where the handler is about to
  disappear and no caller deadline applies. Rationale: the marker keeps a slow
  `log` sink off the flush caller's thread, and the deadline-bound
  `flush_until` contract forbids an unbounded write there, so a closed
  handler's flush leaves its count for the next `close()` or destruction
  instead of reporting it. Date/Author: 2026-11-21, agent.
- Decision: name only the triggering cause. Rationale: one warner counts drops
  from several causes and nothing records which was most recent, so naming a
  "latest cause" would be an unsupported claim. Date/Author: 2026-11-21, agent.

## Verification plan

- Unit tests per boundary: HTTP and socket `drop_reporting_tests.rs` (cumulative
  cause, cause-neutral flush, close and Drop reports), the worker-side flush
  reports in `worker_flush_reporting_tests.rs` (end to end through the handler)
  and `worker_tests.rs` (ordering against the acknowledgement), stream
  shutdown/Drop reports, and the logger destruction report. A closed handler's
  flush is covered too, and asserts the opposite of the first draft: it leaves
  the count pending.
- Existing message-matching tests updated to the new wording.
- Deterministic gates on the candidate: `make fmt`, `make check-fmt`,
  `make lint`, `make typecheck`, `make test`, `make markdownlint`, `make nixie`.

## Concrete steps

1. Port the helpers and call sites for the four consumers.
2. Port and adapt the tests; update old-wording assertions.
3. Update the four documentation files.
4. Run the gates sequentially on the final tree.
5. Push, obtain review, and merge once the PR is at equilibrium.

## Validation and acceptance

Acceptance for this reduced scope: the messages listed in the migration guide
are the ones the code emits; a suppressed count appears in the next report; an
open handler's explicit flush, a close and a destruction each report a pending
count; deadline-bound paths add no report of their own; and the full
deterministic gate suite passes on the published candidate.

## Idempotence and recovery

The port is additive on `main`. A failed gate run is fixed in place and the
gates are rerun on the new tree; the previous candidate's evidence is retained
but not transferred. The superseded branch tip remains at
`refs/backup/issue440-2ab341fd` and the pre-rebase main at
`refs/backup/main-c77e726e`.

## Artefacts and notes

- Old series tip: `2ab341fd` (`refs/backup/issue440-2ab341fd`).
- New base: `c77e726e` (`refs/backup/main-c77e726e`).
- The full pre-rework ExecPlan, with the design history of the removed warner
  machinery and the review rounds it accumulated, is preserved in that tip's
  copy of this file.

## Revision note

2026-10-16: initial draft. 2026-10-25 to 2026-11-21: the original series was
rebased repeatedly and passed several review rounds; that history is in the
backup ref.

2026-11-21: scope reduced after #531 landed the preservation core in `main`.
This plan now covers only the consumer contract, and the old series is retained
as evidence rather than replayed. The rationale and the port decisions are in
the decision log above.

2026-11-22: the review of `71badd37` found the first port draft had broken a
deadline contract and missed two reports. A closed handler's `flush_until`
called the process-wide `log` sink synchronously, which an arbitrary sink can
block past the context deadline the trait promises, and the logger's barrier
did the same. Both reports are removed from deadline-bound paths: a closed
handler's count stays pending for the next `close()` or destruction, and the
logger reports only at destruction. The review also showed the flush marker
carries only the handler-side count, so a worker-side suppressed drop survived
a normal flush unreported; both workers now flush their own warner before
acknowledging. The context-capture fixture's warner used the system clock, so
its suppression window depended on wall-clock timing; it now takes a fixed
clock injection.

2026-11-23: gate evidence for `8dc82f08` (base `0b06a657`). `make fmt`,
`check-fmt`, `markdownlint`, `nixie`, `lint` and `typecheck` passed on the
tree. The four cargo lanes and pytest passed in a full `make test` run; that
run was then interrupted by a host-level process kill during the last lane, so
the tracing-compat lane and `pytest --doctest-modules` were rerun directly on
the same tree and passed (lane exit 0; 1781 passed, 10 skipped). One boundary
needs recording: the final commit refactors a logger test and edits the users
guide after that full run. The refactor leaves the assertions unchanged, the
focused `producer_rejection` tests pass (3 passed), and `make lint`'s clippy
lanes compile every target under all four feature sets, so the earlier lanes'
execution evidence is not transferred as a claim about different code; the only
unexecuted combination is the refactored test body under the `none`,
`log-compat` and `tracing-compat` feature sets, and it is the same body the
python lane executes.
