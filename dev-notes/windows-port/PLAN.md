# План: Windows .exe-сборка ArcanaGlyph (итерация MVP)

> Внутренний dev-документ. Исключён из GitHub-зеркала (`.gitlab-ci.yml` →
> `git rm -rf dev-notes/`). Канонический план также лежит в
> `~/.claude/plans/synchronous-doodling-mountain.md`.

## Context

ArcanaGlyph сейчас собирается только под Linux (`.deb` + `.AppImage`, CI на
`ubuntu-24.04`). Нужен Windows-инсталлятор (`.exe`), чтобы программа ставилась
двойным кликом без «танцев с бубном». У пользователя есть Windows-ноутбук, но
**без dev-окружения** — он может только установить `.exe` и прислать логи.
Отсюда два жёстких требования к MVP:

1. **Полное файловое логирование на Windows** — иначе цикл обратной связи
   невозможен. Сейчас `tracing` пишет только в stdout, а с
   `windows_subsystem = "windows"` (уже стоит в `main.rs:3`) консоли нет → логи
   улетают в никуда. Это шаг №1 по важности.
2. **Минимальный, но рабочий .exe** — собирается на CI, ставится, GigaAM
   распознаёт речь. Всё остальное (авто-обновление, не-AVX CPU, автозапуск)
   допиливаем по логам итерациями.

Код уже ~70% готов к кроссплатформе — Windows-путь ложится на существующий
дизайн (feature `gigaam-system-ort` + `ORT_DYLIB_PATH`), а не ломает его.

## Зафиксированные решения

- **Формат**: NSIS (`.exe`), не MSI. Установка per-user (`currentUser`) — без
  прав администратора.
- **Объём v1**: MVP-first (см. scope ниже).
- **Dev-папка**: `dev-notes/windows-port/`, исключена из GitHub-зеркала.
- **ORT-стратегия**: feature `gigaam-system-ort` (`ort/load-dynamic`) + bundled
  `onnxruntime.dll` через `ORT_DYLIB_PATH` — как на Linux. НЕ `download-binaries`
  (Tauri не бандлит соседние DLL автоматически — tauri#2662).

### В scope (MVP)

- `.exe` собирается на CI (`windows-latest`) и грузится в тот же GitHub-релиз.
- Ставится двойным кликом, GigaAM работает, модель качается при первом запуске.
- Полное файловое логирование + panic-hook в файл.
- Авто-апдейтер на Windows = открыть страницу релиза в браузере (не in-app).
- Один `.exe`, требует CPU с AVX (у большинства Windows-машин есть).
- GNOME-виджет / XDG-портал / Wayland-скрипты = no-op.

### Out of scope (следующие итерации)

- In-app скачивание+запуск нового `.exe` (авто-обновление).
- Поддержка CPU без AVX (второй DLL / runtime-выбор).
- Windows-автозапуск через реестр (`HKCU\...\Run`).
- macOS.

## Шаги реализации

См. `CHECKLIST.md` — там нумерованный список с галочками. Краткая суть шагов:

1. Dev-папка + исключение из зеркала (`.gitlab-ci.yml`, `CLAUDE.md`).
2. 🔴 Файловое логирование: `tracing-appender`, `CoreConfig::logs_dir()`,
   dual-layer subscriber (stdout+file) в `main.rs`, panic-hook → файл.
3. `tauri.windows.conf.json` (merge-only): nsis target, resources
   `onnxruntime.dll`, nsis `currentUser`, webview downloadBootstrapper.
4. Windows-ветка `setup_ort_dylib_path()` в `bootstrap.rs`.
5. Провижн `onnxruntime.dll` win-x64 (CI download + `.gitignore`).
6. Updater: asset-check по платформе (`.exe`); `apply_update` на Windows =
   открыть страницу релиза; Linux-ветки за `cfg`.
7. `widget_ext.rs`: non-linux заглушки для всех GNOME-команд.
8. Хоткей по умолчанию: проверить парс на Windows, скорректировать дефолт.
9. CI: job `build-windows` в `release.yml`.
10. Локальная dogfood-валидация на Linux (fmt/clippy/test + type-check).
11. Bump версии + CHANGELOG для тест-релиза (commit/push — только по
    разрешению пользователя).

## Verification (end-to-end)

1. CI собирает `.exe`, грузит в релиз (job зелёный, asset есть).
2. Пользователь качает `.exe`, ставит на Windows-ноут двойным кликом.
3. Лог пишется в `%LOCALAPPDATA%\arcanaglyph\logs\`.
4. При падении — пользователь присылает лог; правим по факту.
5. Локально (Linux) на каждой правке — `cargo clippy`/`test` зелёные.

Ключевые риски (видны в логе):

- Нет AVX на ноуте → SIGILL при init ORT (тогда не-AVX DLL, следующая итерация).
- `onnxruntime.dll` не найден → `ORT_DYLIB_PATH` пустой/неверный.
- Хоткей не зарегистрировался.
- WebView2 не установлен (должен докачаться downloadBootstrapper).
