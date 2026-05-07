#!/usr/bin/env bash
#
# scripts/build-deb.sh
#
# Собирает self-contained `.deb` И `.AppImage` пакеты ArcanaGlyph, которые
# работают на любом x86_64 Linux (AVX и без AVX) сразу после `dpkg -i` /
# `chmod +x`, без ручной настройки.
#
# Что делает:
#   1. Готовит pre-built нативные библиотеки в assets/libs/
#      (см. scripts/prepare-bundled-libs.sh).
#   2. Собирает бинарь arcanaglyph-avx с whisper.cpp, скомпилированным с AVX/AVX2.
#   3. Собирает бинарь arcanaglyph-noavx с whisper.cpp без AVX (-mno-avx*).
#   4. Запускает `cargo tauri build --bundles deb,appimage` (Tauri бандлит
#      noavx-вариант как /usr/bin/arcanaglyph внутри обоих образов; post-process
#      ниже добавляет туда avx-вариант, либы и wrapper).
#   5. Post-process .deb: разворачивает через dpkg-deb -R, добавляет:
#        /usr/bin/arcanaglyph                            <- shell wrapper
#        /usr/lib/arcanaglyph/arcanaglyph-noavx          <- (был /usr/bin/arcanaglyph)
#        /usr/lib/arcanaglyph/arcanaglyph-avx            <- собран в шаге 2
#        /usr/lib/arcanaglyph/libonnxruntime-avx2.so     <- Microsoft prebuilt 1.20.1
#        /usr/lib/arcanaglyph/libonnxruntime-noavx.so    <- наш self-build 1.20.1
#        /usr/lib/arcanaglyph/libvosk.so                 <- alphacep prebuilt 0.3.45
#      Обновляет Installed-Size в DEBIAN/control. Перепакует через dpkg-deb -b.
#   6. Post-process .AppImage: --appimage-extract разворачивает squashfs,
#      повторяет ту же раскладку (но в `${APPDIR}/usr/lib/arcanaglyph/`),
#      ставит AppRun-wrapper, который выставляет ORT_DYLIB_PATH и LD_LIBRARY_PATH
#      на bundled-копии перед exec'ом нужного бинаря. Перепакует через appimagetool.
#
# Запуск: `bash scripts/build-deb.sh` из корня репо (или `make dist`).
# Требует: cargo, cargo-tauri, dpkg-deb, GNU coreutils. Для AppImage post-process
# нужен appimagetool — скачивается автоматически в target/ если отсутствует.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${REPO_ROOT}"

log()  { printf '\033[32m[build-deb]\033[0m %s\n' "$*"; }
warn() { printf '\033[33m[build-deb]\033[0m %s\n' "$*" >&2; }
err()  { printf '\033[31m[build-deb]\033[0m %s\n' "$*" >&2; exit 1; }

# Все 4 движка одновременно. ORT-фича — load-dynamic (gigaam-system-ort);
# qwen3asr пользуется тем же ort'ом.
APP_FEATURES='gigaam-system-ort,vosk,whisper,qwen3asr'

