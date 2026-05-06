#!/usr/bin/env bash
#
# scripts/prepare-bundled-libs.sh
#
# Готовит нативные библиотеки в `assets/libs/`, которые потом заливаются в `.deb` пакет
# через `bundle.linux.deb.files` в `tauri.conf.json` (см. `scripts/build-deb.sh`).
#
# Что бандлим:
#   * libonnxruntime-noavx.so   (наш self-build 1.20.1, ~25 МБ) — закоммичен в git.
#   * libonnxruntime-avx2.so    (Microsoft pre-built 1.20.1, ~13 МБ) — качаем из GitHub.
#   * libvosk.so                (alphacep pre-built 0.3.45, ~13 МБ) — качаем из GitHub.
#
# Версии и SHA256 пинятся ниже; при апгрейде — обновить тут.
#
# Запуск: `bash scripts/prepare-bundled-libs.sh` из корня репо.
# Идемпотентный: пропускает уже скачанные/проверенные файлы.

set -euo pipefail

# ----- pin'ы версий и SHA256 ----------------------------------------------------------

ORT_VERSION="1.20.1"
ORT_URL="https://github.com/microsoft/onnxruntime/releases/download/v${ORT_VERSION}/onnxruntime-linux-x64-${ORT_VERSION}.tgz"
# SHA256 архива onnxruntime-linux-x64-1.20.1.tgz, посчитан с github.com/microsoft.
# При апгрейде ORT — обновить URL и hash; если не помнишь — поставь "TODO" и скрипт
# распечатает реальный при следующем запуске.
ORT_TGZ_SHA256="67db4dc1561f1e3fd42e619575c82c601ef89849afc7ea85a003abbac1a1a105"

VOSK_VERSION="0.3.45"
VOSK_URL="https://github.com/alphacep/vosk-api/releases/download/v${VOSK_VERSION}/vosk-linux-x86_64-${VOSK_VERSION}.zip"
VOSK_ZIP_SHA256="bbdc8ed85c43979f6443142889770ea95cbfbc56cffb5c5dcd73afa875c5fbb2"

# ----- пути ---------------------------------------------------------------------------

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ASSETS_LIBS="${REPO_ROOT}/assets/libs"
TMP_DIR="${REPO_ROOT}/target/bundled-libs-tmp"

mkdir -p "${ASSETS_LIBS}" "${TMP_DIR}"

# ----- helpers ------------------------------------------------------------------------

log()  { printf '\033[32m[bundled-libs]\033[0m %s\n' "$*"; }
warn() { printf '\033[33m[bundled-libs]\033[0m %s\n' "$*" >&2; }
err()  { printf '\033[31m[bundled-libs]\033[0m %s\n' "$*" >&2; exit 1; }

verify_sha256() {
    local file="$1" expected="$2"
    local actual
    actual="$(sha256sum "$file" | awk '{print $1}')"
    if [[ "$expected" == "TODO" ]]; then
        warn "SHA256 pin для $(basename "$file") не задан. Реальный hash:"
        warn "  $actual"
        warn "Запинь его в начале скрипта перед production-сборкой."
        return 0
    fi
    if [[ "$actual" != "$expected" ]]; then
        err "SHA256 mismatch для $(basename "$file"): ожидался $expected, получен $actual"
    fi
    log "SHA256 OK: $(basename "$file")"
}

require_cmd() {
    command -v "$1" >/dev/null 2>&1 || err "Нужна утилита '$1' (sudo apt install $2)"
}

require_cmd curl curl
require_cmd unzip unzip
require_cmd tar tar
require_cmd sha256sum coreutils

# ----- 1. libonnxruntime-noavx.so -----------------------------------------------------

NOAVX_TARGET="${ASSETS_LIBS}/libonnxruntime-noavx.so"
if [[ ! -f "${NOAVX_TARGET}" ]]; then
    err "${NOAVX_TARGET} отсутствует. Этот файл должен быть закоммичен в git \
(self-build no-AVX onnxruntime 1.20.1, см. memory/n5095-onnxruntime.md как пересобрать)."
fi
log "libonnxruntime-noavx.so присутствует ($(du -h "${NOAVX_TARGET}" | awk '{print $1}'))"

# ----- 2. libonnxruntime-avx2.so (Microsoft prebuilt) ---------------------------------

AVX2_TARGET="${ASSETS_LIBS}/libonnxruntime-avx2.so"
if [[ -f "${AVX2_TARGET}" ]]; then
    log "libonnxruntime-avx2.so уже есть, пропускаю"
else
    log "Качаю Microsoft pre-built ORT v${ORT_VERSION}..."
    ORT_TGZ="${TMP_DIR}/onnxruntime-linux-x64-${ORT_VERSION}.tgz"
    curl -fL --output "${ORT_TGZ}" "${ORT_URL}"
    verify_sha256 "${ORT_TGZ}" "${ORT_TGZ_SHA256}"
    log "Распаковываю..."
    ORT_EXTRACT_DIR="${TMP_DIR}/ort-${ORT_VERSION}"
    rm -rf "${ORT_EXTRACT_DIR}"
    mkdir -p "${ORT_EXTRACT_DIR}"
    tar -xzf "${ORT_TGZ}" -C "${ORT_EXTRACT_DIR}" --strip-components=1
    cp "${ORT_EXTRACT_DIR}/lib/libonnxruntime.so.${ORT_VERSION}" "${AVX2_TARGET}"
    log "${AVX2_TARGET} готов ($(du -h "${AVX2_TARGET}" | awk '{print $1}'))"
fi

# ----- 3. libvosk.so (alphacep prebuilt) ----------------------------------------------

VOSK_TARGET="${ASSETS_LIBS}/libvosk.so"
if [[ -f "${VOSK_TARGET}" ]]; then
    log "libvosk.so уже есть, пропускаю"
else
    log "Качаю vosk pre-built v${VOSK_VERSION}..."
    VOSK_ZIP="${TMP_DIR}/vosk-linux-x86_64-${VOSK_VERSION}.zip"
    curl -fL --output "${VOSK_ZIP}" "${VOSK_URL}"
    verify_sha256 "${VOSK_ZIP}" "${VOSK_ZIP_SHA256}"
    log "Распаковываю..."
    VOSK_EXTRACT_DIR="${TMP_DIR}/vosk-${VOSK_VERSION}"
    rm -rf "${VOSK_EXTRACT_DIR}"
    mkdir -p "${VOSK_EXTRACT_DIR}"
    unzip -q "${VOSK_ZIP}" -d "${VOSK_EXTRACT_DIR}"
    cp "${VOSK_EXTRACT_DIR}/vosk-linux-x86_64-${VOSK_VERSION}/libvosk.so" "${VOSK_TARGET}"
    log "${VOSK_TARGET} готов ($(du -h "${VOSK_TARGET}" | awk '{print $1}'))"
fi

# ----- summary ------------------------------------------------------------------------

log "Все нативные библиотеки готовы в ${ASSETS_LIBS}:"
ls -lh "${ASSETS_LIBS}" | tail -n +2
