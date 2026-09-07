.PHONY: help all clean build release lint lint-rust lint-env-policy fmt check-fmt \
markdownlint tools nixie spelling spelling-helper-test lint-lanes-test test typecheck

CARGO ?= cargo
RUST_MANIFEST ?= rust_extension/Cargo.toml
BUILD_JOBS ?=
RUFF_VERSION ?= 0.15.12
RUFF ?= uvx ruff==$(RUFF_VERSION)
TY_VERSION ?= 0.0.75
TY ?= uvx ty==$(TY_VERSION)
MDLINT ?= markdownlint-cli2
NIXIE ?= nixie
# Single source of truth for the typos version, keeping the Makefile and any
# CI that shells out to this target from drifting apart.
TYPOS_VERSION ?= 1.48.0
UV_ENV = UV_CACHE_DIR=.uv-cache UV_TOOL_DIR=.uv-tools
TYPOS ?= $(UV_ENV) uv tool run typos@$(TYPOS_VERSION)
WHITAKER ?= whitaker
CARGO_BUILD_ENV ?= PYO3_USE_ABI3_FORWARD_COMPATIBILITY=0
TEST_THREADS ?= 1

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

fmt: tools ## Format sources
	$(RUFF) format
	$(CARGO) fmt --manifest-path $(RUST_MANIFEST)
	mdformat-all

check-fmt: ## Verify formatting
	$(RUFF) format --check
	cargo fmt --manifest-path $(RUST_MANIFEST) -- --check

lint: ## Run linters
	$(RUFF) check
	$(MAKE) lint-rust

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
		--with cyclopts --with plumbum \
		python -m pytest scripts/tests/test_lint_rust_lanes.py \
		-c /dev/null --rootdir=. -p no:cacheprovider -p cmd_mox.pytest_plugin

lint-rust: lint-lanes-test lint-env-policy ## Run Rust clippy across feature lanes and the Whitaker Dylint suite
	@$(CARGO_BUILD_ENV) $(UV_ENV) \
		INPUT_MANIFEST=$(RUST_MANIFEST) \
		INPUT_LANES="$(RUST_LINT_FEATURE_LANES)" \
		INPUT_LINT_ARGS="$(RUST_LINT_ARGS)" \
		uv run --script $(LINT_LANES_SCRIPT)
	cd rust_extension && $(CARGO_BUILD_ENV) RUSTFLAGS="-D warnings" $(WHITAKER) --all -- --all-targets --all-features

# Both Markdown gates list tracked files rather than globbing the working
# tree. Build caches under `.uv-cache/` and `.venv/` carry Markdown shipped by
# third-party packages, and a `find` picks those up as soon as a dependency
# changes: adding one Cyclopts dependency put a package LICENSE.md in front of
# the linter. `git ls-files` is what the spelling target below already uses.
markdownlint: spelling ## Lint Markdown files and enforce en-GB-oxendict spelling
	@git ls-files -z '*.md' | xargs -0 -r $(MDLINT) --

spelling: spelling-helper-test ## Enforce en-GB-oxendict spelling in Markdown prose
	@$(UV_ENV) uv run scripts/generate_typos_config.py
	@git ls-files -z '*.md' | \
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
	@git ls-files -z '*.md' | xargs -0 -r $(NIXIE)

test: build ## Run tests
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
	# Pass the environment explicitly: ty 0.0.75 ignores the equivalent
	# `[tool.ty.environment]` settings.
	$(TY) check --python ./.venv --extra-search-path scripts

help: ## Show available targets
	@grep -E '^[a-zA-Z_-]+:.*?##' $(MAKEFILE_LIST) | \
awk 'BEGIN {FS=":"; printf "Available targets:\n"} {printf "  %-20s %s\n", $$1, $$2}'
