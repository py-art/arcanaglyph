.DEFAULT_GOAL := help
.SILENT:


##############################################################################
# Colors
##############################################################################
ESC     := $(shell printf '\e')
RESET   := $(ESC)[0m
BLACK   := $(ESC)[30m
RED     := $(ESC)[31m
GREEN   := $(ESC)[32m
YELLOW  := $(ESC)[33m
BLUE    := $(ESC)[34m
PURPLE  := $(ESC)[35m
AZURE   := $(ESC)[36m
WHITE   := $(ESC)[37m
##############################################################################
# Vosk: линкер не находит libvosk.so в /usr/local/lib по умолчанию
##############################################################################
export LIBRARY_PATH := /usr/local/lib:$(LIBRARY_PATH)
export LD_LIBRARY_PATH := /usr/local/lib:$(LD_LIBRARY_PATH)

##############################################################################
# Dependencies
##############################################################################

.PHONY: install  ## Install dependencies
install:
	cargo build

.PHONY: sync  ## Sync, update and remove extra dependencies
sync:
	cargo update

.PHONY: check   ## Analyze the current package
check:
	echo "${GREEN}INFO : ${AZURE}Start Check${AZURE}${RESET}"
	cargo check

.PHONY: build   ## Build release binary
build:
	cargo build --release -p arcanaglyph-app

##############################################################################
# Run local
##############################################################################
.PHONY: run   ## Run application (single command)
run:
	cargo run -p arcanaglyph-app

##############################################################################
# Code quality
##############################################################################
.PHONY: fmt   ## Format code with rustfmt
fmt:
	cargo fmt

.PHONY: lint   ## Run clippy linter
lint:
	cargo clippy -- -D warnings

.PHONY: test   ## Run tests
test:
	cargo test

.PHONY: all   ## Format, lint, check and test
all: fmt lint check test

##############################################################################
# Packaging
##############################################################################
.PHONY: dist   ## Build distributable packages (.deb, .AppImage)
dist:
	cargo tauri build

.PHONY: clean  ## Clean the build cache
clean:
	echo "${GREEN}INFO : ${AZURE}Start Clean${AZURE}${RESET}"
	rm -f Cargo.lock
	cargo clean
	rm -rf target/

.PHONY: rebuild ## Clean and rebuild the project
rebuild: clean check
	echo "${GREEN}INFO : ${AZURE}Projet success clean and rebuild${AZURE}${RESET}"

##############################################################################
# Create help
##############################################################################
.PHONY: help  ## Display this help
help:
	@grep -E \
		'^.PHONY: .*?## .*$$' $(MAKEFILE_LIST) | \
		sort | \
		awk 'BEGIN {FS = ".PHONY: |## "}; {printf "${AZURE}%-25s${RESET} %s\n", $$2, $$3}'
