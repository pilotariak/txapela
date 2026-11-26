# SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
# SPDX-License-Identifier: Apache-2.0

BANNER = T X A P E L A

SHELL = /bin/bash -o pipefail

.DEFAULT_GOAL := help

.PHONY: help
help: ## Display this help
	@echo -e "\033[32;01m      $(BANNER)\033[0m"
	@echo "------------------------------------------------------------------"
	@awk 'BEGIN {FS = ":.*##"; printf "Usage: make \033[36m<target>\033[0m\n"} /^[a-zA-Z_-]+:.*?##/ { printf "  \033[36m%-25s\033[0m %s\n", $$1, $$2 } /^##@/ { printf "\n\033[1m%s\033[0m\n", substr($$0, 5) } ' $(MAKEFILE_LIST)

guard-%:
	@if [ "${${*}}" = "" ]; then \
		echo "Environment variable $* not set"; \
		exit 1; \
	fi

check-%:
	@if command -v $* > /dev/null 2>&1; then echo "🟢 $*"; else echo "🔴 $*"; fi

##@ Development

.PHONY: check
check: ## Run cargo check
	cargo check --all-features --all-targets

.PHONY: build
build: ## Build the project
	cargo build

.PHONY: build-release
build-release: ## Build the project in release mode
	cargo build --release

.PHONY: run
run: ## Run the project
	cargo run

.PHONY: watch
watch: ## Watch for changes and rebuild
	cargo watch -x check -x test -x run

##@ Testing

.PHONY: test
test: ## Run tests
	cargo test --all-features

.PHONY: test-verbose
test-verbose: ## Run tests with verbose output
	cargo test --all-features -- --nocapture

.PHONY: test-coverage
test-coverage: ## Run tests with coverage (requires cargo-tarpaulin)
	cargo tarpaulin --all-features --out Html --output-dir coverage

##@ Quality

.PHONY: fmt
fmt: ## Format code
	cargo fmt --all

.PHONY: fmt-check
fmt-check: ## Check code formatting
	cargo fmt --all -- --check

.PHONY: clippy
clippy: ## Run clippy linter
	cargo clippy --all-features --all-targets -- -D warnings

.PHONY: audit
audit: ## Run security audit
	cargo audit

.PHONY: outdated
outdated: ## Check for outdated dependencies
	cargo outdated

.PHONY: lint
lint: ## Run the prek hooks on all files
	prek run -a

.PHONY: rust-lint
rust-lint: fmt-check clippy audit ## Run all Rust linters

##@ Documentation

.PHONY: doc
doc: ## Generate documentation
	cargo doc --no-deps --all-features

.PHONY: doc-open
doc-open: ## Generate and open documentation
	cargo doc --no-deps --all-features --open

##@ Maintenance

.PHONY: clean
clean: ## Clean build artifacts
	cargo clean

.PHONY: update
update: ## Update dependencies
	cargo update

.PHONY: install-tools
install-tools: ## Install development tools
	rustup component add rustfmt clippy
	cargo install cargo-audit cargo-outdated cargo-watch cargo-tarpaulin

.PHONY: pre-commit
pre-commit: rust-lint test ## Run pre-commit checks
	@echo "All pre-commit checks passed!"

##@ Docker

.PHONY: docker-build
docker-build: ## Build Docker image
	docker build -t txapela:latest .

.PHONY: docker-run
docker-run: ## Run Docker container
	docker run --rm -it txapela:latest

.PHONY: docker-shell
docker-shell: ## Open shell in Docker container
	docker run --rm -it txapela:latest /bin/sh
