#!/usr/bin/env bash
# uninstall.sh — uninstaller for ArcanaGlyph.
#
# Usage:
#   curl -fsSL https://github.com/py-art/arcanaglyph/raw/main/uninstall.sh | bash
#   curl -fsSL https://github.com/py-art/arcanaglyph/raw/main/uninstall.sh | bash -s -- --purge
#
# По умолчанию удаляет только бинарь и .desktop. С --purge также сносит
# ~/.config/arcanaglyph/, ~/.local/share/arcanaglyph/, ~/.cache/arcanaglyph/.

set -euo pipefail

APP_NAME="arcanaglyph"
DEB_PKG_NAME="arcana-glyph"

PURGE=0
for arg in "$@"; do
    case "${arg}" in
        --purge|-p) PURGE=1 ;;
        --help|-h)
            cat <<EOF
Usage: uninstall.sh [--purge]

  --purge   Also remove user data: ~/.config/arcanaglyph/,
            ~/.local/share/arcanaglyph/, ~/.cache/arcanaglyph/.
EOF
            exit 0
            ;;
        *) echo "Unknown argument: ${arg}" >&2; exit 1 ;;
    esac
done

c_green() { printf '\033[32m%s\033[0m' "$*"; }
c_yellow(){ printf '\033[33m%s\033[0m' "$*"; }

info() { printf '%s %s\n' "$(c_green '==>')"  "$*"; }
warn() { printf '%s %s\n' "$(c_yellow 'warn')" "$*" >&2; }

# ----- stop running app ---------------------------------------------------------

# Wrapper /usr/bin/arcanaglyph — это sh-скрипт, который exec-ит реальный бинарь
# /usr/lib/arcanaglyph/arcanaglyph-{avx,noavx}. После exec имя процесса в /proc/<pid>/comm
# становится именем реального бинаря (с обрезкой до 15 символов: arcanaglyph-noavx → arcanaglyph-noa).
# Поэтому pgrep -x по 'arcanaglyph' не находит запущенный процесс. Используем -f с regex
# по концу cmdline — точный матч на arcanaglyph-avx / arcanaglyph-noavx / wrapper / AppImage.
# Делаем это до detection, чтобы прибить зомби-процесс даже когда файлы пакета уже удалены.
KILL_PATTERN='arcanaglyph(-avx|-noavx)?$'
if pgrep -f "${KILL_PATTERN}" >/dev/null 2>&1; then
    info "Stopping running ArcanaGlyph…"
    pkill -f "${KILL_PATTERN}" 2>/dev/null || true
    sleep 1
    pkill -9 -f "${KILL_PATTERN}" 2>/dev/null || true
fi

# ----- detect installation type -------------------------------------------------

INSTALLED_DEB=0
INSTALLED_APPIMAGE=0

if command -v dpkg >/dev/null 2>&1 && dpkg-query -W -f='${Status}' "${DEB_PKG_NAME}" 2>/dev/null | grep -q '^install ok installed$'; then
    INSTALLED_DEB=1
fi

APPIMAGE_PATH="${HOME}/.local/bin/${APP_NAME}"
DESKTOP_FILE="${HOME}/.local/share/applications/${APP_NAME}.desktop"
ICON_GLOB="${HOME}/.local/share/icons/hicolor/*/apps/${APP_NAME}.png"

if [ -e "${APPIMAGE_PATH}" ] || [ -e "${DESKTOP_FILE}" ]; then
    INSTALLED_APPIMAGE=1
fi

if [ "${INSTALLED_DEB}" -eq 0 ] && [ "${INSTALLED_APPIMAGE}" -eq 0 ]; then
    info "ArcanaGlyph is not installed. Nothing to do."
    exit 0
fi

# ----- remove .deb --------------------------------------------------------------

if [ "${INSTALLED_DEB}" -eq 1 ]; then
    info "Removing system package ${DEB_PKG_NAME} (sudo password may be requested)…"
    if [ "${EUID:-$(id -u)}" -eq 0 ]; then
        apt-get remove -y "${DEB_PKG_NAME}"
    else
        sudo apt-get remove -y "${DEB_PKG_NAME}"
    fi
fi

# ----- remove AppImage ----------------------------------------------------------

if [ "${INSTALLED_APPIMAGE}" -eq 1 ]; then
    info "Removing AppImage and desktop entry…"
    rm -f "${APPIMAGE_PATH}"
    rm -f "${DESKTOP_FILE}"
    # shellcheck disable=SC2086 # глоб должен раскрываться
    rm -f ${ICON_GLOB}

    command -v update-desktop-database >/dev/null 2>&1 \
        && update-desktop-database "${HOME}/.local/share/applications" 2>/dev/null || true
fi

# ----- purge user data ----------------------------------------------------------

CONFIG_DIR="${XDG_CONFIG_HOME:-${HOME}/.config}/${APP_NAME}"
DATA_DIR="${XDG_DATA_HOME:-${HOME}/.local/share}/${APP_NAME}"
CACHE_DIR="${XDG_CACHE_HOME:-${HOME}/.cache}/${APP_NAME}"

if [ "${PURGE}" -eq 1 ]; then
    info "Purging user data…"
    rm -rf "${CONFIG_DIR}" "${DATA_DIR}" "${CACHE_DIR}"
    info "$(c_green 'Done.') ArcanaGlyph and all user data removed."
else
    info "$(c_green 'Done.') Binary removed."
    cat <<EOF

User data was kept:
  ${CONFIG_DIR}/    (settings, database)
  ${DATA_DIR}/      (downloaded models — could be hundreds of MB)
  ${CACHE_DIR}/     (audio cache)

To remove these too, re-run with --purge:
  curl -fsSL https://github.com/py-art/arcanaglyph/raw/main/uninstall.sh | bash -s -- --purge
EOF
fi
