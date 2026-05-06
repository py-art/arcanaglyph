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

.PHONY: install  ## Always rebuild .deb and reinstall (dev-only: проверка пути установки)
install:
	@if pgrep -x arcanaglyph >/dev/null 2>&1; then \
		echo "${YELLOW}INFO : ${RESET}ArcanaGlyph запущен — останавливаю...${RESET}"; \
		pkill -x arcanaglyph && sleep 1; \
	fi; \
	VERSION=$$(grep '"version"' crates/arcanaglyph-app/tauri.conf.json | head -1 | sed 's/.*"version": *"//;s/".*//');\
	DEB="target/release/bundle/deb/ArcanaGlyph_$${VERSION}_amd64.deb"; \
	echo "${YELLOW}INFO : ${RESET}Пересобираю .deb v$${VERSION} (dev-режим — всегда пересобирать)${RESET}"; \
	rm -f "$$DEB"; \
	bash scripts/build-deb.sh || exit 1; \
	echo "${GREEN}INFO : ${AZURE}Устанавливаю $$DEB (apt сам подтянет deps)${RESET}"; \
	sudo apt install --reinstall -y "./$$DEB"; \
	echo "${GREEN}INFO : ${AZURE}Запускаю ArcanaGlyph...${RESET}"; \
	nohup arcanaglyph >/dev/null 2>&1 &

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
.PHONY: run   ## Run application (auto-detect deps via scripts/run-dev.sh)
run:
	@bash scripts/run-dev.sh

##############################################################################
# Code quality
##############################################################################
.PHONY: fmt   ## Format code with rustfmt
fmt:
	cargo fmt

.PHONY: lint   ## Run clippy linter
lint:
	cargo clippy -- -D warnings

.PHONY: test   ## Run tests (default features)
test:
	cargo test

.PHONY: test-all   ## Run tests with all engines enabled (требует libvosk + whisper.cpp toolchain)
test-all:
	cargo test --all-features

.PHONY: all   ## Format, lint, check and test (default features)
all: fmt lint check test

##############################################################################
# Packaging
##############################################################################
.PHONY: dist   ## Build self-contained .deb (универсальный для AVX/no-AVX x86_64)
dist:
	bash scripts/build-deb.sh

.PHONY: clean  ## Clean the build cache
clean:
	echo "${GREEN}INFO : ${AZURE}Start Clean${AZURE}${RESET}"
	cargo clean

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
