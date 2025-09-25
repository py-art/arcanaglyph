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
# Dependencies
##############################################################################

.PHONY: install  ## Install dependencies
install:
	# combine debug release for run local
	cargo build

.PHONY: sync  ## Sync, update and remove extra dependencies
sync:
	cargo update

.PHONY: check   ## Analyze the current package
check:
	echo "${GREEN}INFO : ${AZURE}Start Check${AZURE}${RESET}"
	cargo check

.PHONY: build   ## Build application
build:
	cargo build --release
	cp ./target/release/arcanaglyph .
	chmod +x arcanaglyph

runf:
	./arcanaglyph

##############################################################################
# Run local
##############################################################################
.PHONY: run   ## Run application
run:
	# cargo run
	cargo run -p arcanaglyph-app
	# cargo run -p arcanaglyph-app 2>/dev/null

.PHONY: serv   ## Run server
serv:
	cargo run -p arcanaglyph-core


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
