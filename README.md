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
# 1. Системные зависимости (минимум — для дефолтной сборки с GigaAM v3)
sudo apt-get install build-essential libasound2-dev libgtk-3-dev \
  libwebkit2gtk-4.1-dev libxdo-dev libayatana-appindicator3-dev \
  wl-clipboard netcat-openbsd

# 2. Запуск (по умолчанию — только GigaAM v3, никаких системных STT-библиотек не требуется)
make run
```

При первом запуске GigaAM v3 скачается автоматически в `~/.local/share/arcanaglyph/models/`.

### CPU без AVX (старые Celeron/Atom)

Дефолтный движок GigaAM требует, чтобы CPU поддерживал инструкции **AVX** —
pre-built ONNX Runtime, который тянет `ort/download-binaries`, инициализируется
с AVX и крашит SIGILL'ом на старых CPU (например Intel Celeron N4xxx/N5xxx,
Pentium Silver и т.п.).

`make run` определяет это автоматически: на CPU без AVX он пересобирает
приложение с **Whisper** (`--no-default-features --features whisper`).
Whisper.cpp детектит CPU при сборке и использует только доступные SIMD —
работает без AVX. Дополнительно, при запуске с дефолтной сборкой на CPU без
AVX, само приложение делает runtime AVX-check и переключает движок на
Whisper/Vosk если они скомпилированы — окно UI открывается всегда, без SIGILL.

### Сборка с другими движками (cargo features)

По умолчанию собирается только GigaAM v3 (ONNX, self-contained). Остальные движки
включаются через cargo features — в скобках указаны дополнительные системные требования
для каждого движка:

```bash
# Дефолт: только GigaAM v3 (ничего лишнего ставить не нужно)
cargo build

# Все 4 движка
cargo build --no-default-features --features all-engines

# Дефолт + Vosk (требует libvosk.so в /usr/local/lib/)
cargo build --features vosk

# Дефолт + Whisper (требует CMake и C++ toolchain — `whisper-rs` собирает whisper.cpp)
cargo build --features whisper

# Дефолт + Qwen3-ASR (требует только сетевой доступ — ort качает binaries сам)
cargo build --features qwen3asr
```

| Движок | cargo feature | Системные требования |
| --- | --- | --- |
| GigaAM v3 | `gigaam` (default) | Нет |
| Qwen3-ASR | `qwen3asr` | Нет |
| Whisper | `whisper` | CMake, C++ toolchain |
| Vosk | `vosk` | `libvosk.so` в `/usr/local/lib/` (см. ниже) |

Если в SQLite-конфиге сохранён движок, не включённый в текущую сборку (например, после
переключения с `all-engines` на default), приложение **не падает**: оно автоматически
переключается на дефолтный движок и показывает в UI toast «движок X не включён в сборку».

### libvosk (только если собираете с feature `vosk`)

```bash
# Скачать prebuilt libvosk.so → /usr/local/lib/
wget https://github.com/alphacep/vosk-api/releases/download/v0.3.45/vosk-linux-x86_64-0.3.45.zip
unzip vosk-linux-x86_64-0.3.45.zip
sudo cp vosk-linux-x86_64-0.3.45/libvosk.so /usr/local/lib/
sudo cp vosk-linux-x86_64-0.3.45/vosk_api.h /usr/local/include/
sudo ldconfig
# Или собрать из исходников (часы): scripts/legacy/install_libvosk.bash
```

## Сборка и установка .deb пакета

`make dist` собирает self-contained `.deb`, который работает на любом x86_64 Linux
(с AVX и без AVX) сразу после `dpkg -i`. Внутри:

- два бинаря: `arcanaglyph-avx` (whisper.cpp с AVX/AVX2/FMA/F16C) и `arcanaglyph-noavx`
  (whisper.cpp с `-mno-avx*`); wrapper `/usr/bin/arcanaglyph` выбирает по `/proc/cpuinfo`.
- `libonnxruntime-avx2.so` (Microsoft pre-built 1.20.1, ~16 МБ) и `libonnxruntime-noavx.so`
  (наш self-build 1.20.1, ~24 МБ) — runtime AVX-detection выбирает нужный.
- `libvosk.so` (alphacep pre-built 0.3.45, ~25 МБ) — для движка Vosk.

Если на машине лежит `/usr/local/lib/libonnxruntime.so` (твой self-build) — он
приоритетнее bundled. Аналогично `/usr/local/lib/libvosk.so` через ld.so.cache.

```bash
# Требуются: cargo-tauri-cli, cmake, dpkg-deb, curl, unzip
cargo install tauri-cli --version "^2.0"

# Сборка (~25-40 мин на N5095, ~10-15 мин на современном CPU)
make dist

# Результат — один .deb (~160 МБ)
ls target/release/bundle/deb/ArcanaGlyph_*.deb

# Установка (apt сам подтянет зависимости: wl-clipboard, libwebkit2gtk и т.д.)
sudo apt install ./target/release/bundle/deb/ArcanaGlyph_1.6.0_amd64.deb

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
make all      # fmt + clippy + check + test (default features)
make fmt      # cargo fmt
make lint     # cargo clippy -- -D warnings
make test     # cargo test (default features)
make test-all # cargo test --all-features (требует все системные libs)
make build    # Release-сборка
make dist     # Сборка .deb и .AppImage
make install  # Собрать (если нужно) и установить .deb локально
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
