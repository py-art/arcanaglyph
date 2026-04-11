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
make dist          # cargo tauri build (.deb, .AppImage)
make clean         # Полная очистка
```

## Сборка и установка .deb пакета

```bash
# 1. Сборка (несколько минут)
make dist

# 2. Результат
ls target/release/bundle/deb/arcanaglyph_*.deb

# 3. Установка
sudo dpkg -i target/release/bundle/deb/ArcanaGlyph_1.0.0_amd64.deb
sudo apt-get install -f   # если не хватает зависимостей

# 4. Запуск
arcanaglyph                # из терминала
# или через меню приложений GNOME → ArcanaGlyph

# 5. Удаление
sudo dpkg -r arcanaglyph
```

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
  - `input.rs` — вставка текста: `wl-copy` + XDG RemoteDesktop (Shift+Insert) на Wayland, `enigo` на X11
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
`.taplo.toml`, `rustfmt.toml`, `Makefile`, `NOTE.md`, `Roadmap.md`, `README.public.md`.

## Конвенции

- Комментарии в коде — только на русском, не удалять существующие
- Линтинг: `cargo clippy -- -D warnings`, `cargo fmt`
- Линтинг TOML: `taplo` (конфиг в `.taplo.toml`)
- Линтинг Markdown: `markdownlint` (конфиг в `.markdownlint.yaml`, line_length: 120)
- Rust edition: 2024
- Обработка ошибок: `thiserror` + `Result<T, ArcanaError>`

## Системные зависимости

```bash
sudo apt-get install build-essential libasound2-dev libgtk-3-dev libwebkit2gtk-4.1-dev libxdo-dev libayatana-appindicator3-dev
```

Для вставки текста на Wayland (clipboard + XDG RemoteDesktop portal):

```bash
sudo apt install wl-clipboard
```

Также нужны: `libvosk.so` (в `/usr/local/lib/`) и модели в `models/`:

- Vosk: `models/vosk-model-ru-0.42/`
- Whisper: `models/ggml-large-v3-turbo.bin` (скачать с HuggingFace ggerganov/whisper.cpp)
- GigaAM v3: `models/gigaam-v3-e2e-ctc/` (содержит `v3_e2e_ctc.int8.onnx` + `v3_e2e_ctc_vocab.txt`,
  скачать с HuggingFace istupakov/gigaam-v3-onnx)
