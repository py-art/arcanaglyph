---
description: Полный аудит ArcanaGlyph через ArcanaCodex MCP (диагностика, без правок)
argument-hint: "[опц. path_prefix/модуль для фокус-аудита или ветка для diff-аудита]"
---

# Полный аудит ArcanaGlyph через ArcanaCodex MCP

Проведи **полный аудит кодовой базы**, используя исключительно MCP-сервер
`arcana-codex` (инструменты `mcp__arcana-codex__*`). Это persistent граф символов +
LSP fan-out + vector index по Rust / TypeScript. Он заменяет `Read большого файла` +
`rg по символу` + ручную склейку — а НЕ дополняет их. `Read` / `rg` / `grep` / `fd` —
только fallback (см. правила ниже). **Python в репозитории нет** — Python-специфичные
инструменты (`framework_aware_refs` и пр.) не нужны.

Это **dogfood**: мы разрабатываем ArcanaCodex и не имеем права проповедовать MCP-first,
не гоняя свои же analysis-тулы на своём коде. Аудит **только читает** — никаких правок.
Фиксы/рефактор — отдельный шаг, строго по dogfood-ритуалу из `CLAUDE.md`
(fmt → lint → MCP coverage → MCP refactor → повтор → bump/commit/deploy/live-verify),
только после согласования.

**Аргумент:** `$ARGUMENTS` — если задан путь/`path_prefix`/модуль, сфокусируй аудит на
нём (передавай в `path_prefix`/`scope` где применимо). Если задана ветка/MR — включи
diff-категорию (см. ниже). Если пусто — аудит всего репо.

## Карта проекта (для прицеливания tools)

Cargo workspace, два крейта + фронтенд:

- **arcanaglyph-core** (`crates/arcanaglyph-core/src/`) — библиотека (`[lib]`):
  - `lib.rs` — публичный API: `ArcanaEngine`, `CoreConfig`, `EngineEvent`, `ArcanaError`
  - `engine.rs` — движок: start/stop записи, broadcast событий
  - `transcriber/` — трейт `Transcriber`, фабрика `build_transcriber`, `vosk.rs`,
    `whisper.rs`; `gigaam/`, `qwen3asr/` (mel.rs, transcriber.rs) — ONNX/CTC-движки
  - `dsp/` — `spectral.rs` (STFT/Hann), `preprocess.rs` (trim_silence/resample) — leaf
  - `audio.rs` — захват через `cpal`; `input.rs` — вставка текста (clipboard + XDG/Ctrl+V/
    Shift+Insert, платформо-зависимо); `config.rs`/`db.rs` — SQLite; `error.rs` — `thiserror`
- **arcanaglyph-app** (`crates/arcanaglyph-app/src/`) — Tauri v2 приложение:
  - `main.rs` — точка входа (logging + panic-hook + Tauri Builder + хоткеи)
  - `setup/` — `bootstrap.rs`, `app_setup.rs`, `events.rs`
  - `commands/` — Tauri-команды (`updater_cmds.rs`, `platform.rs`, history/config/models)
  - `updater.rs` — авто-обновления (GitHub Releases, ETag, asset-проверка); `tray.rs`
- **frontend** (`frontend/src/`) — TypeScript + Vite, FSD-структура, IPC через Tauri.
  Тест-харнесс: vitest + jsdom (`npm --prefix frontend test`).

Примеры `path_prefix`: `crates/arcanaglyph-core/src/`, `crates/arcanaglyph-app/src/`,
`frontend/src/`. Арх-правила и плагины — в `.arcanacodex.toml` (`[architecture]`) и
`.arcanacodex/plugins/*.toml`.

## Правила работы

1. **FIRST ACTION — `ping`.** Без исключений. Только после успешного pong — любая другая
   работа. Зафиксируй `server_version` из ответа.
2. **MCP-FIRST.** Первый инструмент по любому вопросу про код = MCP-tool. К `rg`/`Read`/`fd`
   переходи ТОЛЬКО если: (a) MCP-tool вернул `confidence: low` или пустой scope, (b) файл
   вне индекса (vendor, node_modules, `*.md`, бинарь, язык вне Rust/TS), (c) задача
   тривиально-литеральная (точная подстрока, git/npm/cargo команда).
3. **Читай envelope целиком** — `confidence`, `*_polarity`/`legend`, `partial`-флаги. Не
   интерпретируй score без указанной полярности. Где `confidence: low/medium` — помечай
   вывод как требующий ручной проверки, не подавай как факт.
4. **Ничего не правь.** Этот прогон — диагностика и отчёт.

## Покрытие — все тулы, не только сегодняшние

