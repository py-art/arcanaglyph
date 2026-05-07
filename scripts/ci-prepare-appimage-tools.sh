#!/usr/bin/env bash
#
# scripts/ci-prepare-appimage-tools.sh
#
# Готовит AppImage-инструменты в `~/.cache/tauri/appimage/` так, чтобы Tauri
# bundler нашёл их **уже распакованными** и не пытался запустить как AppImage
# (через FUSE-mount). На GitHub Actions FUSE недоступен в transitive subprocess'ах
# даже с APPIMAGE_EXTRACT_AND_RUN=1, поэтому стандартный путь Tauri AppImage
# bundler'а падает на этапе linuxdeploy.
#
# Что делаем для каждого AppImage-инструмента:
#   1. Скачиваем .AppImage с того же URL, откуда его берёт Tauri.
#   2. Распаковываем через `--appimage-extract` → squashfs-root/.
#   3. Переименовываем в <tool>.dir/.
#   4. Кладём вместо <tool>.AppImage shell-shim, который exec-нет AppRun
#      из распакованного AppDir. Tauri запустит shim как обычный exec — никакого FUSE.
#
# Plugin'ы .sh — просто скачиваются как есть (они и так shell-скрипты).
#
# Используется только в CI (release.yml). Локально не нужен — там FUSE
# всегда доступен и Tauri AppImage bundler работает напрямую.

set -euo pipefail

CACHE_DIR="${HOME}/.cache/tauri/appimage"
mkdir -p "${CACHE_DIR}"
cd "${CACHE_DIR}"

log() { printf '\033[32m[ci-appimage-tools]\033[0m %s\n' "$*"; }

# AppImage-инструменты: распакованные AppDir + shell-shim.
prepare_appimage() {
    local name="$1" url="$2"
    log "Подготавливаю ${name}…"
    curl -fsSL -o "${name}.real" "${url}"
    chmod +x "${name}.real"
    rm -rf squashfs-root
    "./${name}.real" --appimage-extract >/dev/null
    rm "${name}.real"
    rm -rf "${name}.dir"
    mv squashfs-root "${name}.dir"
    cat > "${name}" <<EOF
#!/usr/bin/env bash
# Shim: запускает распакованный AppDir вместо AppImage (FUSE не нужен).
exec "\$(dirname "\$(readlink -f "\$0")")/${name}.dir/AppRun" "\$@"
EOF
    chmod +x "${name}"
}

prepare_appimage "linuxdeploy-x86_64.AppImage" \
    "https://github.com/tauri-apps/binary-releases/releases/download/linuxdeploy/linuxdeploy-x86_64.AppImage"

prepare_appimage "linuxdeploy-plugin-appimage-x86_64.AppImage" \
    "https://github.com/linuxdeploy/linuxdeploy-plugin-appimage/releases/download/continuous/linuxdeploy-plugin-appimage-x86_64.AppImage"

# AppRun-x86_64 — это уже обычный ELF, не AppImage. Просто кладём.
log "Скачиваю AppRun-x86_64…"
curl -fsSL -o AppRun-x86_64 \
    https://github.com/tauri-apps/binary-releases/releases/download/apprun-old/AppRun-x86_64
chmod +x AppRun-x86_64

# Plugin'ы — shell-скрипты с GitHub raw.
log "Скачиваю linuxdeploy-plugin-gtk.sh…"
curl -fsSL -o linuxdeploy-plugin-gtk.sh \
    https://raw.githubusercontent.com/tauri-apps/linuxdeploy-plugin-gtk/master/linuxdeploy-plugin-gtk.sh
chmod +x linuxdeploy-plugin-gtk.sh

log "Скачиваю linuxdeploy-plugin-gstreamer.sh…"
curl -fsSL -o linuxdeploy-plugin-gstreamer.sh \
    https://raw.githubusercontent.com/tauri-apps/linuxdeploy-plugin-gstreamer/master/linuxdeploy-plugin-gstreamer.sh
chmod +x linuxdeploy-plugin-gstreamer.sh

log "Готово. Содержимое ${CACHE_DIR}:"
ls -la "${CACHE_DIR}"
