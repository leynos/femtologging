# Architectural decision record (ADR) 005: Four-tier Python lint architecture

## Status

Accepted on 2026-08-28. `femtologging` adopts a four-tier Python lint
architecture: Ruff, PyPy-backed Pylint, `df12-python-lints` with `ambrleaks`,
and a Skylos strict dead-code gate, all reachable through `make lint`.

## Date

2026-08-28.

## Context and problem statement

`femtologging` already ran Ruff as a fast, broad Python lint pass. Ruff alone
does not cover every check the project wants: selected Pylint messages for
logging interpolation, pattern matching, and subprocess safety; house rules
maintained outside Ruff's rule set; secret-like values leaking into Syrupy
snapshot fixtures; and unreachable Python code accumulating in the package as
the Rust extension's surface changes.

Running full Pylint inside the project virtual environment would slow every
lint run and couple an unrelated toolchain to the project's dependency
closure. The project also has no automated check for dead Python code, so
removed call sites can leave orphaned functions and classes behind
indefinitely. A single decision was needed to define the complete Python lint
pipeline, including how dead-code detection avoids false positives without
becoming an easy escape hatch.

## Decision drivers

- Preserve Ruff as the fast, first-line lint and formatting-adjacent tool.
- Add focused Pylint checks without enabling full Pylint by default or
  coupling it to the project virtual environment.
- Run the shared `df12-python-lints` house rules against the project's real
  syntax, and scan Syrupy snapshots for unredacted secrets with `ambrleaks`.
- Detect dead Python code as a blocking production gate, without letting
  ordinary test-only references keep dead production code alive.
- Keep every tier reachable from one command, `make lint`.
- Keep tool revisions reproducible through explicit pins, and keep the
  Makefile and CI pins from silently drifting apart.
- Prevent false-positive dead-code findings from becoming an unreviewed
  suppression habit.

## Options considered

### Option A: Ruff only

Keep Ruff as the sole lint tier. This preserves speed and simplicity but
leaves logging, pattern-matching, and subprocess checks uncovered, and adds no
dead-code or snapshot-secret detection.

### Option B: Ruff plus project-installed Pylint, no dead-code gate

Add Pylint as a project development dependency and stop there. This closes
some of the coverage gap but couples Pylint's runtime to the project
environment and leaves dead code undetected.

### Option C: Four tiers — Ruff, PyPy-backed Pylint, df12/ambrleaks, Skylos

Run Ruff first; a focused, PyPy-isolated Pylint pass second; the pinned
`df12-python-lints` house rules and `ambrleaks` snapshot scan third, under
CPython 3.14; and a strict Skylos dead-code gate fourth, also under Python
3.14. Each tier is invoked through a pinned `uv tool run` or `uvx` command, so
no tier depends on the project's own dependency closure.

| Topic                  | Ruff only | Ruff + project Pylint | Four-tier pipeline     |
| ---------------------- | --------- | --------------------- | ---------------------- |
| Speed                  | Fastest   | Slower                | Slowest, staged        |
| Selected Pylint checks | Missing   | Present               | Present, isolated      |
| House rules (df12)     | Missing   | Missing               | Present                |
| Snapshot secret scan   | Missing   | Missing               | Present (`ambrleaks`)  |
| Dead-code detection    | Missing   | Missing               | Present, strict gate   |
| Environment coupling   | Low       | High                  | Low (pinned tool runs) |

_Table 1: Trade-offs across Python lint pipeline options._

## Decision outcome / proposed direction

Adopt Option C. `make lint` runs `lint-python` followed by `lint-rust`, and
`lint-python` runs the four tiers in order, stopping at the first failure:

1. **Ruff** (pinned `RUFF_VERSION`, currently `0.16.4`, mirrored as a CI job
   environment variable) — fast, broad rule set in preview mode, targeting
   `py312`. The companion typechecker `ty` (pinned `TY_VERSION`, currently
   `0.0.74`) uses the same Makefile/CI pin-sync scheme; both pins are asserted
   equal by `tests/test_lint_version_contract.py`, which does not itself pin a
   version so a deliberate bump only touches the Makefile and CI.
2. **Pylint** (`4.0.7`) — runs on managed PyPy through the pinned
   `leynos/pylint-pypy-shim` revision, isolating the second tier from the
   project's own virtual environment. Configuration lives in
   `pyproject.toml`'s `[tool.pylint]` tables: `py-version = "3.12"`,
   `max-module-lines = 400`, and a curated `enable` list of selected messages.
3. **`df12-python-lints`** (pinned to the `v0.3.0` commit) and its companion
   **`ambrleaks`** — run under CPython 3.14 so the parser stays ahead of the
   project's 3.12 syntax baseline. The enabled message set is
   `R9101,C9102,R9103,R9104,C9105,C9106,C9107,R9108,R9109,R9110,R9111,R9112,C9112`.
   `ambrleaks` sweeps Syrupy `.ambr` snapshots under `tests` and
   `femtologging/unittests` for unredacted secrets.
4. **Skylos** (`4.33.2`) — a blocking production dead-code gate, also run
   under Python 3.14 so Skylos's own runtime `ast` implementation parses the
   project's syntax without producing phantom findings. Skylos scans
   `femtologging` only, excludes `femtologging/unittests` and the native
   `femtologging/_femtologging_rs.pyi` stub, and runs with
   `--category dead_code --gate --format concise --no-upload --no-provenance
   --no-grep-verify`. `[tool.skylos.gate] strict = true` in `pyproject.toml`
   enforces the strict gate mode.

