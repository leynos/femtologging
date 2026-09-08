.PHONY: help all clean build release lint lint-python lint-rust \
        lint-env-policy lint-lanes-test fmt \
        check-fmt markdownlint tools nixie spelling spelling-helper-test \
        test typecheck makeutil skylos-allow

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
NIXIE ?= nixie
# Single source of truth for the typos version, keeping the Makefile and any
# CI that shells out to this target from drifting apart.
TYPOS_VERSION ?= 1.48.0
TYPOS ?= $(UV_ENV) uv tool run typos@$(TYPOS_VERSION)
WHITAKER ?= whitaker
CARGO_BUILD_ENV ?= PYO3_USE_ABI3_FORWARD_COMPATIBILITY=0
TEST_THREADS ?= 1

# Pylint runs on managed PyPy through the pylint-pypy shim, mirroring the lint
# stack in https://github.com/leynos/lading ("lading" is that repository's
# name, not a misspelling). The shim ref and pylint itself are both pinned: the
# shim ref alone would let pylint float, changing lint behaviour without any
# repository change.
PYLINT_PYTHON ?= pypy
PYLINT_TARGETS ?= femtologging tests scripts
PYLINT_PYPY_SHIM_REF ?= 726d09f968b4d729ee4b29c71fc732e744854f3b
PYLINT_PYPY_SHIM = git+https://github.com/leynos/pylint-pypy-shim.git@$(PYLINT_PYPY_SHIM_REF)
PYLINT_VERSION ?= 4.0.7
PYLINT = $(UV_ENV) uv tool run --python $(PYLINT_PYTHON) \
  --from '$(PYLINT_PYPY_SHIM)' --with 'pylint==$(PYLINT_VERSION)' pylint-pypy
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
# Docstring coverage over the production package only, matching the
# leynos/lading and leynos/cuprum lint stacks. Tests are excluded
# deliberately: Ruff's D rules already govern them, and tests/steps/*.py
# ignores D103 because pytest-bdd step names document themselves.
# Pinned like every other tier: an unpinned interrogate could change the
# coverage verdict with no repository change.
INTERROGATE_VERSION ?= 1.7.0
INTERROGATE_TARGETS ?= femtologging
# `basicConfig` is exempted because its three @typ.overload stubs cannot carry
# docstrings (Ruff D418 forbids them) and interrogate 1.7.0 only detects the
# literal `typing.overload`/`overload` spellings, so --ignore-overloaded-functions
# misses the `typ.overload` alias this project's import conventions require.
# Ruff's undocumented-public-function still guards the real implementation.
INTERROGATE_IGNORE_REGEX ?= ^basicConfig$$
INTERROGATE = $(UV_ENV) uv tool run --from 'interrogate==$(INTERROGATE_VERSION)' \
  interrogate --fail-under 100 --ignore-regex '$(INTERROGATE_IGNORE_REGEX)'
SKYLOS_VERSION = 4.33.2
# Skylos parses source using its own runtime AST; pinning Python 3.14 prevents
# phantom dead-code findings on syntax newer than an older tool runtime.
SKYLOS_CLI = $(UV_ENV) uv tool run --python 3.14 --from 'skylos==$(SKYLOS_VERSION)' skylos
# Scan-only global options stay separate from the command-only CLI above so
# the whitelist subcommand dispatches before any scan option.
SKYLOS = $(SKYLOS_CLI) --config-file pyproject.toml
SKYLOS_PRODUCTION_TARGETS ?= femtologging
# The .pyi stub declares native signatures only, so every parameter in it is
# trivially "unused"; exclude it alongside the in-package unit tests.
SKYLOS_EXCLUDE_FOLDERS ?= femtologging/unittests femtologging/_femtologging_rs.pyi
SKYLOS_EXCLUDE_FLAGS = $(foreach path,$(SKYLOS_EXCLUDE_FOLDERS),--exclude $(path))
SKYLOS_WHITELIST_LOCK ?= .skylos-whitelist.lock

all: release spelling ## Build the release artifact and enforce spelling

build: ## Build dev artifact and install into venv
	UV_VENV_CLEAR=1 uv venv
	$(CARGO_BUILD_ENV) uv sync --group dev
	# Install the mixed Rust/Python package into the venv for tests/tools
	$(CARGO_BUILD_ENV) uv run maturin develop --manifest-path $(RUST_MANIFEST) --features python,test-util