# whisper.cpp env-флаги для двух вариантов сборки.
#
# Ключевая проблема: ggml (внутри whisper.cpp) использует опции вида `GGML_AVX`,
# `GGML_FMA`, `GGML_F16C`, `GGML_NATIVE`. whisper.cpp в CMakeLists транслирует только
# `WHISPER_NATIVE`/`WHISPER_CUDA`/etc в `GGML_*` (см. `whisper_option_depr`), но
# **не транслирует** `WHISPER_AVX`/`WHISPER_AVX2`/`WHISPER_FMA`/`WHISPER_F16C`.
# При этом whisper-rs-sys/build.rs форвардит env только с префиксом `WHISPER_*` или
# `CMAKE_*` — `GGML_*` напрямую не пробрасываются.
#
# Решение: влиять на ggml через `CMAKE_C_FLAGS_RELEASE` / `CMAKE_CXX_FLAGS_RELEASE`
# (cmake форвардит) — добавляем `-mavx ...` или `-mno-avx ...` в release-конфиг
# флаги. CMAKE_C_FLAGS (без _RELEASE) уже занят cmake-rs'ом (-ffunction-sections и т.п.),
# поэтому пишем именно в _RELEASE — он аддитивен к базовому CMAKE_C_FLAGS.
#
# AVX-вариант: на AVX-build-host `GGML_NATIVE=ON` (дефолт) дёргает `-march=native`
# и сам подхватывает AVX/AVX2/FMA/F16C. На no-AVX-build-host (N5095) `-march=native`
# даёт no-AVX — но мы дополняем явными `-mavx -mavx2 -mfma -mf16c`, чтобы
# скомпилировать AVX-инструкции даже на N5095-host'е (они выполняются только на
# AVX-CPU, на N5095 после установки .deb wrapper выберет noavx-binary).
#
# noAVX-вариант: добавляем `-mno-avx -mno-avx2 -mno-avx512f -mno-fma -mno-f16c` —
# эти флаги отменяют любое включение AVX, в т.ч. через `-march=native`. Гарантирует
# что noavx-binary не падает SIGILL'ом на CPU без AVX.
AVX_WHISPER_ENV=(
    "CMAKE_C_FLAGS_RELEASE=-O3 -DNDEBUG -mavx -mavx2 -mfma -mf16c"
    "CMAKE_CXX_FLAGS_RELEASE=-O3 -DNDEBUG -mavx -mavx2 -mfma -mf16c"
)
NOAVX_WHISPER_ENV=(
    "CMAKE_C_FLAGS_RELEASE=-O3 -DNDEBUG -mno-avx -mno-avx2 -mno-avx512f -mno-fma -mno-f16c"
    "CMAKE_CXX_FLAGS_RELEASE=-O3 -DNDEBUG -mno-avx -mno-avx2 -mno-avx512f -mno-fma -mno-f16c"
)

# ----- 0. Удаляем артефакты ровно текущей версии ---------------------------------------
# Tauri bundler обычно перезаписывает свой выход, но это не гарантировано — особенно
# если предыдущая сборка упала на середине post-process. Поэтому удаляем артефакты
# текущей версии перед сборкой, чтобы Phase 4-6 точно работали со свежими файлами.
# Артефакты прошлых версий (ArcanaGlyph_1.5.0_*, 1.6.0_* и т.д.) НЕ трогаем — пусть
# лежат локально как архив, чистятся через `make clean`.
CURRENT_VERSION="$(grep '"version"' "${REPO_ROOT}/crates/arcanaglyph-app/tauri.conf.json" | head -1 | sed 's/.*"version": *"//;s/".*//')"
[[ -n "${CURRENT_VERSION}" ]] || err "Не удалось вычитать версию из tauri.conf.json"
log "Phase 0/6: removing prior artifacts of v${CURRENT_VERSION} (older versions kept)"
# Удаляем не только готовые файлы, но и распакованные рабочие каталоги Tauri
# (ArcanaGlyph_<ver>_amd64/ рядом с .deb — там лежит структура до упаковки dpkg-deb,
# в appimage/ — аналогичный AppDir). Если оставить — Tauri может переиспользовать
# устаревшее содержимое или конфликтовать при перепаковке. Wildcard ловит и файл,
# и каталог одной командой; артефакты других версий (с другим префиксом) не трогает.
rm -rf "${REPO_ROOT}/target/release/bundle/deb/ArcanaGlyph_${CURRENT_VERSION}_"*
rm -rf "${REPO_ROOT}/target/release/bundle/appimage/ArcanaGlyph_${CURRENT_VERSION}_"*

# ----- 1. Pre-built нативные либы -----------------------------------------------------

log "Phase 1/6: prepare-bundled-libs"
bash "${REPO_ROOT}/scripts/prepare-bundled-libs.sh"

# Линкер vosk-rs ищет libvosk.so по стандартным путям. Прокидываем assets/libs/ в
# LIBRARY_PATH чтобы линковка прошла без /usr/local/lib/libvosk.so.
export LIBRARY_PATH="${REPO_ROOT}/assets/libs:${LIBRARY_PATH:-}"
# При самом запуске бинаря (тестовом, до .deb) аналогично — иначе loader не найдёт.
# (Cargo сам выставляет LD_LIBRARY_PATH для cargo run/test, но мы пакуем; в .deb
# loader найдёт через RUNPATH=/usr/lib/arcanaglyph.)

# ----- 2. arcanaglyph-avx (whisper с AVX) ---------------------------------------------

