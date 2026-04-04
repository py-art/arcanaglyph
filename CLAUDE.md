# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Описание проекта

ArcanaGlyph — десктопное приложение для голосового ввода текста на Linux (Rust + Tauri v2).
Единый процесс: нажатие глобальной горячей клавиши начинает запись с микрофона,
повторное нажатие останавливает, Vosk транскрибирует речь локально,
enigo вставляет текст в активное окно. Иконка в системном трее.

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

## Архитектура

Cargo workspace из двух крейтов:

- **arcanaglyph-core** (`crates/arcanaglyph-core/`) — библиотека + legacy бинарник:
  - `lib.rs` — публичный API: `ArcanaEngine`, `CoreConfig`, `EngineEvent`, `ArcanaError`
  - `engine.rs` — основной движок: управление записью (start/stop), broadcast событий
  - `audio.rs` — захват аудио через `cpal`, транскрибация через `vosk`
  - `input.rs` — вставка текста: `wl-copy` + `wtype` на Wayland, `enigo` на X11
  - `config.rs` — конфигурация с load/save из `~/.config/ArcanaGlyph/config.toml`
  - `error.rs` — типизированные ошибки через `thiserror`
  - `main.rs` — legacy standalone-сервер (UDP + WebSocket, для отладки)

- **arcanaglyph-app** (`crates/arcanaglyph-app/`) — Tauri v2 приложение:
  - `main.rs` — инициализация engine, Tauri commands (trigger, is_recording),
    регистрация глобальных хоткеев (tauri-plugin-global-shortcut),
    проброс событий engine → фронтенд через `app.emit()`
  - `tray.rs` — иконка в системном трее с меню
  - `dist/index.html` — фронтенд на vanilla JS, общается через Tauri IPC

## Конвенции

- Комментарии в коде — только на русском, не удалять существующие
- Линтинг: `cargo clippy -- -D warnings`, `cargo fmt`
- Линтинг TOML: `taplo` (конфиг в `.taplo.toml`)
- Линтинг Markdown: `markdownlint` (конфиг в `.markdownlint.yaml`, line_length: 120)
- Rust edition: 2024
- Обработка ошибок: `thiserror` + `Result<T, ArcanaError>`

## Системные зависимости

```bash
sudo apt-get install build-essential libasound2-dev libgtk-3-dev libwebkit2gtk-4.1-dev libxdo-dev
```

Для вставки текста на Wayland (clipboard + ydotool через /dev/uinput):

```bash
sudo apt install wl-clipboard ydotool
```

Также нужны: `libvosk.so` (в `/usr/local/lib/`) и Vosk-модель `models/vosk-model-ru-0.42/`.