release: ## Build release artifact
	$(CARGO_BUILD_ENV) $(CARGO) build $(BUILD_JOBS) --manifest-path $(RUST_MANIFEST) --release

clean: ## Remove build artifacts
	$(CARGO) clean --manifest-path $(RUST_MANIFEST)
	find . -type f -name '*.log' -not -path './target/*' -delete

define ensure_tool
$(if $(shell command -v $(1) >/dev/null 2>&1 && echo y),,\
$(error $(1) is required but not installed))
endef

tools:
	$(call ensure_tool,mdformat-all)
	$(call ensure_tool,$(MDLINT))
	$(call ensure_tool,$(CARGO))
	$(call ensure_tool,rustfmt)
	$(call ensure_tool,uv)
makeutil: ## Verify the Makefile parser used by contract tests
	$(call ensure_tool,makeutil)

fmt: tools ## Format sources
	$(RUFF) format
	$(CARGO) fmt --manifest-path $(RUST_MANIFEST)
	mdformat-all

check-fmt: ## Verify formatting
	$(RUFF) format --check
	cargo fmt --manifest-path $(RUST_MANIFEST) -- --check

lint: lint-python lint-rust ## Run linters

lint-python: ## Run Ruff, interrogate, Pylint, df12, ambrleaks, and Skylos
	$(RUFF) check
	$(INTERROGATE) $(INTERROGATE_TARGETS)
	$(PYLINT) $(PYLINT_TARGETS)
	$(DF12_PYLINT) $(PYLINT_TARGETS)
	$(AMBRLEAKS) tests femtologging/unittests
	$(SKYLOS) $(SKYLOS_PRODUCTION_TARGETS) $(SKYLOS_EXCLUDE_FLAGS) --category dead_code --gate --format concise --no-upload --no-provenance --no-grep-verify

skylos-allow: export SKYLOS_SYMBOL = $(value SYMBOL)
skylos-allow: export SKYLOS_REASON = $(value REASON)
skylos-allow: ## Document one named Skylos exception, not an entry point
	@case "$${SKYLOS_SYMBOL}" in *[![:space:]]*) ;; *) printf "Error: SYMBOL is required for a named whitelist exception\\n" >&2; exit 2;; esac
	@case "$${SKYLOS_REASON}" in *[![:space:]]*) ;; *) printf "Error: REASON is required for a named whitelist exception\\n" >&2; exit 2;; esac
	flock "$(SKYLOS_WHITELIST_LOCK)" env $(SKYLOS_CLI) whitelist "$${SKYLOS_SYMBOL}" --reason "$${SKYLOS_REASON}"

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
# `-A clippy::all` in the policy lint arguments is deliberate and temporary:
# the lanes in `lint-rust` omit `--all-targets` because the test tree carries a
# backlog of unrelated Clippy findings, and clearing that backlog is issue
# #421's job. Silencing the rest of Clippy in this one target lets the
# environment policy govern test code today without absorbing that work. Once
# issue #421 lands, these settings fold into its lane list.
LINT_LANES_SCRIPT ?= scripts/lint_rust_lanes.py

ENV_POLICY_FEATURE_LANES ?= none extension-module python test-util log-compat tracing-compat all
ENV_POLICY_CARGO_ARGS ?= --all-targets
ENV_POLICY_LINT_ARGS ?= -A clippy::all -D clippy::disallowed_methods

RUST_LINT_FEATURE_LANES ?= none python log-compat tracing-compat
RUST_LINT_ARGS ?= -D warnings

lint-env-policy: ## Enforce the environment-access policy across all targets and features
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
	@LINT_LANES_TEST=1 PYTHONPATH=scripts $(UV_ENV) uv run --no-project \
		--python 3.13 --with pytest==9.0.2 --with cmd-mox==0.2.0 \
		--with cyclopts --with plumbum --with hypothesis==6.167.1 \
		python -m pytest scripts/tests/test_lint_rust_lanes.py \
		-c /dev/null --rootdir=. -p no:cacheprovider -p cmd_mox.pytest_plugin

lint-rust: lint-lanes-test lint-env-policy ## Run Rust clippy across feature lanes and the Whitaker Dylint suite
	@$(CARGO_BUILD_ENV) $(UV_ENV) \
		INPUT_MANIFEST=$(RUST_MANIFEST) \
		INPUT_LANES="$(RUST_LINT_FEATURE_LANES)" \
		INPUT_LINT_ARGS="$(RUST_LINT_ARGS)" \
		uv run --script $(LINT_LANES_SCRIPT)
	cd rust_extension && $(CARGO_BUILD_ENV) RUSTFLAGS="-D warnings" $(WHITAKER) --all -- --all-targets --all-features

