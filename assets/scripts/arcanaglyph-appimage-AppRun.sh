#!/bin/sh
# AppRun for ArcanaGlyph .AppImage.
#
# AppImage runtime монтирует образ в /tmp/.mount_XXXX/ и запускает этот скрипт,
# выставляя $APPDIR на корень mount'а. Wrapper:
#   1. Выбирает avx/noavx бинарь по /proc/cpuinfo.
#   2. Выставляет $LD_LIBRARY_PATH на bundled libs (libvosk.so линкуется при старте).
#   3. Выставляет $ORT_DYLIB_PATH на bundled libonnxruntime-{avx2,noavx}.so —
#      без этого setup_ort_dylib_path() в бинаре пойдёт в hardcoded /usr/lib/arcanaglyph
#      (которого в AppImage нет) и GigaAM/Qwen3-ASR упадут.

# На случай прямого запуска AppRun (без AppImage runtime) — фоллбек по readlink.
if [ -z "${APPDIR}" ]; then
    APPDIR="$(dirname "$(readlink -f "$0")")"
fi

LIB_DIR="${APPDIR}/usr/lib/arcanaglyph"

# libvosk.so загружается через dlopen при старте Vosk-движка.
export LD_LIBRARY_PATH="${LIB_DIR}:${LD_LIBRARY_PATH}"

# ORT load-dynamic backend: явно указываем bundled libonnxruntime.
if grep -qw avx /proc/cpuinfo 2>/dev/null; then
    export ORT_DYLIB_PATH="${LIB_DIR}/libonnxruntime-avx2.so"
    BIN="${LIB_DIR}/arcanaglyph-avx"
else
    export ORT_DYLIB_PATH="${LIB_DIR}/libonnxruntime-noavx.so"
    BIN="${LIB_DIR}/arcanaglyph-noavx"
fi

exec "${BIN}" "$@"
