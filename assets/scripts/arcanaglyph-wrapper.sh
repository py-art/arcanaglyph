#!/bin/sh
# /usr/bin/arcanaglyph wrapper.
#
# Цель: один и тот же `.deb` пакет работает на любых x86_64 Linux. На AVX-CPU
# запускаем версию бинаря, в которой whisper.cpp скомпилирован с AVX/AVX2/FMA/F16C
# (полная скорость). На CPU без AVX — версию, скомпилированную с -mno-avx*
# (универсально-безопасная, но медленнее на той же машине, если бы там был AVX).
#
# GigaAM/Qwen3-ASR разделяются не через wrapper, а через `setup_ort_dylib_path()`
# внутри бинаря — там подбирается соответствующая `libonnxruntime.so`.
#
# Wrapper не нужен на не-x86_64 (aarch64, riscv64) — там нет AVX. Дистрибуции под
# не-Linux вообще не используют этот скрипт.

if grep -qw avx /proc/cpuinfo 2>/dev/null; then
    exec /usr/lib/arcanaglyph/arcanaglyph-avx "$@"
else
    exec /usr/lib/arcanaglyph/arcanaglyph-noavx "$@"
fi
