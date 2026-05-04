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

.PHONY: install  ## Build (if needed) and install .deb package locally
install:
	@if pgrep -x arcanaglyph >/dev/null 2>&1; then \
		echo "${YELLOW}INFO : ${RESET}ArcanaGlyph запущен — останавливаю...${RESET}"; \
		pkill -x arcanaglyph && sleep 1; \
	fi; \
	VERSION=$$(grep '"version"' crates/arcanaglyph-app/tauri.conf.json | head -1 | sed 's/.*"version": *"//;s/".*//');\
	DEB="target/release/bundle/deb/ArcanaGlyph_$${VERSION}_amd64.deb"; \
	if [ ! -f "$$DEB" ]; then \
		echo "${YELLOW}INFO : ${RESET}.deb v$${VERSION} не найден — собираю...${RESET}"; \
		bash scripts/build-deb.sh || exit 1; \
	fi; \
	echo "${GREEN}INFO : ${AZURE}Устанавливаю $$DEB (apt сам подтянет deps)${RESET}"; \
	sudo apt install -y "./$$DEB"; \
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
.PHONY: run   ## Run application (AVX → all-engines; без AVX → gigaam-system-ort + vosk/whisper если есть deps)
run:
	@if pgrep -x arcanaglyph >/dev/null 2>&1; then \
		echo "${YELLOW}INFO : ${RESET}ArcanaGlyph запущен — останавливаю...${RESET}"; \
		pkill -x arcanaglyph && sleep 1; \
	fi
	@if grep -qw avx /proc/cpuinfo 2>/dev/null; then \
		echo "${GREEN}INFO : ${AZURE}CPU поддерживает AVX — GigaAM через ort + Microsoft pre-built (INT8 ~225 МБ)${RESET}"; \
		cargo run -p arcanaglyph-app --bin arcanaglyph; \
	else \
		LIBORT="$$HOME/.local/lib/libonnxruntime.so"; \
		if [ -f "$$LIBORT" ]; then \
			FEATURES="gigaam-system-ort"; \
			echo "${GREEN}INFO : ${AZURE}CPU без AVX — GigaAM через локально собранный onnxruntime: $$LIBORT${RESET}"; \
			if [ -f /usr/local/lib/libvosk.so ]; then \
				FEATURES="$$FEATURES,vosk"; \
				echo "${GREEN}INFO : ${AZURE}+ vosk (libvosk.so найдена)${RESET}"; \
			else \
				echo "${YELLOW}INFO : ${RESET}- vosk пропущен: нет /usr/local/lib/libvosk.so${RESET}"; \
				echo "${YELLOW}        скачать: https://github.com/alphacep/vosk-api/releases (vosk-linux-x86_64-*.zip)${RESET}"; \
			fi; \
			if command -v cmake >/dev/null 2>&1; then \
				FEATURES="$$FEATURES,whisper"; \
				echo "${GREEN}INFO : ${AZURE}+ whisper (CMake найден)${RESET}"; \
			else \
				echo "${YELLOW}INFO : ${RESET}- whisper пропущен: нет CMake (sudo apt install cmake)${RESET}"; \
			fi; \
			echo "${GREEN}INFO : ${AZURE}features: $$FEATURES${RESET}"; \
			ORT_DYLIB_PATH="$$LIBORT" cargo run -p arcanaglyph-app --bin arcanaglyph --no-default-features --features "$$FEATURES"; \
		else \
			echo "${RED}ERROR: $$LIBORT не найден. Соберите onnxruntime без AVX:${RESET}"; \
			echo "${YELLOW}  cd ~/projects/onnxruntime-build/onnxruntime && \\${RESET}"; \
			echo "${YELLOW}  ./build.sh --config Release --build_shared_lib --parallel 3 --skip_tests \\${RESET}"; \
			echo "${YELLOW}      --cmake_extra_defines CMAKE_CXX_FLAGS='-mno-avx -mno-avx2 -mno-avx512f' \\${RESET}"; \
			echo "${YELLOW}      --cmake_extra_defines CMAKE_C_FLAGS='-mno-avx -mno-avx2 -mno-avx512f' \\${RESET}"; \
			echo "${YELLOW}      --cmake_extra_defines onnxruntime_DISABLE_CONTRIB_OPS=ON && \\${RESET}"; \
			echo "${YELLOW}  mkdir -p ~/.local/lib && cp build/Linux/Release/libonnxruntime.so* ~/.local/lib/${RESET}"; \
			echo "${YELLOW}INFO : Откатываюсь на Whisper Tiny (медленнее, но работает)${RESET}"; \
			cargo run -p arcanaglyph-app --bin arcanaglyph --no-default-features --features whisper; \
		fi \
	fi

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
