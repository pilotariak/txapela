# SPDX-FileCopyrightText: Copyright (C) Nicolas Lamirault <nicolas.lamirault@gmail.com>
# SPDX-License-Identifier: Apache-2.0

BANNER = T X A P E L A

SHELL = /bin/bash -o pipefail

DIR = $(shell pwd)

# Colors for terminal output
NO_COLOR=\033[0m
OK_COLOR=\033[32;01m
ERROR_COLOR=\033[31;01m
WARN_COLOR=\033[33;01m
INFO_COLOR=\033[36m
WHITE_COLOR=\033[1m
MAKE_COLOR=\033[33;01m%-20s\033[0m

.DEFAULT_GOAL := help

# Define common messages
OK=[🟢]
KO=[🔴]
WARN=[🟠]
INFO=[🔵]


.PHONY: help
help: ## Display this help
	@echo -e "$(OK_COLOR)      P I L O T A R I A K  /  $(BANNER)$(NO_COLOR)"
	@echo "------------------------------------------------------------------"
	@echo ""
	@awk 'BEGIN {FS = ":.*##"; printf "Usage: make ${INFO_COLOR}<target>${NO_COLOR}\n"} /^[a-zA-Z_-]+:.*?##/ { printf "  ${INFO_COLOR}%-25s${NO_COLOR} %s\n", $$1, $$2 } /^##@/ { printf "\n${WHITE_COLOR}%s${NO_COLOR}\n", substr($$0, 5) } ' $(MAKEFILE_LIST)
	@echo ""

guard-%:
	@if [ "${${*}}" = "" ]; then \
		echo -e "$(ERROR_COLOR)Environment variable $* not set$(NO_COLOR)"; \
		exit 1; \
	fi

check-%:
	@if $$(hash $* 2> /dev/null); then \
		echo -e "$(OK_COLOR)$(OK)$(NO_COLOR) $*"; \
	else \
		echo -e "$(ERROR_COLOR)$(KO)$(NO_COLOR) $*"; \
	fi

# Keep these pins in step with Cargo.toml, wrangler.toml and .github/workflows/ci.yml.
WORKER_BUILD_VERSION = 0.9.0
CARGO_DENY_VERSION = 0.22.0

##@ Development

.PHONY: install
install: ## Install the Rust toolchain, worker-build, cargo-deny and the bun dependencies
	@echo -e "$(INFO)$(INFO_COLOR)[Install] Installing dependencies$(NO_COLOR)"
	rustup toolchain install
	cargo install --locked worker-build@$(WORKER_BUILD_VERSION)
	cargo install --locked cargo-deny@$(CARGO_DENY_VERSION)
	bun install

.PHONY: build
build: ## Build the Worker into build/ (worker-build)
	@echo -e "$(INFO)$(INFO_COLOR)[Build] Building the Worker$(NO_COLOR)"
	worker-build --release

.PHONY: dev
dev: ## Start local dev server (wrangler dev)
	@echo -e "$(INFO)$(INFO_COLOR)[Dev] Starting Cloudflare Worker$(NO_COLOR)"
	bun run dev

.PHONY: check
check: ## Check formatting and run clippy
	@echo -e "$(INFO)$(INFO_COLOR)[Check] cargo fmt and clippy$(NO_COLOR)"
	cargo fmt --check
	cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings

.PHONY: doc
doc: ## Build the documentation, private items included, and fail on any rustdoc warning
	@echo -e "$(INFO)$(INFO_COLOR)[Doc] cargo doc$(NO_COLOR)"
	RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --document-private-items --target wasm32-unknown-unknown

.PHONY: deny
deny: ## Check the crates: advisories, licenses, bans and sources (deny.toml)
	@echo -e "$(INFO)$(INFO_COLOR)[Deny] cargo deny check$(NO_COLOR)"
	cargo deny check

.PHONY: test
test: build ## Run the Rust unit tests, then the Worker tests in Miniflare
	@echo -e "$(INFO)$(INFO_COLOR)[Test] Running tests$(NO_COLOR)"
	cargo test
	bun run test

.PHONY: lint
lint: ## Run the prek hooks on all files
	@echo -e "$(INFO)$(INFO_COLOR)[Lint] Running prek hooks $(NO_COLOR)"
	@prek run -a

##@ Database

.PHONY: migrate-local
migrate-local: ## Apply the D1 migrations to the local database
	@echo -e "$(INFO)$(INFO_COLOR)[DB] Applying migrations locally$(NO_COLOR)"
	bun run migrate:local

.PHONY: migrate-remote
migrate-remote: ## Apply the D1 migrations to the remote database
	@echo -e "$(INFO)$(INFO_COLOR)[DB] Applying migrations remotely$(NO_COLOR)"
	bun run migrate:remote

.PHONY: migrate-list
migrate-list: ## List applied and pending migrations of the local database
	@echo -e "$(INFO)$(INFO_COLOR)[DB] Listing local migrations$(NO_COLOR)"
	bun run migrate:list

.PHONY: db-local
db-local: guard-SQL ## Run SQL on the local database: make db-local SQL="SELECT ..."
	bun run db:local -- "$(SQL)"

.PHONY: db-remote
db-remote: guard-SQL ## Run SQL on the remote database: make db-remote SQL="SELECT ..."
	bun run db:remote -- "$(SQL)"

##@ Deployment

.PHONY: deploy
deploy: migrate-remote ## Apply the D1 migrations remotely, then deploy the Worker
	@echo -e "$(INFO)$(INFO_COLOR)[Deploy] Deploying to production$(NO_COLOR)"
	bun run deploy

##@ Maintenance

.PHONY: clean
clean: ## Clean project
	@echo -e "$(INFO)$(INFO_COLOR)[Clean] Processing $(NO_COLOR)"
	cargo clean
	rm -rf build node_modules .wrangler
