.PHONY: help all clean build release lint lint-python interrogate lint-rust lint-rust-clippy \
        lint-rust-private-docs-policy-test lint-env-policy lint-lanes-test loom-runner-test fmt \
        check-fmt markdownlint tools nixie spelling spelling-integration-test \
        check-lock test typecheck makeutil skylos skylos-allow \
        install-build-tools check-build-tools

CARGO ?= cargo
RUST_MANIFEST ?= rust_extension/Cargo.toml
BUILD_JOBS ?=
UV_ENV = UV_CACHE_DIR=.uv-cache UV_TOOL_DIR=.uv-tools
# Pin Ruff so `make` and CI invoke the same release. CI repeats the pin as the
# RUFF_VERSION job environment variable in .github/workflows/ci.yml; the
# contract test in tests/test_lint_version_contract.py asserts the two stay in
# sync because rule sets differ between Ruff releases.
RUFF_VERSION ?= 0.16.4
RUFF ?= uvx ruff==$(RUFF_VERSION)
# Pin ty likewise: ty is pre-1.0 and diagnostics shift between releases, so an
# unpinned install breaks the typecheck gate without any code change. CI
# repeats the pin as the TY_VERSION job environment variable; bump both
# deliberately and fix new diagnostics in the same commit.
TY_VERSION ?= 0.0.75
TY ?= $(UV_ENV) uv tool run --from 'ty==$(TY_VERSION)' ty
MDLINT ?= markdownlint-cli2
# `make fmt` and `make check-fmt` call mdtablefix directly. `--git` selects the
# Markdown files Git tracks and `--include-untracked` adds the untracked files
# Git does not ignore, so a new document is formatted before it is staged.
# Both modes need mdtablefix 0.6.0 or later; CI pins the version at the
# install-mdtablefix step.
MDTABLEFIX ?= mdtablefix
MDTABLEFIX_SELECT = --git --include-untracked
MDTABLEFIX_RULES = --wrap --renumber --breaks --ellipsis --fences
NIXIE ?= nixie
# Pin the builder release, not the dictionary it fetches: the builder is
# reviewed code, while the shared en-GB-oxendict dictionary stays live so a
# dictionary fix reaches every consumer without a repin. The gate writes
# typos.toml, then runs the pinned Typos binary and the shared phrase checks.
#
# The builder requires Python 3.14 or newer, so its interpreter is pinned here
# as well. A hosted suite job exports `UV_PYTHON` for its own lane, and an
# explicit `--python` beats that variable: without the pin the 3.12 and 3.13
# lanes hand `uvx` an interpreter the builder refuses, and the spelling gate
# fails on every one of their runs regardless of the repository's spelling.
TYPOS_CONFIG_BUILDER ?= uvx --python 3.14 --from "git+https://github.com/leynos/typos-config-builder.git@v0.1.3" typos-config-builder
# The gate checks the Git index, so Markdown that is new and still unstaged has
# to be checked separately. Running Typos from the same pinned release means
# both stages use the identical spelling rules; pinning a Typos version here
# instead would let the two stages disagree.
TYPOS ?= uvx --quiet --python 3.14 --from "git+https://github.com/leynos/typos-config-builder.git@v0.1.3" typos
WHITAKER ?= whitaker
CARGO_BUILD_ENV ?= PYO3_USE_ABI3_FORWARD_COMPATIBILITY=0
TEST_THREADS ?= 1

# Pylint runs as a uv tool on managed PyPy 3.12. Both the interpreter's minor
# version and pylint itself are pinned: a bare `pypy` or a floating pylint
# would change lint behaviour without any repository change.
PYLINT_PYTHON ?= pypy@3.12
PYLINT_TARGETS ?= femtologging tests scripts
PYLINT_VERSION ?= 4.0.7
PYLINT = $(UV_ENV) uv tool run --managed-python --python $(PYLINT_PYTHON) \
  --from 'pylint==$(PYLINT_VERSION)' pylint