Аудит должен прогнать **каждый** релевантный `mcp__arcana-codex__*` analysis-тул,
**включая добавленные в будущем**. НЕ полагайся только на список ниже — MCP-сервер растёт,
фиксированный перечень устаревает. Перед прогоном обнаружь актуальный набор:

```text
ToolSearch(query="mcp__arcana-codex__ audit health security coverage dependency debt anomaly")
```

и аналогичными запросами — включи новые analysis-тулы в подходящую категорию.

> **`db_schema` намеренно НЕ в списке** — у нас нет ORM (SQLite через `rusqlite`/`rusqlite`-
> обёртки напрямую), тул вернёт пусто. Не тратить на него агента. **Route-тулы**
> (`route_index_inspect`/`route_callers`) тоже НЕ нужны — у Tauri-приложения нет HTTP-роутов
> (IPC-команды framework-dispatched, но это не web-маршруты; их FP покрыт плагином
> `tauri-commands-not-dead`).

## Ловушки интерпретации (читать ПЕРЕД выводом находок)

Не выдавать сырой вывод тула за факт. Каждому суб-агенту передать релевантные:

- `dead_code_candidates` — **высокий FP.** Каждого кандидата подтверждать `who_calls` /
  `references` ПЕРЕД «можно удалить». FP на функциях-значениях (`.map(fn)`), cross-language
  вызовах, и особенно на `#[tauri::command]` (регистрируются макросом `generate_handler!`,
  ребро через макрос граф не видит — см. плагин `tauri-commands-not-dead`).
- `architecture_check` — проверять `layers_coverage` / `layering_zero_coverage`: пустой
  `layering_violations:[]` при НУЛЕВОМ coverage — это глоб-промах (globs в `[architecture]`
  не покрыли файлы), НЕ «слои чисты». Напоминание: cross-crate рёбра Rust-резолвер не строит —
  осмысленны только intra-crate инварианты (см. `.arcanacodex.toml`).
- `taint_paths` — confidence обычно ≤ medium → подавать как «к ручной проверке», не как
  подтверждённую уязвимость.
- `validate_edit` — читать массив `violations`, не только `passed`: `passed:true` бывает при
  warn-нарушениях.
- `naming_consistency` / `css_file_health` — confidence medium (динамика не видна) → это
  КАНДИДАТЫ; css-dead подтверждать `css_references` (0 locations).
- `unused_dependencies` — token-presence, не доказательство. Учитывать platform-cfg deps
  (`[target.'cfg(windows)'.dependencies]`) — под текущей платформой выглядят unused ложно.
- Полярности не путать: `recommendation_score` / `file_health` — **HIGHER IS BETTER**
  (<60 critical, 60–85 warning, ≥85 ok); `hotspot_score` / `divergence_score` /
  `metric_anomalies` modified_z — **HIGHER IS WORSE**. Читать `*_polarity`/`legend` из
  envelope, не додумывать.

## Как прогонять — параллельные суб-агенты по категориям

Чтобы покрыть всё и не забить контекст сырыми envelope, разверни **по одному суб-агенту на
каждую непустую категорию** (ориентир ~7–9 агентов; пустые категории — напр. CSS, если
правок фронта нет — можно не разворачивать). Каждому суб-агенту:

- В начало промпта — блок-преамбулу MCP-first: тулы `mcp__arcana-codex__*` DEFERRED, грузить
  схему через `ToolSearch`, **вызывать напрямую как tool-calls, НЕ через `Skill`** (обёртка в
  `Skill` даёт `Unknown skill` и теряет шаг), первым шагом `ping`. Правило MCP-first в силе.
- Передать релевантные ловушки интерпретации из блока выше.
- Требовать **сжатую** выжимку (таблицы/списки находок с severity и confidence), без сырых
  JSON-портянок.

## Учёт defer-list

- Тулы вызывать с `include_deferred=false` (по умолчанию так) — намеренно отложенное в
  `.arcanacodex.toml` (`[[audit.defer]]`) не должно засорять отчёт. _(Сейчас defer-list пуст —
  но механику соблюдать.)_
- Если тул вернул `defer_suppressed > 0` — отдельной строкой отметить, сколько находок
  исключено by-design.

## Категории и тулы

Скоупь каждый tool на target (`$ARGUMENTS`, если задан). CSS-категория — только для frontend.

### 0. Ориентация

- `code_orientation` и/или `session_brief` — карта проекта, точки входа, языки.
- `module_tree` — структура модулей обоих крейтов + фронтенда.
- `architecture_hubs` — узлы с наибольшей связностью (god-modules, хабы). Ожидаемые
  подозреваемые: `engine.rs`, `app_setup.rs`, `input.rs`, `updater.rs`.
