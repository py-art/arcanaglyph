# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Описание проекта

ArcanaGlyph — десктопное приложение для голосового ввода текста на Linux (Rust + Tauri v2).
Горячая клавиша (Ctrl+Ё) начинает запись, повторная — останавливает и транскрибирует.
Четыре STT-движка: Vosk, Whisper, GigaAM v3 (по умолчанию), Qwen3-ASR.
Вставка текста в активное окно через clipboard + XDG RemoteDesktop portal.
Иконка в трее: белая (idle), красная (запись), оранжевая (пауза).

## Команды

```bash
make run           # Запуск приложения (одна команда)
make all           # fmt + clippy + check + test
make fmt           # cargo fmt
make lint          # cargo clippy -- -D warnings
make test          # cargo test (требует LIBRARY_PATH=/usr/local/lib)
make build         # Release-сборка
make dist          # self-contained .deb (avx/noavx + bundled libs)
make clean         # Полная очистка
```

## Сборка и установка .deb пакета

`make dist` запускает `scripts/build-deb.sh` — один self-contained `.deb` работает на любом
x86_64 Linux (AVX и без AVX) без ручной настройки. Внутри: два бинаря (avx/noavx),
`libonnxruntime-avx2.so` (Microsoft), `libonnxruntime-noavx.so` (наш self-build),
`libvosk.so` (alphacep). Wrapper `/usr/bin/arcanaglyph` выбирает бинарь по `/proc/cpuinfo`.

Требует cargo-tauri-cli (`cargo install tauri-cli --version "^2.0"`), `cmake`, `dpkg-deb`.
Скачивает Microsoft ORT и vosk при первой сборке (см. `scripts/prepare-bundled-libs.sh`).

```bash
# 1. Сборка (~25-40 мин на N5095, ~10-15 мин на современном CPU)
make dist

# 2. Результат
ls target/release/bundle/deb/arcanaglyph_*.deb

# 3. Установка (apt сам подтянет зависимости — wl-clipboard и др.)
sudo apt install ./target/release/bundle/deb/ArcanaGlyph_1.6.0_amd64.deb

# 4. Запуск
arcanaglyph                # из терминала
# или через меню приложений GNOME → ArcanaGlyph

# 5. Удаление
sudo dpkg -r arcanaglyph
```

После установки в логах `arcanaglyph 2>&1` ищите строку `ORT_DYLIB_PATH = ...` —
показывает какая ORT-либа выбрана (bundled .deb / self-build override / dev env).

## XDG-пути (после установки)

- Модели: `~/.local/share/arcanaglyph/models/`
- БД и конфиг: `~/.config/arcanaglyph/`
- Скрипты (Wayland): `~/.config/arcanaglyph/scripts/`
- Аудио-кэш: `~/.cache/arcanaglyph/audio/`

При первом запуске автоматически скачивается GigaAM v3 (~225 МБ).

## Архитектура

Cargo workspace из двух крейтов:

- **arcanaglyph-core** (`crates/arcanaglyph-core/`) — библиотека + legacy бинарник:
  - `lib.rs` — публичный API: `ArcanaEngine`, `CoreConfig`, `EngineEvent`, `ArcanaError`
  - `engine.rs` — основной движок: управление записью (start/stop), broadcast событий
  - `transcriber.rs` — трейт `Transcriber` + реализации `VoskTranscriber`, `WhisperTranscriber`
  - `gigaam/` — модуль GigaAM v3 (SberDevices, ONNX Runtime):
    - `mel.rs` — mel-спектрограмма (STFT, HTK mel filterbank, log)
    - `transcriber.rs` — `GigaAmTranscriber` (ONNX inference + CTC decode)
  - `audio.rs` — захват аудио через `cpal`, передача в transcriber
  - `input.rs` — вставка текста: `wl-copy` + XDG RemoteDesktop на Wayland, `enigo` на X11
  - `config.rs` — конфигурация с load/save из SQLite (`TranscriberType`: Vosk, Whisper, GigaAm)
  - `error.rs` — типизированные ошибки через `thiserror`
  - `main.rs` — legacy standalone-сервер (UDP + WebSocket, для отладки)

- **arcanaglyph-app** (`crates/arcanaglyph-app/`) — Tauri v2 приложение:
  - `main.rs` — инициализация engine, Tauri commands (trigger, is_recording),
    регистрация глобальных хоткеев (tauri-plugin-global-shortcut),
    проброс событий engine → фронтенд через `app.emit()`
  - `tray.rs` — иконка в системном трее с меню
  - `dist/index.html` — фронтенд на vanilla JS, общается через Tauri IPC

## GitHub-зеркало

Репозиторий зеркалируется в GitHub (py-art/arcanaglyph) через GitLab CI.
При зеркалировании dev-файлы удаляются (job `mirror-to-github` в `.gitlab-ci.yml`).

**При добавлении нового dev-only файла** — добавить его в `git rm` список в `.gitlab-ci.yml`.

Текущий список исключений: `CLAUDE.md`, `.gitlab-ci.yml`, `.markdownlint.yaml`,
`.taplo.toml`, `rustfmt.toml`, `Makefile`, `NOTE.md`, `Roadmap.md`, `README.public.md`,
`.env.example`, `scripts/`.

## Конвенции

- Комментарии в коде — только на русском, не удалять существующие
- Линтинг: `cargo clippy -- -D warnings`, `cargo fmt`
- Линтинг TOML: `taplo` (конфиг в `.taplo.toml`)
- Линтинг Markdown: `markdownlint` (конфиг в `.markdownlint.yaml`, line_length: 120)
- Rust edition: 2024
- Обработка ошибок: `thiserror` + `Result<T, ArcanaError>`

