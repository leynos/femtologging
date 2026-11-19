# Architectural decision record (ADR) 0009: external release-pinned spelling configuration builder

## Status

Accepted — 2026-11-18: replace the vendored spelling generator with the
release-pinned `typos-config-builder` gate and pin its interpreter to Python
3.14.

## Date

2026-11-18

## Context and problem statement

Markdown spelling is enforced with `typos` under the shared en-GB-oxendict
dictionary, which this repository extends with a local overlay. The merged
configuration had been produced by three scripts kept in this repository.
`scripts/generate_typos_config.py` built `typos.toml`;
`scripts/typos_rollout.py` and `scripts/typos_rollout_cache.py` refreshed and
cached the shared dictionary; and `scripts/tests/test_typos_rollout.py`
exercised the cache. Every consumer of the shared dictionary carried its own
copy of that merge, cache and offline logic, so a fix reached one repository at
a time and the copies could drift apart without any signal.

The drift had already produced a concrete defect. The fixture overlay comment
claimed the generator withdrew the shared dictionary's broad colour-option
pattern, but nothing did, so the mask stayed in force. The expression requires
a leading `--`, so it can hide a misspelt colour inside a command-line option
such as `--mispell-color`; it cannot match a standalone `color` in prose. The
withdrawal therefore matters for option names, not for prose.

Two further constraints bear on any replacement. The gate must run on every
supported Python lane of the hosted suite, and each `build-test` matrix job
exports `UV_PYTHON` for the lane it tests. A gate whose interpreter is chosen
from the ambient environment therefore behaves differently per lane.

## Decision drivers

- One reviewed implementation of the merge, cache and offline logic, rather
  than a copy per consumer repository.
- A builder pinned to a release, so a lint gate cannot change without a commit.
- A dictionary that stays live, so a dictionary fix reaches every consumer
  without repinning the builder.
- A generated `typos.toml` that no one hand-edits, with repository-specific
  exceptions in a declarative overlay.
- A gate that passes on every supported Python lane.
- Behavioural and end-to-end coverage kept in this repository even after the
  scripts that carried it are deleted.

## Decision

1. `make spelling` invokes `TYPOS_CONFIG_BUILDER` and runs its `gate`
   operation in write mode. `Makefile` pins the command to:

   ```shell
   uvx --python 3.14 \
     --from "git+https://github.com/leynos/typos-config-builder.git@v0.1.3" \
     typos-config-builder
   ```

   The gate refreshes the shared dictionary, merges the overlay, writes
   `typos.toml`, then checks spelling and the shared phrase corrections. The
   gate has no drift mode: `--check` belongs to standalone configuration
   generation, so `gate --check` is not an inspection command.
2. `make spelling` runs the gate and then checks the Markdown the gate cannot
   see. The gate enumerates the Git index, so a new document would pass locally
   and fail once committed; the same pinned release supplies the Typos binary
   that checks the still-unstaged files against the configuration the gate has
   just written.
3. The three vendored scripts and their tests are deleted. The builder owns
   the merge, cache, offline and pattern-withdrawal behaviour and tests it at
   the pinned release; this repository keeps only its configuration and its own
   coverage.
4. In-repository coverage is split by what it needs.
   `tests/test_spelling_gate.py` drives both recipe stages against stub
   executables in a scratch repository: the gate argument, an unstaged document
   reaching the second stage, a staged document staying with the gate, and each
   stage's failure failing the target. `tests/test_spelling_gate_end_to_end.py`
   drives a forwarding builder and the pinned Typos binary against a fixture
   repository, a fixture overlay and a local dictionary, including a seeded
   misspelling, an unstaged misspelling before and after staging, the overlay's
   accepted word and the withdrawn shared pattern. It is skipped unless
   `make spelling-integration-test` provisions the pin first. The Makefile text
   contract in `tests/test_make_target_contract.py` stays and pins the release,
   the interpreter and both recipes.
5. The builder's interpreter is pinned to 3.14. It requires Python 3.14 or
   newer, while each hosted lane exports `UV_PYTHON`; an explicit `--python`
   outranks that variable, so the 3.12 and 3.13 lanes still hand the builder an
   interpreter it accepts. The pin appears in the `Makefile` and in the
   documented standalone command.
6. `typos.local.toml` carries `schema = 1` and uses `[patterns] remove` to
   withdraw a shared pattern that is too broad for this repository, which is
   how the colour mask is now actually removed.

## Consequences

- The merge, cache and offline logic has one implementation, tested once at a
  pinned release, and a dictionary fix no longer waits for a script change in
  each consumer.
- The withdrawn colour mask takes effect, so a misspelled colour inside a
  command-line option name now fails the gate; the live dictionary's other new
  findings are corrected in place rather than exempted.
- `make spelling` needs `uvx` and the pinned release, which `uvx` fetches on
  first use and then serves from the local uv cache. Provisioning the tool
  therefore needs the network once per machine or cache, which is a separate
  need from refreshing the dictionary.
- The dictionary needs either the network or the untracked
  `.typos-oxendict-base.toml` and `.typos-oxendict-base.json` cache files. When
  neither is available, the builder seeds the cache from the snapshot bundled
  in the pinned release and reports the refresh as `bootstrap`, so a checkout
  whose tool is already provisioned can run the gate without reaching the
  dictionary authority.
- The interpreter floor is stricter than the project's own test floor for this
  one tool. It is a property of the builder, documented in the developers
  guide, and the pin must be revisited when the builder raises its own floor.
- `typos.toml` grows when the live dictionary grows. That is generated content
  and is reviewed as a dictionary change, not hand-edited.

## Alternatives considered

- **Keep the vendored generator.** Rejected: it reproduced merge, cache and
  offline logic in every consumer, and the copies already disagreed with their
  own documentation about the withdrawn pattern.
- **Track the builder's `main` branch.** Rejected: a lint gate would then
  change without a commit, which is the same hazard that made the rolling
  Whitaker suite expensive to consume.
- **Pin the shared dictionary as well as the builder.** Rejected: a dictionary
  fix would then need a repin in every consumer, which is the problem the
  shared dictionary exists to avoid.
- **Rely on the lane's `UV_PYTHON`.** Rejected: the builder refuses 3.12 and
  3.13, so the spelling gate failed on those hosted lanes on every run
  regardless of the repository's spelling.
- **Invoke the gate with `--check` in CI.** Rejected: `--check` is an option of
  standalone configuration generation, where it reports drift. `gate` has no
  drift mode; it always regenerates before checking, which is what CI needs.

## References

- `Makefile` — `TYPOS_CONFIG_BUILDER` and the `spelling` recipe.
- `docs/developers-guide.md` — the "Typos spelling checker" section.
- `tests/test_spelling_gate.py` and `tests/test_make_target_contract.py` — the
  retained coverage.
- `typos.local.toml` — the repository overlay.
- [typos-config-builder v0.1.3](https://github.com/leynos/typos-config-builder/releases/tag/v0.1.3)
- `leynos/agent-helper-scripts` — the shared en-GB-oxendict dictionary.
