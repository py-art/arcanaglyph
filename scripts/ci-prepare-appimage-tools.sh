#!/usr/bin/env bash
#
# scripts/ci-prepare-appimage-tools.sh
#
# Готовит AppImage-инструменты в `~/.cache/tauri/` (точный путь Tauri-bundler'а v2,
# см. crates/tauri-bundler/src/bundle/linux/appimage/linuxdeploy.rs:
#   tools_path = dirs::cache_dir().join("tauri")
# Файлы по точным именам, которые ожидает prepare_tools()):
#
#   AppRun-x86_64                              — обычный ELF (не AppImage), просто скачиваем.
#   linuxdeploy-x86_64.AppImage                — оставляем как AppImage. Tauri запускает его
#                                                с APPIMAGE_EXTRACT_AND_RUN=1 (Phase 3 build-deb.sh
#                                                выставляет). Кроме того, Tauri делает dd seek=8
#                                                count=3 поверх него (стирает AppImage magic) —
#                                                shim'у это сломает shebang.
#   linuxdeploy-plugin-gtk.sh                  — shell-скрипт, скачиваем как есть.
#   linuxdeploy-plugin-gstreamer.sh            — shell-скрипт, скачиваем как есть.
#   linuxdeploy-plugin-appimage.AppImage       — ВОТ это критично: linuxdeploy форкает его как
#                                                child-процесс, который не наследует
#                                                APPIMAGE_EXTRACT_AND_RUN. Ему-то и нужен FUSE.
#                                                Подменяем на shell-shim, который exec-нет
#                                                распакованный AppRun. dd на этот файл не пишет,
#                                                поэтому shim сохранится.
#
# Используется только в CI (release.yml). Локально не нужен — там FUSE доступен.

set -euo pipefail

CACHE_DIR="${HOME}/.cache/tauri"
mkdir -p "${CACHE_DIR}"
cd "${CACHE_DIR}"

log() { printf '\033[32m[ci-appimage-tools]\033[0m %s\n' "$*"; }

# AppRun-x86_64: обычный ELF.
log "Скачиваю AppRun-x86_64…"
curl -fsSL -o AppRun-x86_64 \
    https://github.com/tauri-apps/binary-releases/releases/download/apprun-old/AppRun-x86_64
chmod +x AppRun-x86_64

# linuxdeploy: оставляем как AppImage (Tauri патчит его dd-ом, shim бы испортился).
# С APPIMAGE_EXTRACT_AND_RUN=1 он сам распакуется и запустит свой AppRun.
log "Скачиваю linuxdeploy-x86_64.AppImage…"
curl -fsSL -o linuxdeploy-x86_64.AppImage \
    https://github.com/tauri-apps/binary-releases/releases/download/linuxdeploy/linuxdeploy-x86_64.AppImage
chmod +x linuxdeploy-x86_64.AppImage

# Plugin'ы — shell-скрипты с GitHub raw, как есть.
log "Скачиваю linuxdeploy-plugin-gtk.sh…"
curl -fsSL -o linuxdeploy-plugin-gtk.sh \
    https://raw.githubusercontent.com/tauri-apps/linuxdeploy-plugin-gtk/master/linuxdeploy-plugin-gtk.sh
chmod +x linuxdeploy-plugin-gtk.sh

log "Скачиваю linuxdeploy-plugin-gstreamer.sh…"
curl -fsSL -o linuxdeploy-plugin-gstreamer.sh \
    https://raw.githubusercontent.com/tauri-apps/linuxdeploy-plugin-gstreamer/master/linuxdeploy-plugin-gstreamer.sh
chmod +x linuxdeploy-plugin-gstreamer.sh

# linuxdeploy-plugin-appimage.AppImage: КЛЮЧЕВОЙ файл — подменяем на shim.
# linuxdeploy запускает его без forwarding'а APPIMAGE_EXTRACT_AND_RUN, поэтому
# в CI он падает на FUSE. Распаковываем заранее и подменяем на bash-shim.
# Имя файла без -<arch>, как ожидает Tauri (см. prepare_tools).
log "Подготавливаю linuxdeploy-plugin-appimage.AppImage (shim без FUSE)…"
curl -fsSL -o plugin-appimage.real \
    https://github.com/linuxdeploy/linuxdeploy-plugin-appimage/releases/download/continuous/linuxdeploy-plugin-appimage-x86_64.AppImage
chmod +x plugin-appimage.real
rm -rf squashfs-root linuxdeploy-plugin-appimage.AppImage.dir
./plugin-appimage.real --appimage-extract >/dev/null
rm plugin-appimage.real
mv squashfs-root linuxdeploy-plugin-appimage.AppImage.dir
cat > linuxdeploy-plugin-appimage.AppImage <<'EOF'
#!/usr/bin/env bash
# Shim: запускает распакованный plugin-appimage AppDir вместо AppImage (без FUSE).
exec "$(dirname "$(readlink -f "$0")")/linuxdeploy-plugin-appimage.AppImage.dir/AppRun" "$@"
EOF
chmod +x linuxdeploy-plugin-appimage.AppImage

log "Готово. Содержимое ${CACHE_DIR}:"
ls -la "${CACHE_DIR}" | grep -v '^total'
