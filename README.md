# ArcanaGlyph

Десктопное приложение для голосового ввода текста на Linux (Wayland + X11).
Нажимаете горячую клавишу — говорите — нажимаете ещё раз — распознанный текст
автоматически вставляется в активное окно. Вся транскрибация происходит локально,
без передачи данных в облако.

## STT-движки

| Движок | Модель | WER (рус.) | Размер | Скорость |
| --- | --- | --- | --- | --- |
| **GigaAM v3** (по умолчанию) | v3_e2e_ctc.int8.onnx | **~8.4%** | 225 МБ | ~0.8 сек / 5 сек аудио |
| Whisper | ggml-large-v3-turbo.bin | ~14% | 1.5 ГБ | 30-70 сек / 10 сек аудио |
| Vosk | vosk-model-ru-0.42 | ~11% | 42 МБ | Реальное время (streaming) |
| Qwen3-ASR | 0.6B ONNX | ~6% (мульти) | 2.5 ГБ | ~5 сек / 5 сек аудио |

При первом запуске автоматически скачивается GigaAM v3 (~225 МБ).
Остальные модели можно скачать из настроек приложения (вкладка «Модели»).

## Горячие клавиши

| Действие | Комбинация |
| --- | --- |
| Запись (старт/стоп) | **Ctrl+Ё** (`Ctrl+grave`) |
| Пауза/возобновление | **Ctrl+Shift+Ё** |

Работают в обеих раскладках (русской и английской).
Настраиваются в приложении: Настройки → вкладка «Клавиши».

## Быстрый старт (из исходников)

```bash
# 1. Системные зависимости
sudo apt-get install build-essential libasound2-dev libgtk-3-dev \
  libwebkit2gtk-4.1-dev libxdo-dev libayatana-appindicator3-dev \
  wl-clipboard netcat-openbsd

# 2. libvosk (если не установлена)
# Скачать libvosk.so → /usr/local/lib/
# Или: scripts/legacy/install_libvosk.bash

# 3. Запуск
make run
```

При первом запуске GigaAM v3 скачается автоматически в `~/.local/share/arcanaglyph/models/`.

## Сборка и установка .deb пакета

```bash
# Требуется Tauri CLI
cargo install tauri-cli

# Сборка (несколько минут)
make dist

# Результат
ls target/release/bundle/deb/ArcanaGlyph_*.deb

# Установка
sudo dpkg -i target/release/bundle/deb/ArcanaGlyph_1.1.0_amd64.deb
sudo apt-get install -f   # если не хватает зависимостей

# Запуск
arcanaglyph

# Удаление
sudo dpkg -r arcanaglyph
```

## Настройки

Всё настраивается через UI приложения (три вкладки):

**Основное:**
движок, предзагрузка, частота, таймаут, авто-стоп при тишине (VAD),
удаление слов-паразитов, автоочистка записей, автозапуск, запуск в трей

**Модели:**
пути к моделям, карточки с описанием/размером/статусом, кнопка «Скачать»

**Клавиши:**
композер горячих клавиш с кнопками модификаторов + рекордер основной клавиши

Настройки хранятся в SQLite: `~/.config/arcanaglyph/history.db`

## XDG-пути

| Что | Путь |
| --- | --- |
| Модели | `~/.local/share/arcanaglyph/models/` |
| БД и конфиг | `~/.config/arcanaglyph/` |
| Скрипты (Wayland) | `~/.config/arcanaglyph/scripts/` |
| Аудио-кэш | `~/.cache/arcanaglyph/audio/` |

## Меню трея

- Открыть приложение
- Начать запись
- Настройки
- Выход

Иконка трея: белая (готов), красная (запись), оранжевая (пауза).

## Разработка

```bash
make help     # Показать все команды
make run      # Запустить приложение
make all      # fmt + clippy + check + test
make fmt      # cargo fmt
make lint     # cargo clippy -- -D warnings
make test     # cargo test
make build    # Release-сборка
make dist     # Сборка .deb и .AppImage
make clean    # Очистка кэша сборки
```

## Структура проекта

```text
crates/
  arcanaglyph-core/       # Библиотека: STT-движки, аудио, конфиг, история
    src/gigaam/            # GigaAM v3 (mel + ONNX + CTC decode)
    src/qwen3asr/          # Qwen3-ASR (mel + ONNX + autoregressive decoder)
  arcanaglyph-app/         # Tauri v2 приложение (GUI + tray + hotkeys)
dist/
  index.html               # Фронтенд (vanilla HTML/JS)
scripts/
  ag-trigger               # UDP-скрипт для Wayland (запись)
  ag-pause                 # UDP-скрипт для Wayland (пауза)
assets/
  arcanaglyph.desktop      # Шаблон .desktop файла
```

## Troubleshooting

### Ошибка "unable to find library -lvosk"

```bash
export LIBRARY_PATH=/usr/local/lib
```

Или установите libvosk через `scripts/legacy/install_libvosk.bash`.

### Горячая клавиша не работает в русской раскладке

На Wayland используйте `Ctrl+Ё` — клавиша `grave` не зависит от раскладки.
Буквенные комбинации (`Super+W`) в русской раскладке не работают — это ограничение GNOME.

### Wayland: текст не вставляется

Установите `wl-clipboard` и разрешите XDG RemoteDesktop portal:

```bash
sudo apt install wl-clipboard
```

При первом использовании GNOME покажет диалог подтверждения доступа.

### X11: Ошибка "Не удалось создать Enigo"

```bash
sudo apt-get install libxdo-dev
```

### Папка target/ занимает много места

```bash
make clean   # Удаляет кэш сборки (~10-15 ГБ в debug-режиме)
```
