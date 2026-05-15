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
make dist          # self-contained .deb (двойной бинарь avx/noavx + bundled libs, см. ниже)
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
`.taplo.toml`, `rustfmt.toml`, `Makefile`, `NOTE.md`, `Roadmap.md`, `README.public.md`,
`.env.example`, `scripts/`.

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

<!-- arcanacodex:rules:begin v=0.1.0 -->
## ArcanaCodex MCP — правила использования

> _Auto-managed by `arcanacodex project install` (v0.1.0, 2026-05-15T00:05:26Z)._
> _Не редактировать руками — изменения затрутся при следующем
> `arcanacodex project update`. Чтобы убрать секцию — `arcanacodex project uninstall <path>`._

В этом проекте подключён MCP-сервер `arcana-codex`. Он даёт ~25 узких
tools для навигации по коду через persistent графовый индекс + LSP fan-out.
Использование MCP здесь — **default**, а не fallback: средняя экономия
input-токенов на типичных задачах −67…−85% vs `Read` + `rg`.

### Первое действие в сессии

`mcp__arcana-codex__ping` — проверить что daemon работает. Если ошибка —
сказать пользователю поднять (`systemctl --user start arcanacodex` или
`make install` если бинарь устарел). Только потом начинать работу.

### Когда использовать MCP вместо Read/rg

Колонки Rust / TS / Py показывают где tool поддержан полностью (✅), частично
(⚠️) или не работает (❌). При ❌ даны fallback-альтернативы.

| Задача | MCP tool | Rust | TS | Py | Что заменяет |
|---|---|---|---|---|---|
| Файл >300 строк, нужна структура | `function_skeleton` | ✅ | ✅ | ✅ | `Read` целиком |
| Кто вызывает X | `who_calls` | ✅ | ✅ | ✅ | `rg "X\("` + Read |
| Сигнатура / docstring X | `signature` | ✅ | ✅ | ✅ | grep + Read |
| Что сломается при изменении X | `affected_by` | ✅ | ✅ | ✅ | manual reasoning |
| Поиск похожего кода (DRY) | `find_similar` | ✅ | ✅ | ✅ | `rg` + интуиция |
| Поиск кода по описанию | `code_for_intent` | ✅ | ✅ | ✅ | угадывание имён |
| Все методы / поля типа | `type_members` | ✅ | ✅ | ⚠️ | Read + сборка руками |
| Карта крейта (модули + pub-items) | `module_tree` | ✅ | ⚠️ via index.ts | ❌ → `function_skeleton` per-file | N×`function_skeleton` |
| Кто реализует trait/protocol | `who_implements` | ✅ | ✅ | ⚠️ | `rg "impl X for"` |
| Цепочка вызовов A → B | `paths_between` | ✅ | ✅ | ✅ | DFS руками |
| Git-история вокруг функции | `recent_changes_around` | ✅ | ✅ | ✅ | `git log -L` (использует line-anchored) |
| Символы тронутые diff'ом | `changed_symbols` | ✅ | ✅ | ✅ | git diff + ts-grep |
| Точное определение символа | `definition` | ✅ | ✅ | ⚠️ | grep + Read |
| Поиск references (LSP) | `references` | ✅ | ✅ | ✅ через pyright | `rg` + типы |
| Грep с фильтром по AST-kind | `grep_with_kind` | ✅ | ✅ | ✅ | rg без классификации |
| Тесты, покрывающие символ | _композиция_ | ✅ | ✅ | ✅ | `who_calls(scope='all')` + фильтр `tests/` |

### Когда НЕ использовать MCP

- Документы (`*.md`, `README`, `CHANGELOG`) — нет в графе, читать через `Read`.
- `git` / `cargo` / `npm` / `make` команды — через `Bash`.
- Literal-string lookup (точная подстрока) — `rg` дешевле; `find_similar`
  только для **semantic** похожести.
- Файлы вне индексированных языков (Rust / TypeScript / Python) — `Read` + `rg`.

### Scope и гарантии — что инструмент видит и что **не** видит

Это критически важно: **доверять можно только тому что внутри scope**.
Использовать MCP «авось покроет» — анти-паттерн.

**Текущий scope** — workspace, в котором запущен shim (значение `--workspace`
в `~/.claude.json` → `.projects.<path>.mcpServers.arcana-codex.args`).
Один shim = один workspace. Multi-workspace API с явным параметром
`workspace` запланирован, но **ещё не реализован**.