log "Phase 2/6: cargo build arcanaglyph-avx (whisper с AVX)"
AVX_TARGET_DIR="${REPO_ROOT}/target/avx-build"
# Принудительная чистка whisper-rs-sys и зависимых .rmeta — без этого cargo
# не пересоберёт whisper.cpp с GGML_NATIVE=OFF + GGML_AVX=ON флагами
# (build.rs не имеет cargo:rerun-if-env-changed=GGML_*).
rm -rf "${AVX_TARGET_DIR}/release/build/whisper-rs-sys-"* \
       "${AVX_TARGET_DIR}/release/deps/libwhisper_rs_sys-"* \
       "${AVX_TARGET_DIR}/release/deps/whisper_rs_sys-"* \
       "${AVX_TARGET_DIR}/release/deps/libwhisper_rs-"* \
       "${AVX_TARGET_DIR}/release/deps/whisper_rs-"* \
       "${AVX_TARGET_DIR}/release/deps/libarcanaglyph_core-"* \
       "${AVX_TARGET_DIR}/release/deps/arcanaglyph_core-"* \
       "${AVX_TARGET_DIR}/release/deps/libarcanaglyph_app-"* \
       "${AVX_TARGET_DIR}/release/deps/arcanaglyph_app-"* \
       "${AVX_TARGET_DIR}/release/arcanaglyph" 2>/dev/null || true
env "${AVX_WHISPER_ENV[@]}" \
    CARGO_TARGET_DIR="${AVX_TARGET_DIR}" \
    cargo build --release -p arcanaglyph-app \
        --no-default-features --features "${APP_FEATURES}"
AVX_BINARY="${AVX_TARGET_DIR}/release/arcanaglyph"
[[ -f "${AVX_BINARY}" ]] || err "AVX-бинарь не собрался: ${AVX_BINARY}"
log "arcanaglyph-avx собран ($(du -h "${AVX_BINARY}" | awk '{print $1}'))"

# ----- 3. arcanaglyph-noavx + .deb (через cargo tauri build) --------------------------

log "Phase 3/6: cargo tauri build (noavx, whisper без AVX, .deb + .AppImage)"
# Принудительная чистка whisper-rs-sys из target/release/. cargo clean -p не работает
# для transitive deps (выдаёт "Removed 0 files"). А whisper-rs-sys/build.rs не объявляет
# cargo:rerun-if-env-changed=WHISPER_* — без удаления build artifacts cmake возьмёт
# старый CMakeCache.txt с прошлыми WHISPER_AVX/AVX2/FMA/F16C флагами.
# Также нужно удалить .rmeta/.rlib бинаря-арт (deps/libwhisper_rs_sys-*) и сам бинарь
# приложения, чтобы cargo пересобрал зависимую цепочку.
rm -rf "${REPO_ROOT}/target/release/build/whisper-rs-sys-"* \
       "${REPO_ROOT}/target/release/deps/libwhisper_rs_sys-"* \
       "${REPO_ROOT}/target/release/deps/whisper_rs_sys-"* \
       "${REPO_ROOT}/target/release/deps/libwhisper_rs-"* \
       "${REPO_ROOT}/target/release/deps/whisper_rs-"* \
       "${REPO_ROOT}/target/release/deps/libarcanaglyph_core-"* \
       "${REPO_ROOT}/target/release/deps/arcanaglyph_core-"* \
       "${REPO_ROOT}/target/release/deps/libarcanaglyph_app-"* \
       "${REPO_ROOT}/target/release/deps/arcanaglyph_app-"* \
       "${REPO_ROOT}/target/release/arcanaglyph"
# Собираем оба бандла за один проход. Tauri разделяет cargo build (один) и
# собственно bundlers (deb / appimage параллельно по одному и тому же бинарю).
# `cargo tauri build` сам не имеет флага `--no-default-features` — пробрасываем
# его через `-- ARGS` в нижележащий cargo. `--features` Tauri знает и форвардит сам.
# APPIMAGE_EXTRACT_AND_RUN=1 — fallback на распаковку, если в системе нет рабочего
# FUSE (Tauri использует appimagetool/linuxdeploy, оба сами AppImage).
env "${NOAVX_WHISPER_ENV[@]}" \
    APPIMAGE_EXTRACT_AND_RUN=1 \
    cargo tauri build --bundles deb,appimage --features "${APP_FEATURES}" -- --no-default-features

# ----- 4. Локализуем сгенерированный .deb ---------------------------------------------