# df12-python-lints v0.3.0, pinned by commit so the tag cannot move silently.
DF12_PYTHON_LINTS_REF ?= 4cf41736cce2f7ba2778882a5c629c044568a0e5
DF12_PYTHON_LINTS = git+https://github.com/leynos/df12-python-lints.git@$(DF12_PYTHON_LINTS_REF)
# The df12 checkers and ambrleaks run under CPython 3.14 so their parser stays
# ahead of the project's Python 3.12 syntax baseline.
DF12_PYTHON ?= 3.14
DF12_PYLINT_MESSAGES = R9101,C9102,R9103,R9104,C9105,C9106,C9107,R9108,R9109,R9110,R9111,R9112,C9112
DF12_PYLINT = $(UV_ENV) uv tool run --python $(DF12_PYTHON) \
  --from 'pylint==$(PYLINT_VERSION)' --with '$(DF12_PYTHON_LINTS)' pylint \
  --disable=all --load-plugins=df12_python_lints \
  --enable=$(DF12_PYLINT_MESSAGES)
AMBRLEAKS = $(UV_ENV) uv tool run --python $(DF12_PYTHON) \
  --from '$(DF12_PYTHON_LINTS)' ambrleaks
# Docstring coverage over every Python tree the repository owns, tests and
# pytest-bdd step functions included. Interrogate scans each target recursively,
# subject to its default exclusions.
# Pinned like every other tier: an unpinned interrogate could change the
# coverage verdict with no repository change.
INTERROGATE_VERSION ?= 1.7.0
INTERROGATE_TARGETS ?= femtologging tests scripts examples rust_extension conftest.py
# `basicConfig` is exempted because its three @typ.overload stubs cannot carry
# docstrings (Ruff D418 forbids them) and interrogate 1.7.0 only detects the
# literal `typing.overload`/`overload` spellings, so --ignore-overloaded-functions
# misses the `typ.overload` alias this project's import conventions require.
# Ruff's undocumented-public-function still guards the real implementation.
INTERROGATE_IGNORE_REGEX ?= ^basicConfig$$
INTERROGATE = $(UV_ENV) uv tool run --from 'interrogate==$(INTERROGATE_VERSION)' interrogate
# Level 2 prints a MISSED row per undocumented definition, which the
# `interrogate` target fails on: the rounded percentage alone can pass a lone gap.
# Stub files (`.pyi`) are not scanned.
INTERROGATE_FLAGS ?= -vv --fail-under 100 --ignore-regex '$(INTERROGATE_IGNORE_REGEX)'
# Cargo target directories (`rust_extension/target` holds maturin artefacts and
# generated stubs) are build output the repository does not own. Interrogate
# 1.7.0 resolves each `--exclude` to an absolute path and skips a file when its
# path matches "<exclude>*" under fnmatch, where `*` also crosses `/`. A pattern
# must therefore end in `/*`: a bare `target` would also skip `target_utils.py`.
# `target/*` prunes the root directory and `*/target/*` a nested crate's.
# `CARGO_TARGET_DIR` and `CARGO_BUILD_TARGET_DIR` are excluded too, with the
# fnmatch metacharacters in their values escaped so that they match literally.
# Limitation: owned Python under a directory literally named `target` would be
# skipped by `*/target/*`, so the recipe fails if Git tracks any such file.
fnmatch_escape =$(subst ?,[?],$(subst *,[*],$(subst [,[[],$(1))))
INTERROGATE_EXCLUDES ?= --exclude 'target/*' --exclude '*/target/*' \
  $(if $(CARGO_TARGET_DIR),--exclude '$(call fnmatch_escape,$(CARGO_TARGET_DIR))/*') \
  $(if $(CARGO_BUILD_TARGET_DIR),--exclude '$(call fnmatch_escape,$(CARGO_BUILD_TARGET_DIR))/*')
# The dead-code gate is the installed df12-skylos command; its `skylos` extra
# carries the one Skylos pin. Skylos parses source with its own runtime AST, so
# the tool environment runs Python 3.14.
DF12_SKYLOS_SHA = 56ad71721c3f1f2164ec84b69150582488f1a927
SKYLOS = $(UV_ENV) uv tool run --python 3.14 --from \
  'df12-python-lints[skylos] @ git+https://github.com/leynos/df12-python-lints.git@$(DF12_SKYLOS_SHA)' \
  df12-skylos

all: release spelling ## Build the release artifact and enforce spelling

build: check-build-tools ## Build dev artifact and install into venv
	UV_VENV_CLEAR=1 uv venv
	UV_CONFIG_FILE=$(CURDIR)/.github/uv-test-util.toml $(CARGO_BUILD_ENV) uv sync --locked --group dev
	# Install the mixed Rust/Python package into the venv for tests/tools
	$(CARGO_BUILD_ENV) uv run --locked maturin develop --manifest-path $(RUST_MANIFEST) --features python,test-util

release: check-build-tools ## Build release artifact
	$(CARGO_BUILD_ENV) $(CARGO) build $(BUILD_JOBS) --manifest-path $(RUST_MANIFEST) --release

clean: ## Remove build artifacts
	$(CARGO) clean --manifest-path $(RUST_MANIFEST)
	find . -type f -name '*.log' -not -path './target/*' -delete

define ensure_tool
$(if $(shell command -v $(1) >/dev/null 2>&1 && echo y),,\
$(error $(1) is required but not installed))
endef

check-lock: ## Verify uv.lock is tracked, not ignored, and matches pyproject.toml
	uv lock --check
	git ls-files --error-unmatch uv.lock
	@git check-ignore --no-index -q uv.lock; status=$$?; \
	if [ $$status -eq 0 ]; then \
		echo "uv.lock is ignored; remove the ignore rule from .gitignore" >&2; \
		exit 1; \
	fi; \
	if [ $$status -ne 1 ]; then \
		echo "git check-ignore failed with status $$status" >&2; \
		exit $$status; \
	fi

tools:
	$(call ensure_tool,$(MDLINT))
	$(call ensure_tool,$(CARGO))
	$(call ensure_tool,rustfmt)
	$(call ensure_tool,uv)

# The development build standard needs two things a fresh host may lack: the
# toolchain rust-toolchain.toml pins and, on Linux, the mold linker that
# .cargo/config.toml selects. Every target that compiles Rust depends on the
# check, so a missing prerequisite fails with an installation command rather
# than an opaque linker error. The check is a capability probe only: it
# installs nothing, so it is safe to run before every gate.
install-build-tools: ## Install the pinned Rust toolchain and the mold linker
	@scripts/install-build-tools.sh

check-build-tools: ## Verify the pinned toolchain and mold are available
	@scripts/check-build-tools.sh

makeutil: ## Verify the Makefile parser used by contract tests
	$(call ensure_tool,makeutil)

fmt: tools ## Format sources
	$(RUFF) format
	$(CARGO) fmt --manifest-path $(RUST_MANIFEST)
	$(MDTABLEFIX) --in-place $(MDTABLEFIX_SELECT) $(MDTABLEFIX_RULES)
	@unset FORCE_COLOR; $(MDLINT) --fix "**/*.md"

check-fmt: ## Verify formatting
	$(RUFF) format --check
	$(CARGO) fmt --manifest-path $(RUST_MANIFEST) -- --check
	$(MDTABLEFIX) --check $(MDTABLEFIX_SELECT) $(MDTABLEFIX_RULES)

lint: lint-python lint-rust ## Run linters

# Interrogate rounds its percentage to one decimal, so `--fail-under 100` alone
# can pass a lone gap in a large repository; fail on any MISSED row as well.
interrogate: ## Enforce complete Python docstring coverage
	@owned="$$(git ls-files -- ':(glob)target/**/*.py' ':(glob)**/target/**/*.py' 2>/dev/null)"; \
	  if [ -n "$$owned" ]; then \
	    printf '%s\n' "interrogate: tracked Python under a target/ directory is excluded from the gate; move it:" "$$owned" >&2; \
	    exit 1; \
	  fi
	@report="$$($(INTERROGATE) $(INTERROGATE_FLAGS) $(INTERROGATE_EXCLUDES) $(INTERROGATE_TARGETS))"; \
	  status=$$?; \
	  printf '%s\n' "$$report"; \
	  test "$$status" -eq 0 || exit "$$status"; \
	  if printf '%s\n' "$$report" | grep -q '| *MISSED *|'; then \
	    printf '%s\n' 'interrogate: undocumented definitions remain; see the MISSED rows above' >&2; \
	    exit 1; \
	  fi

lint-python: ## Run Ruff, interrogate, Pylint, df12, ambrleaks, and Skylos
	$(RUFF) check
	$(MAKE) --no-print-directory interrogate
	$(PYLINT) $(PYLINT_TARGETS)
	$(DF12_PYLINT) $(PYLINT_TARGETS)
	$(AMBRLEAKS) tests femtologging/unittests
	$(SKYLOS) check --repository .

skylos: ## Detect dead production code
	$(SKYLOS) check --repository .

skylos-allow: export SKYLOS_SYMBOL = $(value SYMBOL)
skylos-allow: export SKYLOS_REASON = $(value REASON)
skylos-allow: ## Document one named Skylos exception (SYMBOL=name REASON=text)
	$(SKYLOS) allow --repository . --symbol="$${SKYLOS_SYMBOL}" --reason="$${SKYLOS_REASON}"

# The environment-access policy (issue #423) has to hold in every target kind
# and every feature, including tests and benches, because a test that mutates
# the parent environment forces the whole suite to serialize.
#
# One lane per declared feature, plus `none` and `all`. `--all-features` alone
# would never compile a `#[cfg(not(feature = ...))]` block, and the crate has
# such blocks; `none` and `all` between them compile both arms of every feature
# gate, and each named lane compiles that feature's code with the others
# absent.
#
# The lanes are walked by `scripts/lint_rust_lanes.py` rather than by a shell
# loop here. A shell `for` loop reports the status of its last command, so a
# rejection in any earlier lane is discarded unless every call carries a
# guard — a guard that is easy to drop and whose absence leaves a gate that
# still looks green. The script fails on the first failing lane and names it,
# and `scripts/tests/test_lint_rust_lanes.py` covers that path directly.
#
# `-A clippy::all` in the policy lint arguments keeps its contract focused on
# environment access. The ordinary lint matrix below denies every warning over
# every target, so unrelated test and benchmark warnings remain build failures.
LINT_LANES_SCRIPT ?= scripts/lint_rust_lanes.py

ENV_POLICY_FEATURE_LANES ?= none extension-module python test-util log-compat tracing-compat all
ENV_POLICY_CARGO_ARGS ?= --all-targets
ENV_POLICY_LINT_ARGS ?= -A clippy::all -D clippy::disallowed_methods

RUST_LINT_FEATURE_LANES ?= none extension-module python log-compat tracing-compat test-util default all
RUST_LINT_CARGO_ARGS ?= --all-targets
RUST_LINT_ARGS ?= -D warnings

lint-env-policy: check-build-tools ## Enforce the environment-access policy across all targets and features
	@$(CARGO_BUILD_ENV) $(UV_ENV) \
		INPUT_MANIFEST=$(RUST_MANIFEST) \
		INPUT_LANES="$(ENV_POLICY_FEATURE_LANES)" \
		INPUT_CARGO_ARGS="$(ENV_POLICY_CARGO_ARGS)" \
		INPUT_LINT_ARGS="$(ENV_POLICY_LINT_ARGS)" \
		uv run --script $(LINT_LANES_SCRIPT)

lint-lanes-test: ## Unit-test the Rust lint lane driver
	@$(UV_ENV) uv tool run ruff@$(RUFF_VERSION) format --isolated \
		--target-version py313 --check $(LINT_LANES_SCRIPT) \
		scripts/tests/test_lint_rust_lanes.py scripts/tests/conftest.py
	@$(UV_ENV) uv tool run ruff@$(RUFF_VERSION) check --isolated \
		--target-version py313 $(LINT_LANES_SCRIPT) \
		scripts/tests/test_lint_rust_lanes.py scripts/tests/conftest.py
	# The driver itself is collected too: its docstring examples are the only
	# executable record of how a lane selection resolves, and `--doctest-modules`
	# is what turns them from comments into checks.
	@LINT_LANES_TEST=1 PYTHONPATH=scripts $(UV_ENV) uv run --no-project \
		--python 3.13 --with pytest==9.0.2 --with cmd-mox==0.2.0 \
		--with cyclopts --with plumbum --with hypothesis==6.167.1 \
		python -m pytest scripts/tests/test_lint_rust_lanes.py \
		$(LINT_LANES_SCRIPT) --doctest-modules \
		-c /dev/null --rootdir=. -p no:cacheprovider -p cmd_mox.pytest_plugin

LOOM_RUNNER_SCRIPT ?= scripts/run_loom_models.py

loom-runner-test: ## Unit-test the Loom model runner
	@$(UV_ENV) uv tool run ruff@$(RUFF_VERSION) format --isolated \
		--target-version py313 --check $(LOOM_RUNNER_SCRIPT) \
		scripts/tests/test_run_loom_models.py
	@$(UV_ENV) uv tool run ruff@$(RUFF_VERSION) check --isolated \
		--target-version py313 $(LOOM_RUNNER_SCRIPT) \
		scripts/tests/test_run_loom_models.py
	@LOOM_RUNNER_TEST=1 PYTHONPATH=scripts $(UV_ENV) uv run --no-project \
		--python 3.13 --with pytest==9.0.2 --with cmd-mox==0.2.0 \
		--with cyclopts --with plumbum \
		python -m pytest scripts/tests/test_run_loom_models.py \
		$(LOOM_RUNNER_SCRIPT) --doctest-modules \
		-c /dev/null --rootdir=. -p no:cacheprovider -p cmd_mox.pytest_plugin

lint-rust-clippy: check-build-tools ## Run standard Rust Clippy feature lanes across every target
	@$(CARGO_BUILD_ENV) $(UV_ENV) \
		INPUT_MANIFEST=$(RUST_MANIFEST) \
		INPUT_LANES="$(RUST_LINT_FEATURE_LANES)" \
		INPUT_CARGO_ARGS="$(RUST_LINT_CARGO_ARGS)" \
		INPUT_LINT_ARGS="$(RUST_LINT_ARGS)" \
		uv run --script $(LINT_LANES_SCRIPT)

lint-rust-test-docs: check-build-tools ## Check Rust test-support type and field documentation
	@$(CARGO_BUILD_ENV) $(CARGO) test --manifest-path $(RUST_MANIFEST) \
		--test test_support_docs --all-features

lint-rust-private-docs-policy-test: ## Exercise private-doc policy through Make's Clippy lanes
	@$(UV_ENV) uv run --no-project --python 3.13 \
		--with pytest==9.0.2 --with pytest-timeout==2.4.0 \
		python -m pytest tests/test_private_docs_lint_contract.py \
		-c /dev/null --rootdir=. --noconftest -p no:cacheprovider

lint-rust: lint-lanes-test lint-env-policy lint-rust-clippy lint-rust-test-docs lint-rust-private-docs-policy-test ## Run Rust lint lanes, private-doc policy, and the Whitaker Dylint suite
	RUSTFLAGS="-D warnings" PYO3_USE_ABI3_FORWARD_COMPATIBILITY=0 $(WHITAKER) --all --manifest-path $(RUST_MANIFEST) -- --all-targets --all-features

markdownlint: spelling ## Lint Markdown files and enforce en-GB-oxendict spelling
	# Lint only repository Markdown (tracked plus new non-ignored files):
	# caches such as .uv-cache carry third-party files that are not ours to
	# police.
	git ls-files -z --cached --others --exclude-standard '*.md' | \
		xargs -0 -r $(MDLINT) --

SPELLING_PHRASE_SCAN ?= $(TYPOS_CONFIG_BUILDER) check-phrases

spelling: ## Enforce en-GB-oxendict spelling and shared phrase corrections
	@$(UV_ENV) $(TYPOS_CONFIG_BUILDER) gate
	# The gate enumerates the Git index, so a new Markdown file would pass here
	# and fail once it is committed. The gate has just regenerated `typos.toml`,
	# so the same pinned Typos binary checks the remaining files with the same
	# configuration. `--force-exclude` keeps that configuration's exclusions in
	# force: Typos checks an explicitly named path even when the policy excludes
	# it, so an untracked file under `node_modules` would otherwise fail the
	# target although the gate ignores it. `env` passes the uv cache settings
	# through `xargs`, which would otherwise read them as the command to run.
	git ls-files -z --others --exclude-standard '*.md' | \
		xargs -0 -r env $(UV_ENV) $(TYPOS) \
			--config typos.toml --force-exclude --
	# Typos tokenizes on punctuation, so it never sees a hyphenated prohibited
	# phrase; the shared phrase corrections are the gate's other check, and that
	# check also enumerates the index. The same configuration is therefore used
	# to run it over the untracked Markdown as well, by adding those files to a
	# throwaway index: the gate has already phrase-checked the tracked text, so
	# an empty index is enough, and an intent-to-add entry is read from the
	# worktree. The trap removes the index file without disturbing the exit
	# statuses, so a finding still fails the target and an empty untracked list
	# still runs the phrase check.
	@spelling_index=$$(mktemp) && trap 'rm -f "$$spelling_index"' EXIT; \
		GIT_INDEX_FILE="$$spelling_index" git read-tree --empty && \
		{ git ls-files -z --others --exclude-standard '*.md' | \
			xargs -0 -r env GIT_INDEX_FILE="$$spelling_index" \
				git add --intent-to-add -- ; } && \
		$(UV_ENV) GIT_INDEX_FILE="$$spelling_index" $(SPELLING_PHRASE_SCAN)

SPELLING_INTEGRATION_ENV ?= FEMTOLOGGING_SPELLING_INTEGRATION=1

spelling-integration-test: ## Exercise the pinned spelling builder end to end
	# The end-to-end tests resolve the pinned builder from its Git tag, so they
	# need the network on a cold cache and stay out of the ordinary suite.
	# Provisioning the pin first makes a missing network this target's failure
	# rather than a contributor's, and CI runs this after `make spelling`, where
	# the cache is already warm.
	@$(UV_ENV) $(TYPOS_CONFIG_BUILDER) --help > /dev/null
	# `--noconftest` keeps the suite's shared conftest out of this run: it
	# imports the compiled extension, which this target deliberately does not
	# require, and none of these tests ask for a fixture it defines.
	@$(SPELLING_INTEGRATION_ENV) PYTHONPATH=. $(UV_ENV) uv run --no-project \
		--python 3.13 --with pytest==9.0.2 --with pytest-timeout==2.4.0 \
		--with pyyaml python -m pytest tests/test_spelling_gate_end_to_end.py \
		-c /dev/null --rootdir=. --noconftest -p no:cacheprovider

nixie: ## Validate Mermaid diagrams
	git ls-files -z --cached --others --exclude-standard '*.md' | \
		xargs -0 -r $(NIXIE)

test: check-build-tools build makeutil lint-rust-clippy loom-runner-test ## Run tests
	$(CARGO) fetch --locked --manifest-path $(RUST_MANIFEST)
	$(CARGO) fmt --manifest-path $(RUST_MANIFEST) -- --check
	# Test baseline without optional features, then with python, then with Rust compatibility bridges.
	$(CARGO_BUILD_ENV) $(CARGO) test --manifest-path $(RUST_MANIFEST) --no-default-features -- --test-threads=$(TEST_THREADS)
	$(CARGO_BUILD_ENV) $(CARGO) test --manifest-path $(RUST_MANIFEST) --no-default-features --features python -- --test-threads=$(TEST_THREADS)
	$(CARGO_BUILD_ENV) $(CARGO) test --manifest-path $(RUST_MANIFEST) --no-default-features --features log-compat -- --test-threads=$(TEST_THREADS)
	$(CARGO_BUILD_ENV) $(CARGO) test --manifest-path $(RUST_MANIFEST) --no-default-features --features tracing-compat -- --test-threads=$(TEST_THREADS)
	# `--doctest-modules` is what makes the documented examples load-bearing.
	# Nothing executed them before, and three test-helper examples named
	# variables that were never bound. An example nobody runs is a comment
	# that looks like evidence.
	FEMTOLOGGING_EXPECT_TEST_UTIL=1 UV_CONFIG_FILE=$(CURDIR)/.github/uv-test-util.toml uv run --locked pytest -v --doctest-modules

check-api-boundary: build ## Verify production builds withhold test-only APIs
	# The release wheel under test is built from the production maturin
	# configuration, so this suite never inherits the development feature set.
	# `-o addopts=` clears the repository-wide ignore that keeps this suite out
	# of default collection and out of every coverage lane.
	$(CARGO_BUILD_ENV) UV_CONFIG_FILE=$(CURDIR)/.github/uv-test-util.toml uv run pytest -v -o addopts= tests/test_release_api_boundary.py

typecheck: check-build-tools build ## Static type analysis
	# ty 0.0.75 runs outside the project venv, so point it at the interpreter
	# that has the compiled extension installed; it does not reliably apply the
	# equivalent `[tool.ty.environment]` settings.
	$(TY) check --python .venv

help: ## Show available targets
	@grep -E '^[a-zA-Z_-]+:.*?##' $(MAKEFILE_LIST) | \
		awk 'BEGIN {FS=":"; printf "Available targets:\n"} {printf "  %-20s %s\n", $$1, $$2}'
