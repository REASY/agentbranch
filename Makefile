CARGO        ?= cargo
AGBRANCH_BIN ?= target/debug/agbranch
E2E_ARGS     ?=

.DEFAULT_GOAL := help

.PHONY: all help build build-release check fmt fmt-check clippy lint test ci e2e clean

all: build ## Alias for build (satisfies convention)

help: ## Show available targets
	@awk 'BEGIN {FS = ":.*?## "} /^[a-zA-Z0-9_-]+:.*?## / {printf "  \033[36m%-14s\033[0m %s\n", $$1, $$2}' $(MAKEFILE_LIST)

build: ## cargo build (debug)
	$(CARGO) build

build-release: ## cargo build --release
	$(CARGO) build --release

check: ## cargo check (fast type-check, no codegen)
	$(CARGO) check --all-targets --all-features

fmt: ## cargo fmt --all (auto-fix)
	$(CARGO) fmt --all

fmt-check: ## cargo fmt --all --check (no changes, just verify)
	$(CARGO) fmt --all --check

clippy: ## cargo clippy with -D warnings
	$(CARGO) clippy --all-targets --all-features -- -D warnings

lint: fmt-check clippy ## fmt-check + clippy

test: ## cargo test --all-targets --all-features
	$(CARGO) test --all-targets --all-features

ci: lint test ## Exactly what .github/workflows/ci.yml runs

e2e: build ## Run scripts/smoke-e2e.sh against the debug binary (pass E2E_ARGS=--verbose to tail)
	scripts/smoke-e2e.sh --binary $(AGBRANCH_BIN) $(E2E_ARGS)

clean: ## cargo clean
	$(CARGO) clean