DEB_DIR="${REPO_ROOT}/target/release/bundle/deb"
# Точное имя по версии из tauri.conf.json. Раньше брали `ls *.deb | head -1`,
# но это даёт неверный файл если в директории остался .deb старой версии:
# `ls` возвращает алфавитно отсортированный список → первой попадается старая
# версия (например 1.5.0 при пересборке 1.6.0), post-process залезает в неё,
# а свежесобранный .deb для текущей версии остаётся без обработки и потом
# apt install ставит broken пакет (без bundled libs и wrapper'а).
VERSION="$(grep '"version"' "${REPO_ROOT}/crates/arcanaglyph-app/tauri.conf.json" | head -1 | sed 's/.*"version": *"//;s/".*//')"
[[ -n "${VERSION}" ]] || err "Не удалось вычитать версию из tauri.conf.json"
DEB_FILE="${DEB_DIR}/ArcanaGlyph_${VERSION}_amd64.deb"
[[ -f "${DEB_FILE}" ]] || err "Не нашёл .deb по ожидаемому пути: ${DEB_FILE}"
log "Phase 4/6: post-process ${DEB_FILE}"

# ----- 5. Расширяем .deb: добавляем avx-бинарь, либы, wrapper -------------------------

EXTRACT_DIR="${REPO_ROOT}/target/deb-extract"
rm -rf "${EXTRACT_DIR}"
mkdir -p "${EXTRACT_DIR}"
dpkg-deb -R "${DEB_FILE}" "${EXTRACT_DIR}"

# Tauri положил основной бинарь как /usr/bin/arcanaglyph — это наш noavx-вариант.
# Перемещаем его в /usr/lib/arcanaglyph/arcanaglyph-noavx.
mkdir -p "${EXTRACT_DIR}/usr/lib/arcanaglyph"
[[ -f "${EXTRACT_DIR}/usr/bin/arcanaglyph" ]] || err "В .deb нет /usr/bin/arcanaglyph (структура изменилась?)"
mv "${EXTRACT_DIR}/usr/bin/arcanaglyph" "${EXTRACT_DIR}/usr/lib/arcanaglyph/arcanaglyph-noavx"

# Кладём avx-вариант рядом.
cp "${AVX_BINARY}" "${EXTRACT_DIR}/usr/lib/arcanaglyph/arcanaglyph-avx"

# Кладём нативные либы.
cp "${REPO_ROOT}/assets/libs/libonnxruntime-avx2.so" "${EXTRACT_DIR}/usr/lib/arcanaglyph/libonnxruntime-avx2.so"
cp "${REPO_ROOT}/assets/libs/libonnxruntime-noavx.so" "${EXTRACT_DIR}/usr/lib/arcanaglyph/libonnxruntime-noavx.so"
cp "${REPO_ROOT}/assets/libs/libvosk.so" "${EXTRACT_DIR}/usr/lib/arcanaglyph/libvosk.so"

# Кладём wrapper-script на /usr/bin/arcanaglyph.
cp "${REPO_ROOT}/assets/scripts/arcanaglyph-wrapper.sh" "${EXTRACT_DIR}/usr/bin/arcanaglyph"
chmod 755 "${EXTRACT_DIR}/usr/bin/arcanaglyph"
chmod 755 "${EXTRACT_DIR}/usr/lib/arcanaglyph/arcanaglyph-avx"
chmod 755 "${EXTRACT_DIR}/usr/lib/arcanaglyph/arcanaglyph-noavx"
chmod 644 "${EXTRACT_DIR}/usr/lib/arcanaglyph/"libonnxruntime-*.so
chmod 644 "${EXTRACT_DIR}/usr/lib/arcanaglyph/libvosk.so"

