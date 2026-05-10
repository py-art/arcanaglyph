#!/usr/bin/env bash
# install.sh — one-line installer for ArcanaGlyph (Linux x86_64).
#
# Usage:
#   curl -fsSL https://github.com/py-art/arcanaglyph/raw/main/install.sh | bash
#   curl -fsSL https://github.com/py-art/arcanaglyph/raw/main/install.sh | VERSION=1.7.2 bash
#
# Что делает:
#   1. Узнаёт URL последнего (или указанного через VERSION) релиза из GitHub API.
#   2. На Debian/Ubuntu (наличие apt+dpkg) ставит .deb через apt.
#   3. На остальных дистрах кладёт AppImage в ~/.local/bin/arcanaglyph и регистрирует
#      .desktop-файл в ~/.local/share/applications/.
#   4. Проверяет SHA256 каждого скачанного артефакта.
#
# Идемпотентность: повторный запуск перезаписывает старую установку — это и есть
# upgrade-механизм для текущей версии скриптов.

set -euo pipefail

REPO="py-art/arcanaglyph"
APP_NAME="arcanaglyph"
DEB_PKG_NAME="arcana-glyph"

# ----- helpers ------------------------------------------------------------------

c_red()   { printf '\033[31m%s\033[0m' "$*"; }
c_green() { printf '\033[32m%s\033[0m' "$*"; }
c_yellow(){ printf '\033[33m%s\033[0m' "$*"; }

info()  { printf '%s %s\n' "$(c_green '==>')"  "$*"; }
warn()  { printf '%s %s\n' "$(c_yellow 'warn')" "$*" >&2; }
err()   { printf '%s %s\n' "$(c_red 'error')"  "$*" >&2; exit 1; }

need() {
    command -v "$1" >/dev/null 2>&1 || err "'$1' is required but not installed."
}

# ----- preflight ----------------------------------------------------------------

case "$(uname -s)" in
    Linux) ;;
    Darwin) err "macOS is not supported yet. See https://github.com/${REPO}" ;;
    *) err "Unsupported OS: $(uname -s). Linux x86_64 only." ;;
esac

case "$(uname -m)" in
    x86_64|amd64) ;;
    *) err "Unsupported architecture: $(uname -m). Only x86_64 is supported." ;;
esac

need curl
need sha256sum
need install
need awk
need sed

# Временный каталог — глобальный, чтобы trap EXIT видел его в любой ветке.
TMP_DIR=""
cleanup() { [ -n "${TMP_DIR}" ] && rm -rf "${TMP_DIR}"; }
trap cleanup EXIT

# ----- resolve release URLs -----------------------------------------------------

if [ -n "${VERSION:-}" ]; then
    TAG="v${VERSION#v}"
    API_URL="https://api.github.com/repos/${REPO}/releases/tags/${TAG}"
    info "Resolving release ${TAG}…"
else
    API_URL="https://api.github.com/repos/${REPO}/releases/latest"
    info "Resolving latest release…"
fi

# Без jq — простой парсинг "browser_download_url": "..." из JSON.
fetch_assets() {
    curl -fsSL \
        -H 'Accept: application/vnd.github+json' \
        -H 'X-GitHub-Api-Version: 2022-11-28' \
        "${API_URL}" \
        | sed -n 's/.*"browser_download_url": *"\([^"]*\)".*/\1/p'
}

ASSETS="$(fetch_assets)" || err "Failed to fetch release info from GitHub API."
[ -n "${ASSETS}" ] || err "No assets found in release. Check ${API_URL}"

DEB_URL="$(printf '%s\n' "${ASSETS}" | grep -E '\.deb$' | head -1 || true)"
APPIMAGE_URL="$(printf '%s\n' "${ASSETS}" | grep -Ei '\.appimage$' | head -1 || true)"
SUMS_URL="$(printf '%s\n' "${ASSETS}" | grep -E '/SHA256SUMS\.txt$' | head -1 || true)"

# ----- shared: SHA256 verification ----------------------------------------------

