.PHONY: help all clean build release lint lint-clippy lint-whitaker fmt check-fmt \
markdownlint tools nixie spelling spelling-helper-test test typecheck package-check test-doc

# The lint targets share Cargo's build cache and must not overlap under `make -j`.
.NOTPARALLEL: lint

CARGO ?= cargo
RUST_MANIFEST ?= rust_extension/Cargo.toml
FMT_TOOLCHAIN ?= nightly-2026-05-28
BUILD_JOBS ?=
RUSTDOC_FLAGS ?= --cfg docsrs -D warnings
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
# `--all` selects Dylint libraries; this selects every Cargo package.
WHITAKER_PACKAGES ?= --workspace
CARGO_BUILD_ENV ?= PYO3_USE_ABI3_FORWARD_COMPATIBILITY=0
TEST_THREADS ?= 1

all: release spelling ## Build the release artefact and enforce spelling

build: ## Build dev artefact and install into venv
	UV_VENV_CLEAR=1 uv venv
	$(CARGO_BUILD_ENV) uv sync --group dev
	# Install the mixed Rust/Python package into the venv for tests/tools
	$(CARGO_BUILD_ENV) uv run maturin develop --manifest-path $(RUST_MANIFEST) --features python,test-util

release: ## Build release artefact
	$(CARGO_BUILD_ENV) $(CARGO) build $(BUILD_JOBS) --manifest-path $(RUST_MANIFEST) --release

clean: ## Remove build artefacts
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
	$(CARGO) +$(FMT_TOOLCHAIN) fmt --manifest-path $(RUST_MANIFEST) --all
	mdformat-all

check-fmt: ## Verify formatting
	$(RUFF) format --check
	$(CARGO) +$(FMT_TOOLCHAIN) fmt --manifest-path $(RUST_MANIFEST) --all -- --check

lint: lint-clippy lint-whitaker ## Run linters

lint-clippy: ## Run Ruff and Rust Clippy across feature lanes
	$(RUFF) check
	@set -e; for features in none python log-compat tracing-compat; do \
		if [ "$$features" = none ]; then flags=""; else flags="--features $$features"; fi; \
		echo "# Lint Rust features: $$features"; \
		$(CARGO_BUILD_ENV) $(CARGO) clippy --manifest-path $(RUST_MANIFEST) --workspace --no-default-features $$flags -- -D warnings; \
	done
	$(CARGO_BUILD_ENV) $(CARGO) clippy --manifest-path $(RUST_MANIFEST) --workspace --all-targets --all-features -- -D warnings
	RUSTDOCFLAGS="$(RUSTDOC_FLAGS)" $(CARGO) doc --manifest-path $(RUST_MANIFEST) --workspace --no-deps

lint-whitaker: ## Run Whitaker across every workspace package and target
	cd rust_extension && $(CARGO_BUILD_ENV) RUSTFLAGS="-D warnings" $(WHITAKER) --all $(WHITAKER_PACKAGES) -- --all-targets --all-features

markdownlint: spelling ## Lint Markdown files and enforce en-GB-oxendict spelling
	find . -type f -name '*.md' -not -path '*/target/*' -print0 | xargs -0 $(MDLINT) --

spelling: spelling-helper-test ## Enforce en-GB-oxendict spelling in tracked source and prose
	@$(UV_ENV) uv run scripts/generate_typos_config.py
	@git ls-files -z | \
		xargs -0 -r env $(UV_ENV) uv tool run typos@$(TYPOS_VERSION) \
		--config typos.toml --force-exclude

spelling-helper-test: ## Validate the shared spelling-policy integration
	@$(UV_ENV) uv tool run ruff@$(RUFF_VERSION) format --isolated \
		--target-version py313 --check scripts/generate_typos_config.py \
		scripts/typos_rollout.py scripts/typos_rollout_cache.py \
		scripts/tests
	@$(UV_ENV) uv tool run ruff@$(RUFF_VERSION) check --isolated \
		--target-version py313 scripts/generate_typos_config.py \
		scripts/typos_rollout.py scripts/typos_rollout_cache.py \
		scripts/tests
	@PYTHONPATH=scripts $(UV_ENV) uv run --no-project --python 3.13 \
		--with pytest==9.0.2 --with pytest-cov==7.0.0 \
		python -m pytest scripts/tests \
		-c /dev/null --rootdir=. -p no:cacheprovider \
		--cov=generate_typos_config --cov=typos_rollout \
		--cov=typos_rollout_cache --cov-fail-under=90

nixie: ## Validate Mermaid diagrams
	find . -type f -name '*.md' -not -path '*/target/*' -print0 | xargs -0 $(NIXIE)

test: build ## Run tests
	$(CARGO) +$(FMT_TOOLCHAIN) fmt --manifest-path $(RUST_MANIFEST) --all -- --check
	$(CARGO_BUILD_ENV) $(CARGO) clippy --manifest-path $(RUST_MANIFEST) --workspace --no-default-features -- -D warnings
	$(CARGO_BUILD_ENV) $(CARGO) clippy --manifest-path $(RUST_MANIFEST) --workspace --no-default-features --features python -- -D warnings
	$(CARGO_BUILD_ENV) $(CARGO) clippy --manifest-path $(RUST_MANIFEST) --workspace --no-default-features --features log-compat -- -D warnings
	$(CARGO_BUILD_ENV) $(CARGO) clippy --manifest-path $(RUST_MANIFEST) --workspace --no-default-features --features tracing-compat -- -D warnings
	# Test baseline without optional features, then with python, then with Rust compatibility bridges.
	$(CARGO_BUILD_ENV) cargo test --manifest-path $(RUST_MANIFEST) --no-default-features -- --test-threads=$(TEST_THREADS)
	$(CARGO_BUILD_ENV) cargo test --manifest-path $(RUST_MANIFEST) --no-default-features --features python -- --test-threads=$(TEST_THREADS)
	$(CARGO_BUILD_ENV) cargo test --manifest-path $(RUST_MANIFEST) --no-default-features --features log-compat -- --test-threads=$(TEST_THREADS)
	$(CARGO_BUILD_ENV) cargo test --manifest-path $(RUST_MANIFEST) --no-default-features --features tracing-compat -- --test-threads=$(TEST_THREADS)
	$(MAKE) test-doc
	uv run pytest -v
	$(MAKE) package-check

test-doc: ## Compile workspace documentation with warnings denied
	# PyO3 extension-module omits libpython linkage, so its doctests cannot link standalone.
	RUSTFLAGS="-D warnings" $(CARGO) test --manifest-path $(RUST_MANIFEST) --workspace --doc --all-features --exclude femtologging_rs $(BUILD_JOBS)
	# Exercise the extension's examples with every runtime feature in embedding mode.
	RUSTFLAGS="-D warnings" $(CARGO) test --manifest-path $(RUST_MANIFEST) --package femtologging_rs --doc --no-default-features --features python,test-util,log-compat,tracing-compat $(BUILD_JOBS)

package-check: ## Verify publishable crates without private test dependencies
	$(CARGO) package --manifest-path $(RUST_MANIFEST) --workspace --allow-dirty $(BUILD_JOBS)

typecheck: build ## Static type analysis
	# Pass the environment explicitly: ty 0.0.75 ignores the equivalent
	# `[tool.ty.environment]` settings.
	$(TY) check --python ./.venv --extra-search-path scripts

help: ## Show available targets
	@grep -E '^[a-zA-Z_-]+:.*?##' $(MAKEFILE_LIST) | \
awk 'BEGIN {FS=":"; printf "Available targets:\n"} {printf "  %-20s %s\n", $$1, $$2}'