# Кладём GNOME Shell extension для позиционирования виджета на Wayland.
# Пользователь включает его через UI настроек — приложение копирует
# /usr/share/arcanaglyph/extension/<uuid>/ → ~/.local/share/gnome-shell/extensions/<uuid>/
# и активирует через `gnome-extensions enable`.
WIDGET_EXT_UUID="arcanaglyph-widget@arfi.tech"
WIDGET_EXT_SRC="${REPO_ROOT}/extension/${WIDGET_EXT_UUID}"
WIDGET_EXT_DST="${EXTRACT_DIR}/usr/share/arcanaglyph/extension/${WIDGET_EXT_UUID}"
if [[ -d "${WIDGET_EXT_SRC}" ]]; then
    mkdir -p "${WIDGET_EXT_DST}"
    cp "${WIDGET_EXT_SRC}/metadata.json" "${WIDGET_EXT_DST}/"
    cp "${WIDGET_EXT_SRC}/extension.js" "${WIDGET_EXT_DST}/"
    if [[ -d "${WIDGET_EXT_SRC}/schemas" ]]; then
        mkdir -p "${WIDGET_EXT_DST}/schemas"
        cp "${WIDGET_EXT_SRC}/schemas/"*.gschema.xml "${WIDGET_EXT_DST}/schemas/"
        # Перекомпилируем gschemas из чистого XML — версия glib на машине пользователя
        # может отличаться от той, что собирала закоммиченный gschemas.compiled.
        if command -v glib-compile-schemas >/dev/null 2>&1; then
            glib-compile-schemas "${WIDGET_EXT_DST}/schemas/"
        fi
    fi
    chmod 644 "${WIDGET_EXT_DST}/metadata.json"
    chmod 644 "${WIDGET_EXT_DST}/extension.js"
    [[ -f "${WIDGET_EXT_DST}/schemas/gschemas.compiled" ]] && chmod 644 "${WIDGET_EXT_DST}/schemas/gschemas.compiled"
    [[ -f "${WIDGET_EXT_DST}/schemas/"*.gschema.xml ]] && chmod 644 "${WIDGET_EXT_DST}/schemas/"*.gschema.xml
    log "Расширение виджета упаковано: ${WIDGET_EXT_DST}"
else
    warn "Расширение виджета не найдено в ${WIDGET_EXT_SRC}, пропускаю"
fi

# Обновляем Installed-Size в DEBIAN/control (в килобайтах). Иначе apt будет писать
# неверные цифры в `apt show` и при апгрейде/удалении.
NEW_SIZE_KB="$(du -sk "${EXTRACT_DIR}/usr" | awk '{print $1}')"
sed -i "s/^Installed-Size: .*/Installed-Size: ${NEW_SIZE_KB}/" "${EXTRACT_DIR}/DEBIAN/control"
log "Installed-Size обновлён: ${NEW_SIZE_KB} KB"

# Пересобираем .deb. Имя — то же, перезатираем.
log "Phase 5/6: dpkg-deb -b"
rm -f "${DEB_FILE}"
dpkg-deb --build --root-owner-group "${EXTRACT_DIR}" "${DEB_FILE}"

# ----- 6. Расширяем .AppImage аналогично .deb -----------------------------------------

APPIMAGE_DIR="${REPO_ROOT}/target/release/bundle/appimage"
# Tauri именует AppImage по productName и версии. Стандартный паттерн совпадает с .deb.
APPIMAGE_FILE="${APPIMAGE_DIR}/ArcanaGlyph_${VERSION}_amd64.AppImage"
if [[ ! -f "${APPIMAGE_FILE}" ]]; then
    # Fallback: ищем любой .AppImage в директории (Tauri иногда меняет шаблон).
    APPIMAGE_FILE="$(find "${APPIMAGE_DIR}" -maxdepth 1 -name '*.AppImage' -type f | head -1 || true)"
fi
[[ -n "${APPIMAGE_FILE}" && -f "${APPIMAGE_FILE}" ]] \
    || err "Не нашёл .AppImage в ${APPIMAGE_DIR} — Tauri appimage bundler упал?"
log "Phase 6/6: post-process ${APPIMAGE_FILE}"

APPIMAGE_EXTRACT_DIR="${REPO_ROOT}/target/appimage-extract"
rm -rf "${APPIMAGE_EXTRACT_DIR}"
mkdir -p "${APPIMAGE_EXTRACT_DIR}"
(
    cd "${APPIMAGE_EXTRACT_DIR}"
    "${APPIMAGE_FILE}" --appimage-extract >/dev/null
)
APPDIR="${APPIMAGE_EXTRACT_DIR}/squashfs-root"
[[ -d "${APPDIR}" ]] || err "appimage-extract не создал ${APPDIR}"

# Tauri положил основной бинарь как usr/bin/arcanaglyph — это наш noavx-вариант.
mkdir -p "${APPDIR}/usr/lib/arcanaglyph"
[[ -f "${APPDIR}/usr/bin/arcanaglyph" ]] \
    || err "В AppImage нет usr/bin/arcanaglyph (структура изменилась?)"
mv "${APPDIR}/usr/bin/arcanaglyph" "${APPDIR}/usr/lib/arcanaglyph/arcanaglyph-noavx"

# avx-вариант рядом.
cp "${AVX_BINARY}" "${APPDIR}/usr/lib/arcanaglyph/arcanaglyph-avx"

