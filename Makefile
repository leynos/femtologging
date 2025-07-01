.PHONY: help all clean build release lint fmt check-fmt markdownlint \
tools nixie test typecheck

CARGO ?= cargo
RUST_MANIFEST ?= rust_extension/Cargo.toml
BUILD_JOBS ?=
MDLINT ?= markdownlint
NIXIE ?= nixie

all: release ## Build the release artifact

build: ## Build debug artifact
	uv venv
	PYO3_USE_ABI3_FORWARD_COMPATIBILITY=1 uv sync --group dev

release: ## Build release artifact
	PYO3_USE_ABI3_FORWARD_COMPATIBILITY=1 $(CARGO) build $(BUILD_JOBS) --manifest-path $(RUST_MANIFEST) --release

clean: ## Remove build artifacts
	$(CARGO) clean --manifest-path $(RUST_MANIFEST)

define ensure_tool
$(if $(shell command -v $(1) >/dev/null 2>&1 && echo y),,\
$(error $(1) is required but not installed))
endef

tools:
	$(call ensure_tool,mdformat-all)
	$(call ensure_tool,$(CARGO))
	$(call ensure_tool,rustfmt)
	$(call ensure_tool,ruff)
	$(call ensure_tool,ty)

fmt: tools ## Format sources
	ruff format
	$(CARGO) fmt --manifest-path $(RUST_MANIFEST)
	mdformat-all

check-fmt: ## Verify formatting
	ruff format --check
	cargo fmt --manifest-path $(RUST_MANIFEST) -- --check

lint: ## Run linters
	ruff check
	PYO3_USE_ABI3_FORWARD_COMPATIBILITY=1 cargo clippy --manifest-path $(RUST_MANIFEST) -- -D warnings

markdownlint: ## Lint Markdown files
	find . -type f -name '*.md' -not -path './target/*' -print0 | xargs -0 $(MDLINT)

nixie: ## Validate Mermaid diagrams
	find . -type f -name '*.md' -not -path './target/*' -print0 | xargs -0 $(NIXIE)

test: build ## Run tests
	cargo fmt --manifest-path $(RUST_MANIFEST) -- --check
	PYO3_USE_ABI3_FORWARD_COMPATIBILITY=1 cargo clippy --manifest-path $(RUST_MANIFEST) -- -D warnings
	PYO3_USE_ABI3_FORWARD_COMPATIBILITY=1 cargo test --manifest-path $(RUST_MANIFEST)
	uv run pytest -q

typecheck: build ## Static type analysis
	ty check

help: ## Show available targets
	@grep -E '^[a-zA-Z_-]+:.*?##' $(MAKEFILE_LIST) | \
awk 'BEGIN {FS=":"; printf "Available targets:\n"} {printf "  %-20s %s\n", $$1, $$2}'
