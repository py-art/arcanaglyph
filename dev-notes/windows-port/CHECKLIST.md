# Чек-лист: Windows .exe-сборка (MVP)

> Отмечаем по мере выполнения. `[ ]` — todo, `[x]` — готово, `[~]` — в работе.

## Шаг 1 — Dev-папка + исключение из зеркала

- [x] Создать `dev-notes/windows-port/PLAN.md`
- [x] Создать `dev-notes/windows-port/CHECKLIST.md`
- [x] Добавить `dev-notes/` в `git rm -rf` список `.gitlab-ci.yml`
- [x] Дописать `dev-notes/` в список исключений `CLAUDE.md`

## Шаг 2 — 🔴 Файловое логирование + panic-hook

- [x] `tracing-appender` в `crates/arcanaglyph-app/Cargo.toml`
- [x] `CoreConfig::logs_dir()` в `crates/arcanaglyph-core/src/config.rs`
- [x] Dual-layer subscriber (stdout + rolling file) в `main.rs`, держать `WorkerGuard`
- [x] `std::panic::set_hook` → `tracing::error!`
- [x] Стартовая диагностика: версия, OS, AVX, путь логов (`ORT_DYLIB_PATH` логируется в bootstrap)
- [x] Тест на `logs_dir()`

## Шаг 3 — Windows-конфиг Tauri

- [x] Создать `crates/arcanaglyph-app/tauri.windows.conf.json`
- [x] `bundle.targets = ["nsis"]`
- [x] `bundle.resources = ["libs/onnxruntime.dll"]`
- [x] `bundle.windows.nsis.installMode = "currentUser"` + иконки
- [x] `bundle.windows.webviewInstallMode = downloadBootstrapper`

## Шаг 4 — ORT: путь к DLL на Windows

- [x] Windows-ветка `setup_ort_dylib_path()` в `bootstrap.rs`
- [x] Уважать существующий `ORT_DYLIB_PATH`
- [x] Искать `onnxruntime.dll` рядом с `current_exe()`, в `libs/` и `resources/libs/`

## Шаг 5 — Провижн onnxruntime.dll (win-x64)

- [x] CI-шаг скачивания Microsoft ORT v1.20.1 win-x64 (в job build-windows-test)
- [x] Кладётся в `crates/arcanaglyph-app/libs/onnxruntime.dll`
- [x] `.gitignore` на DLL

## Шаг 6 — Updater: учёт Windows

- [x] Asset-check по платформе (`.exe` на Windows) в `updater.rs` (`release_has_installable_asset`)
- [x] `apply_update` на Windows = открыть страницу релиза
- [x] Кроссплатформенный open-url (`xdg-open`/`cmd /c start`/`open`)
- [x] Linux-ветки (`detect_terminal`, `terminal_args`, `apply_update_inner`) за `cfg(linux)`

## Шаг 7 — GNOME-поверхность → no-op

- [x] `widget_extension_status` short-circuit на non-linux (без падающего gsettings-спавна)
- [~] Полное cfg-гейтинг install/disable/logout — НЕ НУЖНО для MVP: компилируются,
      недостижимы (фронт скрывает ряд при `is_gnome()==false`, settings.ts:220)

## Шаг 8 — Хоткей по умолчанию

- [x] Дефолт `"Control+\`"` парсится плагином и на Windows (источник: global-hotkey 0.7)
- [x] Handler переведён на сравнение распарсенных `Shortcut` (Display-строки не round-trip'ятся)

## Шаг 9a — CI полигон: GitLab Windows-job → artifact

- [x] Job `build-windows-test` в `.gitlab-ci.yml`, тег `saas-windows-medium-amd64`
- [x] `rules`: feature-ветка (не main), `when: manual`
- [x] Toolchain: rustup MSVC (ставим), Node/VS Build Tools/cmake — предустановлены на образе
- [x] Frontend build, download `onnxruntime.dll` (ORT 1.20.1), cargo cache
- [x] Tauri CLI через npm (prebuilt, экономит CI-минуты vs cargo install)
- [x] `tauri build --bundles nsis -- --no-default-features --features gigaam-system-ort`
- [x] `artifacts.paths` на `*-setup.exe`
- [x] ✅ ЗЕЛЁНАЯ (2026-06-13, коммит 36dace1, ~57 мин). Артефакт .exe 7.5 МБ;
      7z подтвердил: внутри arcanaglyph.exe + libs/onnxruntime.dll.
- [x] 3 CI-фикса: PowerShell `--`/`--%` → решение `cmd /c "tauri build ... -- ..."`;
      `cache: when: always`. NB: кэш target/ большой, архивация ~5 мин (можно урезать).

## Шаг 9b — CI витрина (ПОСЛЕДНИМ): GitHub Actions Windows-job

- [ ] Job `build-windows` (`windows-latest`) в `release.yml`
- [ ] Frontend build, download DLL, `cargo tauri build --bundles nsis`
- [ ] SHA256 + `gh release upload $TAG *.exe`

## Шаг 10 — Локальная валидация (dogfood) на Linux

- [x] `cargo fmt --all` + `cargo clippy --workspace --all-targets -- -D warnings` (чисто)
- [x] MCP coverage/refactor на изменённых модулях (новый код здоров; флаги pre-existing)
- [x] Покрыт новый pub-символ `logs_dir()` тестом; updater-тесты переименованы
- [x] `cargo test --workspace` (74 core + 16 app updater + ост.) + frontend type-check — зелёные

## Шаг 11 — Bump + CHANGELOG + тест-релиз

- [ ] Bump версии (6 точек + 2 lock-файла) по `version-bump.md`
- [ ] CHANGELOG (Unreleased)
- [ ] ⛔ STOP перед commit/push/release — только по явному разрешению пользователя
- [ ] Пользователь публикует релиз → CI собирает `.deb` + `.exe`
- [ ] Установка `.exe` на Windows + сбор логов