**Что НЕ в scope (пустой ответ tool'а ≠ «нигде нет»):**

- **Subdirs из `.arcanacodexignore`** — daemon их не индексирует. Если в
  проекте есть `.arcanacodexignore` со строкой `vendor/`, `node_modules/`,
  `sntz_mockups/` — MCP-tools по этим директориям всегда вернут пусто.
  Использовать `rg` / `Read` напрямую, не делать вид что MCP это покрыл.
- **Reference-репозитории заказчика / vendor-форки** — это частный случай
  выше. Часто в `.arcanacodexignore`. Сверка вручную через `rg`.
- **Файлы вне Rust / TypeScript / Python** — нет парсера, нет графа.
- **Документы (`*.md`, `README`)** — даже если в индексируемой директории.

**Confidence сигналы — как читать:**

| `confidence` | `source` | Доверие | Что делать |
|---|---|---|---|
| `high` | `graph` | полное | использовать напрямую |
| `high` | `lsp` / `graph+lsp` | полное | использовать напрямую (LSP подтвердил) |
| `medium` | `graph` + `stale_warning` | условное | watcher debounce 300ms — подождать или сверить `Read` |
| `medium` | `lsp` fallback | условное | граф не нашёл, LSP подтвердил — обычно ок |
| `low` | любой | минимальное | scope пустой / новый workspace до bootstrap'а / запрос не подошёл — использовать `rg` / `Read` |

**`source` field:**

- `graph` — из persistent графа, мгновенно, видит только что в индексе
- `lsp` — от rust-analyzer / typescript-language-server / pyright-langserver,
  медленнее, но cross-file references по семантике типов (часто точнее
  графа на dyn-dispatch, generic'ах, re-export'ах)
- `graph+lsp` — объединено через `OutputEnvelope::merge`; если оба
  согласны — высокое доверие, если расхождение — обычно граф out-of-sync,
  предпочесть LSP

**Когда tool вернул пустой массив:**

Это значит **«в моём scope нет»**, а не «нигде нет». Если scope узкий
(тестовая директория в ignore, vendor исключён) — результат заведомо
неполный. Перед выводом «функция X не используется» — проверить чем
ограничен scope (есть ли `.arcanacodexignore`).

### Language conventions в envelope output

Внутренняя модель символа унифицирована, но JSON-вывод адаптируется под
язык файла. Знать что есть что:

- `kind` поле для Python `class` отдаётся как `"class"` (исторически —
  `"struct"`, оставшееся в graph-схеме). Для Rust остаётся `"struct"`,
  для TS-классов — тоже `"class"`.
- `fallback_hint` с примером qualified-form подбирается по primary
  language workspace'а: Rust → `Type::method`, Python → `ClassName.method`,
  TypeScript → `ClassName.method`.
- `type_members` поля «inherent_methods» / «trait_methods» — Rust
  terminology; для Python это просто «methods» / «inherited methods»,
  для TS — «methods» / «implemented from interfaces».

### Framework patterns — что граф НЕ видит (use `references` instead)

Граф `who_calls` строит CALLS-edges по прямым вызовам `func()` /
`Type::method()`. Это **не покрывает** реверенс-передачу через
DI-фреймворки. Если кажется что «функция не вызывается» — проверь
`references()`:

- **FastAPI `Depends(<sym>)`** — это передача reference, не call.
  `who_calls("jwt_bearer_dependency")` пропустит. Используй
  `references(symbolName="jwt_bearer_dependency")`.
- **Annotated DI** (`Annotated[T, Depends(...)]`) — то же самое.
- **pytest fixtures** — параметр-resolution через имя, не call.
  `references` находит, `who_calls` нет.
- **Django CBV / Flask blueprints** — регистрация через decorator/router
  config, не direct call.
- **Pydantic `@field_validator` / `@model_validator`** — вызывается
  фреймворком, не пользовательским кодом.

Эти ограничения — правильное поведение CALLS-графа (статически их не
отличить от dead reference). Workaround всегда — `references()`.

### Known envelope quirks (M5b в работе)

Прозрачно фиксируем шероховатости, по которым в работе fixes. Пока
читать с поправкой:

- `truncated: true` без явного клиентского `max_results` — server-side
  обрезка. Если **клиент сам** передал `max_results=N` и получил N
  элементов — смотри `more_available: true/false` (новое поле); реальной
  «обрезки» не было.
- Для `code_for_intent` outer `source` пишется `"vector"` (M5b+);
  на старых daemon до M5b там может быть `"graph"` — в этом случае
  смотри inner `result.source: "vector"`.
- `stale_warning` теперь сопровождается `stale_reason`:
  `"file_modified"` (mtime > embedded_at + 2s),
  `"recent_edit"` (file менялся в пределах 5s — reembed догоняет),
  `"embed_aged_out"` (embed старше 300s, но содержимое не менялось),
  `"graph_stale"` (общий graph-snapshot устарел).
  Для `embed_aged_out` обычно можно доверять данным если код стабилен;
  для `file_modified` — fallback `Read` критичных результатов.
- `kind: "struct"` для Python-файла — читать как `class` (legacy).
  В выводе автоматически конвертируется (M5b+).

### Если tool вернул `confidence: "medium"` + `stale_warning`

Граф мог не подхватить недавние правки (watcher debounce 300 ms). Сверять
с `Read` критичные результаты или подождать секунду и повторить вызов.
Смотри также `stale_reason` поле — оно подсказывает что именно произошло.

### Если tool не справился

`mcp__arcana-codex__report_issue` — feedback пишется в `docs/feedback/`
репозитория ArcanaCodex, разработчик инструмента увидит. Указывать
конкретный input + expected vs actual.
<!-- arcanacodex:rules:end -->