# Bundled нативные либы (как в .deb).
cp "${REPO_ROOT}/assets/libs/libonnxruntime-avx2.so" "${APPDIR}/usr/lib/arcanaglyph/libonnxruntime-avx2.so"
cp "${REPO_ROOT}/assets/libs/libonnxruntime-noavx.so" "${APPDIR}/usr/lib/arcanaglyph/libonnxruntime-noavx.so"
cp "${REPO_ROOT}/assets/libs/libvosk.so" "${APPDIR}/usr/lib/arcanaglyph/libvosk.so"

# Wrapper. Кладём как usr/bin/arcanaglyph; AppRun (symlink на usr/bin/arcanaglyph)
# Tauri уже создал — так что цепочка AppRun → usr/bin/arcanaglyph (наш wrapper) → exec
# на правильный бинарь сработает автоматически.
cp "${REPO_ROOT}/assets/scripts/arcanaglyph-appimage-AppRun.sh" "${APPDIR}/usr/bin/arcanaglyph"
chmod 755 "${APPDIR}/usr/bin/arcanaglyph"
chmod 755 "${APPDIR}/usr/lib/arcanaglyph/arcanaglyph-avx"
chmod 755 "${APPDIR}/usr/lib/arcanaglyph/arcanaglyph-noavx"
chmod 644 "${APPDIR}/usr/lib/arcanaglyph/"libonnxruntime-*.so
chmod 644 "${APPDIR}/usr/lib/arcanaglyph/libvosk.so"

# GNOME Shell extension — кладём по тому же относительному пути что и в .deb.
APPIMG_WIDGET_DST="${APPDIR}/usr/share/arcanaglyph/extension/${WIDGET_EXT_UUID}"
if [[ -d "${WIDGET_EXT_SRC}" ]]; then
    mkdir -p "${APPIMG_WIDGET_DST}"
    cp "${WIDGET_EXT_SRC}/metadata.json" "${APPIMG_WIDGET_DST}/"
    cp "${WIDGET_EXT_SRC}/extension.js" "${APPIMG_WIDGET_DST}/"
    if [[ -d "${WIDGET_EXT_SRC}/schemas" ]]; then
        mkdir -p "${APPIMG_WIDGET_DST}/schemas"
        cp "${WIDGET_EXT_SRC}/schemas/"*.gschema.xml "${APPIMG_WIDGET_DST}/schemas/"
        if command -v glib-compile-schemas >/dev/null 2>&1; then
            glib-compile-schemas "${APPIMG_WIDGET_DST}/schemas/"
        fi
    fi
    chmod 644 "${APPIMG_WIDGET_DST}/metadata.json" "${APPIMG_WIDGET_DST}/extension.js"
    [[ -f "${APPIMG_WIDGET_DST}/schemas/gschemas.compiled" ]] \
        && chmod 644 "${APPIMG_WIDGET_DST}/schemas/gschemas.compiled"
fi

# Качаем appimagetool если его нет — официальный с GitHub releases continuous.
APPIMAGETOOL="${REPO_ROOT}/target/appimagetool-x86_64.AppImage"
if [[ ! -x "${APPIMAGETOOL}" ]]; then
    log "Скачиваю appimagetool…"
    curl -fSL -o "${APPIMAGETOOL}" \
        https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage
    chmod +x "${APPIMAGETOOL}"
fi

# Перепакуем. APPIMAGE_EXTRACT_AND_RUN=1 — обходит FUSE если он недоступен
# (например в Docker/CI без --privileged).
log "Перепаковка AppImage…"
rm -f "${APPIMAGE_FILE}"
APPIMAGE_EXTRACT_AND_RUN=1 "${APPIMAGETOOL}" \
    --no-appstream \
    "${APPDIR}" "${APPIMAGE_FILE}" >/dev/null
chmod 755 "${APPIMAGE_FILE}"

log "Готово:"
log "  ${DEB_FILE} ($(du -h "${DEB_FILE}" | awk '{print $1}'))"
log "  ${APPIMAGE_FILE} ($(du -h "${APPIMAGE_FILE}" | awk '{print $1}'))"

# Чистим временную распаковку.
rm -rf "${EXTRACT_DIR}"

log "Готово: ${DEB_FILE} ($(du -h "${DEB_FILE}" | awk '{print $1}'))"
log "Содержимое:"
dpkg-deb -c "${DEB_FILE}" | grep -E "(arcanaglyph|libonnxruntime|libvosk)" | awk '{print $1, $5, $6}'