## 🔁 Dogfood-ритуал — ОБЯЗАТЕЛЕН после каждой содержательной правки

После любого содержательного Write/Edit в `.rs`/`.ts`/`.py` (новая
функция / хендлер / walker / модуль) — **полный** цикл, не урезанный
до «fmt+test». Это project-level mandatory rule (user явно: «не хочу
потом тратить сутки на рефакторинг»). Source of truth — memory
`feedback_format_lint_then_dogfood_refactor.md`. PreToolUse-хук
`scripts/dogfood_ritual_reminder.sh` впрыскивает чеклист на `git commit`.

1. **fmt + lint:** `cargo fmt --all` → `cargo clippy --workspace
--all-targets -- -D warnings` (clippy НЕ ловит fmt-нарушения — fmt
   первым). Python — `ruff format`+`ruff check`; TS — `pnpm -C ui lint`.
2. **MCP coverage-check:** `untested_publics(path_prefix=<изменённый
модуль>)` или `test_coverage_overview`. Новый pub-символ без теста →
   **дописать тест сейчас**, пока контекст свежий. (v0.216.1 закрыл прежнюю
   слепоту: inline `#[cfg(test)] mod tests` callers теперь корректно
   считаются test, а не prod — coverage-сигнал на такие символы достоверен.
   Корень был не в эвристике, а в том что task-local `WORKSPACE_ROOT` не
   пробрасывался в `spawn_blocking`.)
3. **MCP refactor-check:** `file_health(<изменённый файл>)` +
   `refactor_candidates(scope="staged")`. Полярность `recommendation_score`
   = HIGHER IS BETTER (<60 critical, 60–85 warning, ≥85 ok).
4. **Если flagged warning/critical с actionable recommendations** → рефакторить
   сейчас (typ. дёшево: вынос inline-тестов в `#[path]`-sibling срезает
   loc/godobject; cc-reduction handler-split). НЕ откладывать.
5. **После рефактора — fmt + lint снова**, затем `cargo test --workspace`
   - `cargo fmt --all -- --check` (финальный локальный gate; CI нет, см.
     [docs/rule_enforcement.md](docs/rule_enforcement.md)).

Шаги 2–4 — через НАШ MCP (dogfood: не имеем права проповедовать MCP-first
и не использовать его на свежем коде). Бонус — live-валидация наших же
coverage/refactor тулов на реальном diff'е. После — bump+commit+deploy
(`make install && restart && clean-local`) + live-verify (guard/detached/
collision-пути — обязателен).

## Системные зависимости

```bash
sudo apt-get install build-essential libasound2-dev libgtk-3-dev libwebkit2gtk-4.1-dev \
  libxdo-dev libayatana-appindicator3-dev
```

Для вставки текста на Wayland (clipboard + XDG RemoteDesktop portal):

```bash
sudo apt install wl-clipboard
```

Также нужны: `libvosk.so` (в `/usr/local/lib/`) и модели в `models/`:

- Vosk: `models/vosk-model-ru-0.42/`
- Whisper: `models/ggml-large-v3-turbo.bin` (HuggingFace ggerganov/whisper.cpp)
- GigaAM v3: `models/gigaam-v3-e2e-ctc/` (istupakov/gigaam-v3-onnx)

<!-- arcanacodex:rules:begin v=0.213.1 -->

## ArcanaCodex MCP — highest-priority rule in this file

> _Auto-managed by `arcanacodex project install` (v0.213.1, 2026-06-12T01:22:15Z)._
> _Не редактировать руками — изменения затрутся при следующем
> `arcanacodex project update`. Чтобы удалить — `arcanacodex project uninstall <path>`._

**HARD RULE:** первый инструмент по любому вопросу про код в этом
репозитории — MCP-tool `mcp__arcana-codex__*`. `Read` / `rg` / `grep` /
`fd` — fallback'и, не default. Полный ruleset (anti-pattern checklist,
cheatsheet, escape hatches, hook behavior, scope guarantees) — в
`.arcanacodex/AGENT_RULES.md`.

**FIRST ACTION:** `mcp__arcana-codex__ping` без исключений в начале
любой сессии в этом workspace.

**Quick cheatsheet** — что вызывать вместо `rg`/`Read`:

| Tool                                  | Вопрос                                   |
| ------------------------------------- | ---------------------------------------- |
| `who_calls("X")`                      | Кто вызывает функцию X?                  |
| `function_skeleton(file)`             | Структура файла >300 строк?              |
| `signature` / `hover`                 | Сигнатура / docstring qualified-name?    |
| `affected_by(symbol)`                 | Что сломается если изменить этот символ? |
| `code_for_intent("...")`              | Не помню имя — найти по NL описанию?     |
| `file_health(file_path)`              | Один файл — стоит ли рефакторить?        |
| `refactor_candidates(scope="staged")` | Что рефакторить перед коммитом?          |

**Escape:** добавить `# arcana: allow` (с пробелом перед `#` в реальной
команде) в конец `rg`/`grep` команды для one-off bypass.
`export ARCANACODEX_BYPASS=1` отключает hooks на сессию.

**MUST READ:** прочитать `.arcanacodex/AGENT_RULES.md` ПЕРЕД первым
ответом про код в этом репо — там полный anti-pattern checklist, scope
гарантии, поведение hooks, объяснение envelope confidence-сигналов.

<!-- arcanacodex:rules:end -->