- `concept_glossary` — доменный словарь (ключевые сущности и где определены).

### 1. Здоровье и рефактор-кандидаты

- `refactor_candidates(scope="all", exclude_tiers=["tooling","tests","docs","generated"],
  detect_semantic_clones=true)` — топ-кандидаты (cc-hotspots, god-objects, data-clumps).
  Полярность `recommendation_score` = HIGHER IS BETTER (<60 critical, 60–85 warning, ≥85 ok).
- `file_health(<path>)` для каждого подозрительного файла из топа.
- `function_skeleton(<file>)` / `directory_function_skeleton(<dir>)` — структура вместо
  чтения целиком (особенно `engine.rs`, `gigaam/transcriber.rs`, `setup/app_setup.rs`).
- `extract_module_suggestion(<god-object>)` / `port_module` — предложения по декомпозиции.
- `metric_anomalies(path_prefix=<target>, peer_group="module")` — функции-выбросы по
  cc/loc/param_count/nesting **относительно своего модуля** (robust median+MAD): `cc=18` в
  модуле с медианой 4 всплывёт, даже если ниже глобального порога `refactor_candidates`.
- `magic_number(path_prefix=<target>)` — хардкод-литералы под вынос в именованную конст
  (`* 16000` → `SAMPLE_RATE_HZ`). По умолчанию `scope=staged` — для полного прогона дай
  `path_prefix`.

### 2. Дубли, клоны и реинвент

- `find_similar(cluster_mode=true, scope="functions", exclude_test_functions=true)` —
  кластеры семантически похожих функций (Type-3/-4 клоны). Кандидаты: платформо-ветки в
  `input.rs`, препроцессинг в движках `transcriber/` (vosk/whisper/gigaam/qwen3asr).
- `find_similar(cluster_mode=true, aggregate="file")` — какие ФАЙЛЫ дублируют друг друга.
- `find_similar(symbol="<якорь>")` — точечно по символу из топа hotspot'ов.
- `code_for_intent("<NL-описание>")` — разбросанные реализации одной логики (напр. «resample
  аудио к 16 кГц», «выбор бинаря по CPU avx/noavx»).
- `naming_consistency(path=<target>)` — одна операция под разными глаголами (`get_`/`fetch_`/
  `load_`) — реинвент/реюз-сигнал (confidence medium, кандидаты).
- `message_chains` — нарушения Law of Demeter (длинные цепочки вызовов).
- Data-clumps приходят в `refactor_candidates` — выпиши отдельным списком.

### 3. Тесты, покрытие и техдолг

- `test_coverage_overview` — общая картина (отдельно Rust-крейты и `frontend/`).
- `untested_publics(path_prefix=<модуль>)` по ключевым модулям: `crates/arcanaglyph-core/src/`,
  `crates/arcanaglyph-app/src/commands/`, `frontend/src/`. Публичные символы без тестов.
- `assertion_free_tests` — тесты без ассертов (фиктивное покрытие).
- `test_impact` — какие тесты затрагивают критичные пути (DSP, transcriber, updater).
- `debt_markers(path_prefix=<target>, min_age_days=180)` — застоявшийся техдолг
  (TODO/FIXME/HACK/XXX только в комментариях) с git-blame: возраст, автор, плотность.
  severity: FIXME/XXX=high, HACK=medium, TODO=low. Полярность: СТАРШЕ = тревожнее.

### 4. Мёртвый код и зависимости

- `dead_code_candidates` — кандидаты. **🛑 Высокий FP:** каждый подтверждай `references` /
  `who_calls` ПЕРЕД «можно удалить». Держи в уме FP на `#[tauri::command]` (макрос-регистрация).
- `unused_dependencies` — объявленные, но неиспользуемые (token-presence, помечай confidence;
  учти platform-cfg deps). Проверь `Cargo.lock` и `frontend/package-lock.json`.
- `dependency_bloat(path=<target>)` — какие прямые deps тянут самое тяжёлое дерево
  (`exclusive_total`). Offline.
- `dependency_audit(path=<target>)` — supply-chain CVE (OSV.dev + EPSS/KEV). **Network.**
- `outdated_dependencies(path=<target>)` — отставание от latest (major/minor/patch). **Network.**
- `license_compliance(path=<target>)` — copyleft/AGPL/GPL риск для закрытого продукта. **Network.**

### 5. Безопасность и корректность

- `taint_paths(inter_procedural=true)` — пути source→sink, 8 классов + CWE. Прицельные зоны:
  `input.rs` (вставка в чужое окно), `updater.rs` (скачивание/применение апдейтов — asset/
  подпись), `config.rs`/`db.rs` (SQLite). Confidence обычно ≤medium — «к ручной проверке».