verify_sha256() {
    local file="$1" name="$2"
    [ -n "${SUMS_URL}" ] || { warn "SHA256SUMS.txt not found in release — skipping checksum"; return 0; }
    local sums
    sums="$(curl -fsSL "${SUMS_URL}")" || { warn "Failed to download SHA256SUMS.txt — skipping"; return 0; }
    local expected
    # SHA256SUMS.txt: "<hash>  deb/ArcanaGlyph_x.y.z_amd64.deb". Сравниваем basename($2),
    # без regex — чтобы точки/подчёркивания в имени не попали в metacharacter'ы.
    expected="$(printf '%s\n' "${sums}" | awk -v n="${name}" '
        {
            p = $2
            sub(/^.*\//, "", p)
            if (p == n) { print $1; exit }
        }
    ')"
    [ -n "${expected}" ] || { warn "Hash for ${name} not in SHA256SUMS.txt — skipping"; return 0; }
    local actual
    actual="$(sha256sum "${file}" | awk '{print $1}')"
    [ "${actual}" = "${expected}" ] || err "Checksum mismatch for ${name} (got ${actual}, expected ${expected})"
    info "SHA256 OK"
}

# ----- choose installer ---------------------------------------------------------

if command -v apt-get >/dev/null 2>&1 && command -v dpkg >/dev/null 2>&1; then
    INSTALLER=deb
elif [ -n "${APPIMAGE_URL}" ]; then
    INSTALLER=appimage
else
    err "This system has no apt and no AppImage was found in the release."
fi

# ----- .deb path ----------------------------------------------------------------

install_deb() {
    [ -n "${DEB_URL}" ] || err ".deb asset not found in release."
    TMP_DIR="$(mktemp -d)"
    # mktemp -d создаёт каталог 700. apt пытается уронить привилегии до user '_apt'
    # и не может прочитать .deb — падает с warning'ом. Открываем каталог на read+exec
    # для всех; сам .deb-файл ничего секретного не содержит.
    chmod 755 "${TMP_DIR}"
    local deb_name
    deb_name="$(basename "${DEB_URL}")"

    info "Downloading ${deb_name}…"
    curl -#fSL -o "${TMP_DIR}/${deb_name}" "${DEB_URL}"

    verify_sha256 "${TMP_DIR}/${deb_name}" "${deb_name}"

    info "Installing via apt (sudo password may be requested)…"
    if [ "${EUID:-$(id -u)}" -eq 0 ]; then
        apt-get install -y "${TMP_DIR}/${deb_name}"
    else
        sudo apt-get install -y "${TMP_DIR}/${deb_name}"
    fi

    info "$(c_green 'Installed.') Run: ${APP_NAME}"
}

# ----- AppImage path ------------------------------------------------------------

install_appimage() {
    [ -n "${APPIMAGE_URL}" ] || err "AppImage asset not found in release."

    if ! command -v fusermount >/dev/null 2>&1 && ! command -v fusermount3 >/dev/null 2>&1; then
        warn "FUSE tools not found. AppImage will fall back to extract-and-run mode."
        warn "For best results: sudo apt install libfuse2  (or your distro's equivalent)"
    fi

    if [ "${EUID:-$(id -u)}" -eq 0 ]; then
        warn "Running as root — installing into /root/.local/. Run as a regular user instead."
    fi

    local bin_dir="${HOME}/.local/bin"
    local apps_dir="${HOME}/.local/share/applications"
    local icon_dir="${HOME}/.local/share/icons/hicolor/256x256/apps"
    local target="${bin_dir}/${APP_NAME}"
    local appimage_name
    appimage_name="$(basename "${APPIMAGE_URL}")"

    mkdir -p "${bin_dir}" "${apps_dir}" "${icon_dir}"

    TMP_DIR="$(mktemp -d)"

    info "Downloading ${appimage_name}…"
    curl -#fSL -o "${TMP_DIR}/${appimage_name}" "${APPIMAGE_URL}"

    verify_sha256 "${TMP_DIR}/${appimage_name}" "${appimage_name}"

    install -m 0755 "${TMP_DIR}/${appimage_name}" "${target}"

    info "Extracting icon…"
    (
        cd "${TMP_DIR}"
        # AppImage в режиме --appimage-extract разворачивает payload в squashfs-root/.
        # Не падаем, если иконки нет внутри.
        APPIMAGE_EXTRACT_AND_RUN=1 "${target}" --appimage-extract '*.png' >/dev/null 2>&1 || true
        if [ -d squashfs-root ]; then
            local icon
            icon="$(find squashfs-root -maxdepth 2 -type f -name '*.png' | head -1 || true)"
            [ -n "${icon}" ] && cp "${icon}" "${icon_dir}/${APP_NAME}.png" || true
        fi
    )

    info "Writing .desktop entry…"
    cat > "${apps_dir}/${APP_NAME}.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=ArcanaGlyph
Comment=Voice input for Linux
Exec=${target}
Icon=${APP_NAME}
Categories=Utility;AudioVideo;
Terminal=false
StartupNotify=true
EOF

    command -v update-desktop-database >/dev/null 2>&1 \
        && update-desktop-database "${apps_dir}" 2>/dev/null || true

    case ":${PATH}:" in
        *":${bin_dir}:"*) ;;
        *)
            warn "${bin_dir} is not in your PATH."
            warn "Add this to ~/.bashrc or ~/.zshrc:"
            warn "    export PATH=\"\$HOME/.local/bin:\$PATH\""
            ;;
    esac

    info "$(c_green 'Installed.') Run: ${target}"
    info "Or from menu: ArcanaGlyph"
}

# ----- run ----------------------------------------------------------------------

case "${INSTALLER}" in
    deb)      install_deb ;;
    appimage) install_appimage ;;
esac

# ----- post-install: pre-grant XDG RemoteDesktop on Wayland ---------------------
# Wayland-only задача: при первом нажатии Ctrl+Ё внутри приложения GNOME
# показывает popup «Дать доступ» (XDG RemoteDesktop portal). Делаем popup
# здесь, в момент инсталляции, где пользователь ожидает диалогов. Best-effort:
# не падаем, если запустить не получилось (нет DISPLAY, нет binary в PATH,
# X11-сессия и т.д.) — приложение попросит разрешение из UI самостоятельно.
if [ "${XDG_SESSION_TYPE:-}" = "wayland" ] && command -v "${APP_NAME}" >/dev/null 2>&1; then
    if command -v notify-send >/dev/null 2>&1; then
        notify-send "ArcanaGlyph" "Сейчас система спросит разрешение на ввод текста — нажмите «Дать доступ»." 2>/dev/null || true
    fi
    info "Requesting XDG RemoteDesktop permission (popup может появиться)…"
    "${APP_NAME}" --grant-portal 2>&1 | sed 's/^/    /' || \
        warn "Pre-grant не удался — приложение попросит разрешение из UI при первом запуске."
fi

cat <<EOF

Tip: enable autostart from in-app Settings to launch ArcanaGlyph at login.

To uninstall later:
    curl -fsSL https://github.com/${REPO}/raw/main/uninstall.sh | bash

To remove user data too (config, models, cache):
    curl -fsSL https://github.com/${REPO}/raw/main/uninstall.sh | bash -s -- --purge
EOF
