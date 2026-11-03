# Building toolwing in one word: `make debug` runs a debug build of the app,
# `make release` a release one; `make` alone lists the rest. Needs Rust (rustup)
# and, on Linux, the development libraries of the window systems it builds for.
#
# On Linux the window builds for Wayland by default:
#   make release FEATURES=wayland,x11
#
# `tw`, the command line, needs no window system:
#   make tw ARGS="base64 --help"

# rustup's tools, for shells that have not sourced ~/.cargo/env. Make 3.81 (macOS)
# looks commands up on the PATH it started with, so cargo is named by path too.
CARGO_BIN := $(HOME)/.cargo/bin
export PATH := $(CARGO_BIN):$(PATH)
CARGO ?= $(if $(wildcard $(CARGO_BIN)/cargo),$(CARGO_BIN)/cargo,cargo)

# The window systems to build for, and any other cargo features, like x11.
FEATURES ?= wayland
APP := -p toolwing --no-default-features --features $(FEATURES)
CLI := -p toolwing-cli
ALL := --workspace --no-default-features --features $(FEATURES)

.DEFAULT_GOAL := help
.PHONY: help debug release build build-debug tw install-tw test test-window lint fmt \
	compat clean

help: ## List the targets
	@grep -E '^[a-z-]+:.*## ' $(MAKEFILE_LIST) | awk -F ':.*## ' '{printf "  make %-14s %s\n", $$1, $$2}'

debug: ## Build and run the app, debug version: quicker to build, slower to run
	$(CARGO) run $(APP)

release: ## Build and run the app, release version
	$(CARGO) run --release $(APP)

build-debug: ## Build the debug versions: target/debug/toolwing and target/debug/tw
	$(CARGO) build $(APP) $(CLI)

build: ## Build the release versions: target/release/toolwing and target/release/tw
	$(CARGO) build --release $(APP) $(CLI)

tw: ## Run tw, debug version, with ARGS: make tw ARGS="uuid --v7 -n 3"
	$(CARGO) run -q $(CLI) -- $(ARGS)

install-tw: ## Install tw, release version, into ~/.cargo/bin
	$(CARGO) install --locked --path crates/cli

test: ## Run every test, without the ones that draw the window
	$(CARGO) test $(ALL)

test-window: ## Run every test, drawing every page to target/shots with no display (Linux)
	$(CARGO) test $(ALL),headless

lint: ## Check the format and run clippy
	$(CARGO) fmt --all -- --check
	$(CARGO) clippy $(ALL) --all-targets -- -D warnings
	$(CARGO) clippy $(APP),headless --all-targets -- -D warnings

fmt: ## Format the code
	$(CARGO) fmt --all

compat: ## Record the compatibility cases from the real tools, in Docker: make compat GROUP=base64
	scripts/compat_record.sh $(GROUP)

clean: ## Remove what was built
	$(CARGO) clean