- `async_safety(path_prefix=<target>)` — блокирующие вызовы в async / забытые await
  (Tauri-команды, updater, audio-поток).
- `error_handling` — проглоченные ошибки, `unwrap`/`expect` на горячих путях. Кодекс ошибок —
  `ArcanaError` через `thiserror`.
- `literal_comparison` — подозрительные сравнения с литералами (TS-фронт).
- `diagnostics(<file>)` — LSP-ошибки/предупреждения (типы, компилятор) по тронутым файлам.

### 6. Архитектура и эволюция

- `architecture_check(mode="all")` — циклы + нарушения слоёв (правила в `.arcanacodex.toml`
  `[architecture]`: intra-crate изоляция `gigaam`⊥`qwen3asr`, `dsp` — leaf). **🛑 Проверяй
  `layers_coverage` / `layering_zero_coverage`** — пустой список при нулевом coverage = промах.
- `change_coupling` / `change_risk` — модули, меняющиеся вместе (скрытая связность).
- `ownership` — bus-factor (solo-разработчик → сигнал условен, отмечай).
- `doc_coverage` — покрытие документацией + drift (рассинхрон сигнатур и доков).

### 7. Соответствие командным правилам (плагины)

Свежий код против командных правил, не только метрик. Плагины — в `.arcanacodex/plugins/*.toml`:

- `applicable_rules(file_path=<файл>)` — какие плагин-правила применимы к файлу
  (whisper abort-callback, платформо-вставка, changelog version-bump, tauri dead-code).
- `validate_edit(file_path=<файл>, content=<содержимое>)` — прогнать тронутые файлы против
  плагинов. Смотри массив `violations` (`rule`/`line`/`severity`), не только `passed`.

### 8. CSS-здоровье (только frontend)

- `css_file_health(file_path="frontend/src/.../*.css")` — мёртвые селекторы, неиспользуемые
  `--vars`, дубли. Confidence medium (динамические `className` не видны) → кандидаты.
- `css_references(class_name=<кандидат>)` — подтвердить dead-селектор (0 locations → удалять).

### 9. Diff-аудит (если фокус на ветке/MR)

- `changed_symbols` / `diff_since` — что реально изменилось.
- `semantic_diff` — семантические изменения по тронутым символам.
- `breaking_changes` — поехали ли публичные сигнатуры/контракты.
- `affected_by(<symbol>)` — радиус поражения тронутого символа.

## Формат итогового отчёта

Собери всё в один markdown-отчёт (соблюдай `.markdownlint.yaml`: пустые строки вокруг
заголовков/списков/таблиц, язык в кодовых блоках, line_length 120), **от критичного к
низкоприоритетному**:

```text
# Аудит ArcanaGlyph — <дата>
server_version: <из ping>

## Сводка (TL;DR)
- 3–5 главных проблем по убыванию серьёзности.

## 1. Рефактор-приоритеты
| Файл/символ | Проблема | score/cc | Рекомендация | confidence |
- Плюс: metric_anomalies (выбросы по модулю), magic_number (хардкод-литералы).

## 2. Дубли и реинвент
- Кластеры клонов (function/file), naming-реинвент, data-clumps — с путями.

## 3. Тесты и техдолг
- Покрытие, untested publics (топ), assertion-free тесты, debt_markers (старый TODO/FIXME).

## 4. Мёртвый код и зависимости
- Только ПОДТВЕРЖДённые кандидаты (доказательство references/who_calls). unused/bloat/CVE/outdated.

## 5. Безопасность
- taint / async / error-handling / diagnostics. Каждая запись — confidence + «нужна ручная проверка».

## 6. Архитектура
- Циклы, нарушения слоёв (.arcanacodex.toml [architecture]), coupling, bus-factor, doc-drift.

## 7. Соответствие правилам (плагины)
- Нарушения applicable_rules / validate_edit по свежим файлам.

## 8. CSS (frontend)
- Мёртвые селекторы/переменные (подтверждённые css_references).

## Предлагаемый план действий
- Упорядоченный список: что чинить первым, оценка усилий, риск.
```

Для каждой находки указывай **инструмент-источник**, **confidence** и **где именно**
(`file:line` / qualified-name). Honesty-сигналы не прятать: `layering_zero_coverage`,
`confidence: low`, `available: false`, `defer_suppressed` — выносить явно (пустой результат
≠ «чисто»). Low/medium не выдавай за факт. Ничего не правь — только диагностируй и предложи план.

$ARGUMENTS
