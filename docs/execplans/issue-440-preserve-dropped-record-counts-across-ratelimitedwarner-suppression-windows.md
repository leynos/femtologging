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
- An explicit flush, a close, and destruction report any count still pending,
  because on those paths no later warning will come.
- A logger flush and logger destruction report pending dispatch-admission and
  context-capture drops with their own wordings.

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
  synchronous report only where no worker exists (closed-handler flush) or
  where the handler is about to disappear (`close`, `Drop`). Rationale: the
  marker keeps a slow `log` sink off the flush caller's thread; the new reports
  are exactly the cases where no marker can be sent. Date/Author: 2026-11-21,
  agent.
- Decision: name only the triggering cause. Rationale: one warner counts drops
  from several causes and nothing records which was most recent, so naming a
  "latest cause" would be an unsupported claim. Date/Author: 2026-11-21, agent.

## Verification plan

- Unit tests per boundary: HTTP and socket `drop_reporting_tests.rs` (cumulative
  cause, cause-neutral flush, closed-handler flush, close and Drop reports),
  stream shutdown/Drop reports, logger flush and destruction reports.
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
explicit flush, a close and a destruction each report a pending count; and the
full deterministic gate suite passes on the published candidate.

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
