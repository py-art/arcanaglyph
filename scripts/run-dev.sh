#!/usr/bin/env bash
#
# scripts/run-dev.sh
#
# Дев-запуск ArcanaGlyph через `cargo run`, симметрично `make dist → build-deb.sh`.
# Автодетектит наличие системных deps и набирает максимальный набор cargo-features:
#
#   gigaam       — всегда (default), на AVX через ort+download-binaries,
#                  на no-AVX замещается gigaam-system-ort (load-dynamic).
#   vosk         — если найден libvosk.so в /usr/local/lib или /usr/lib/arcanaglyph.
#   whisper      — если в PATH есть cmake.
#   qwen3asr     — всегда (без системных deps; разделяет ort с gigaam/system-ort).
#
# Цель — чтобы `make run` показывал тот же набор движков, что собирается в .deb
# через `make install` / `make dist` (см. scripts/build-deb.sh:39, APP_FEATURES).
#
# Запуск: `bash scripts/run-dev.sh` из любой директории. Обычно вызывается через
# `make run`. Требует: cargo, grep, pgrep/pkill.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${REPO_ROOT}"

# ANSI escapes, совместимые с zsh/bash (Makefile использует тот же стиль)
RESET=$'\033[0m'
GREEN=$'\033[32m'
YELLOW=$'\033[33m'
RED=$'\033[31m'
AZURE=$'\033[36m'

info()  { printf '%s\n' "${GREEN}INFO :${RESET} ${AZURE}$*${RESET}"; }
warn()  { printf '%s\n' "${YELLOW}INFO :${RESET} $*"; }
err()   { printf '%s\n' "${RED}ERROR:${RESET} $*" >&2; }

# 1. Останавливаем уже запущенный экземпляр (как в прежнем Makefile)
if pgrep -x arcanaglyph >/dev/null 2>&1; then
    warn "ArcanaGlyph запущен — останавливаю..."
    pkill -x arcanaglyph || true
    sleep 1
fi

# 2. Детект AVX
if grep -qw avx /proc/cpuinfo 2>/dev/null; then
    HAS_AVX=1
else
    HAS_AVX=0
fi

# 3. Общие проверки deps (одинаково для AVX и no-AVX)
LIBVOSK_DIR=""
HAS_VOSK=0
if [ -f /usr/local/lib/libvosk.so ]; then
    HAS_VOSK=1
    info "+ vosk (/usr/local/lib/libvosk.so)"
elif [ -f /usr/lib/arcanaglyph/libvosk.so ]; then
    HAS_VOSK=1
    LIBVOSK_DIR="/usr/lib/arcanaglyph"
    info "+ vosk (/usr/lib/arcanaglyph/libvosk.so из установленного .deb)"
else
    warn "- vosk пропущен: нет libvosk.so ни в /usr/local/lib, ни в /usr/lib/arcanaglyph"
    warn "        скачать: https://github.com/alphacep/vosk-api/releases (vosk-linux-x86_64-*.zip)"
    warn "        или: make install (поставит .deb с bundled libvosk.so)"
fi

HAS_WHISPER=0
if command -v cmake >/dev/null 2>&1; then
    HAS_WHISPER=1
    info "+ whisper (CMake найден)"
else
    warn "- whisper пропущен: нет CMake (sudo apt install cmake)"
fi

# qwen3asr идёт через тот же ort-крейт что и gigaam — отдельных system-deps нет.
info "+ qwen3asr (без системных deps, разделяет ort с gigaam)"

if [ "${HAS_AVX}" = "1" ]; then
    # ─── AVX-блок ──────────────────────────────────────────────────────────────
    info "CPU поддерживает AVX — GigaAM через ort + Microsoft pre-built (INT8 ~225 МБ)"

    # gigaam уже в default-features, поэтому собираем аддитивно через --features.
    EXTRA_FEATURES=""
    add_feat() { EXTRA_FEATURES="${EXTRA_FEATURES:+${EXTRA_FEATURES},}$1"; }
    [ "${HAS_VOSK}" = "1" ]    && add_feat vosk
    [ "${HAS_WHISPER}" = "1" ] && add_feat whisper
    add_feat qwen3asr

    info "features (over default=gigaam): ${EXTRA_FEATURES}"

    # LIBRARY_PATH для libvosk.so из /usr/lib/arcanaglyph (если он там, а не в /usr/local/lib)
    export LIBRARY_PATH="${LIBVOSK_DIR}${LIBVOSK_DIR:+:}${LIBRARY_PATH:-}"

    exec cargo run -p arcanaglyph-app --bin arcanaglyph --features "${EXTRA_FEATURES}"
else
    # ─── no-AVX-блок ───────────────────────────────────────────────────────────
    LIBORT="${HOME}/.local/lib/libonnxruntime.so"
    if [ ! -f "${LIBORT}" ]; then
        err "${LIBORT} не найден. Соберите onnxruntime без AVX:"
        warn "  cd ~/projects/onnxruntime-build/onnxruntime && \\"
        warn "  ./build.sh --config Release --build_shared_lib --parallel 3 --skip_tests \\"
        warn "      --cmake_extra_defines CMAKE_CXX_FLAGS='-mno-avx -mno-avx2 -mno-avx512f' \\"
        warn "      --cmake_extra_defines CMAKE_C_FLAGS='-mno-avx -mno-avx2 -mno-avx512f' \\"
        warn "      --cmake_extra_defines onnxruntime_DISABLE_CONTRIB_OPS=ON && \\"
        warn "  mkdir -p ~/.local/lib && cp build/Linux/Release/libonnxruntime.so* ~/.local/lib/"
        warn "Откатываюсь на Whisper Tiny (медленнее, но работает)"
        exec cargo run -p arcanaglyph-app --bin arcanaglyph --no-default-features --features whisper
    fi

    info "CPU без AVX — GigaAM через локально собранный onnxruntime: ${LIBORT}"

    FEATURES="gigaam-system-ort"
    [ "${HAS_VOSK}" = "1" ]    && FEATURES="${FEATURES},vosk"
    [ "${HAS_WHISPER}" = "1" ] && FEATURES="${FEATURES},whisper"
    FEATURES="${FEATURES},qwen3asr"

    info "features: ${FEATURES}"

    export ORT_DYLIB_PATH="${LIBORT}"
    export LIBRARY_PATH="${LIBVOSK_DIR}${LIBVOSK_DIR:+:}${LIBRARY_PATH:-}"

    exec cargo run -p arcanaglyph-app --bin arcanaglyph \
        --no-default-features --features "${FEATURES}"
fi