Skylos false positives are resolved by investigation, not by reflexive
suppression. Every finding is investigated; genuine dead code is removed.
Verified false positives — implicit runtime callers such as re-exports,
test-util hooks, or protocol-shaped parameters — are modelled first as typed
`[[tool.skylos.dead_code.entrypoints]]` rules with a caller-specific reason.
Only a boundary that an entry-point rule cannot describe is recorded through
`make skylos-allow SYMBOL=<symbol> REASON="<reason>"`, which requires
non-whitespace `SYMBOL` and `REASON` values (exit code 2 otherwise). The
target uses `SYMBOL` rather than `NAME` because Windows Subsystem for Linux
(WSL) injects `NAME` with the host name, and it serializes whitelist writes
with `flock` against the ignored `.skylos-whitelist.lock` file so concurrent
recordings do not overwrite one another.

The complete lint interface — the Skylos scan command, its exclusions, the
gate's strict mode, the current (empty) documented-whitelist set, and the
current entry-point rule set — is pinned by
`tests/test_skylos_lint_contract.py` and
`tests/test_skylos_whitelist_boundary.py`, which parse the Makefile with the
pinned `makeutil` executable (`makeutil parse Makefile`, JSON) rather than
matching Makefile text. Recording a new false-positive exception therefore
requires a conscious update to `test_skylos_lint_contract.py`; the interface
cannot drift silently. `makeutil` (pinned revision
`29fc5a1634ffbaa18a773eed9dff1b2838a45d9c`, built with the
`nightly-2026-05-28` toolchain and Polonius) is a prerequisite of `make test`
and is installed independently by every full-suite CI job (`ci.yml`
`build-test` and `heavy-tests.yml` `heavy`) using the same pinned toolchain
and revision.

## Consequences

### Positive

- Contributors run one command, `make lint`, to exercise the complete Python
  lint pipeline.
- Ruff continues to provide fast, high-signal feedback ahead of the slower
  tiers.
- Pylint adds selected checks Ruff does not fully cover, isolated from the
  project's own dependency closure.
- `df12-python-lints` enforces house rules under a syntax-forward CPython
  runtime, and `ambrleaks` catches unredacted secrets in snapshot fixtures.
- The Skylos gate prevents dead Python code from accumulating silently, while
  the entry-point-first policy keeps false-positive handling auditable and
  reviewable rather than an unreasoned escape hatch.
- The Makeutil-backed contract tests make the lint interface — including
  exclusions, gate strictness, and every recorded exception — an explicit,
  reviewed artefact rather than implicit Makefile text.

### Negative

- The full lint target is slower than Ruff alone, and slower again than the
  prior two-tier state.
- Local machines need `uv` to resolve PyPy for the Pylint tier and CPython
  3.14 for the `df12`/`ambrleaks`/Skylos tiers.
- `make test` now depends on a working Rust toolchain able to build the
  pinned `makeutil` revision, in addition to the project's own build
  requirements.
- Toolchain updates must consider four separate pins (Ruff/`ty`, the Pylint
  shim, `df12-python-lints`, and Skylos) rather than one.

## Known risks and limitations

- The Skylos entry-point and whitelist mechanisms are only as complete as the
  reasons recorded against them; an unreviewed or overly broad reason
  defeats the purpose of the gate.
- CPython 3.14 must remain resolvable by `uv` for the third and fourth tiers;
  an unavailable interpreter blocks `make lint` entirely rather than
  degrading gracefully.
- The pinned `makeutil` revision requires a specific nightly Rust toolchain
  and the Polonius borrow checker; a contributor without a working Rust
  toolchain cannot run `make test` locally until `makeutil` is installed.
- Pylint's PyPy shim is intentionally focused; messages outside the curated
  `enable` list remain out of scope unless the policy is updated
  deliberately.

## Addendum: docstring-coverage tier (2026-08-28)

A docstring-coverage stage, run with `interrogate`, was added to the Python
lint gate. It runs second, immediately after Ruff and before Pylint, so
`lint-python` now runs five stages rather than four: Ruff, `interrogate`,
PyPy-backed Pylint, `df12-python-lints` with `ambrleaks`, and the Skylos
strict dead-code gate. This ADR retains its original "four-tier" title for
link stability; readers should take the title as historical and this
addendum as the current tier count.

`interrogate` is pinned by `INTERROGATE_VERSION` (currently `1.7.0`) in the
Makefile, following the same pinned-`uv tool run` pattern as every other
tier, so an unpinned version bump cannot silently change the coverage
verdict. It runs with `--fail-under 100` against `INTERROGATE_TARGETS`
(`femtologging`), enforcing 100% docstring coverage over the production
package only.

Tests are deliberately excluded from this tier. Ruff's `D` rule family
already governs docstrings in test code, and `tests/steps/*.py` ignores
`undocumented-public-function` (D103) because pytest-bdd step function names
are self-documenting. Running `interrogate` over tests as well would also
demand docstrings on nested helper closures for little practical benefit.

This addition follows the same pattern as the `leynos/lading` and
`leynos/cuprum` Python lint stacks, which both run a pinned, production-only
docstring-coverage gate as part of their tiered lint pipelines.