markdownlint: spelling ## Lint Markdown files and enforce en-GB-oxendict spelling
	# Lint only repository Markdown (tracked plus new non-ignored files):
	# caches such as .uv-cache carry third-party files that are not ours to
	# police.
	git ls-files -z --cached --others --exclude-standard '*.md' | \
		xargs -0 -r $(MDLINT) --

spelling: spelling-helper-test ## Enforce en-GB-oxendict spelling in Markdown prose
	@$(UV_ENV) uv run scripts/generate_typos_config.py
	@git ls-files -z --cached --others --exclude-standard '*.md' | \
		xargs -0 -r env $(UV_ENV) uv tool run typos@$(TYPOS_VERSION) \
		--config typos.toml --force-exclude

spelling-helper-test: ## Validate the shared spelling-policy integration
	@$(UV_ENV) uv tool run ruff@$(RUFF_VERSION) format --isolated \
		--target-version py313 --check scripts/generate_typos_config.py \
		scripts/typos_rollout.py scripts/typos_rollout_cache.py \
		scripts/tests/test_typos_rollout.py
	@$(UV_ENV) uv tool run ruff@$(RUFF_VERSION) check --isolated \
		--target-version py313 scripts/generate_typos_config.py \
		scripts/typos_rollout.py scripts/typos_rollout_cache.py \
		scripts/tests/test_typos_rollout.py
	@PYTHONPATH=scripts $(UV_ENV) uv run --no-project --python 3.13 \
		--with pytest==9.0.2 --with pytest-cov==7.0.0 \
		python -m pytest scripts/tests/test_typos_rollout.py \
		-c /dev/null --rootdir=. -p no:cacheprovider \
		--cov=generate_typos_config --cov=typos_rollout \
		--cov=typos_rollout_cache --cov-fail-under=90

nixie: ## Validate Mermaid diagrams
	git ls-files -z --cached --others --exclude-standard '*.md' | \
		xargs -0 -r $(NIXIE)

test: build makeutil ## Run tests
	cargo fmt --manifest-path $(RUST_MANIFEST) -- --check
	$(CARGO_BUILD_ENV) cargo clippy --manifest-path $(RUST_MANIFEST) --no-default-features -- -D warnings
	$(CARGO_BUILD_ENV) cargo clippy --manifest-path $(RUST_MANIFEST) --no-default-features --features python -- -D warnings
	$(CARGO_BUILD_ENV) cargo clippy --manifest-path $(RUST_MANIFEST) --no-default-features --features log-compat -- -D warnings
	$(CARGO_BUILD_ENV) cargo clippy --manifest-path $(RUST_MANIFEST) --no-default-features --features tracing-compat -- -D warnings
	# Test baseline without optional features, then with python, then with Rust compatibility bridges.
	$(CARGO_BUILD_ENV) cargo test --manifest-path $(RUST_MANIFEST) --no-default-features -- --test-threads=$(TEST_THREADS)
	$(CARGO_BUILD_ENV) cargo test --manifest-path $(RUST_MANIFEST) --no-default-features --features python -- --test-threads=$(TEST_THREADS)
	$(CARGO_BUILD_ENV) cargo test --manifest-path $(RUST_MANIFEST) --no-default-features --features log-compat -- --test-threads=$(TEST_THREADS)
	$(CARGO_BUILD_ENV) cargo test --manifest-path $(RUST_MANIFEST) --no-default-features --features tracing-compat -- --test-threads=$(TEST_THREADS)
	uv run pytest -v

typecheck: build ## Static type analysis
	# ty 0.0.75 runs outside the project venv, so point it at the interpreter
	# that has the compiled extension installed; it does not reliably apply the
	# equivalent `[tool.ty.environment]` settings.
	# The spelling-policy helpers import siblings with PYTHONPATH=scripts.
	$(TY) check --python .venv --extra-search-path scripts

help: ## Show available targets
	@grep -E '^[a-zA-Z_-]+:.*?##' $(MAKEFILE_LIST) | \
		awk 'BEGIN {FS=":"; printf "Available targets:\n"} {printf "  %-20s %s\n", $$1, $$2}'
