.PHONY: help all clean build release lint fmt check-fmt markdownlint \
tools nixie test typecheck

CARGO ?= cargo
RUST_MANIFEST ?= rust_extension/Cargo.toml
BUILD_JOBS ?=
MDLINT ?= markdownlint
NIXIE ?= nixie
MDFORMAT_ALL ?= mdformat-all
UV_PYTHON := python3.12
CARGO_ENV := PYO3_PYTHON=$(UV_PYTHON)

all: release ## Build the release artifact

build: ## Build debug artifact
	uv venv -p $(UV_PYTHON)
	uv sync -p $(UV_PYTHON) --group dev

release: ## Build release artifact
	PYO3_USE_ABI3_FORWARD_COMPATIBILITY=1 $(CARGO_ENV) $(CARGO) build $(BUILD_JOBS) --manifest-path $(RUST_MANIFEST) --release

clean: ## Remove build artifacts
	$(CARGO) clean --manifest-path $(RUST_MANIFEST)
	rm -rf build dist *.egg-info \
	  .mypy_cache .pytest_cache .coverage coverage.* \
	  lcov.info htmlcov .venv
	find . -type d -name '__pycache__' -print0 | xargs -0 -r rm -rf

define ensure_tool
$(if $(shell command -v $(1) >/dev/null 2>&1 && echo y),,\
$(error $(1) is required but not installed))
endef

tools:
	$(call ensure_tool,$(MDFORMAT_ALL))
	$(call ensure_tool,$(CARGO))
	$(call ensure_tool,rustfmt)
	$(call ensure_tool,uv)
	$(call ensure_tool,ruff)
	$(call ensure_tool,ty)

fmt: tools ## Format sources
	ruff format
	$(CARGO_ENV) $(CARGO) fmt --manifest-path $(RUST_MANIFEST)
	$(MDFORMAT_ALL)

check-fmt: ## Verify formatting
	ruff format --check
	$(CARGO_ENV) $(CARGO) fmt --manifest-path $(RUST_MANIFEST) -- --check

lint: ## Run linters
	ruff check
	$(CARGO_ENV) $(CARGO) clippy --manifest-path $(RUST_MANIFEST) -- -D warnings

markdownlint: ## Lint Markdown files
	find . -type f -name '*.md' -not -path './target/*' -print0 | xargs -0 $(MDLINT)

nixie: ## Validate Mermaid diagrams
	find . -type f -name '*.md' -not -path './target/*' -print0 | xargs -0 $(NIXIE)

test: build ## Run tests
	$(CARGO_ENV) $(CARGO) fmt --manifest-path $(RUST_MANIFEST) -- --check
	$(CARGO_ENV) $(CARGO) test --manifest-path $(RUST_MANIFEST)
	uv run pytest -q

typecheck: build ## Static type analysis
	ty check

help: ## Show available targets
	@grep -E '^[a-zA-Z_-]+:.*?##' $(MAKEFILE_LIST) | \
	  awk 'BEGIN {FS=":"; printf "Available targets:\n"} {printf "  %-20s %s\n", $$1, $$2}'
