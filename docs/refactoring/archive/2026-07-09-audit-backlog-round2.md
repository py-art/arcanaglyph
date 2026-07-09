---
status: closed
created: 2026-07-09
closed: 2026-07-09
source: /aidit round 2 (ArcanaCodex MCP full audit, server v0.461.0, HEAD b20c99e)
---

# Refactoring backlog — 2026-07-09 (round 2)

Источник — второй прогон `/aidit` (после закрытия и архивации первого бэклога
2026-07-09). Крупного долга нет: архитектура чиста (0 циклов, layering valid,
module-isolation соблюдён), 0 debt-маркеров, 0 assertion-free тестов, мёртвого кода нет,
0 copyleft-риска, CSS чист. Ниже — небольшие новые actionable-пункты (часть — «остатки вне
scope» прошлых правок, теперь стоящие самостоятельно). Исполнение — `/refac` (по одному
пункту, plan mode, полный dogfood-ритуал, коммит после каждого пункта).

## Регламент (кратко)

- 🛑 **НЕ пушить** — ни в одном режиме, включая `auto`. Коммит можно; финальный
  squash + push делает пользователь вручную.
- Правки **behaviour-preserving** (кроме S1 — bump зависимости), минимальные под пункт.
  Перед крупным сносом (⚠️) или footgun-gated (🔒) — подтверждение пользователя.
- Verify-before-refactor: перед правкой повторно проверить актуальность находки через MCP.
- Пометки: 🔒 footgun-gated (advisory-плагин), ⚠️ крупная/структурная правка.

## Группа S — Security (зависимости)

- [x] S1. 🔒 **2×high CVE** `quick-xml` 0.39.4 (RUSTSEC-2026-0194/0195: DoS —
  quadratic parsing + unbounded namespace alloc) → fix 0.41.0. Тянется **транзитивно**
  через `plist 1.9.0` (→ `tauri-utils`). Прямой `--precise 0.41.0` невозможен (plist 1.9.0
  требует `^0.39.2`); решено `cargo update -p plist` → plist 1.10.0 тянет quick-xml 0.41.0.
  `cargo tree -i quick-xml` подтверждает 0.39.4 ушла. Version-bump приложения 1.10.5→1.10.6
  (patch, dep-security-pin, поведение не меняется, 6 точек + 2 lock) + CHANGELOG [Unreleased]
  «Безопасность». 92 теста ✅, clippy/markdownlint чисты. Готово: `cb3eec4`.

## Группа D — Сложность / god-функции (⚠️ структурные)

- [x] D1. ⚠️ `crates/arcanaglyph-app/src/models/download.rs` — `ensure_active_model`
  расщеплён на 5 под-функций: `resolve_active_model_path` / `select_registry_model`
  (чистые, покрыты 5 юнит-тестами) + async `prepare_main_dest` / `remove_corrupted_main_file`
  / `download_model_files`. cc **34 → 8** (файл score 67 → 80). Cover-before-refactor:
  чистые ядра покрыты тестами до/при разбиении; async-обёртки — дословный перенос строк.
  breaking_changes: 0 breaking (публичная сигнатура сохранена, 12 internal → patch).
  Остаточный warning-badge на `download_file` (cc17, sub-threshold) — вне scope D1.
  Готово: `42a168c`.
- [x] D2. ⚠️ `frontend/src/features/settings/load-config.ts` — `loadSettings`
  декомпозирован: чистые `buildSnapshot` / `resolveTranscriberDropdown` (покрыты 6 vitest,
  cover-before-refactor) + секции-заполнители `loadCpuFeaturesOnce` / `fillGeneralFields`
  / `fillPrivacyAndWidgetFields` / `fillPreloadToggles`. cc **32 → 1** (файл score 80 → 95,
  severity warning → **ok**). Behaviour-preserving: порядок DOM-заполнения сохранён.
  102 vitest ✅, type-check чист. Готово: `7822596`.
- [x] D3. Оценены три функции; резал только там, где чистый seam:
  - `getFormConfig` (save-config.ts, cc **25 → 14**, файл score→85 ok): вынесены чистая
    `resolveWhisperSelection` (4 vitest) + `collectPreloadSelection`. Готово: `43674d5`.
  - `mountHistoryPage` (history-page.ts, cc **25 → 11**, nesting 3→2, файл score→95 ok):
    вынесены bind-группы listener'ов + `restoreHistoryUiState`. Готово: `2de23f9`.
  - `download_model` (registry.rs, cc 23): в рамках D3 не резал (локальный helper = третий
    клон). Позже **устранён дедупом в этой же сессии** — унификация с `download_model_files`
    (см. Заметки, cc 23 → 10). 106 vitest ✅, type-check чист.

## Группа A — Дубли (near-clone внутри файла)

- [~] A1. **verify-FP.** `add_recording` / `add_transcription` (sim 0.94) — находка закрылась
  побочно: обе функции **уже дедуплицированы** через общий `execute_write(sql, params, err_ctx)`
  и представляют собой 2-3 строки `let ts = Utc::now().timestamp(); self.execute_write(INSERT…,
  params![…], ctx)`. Вся варьирующаяся часть (SQL/params/err_ctx) — ровно аргументы execute_write.
  Единственное общее сверх него — `Utc::now().timestamp()`, но ts на **разных позициях** params
  (recording ?2, transcription ?5) → `execute_insert_now`-хелпер невозможен без переупорядочивания
  SQL-колонок. Дальнейшая экстракция воспроизвела бы уже существующий helper. Defer-sync:
  `[[audit.defer]]` add_recording + add_transcription в `.arcanacodex.toml`.

## Группа C — Корректность (unwrap на горячем пути)

- [x] C1. `qwen3asr/transcriber.rs` — оба unwrap убраны: `:372` `row.as_slice().unwrap()` →
  `embed_and_fuse` переведён на `Result<Vec<f32>, ArcanaError>` с `.ok_or_else(ArcanaError::Recognizer)?`
  (call-site получил `?`); `:538` `generated.last().unwrap()` → `let Some(&last_token) = … else { break }`
  (пустой вектор → конец генерации без паники). Behaviour-preserving на нормальном пути.
  error_handling(файл): 2 unwrap → **0**. clippy/test с `--features qwen3asr` (99 тестов) ✅.
  breaking_changes: 0 breaking (embed_and_fuse private, 1 internal → patch). Готово: `fd5963f`.

## Группа M — Magic-number гигиена (Introduce Constant)

Снижает magic_number-шум и риск дрейфа. Резать группами, не ради одной метрики.

- [x] M1. GSettings-строки вынесены в именованные `const`. `hotkeys.rs`: `GSETTINGS`,
  `MEDIA_KEYS_SCHEMA`, `CUSTOM_KEYBINDINGS_KEY`, `EMPTY_GVARIANT_ARRAY` (`@as []`) — magic_number
  35 → 22. `commands/widget_ext.rs` (не `tray/`): `GSETTINGS`, `GNOME_SHELL_SCHEMA`,
  `ENABLED_EXTENSIONS_KEY` — 15 → 3. Остаток (gsettings CLI-subcommands `"get"`/`"set"`,
  property-ключи `"name"`/`"binding"`, placeholder `"???"`) оставлен строками — самодокументируем
  как аргументы команды, вынос в const ухудшил бы читаемость. Behaviour-preserving, clippy/тесты ✅.
  Готово: `8d93599`.
- [x] M2. ⚠️ Свёл engine-id к единому источнику. **config.rs** (magic_number 41 → 24):
  `transcriber_type_str()` — весь match заменён на `self.transcriber.as_str().to_string()`;
  `model_name_for()` fallback → `t_type.as_str()` (5 литералов убраны); модель-файловые имена
  вынесены в const `VOSK_MODEL_DIR`/`WHISPER_MODEL_FILE`/`QWEN3ASR_MODEL_DIR`/`GIGAAM_CTC_MODEL_DIR`/
  `GIGAAM_RNNT_MODEL_DIR` (устраняет дрейф между `default_*_model_path()` и `Default`).
  Остаток engine-id в config.rs — **источники истины** (`as_str` L66-70, `from_id` L77-81) и
  `cfg!(feature=…)` (макрос требует литерал) — by-design, неизбежны (const в match-pattern =
  footgun, поэтому from_id держит литералы). **models/{installed,registry,download}.rs**: engine-id
  в `&str`-match — идиоматичный парсинг входа (аналог `from_id`); перевод на enum добавил бы
  app→core coupling ради 5 строк/файл и сменил unknown-handling → by-design, defer-sync
  `[[audit.defer]]` block=magic_number для трёх файлов. clippy/73+ тестов ✅. Готово: `dbbac70`.
- [x] M3. Числовые дубли вынесены. `setup/events.rs`: `86400` ×4 → `SECONDS_PER_DAY`,
  `23*3600` → `UPDATE_CHECK_THROTTLE_SECS`, `7*86400` → `MAX_UPDATE_BACKOFF`. Размеры виджета
  `220.0`/`40.0` (дублировались между `app_setup.rs::build_recording_widget` и
  `config_history.rs`, ОБЯЗАНЫ совпадать иначе позиция разъедется с окном) → `pub const
  WIDGET_WIDTH`/`WIDGET_HEIGHT` в core `config.rs`, оба места импортируют. Одиночные `60`/`1024`
  (events) и `3600` sleep-тайминги (app_setup) оставлены — эссенциальны/самодокументируемы
  контекстом (по регламенту группы M). clippy/73+92 тестов ✅. Готово: `75f1d3c`.

## Группа L — Low priority (тесты + доки)

- [x] L1. Юнит-тесты добавлены: `test_scripts_dir_ends_with_scripts`,
  `test_trigger_socket_path_ends_with_sock` (cfg linux) в config.rs; `test_audio_cache_path_returns_configured_dir`
  в history.rs. MCP `untested_publics(config.rs)` теперь structural_coverage **1.0** (0 untested).
  core-тестов 92 → 95, clippy ✅. Готово: `9f52598`.
- [x] L2. `///`-доки добавлены: `record_and_transcribe` (audio.rs — цикл запись→транскрибация)
  и `add_recording` (history.rs — вставка recording, возврат rowid). `doc_coverage` изменённых
  файлов: pct **1.0**, 0 undocumented, 0 drift. clippy ✅. Готово: `9ebdfb1`.
  _(6× `MODEL` const в `transcription_models/*` — самоописательны, низкий приоритет / кандидат в defer.)_

## Заметки (НЕ в бэклог — by-design / вне охвата)

- `@tauri-apps/api` помечен `unused_dependencies` (medium) — **FP**: используется через
  `window.__TAURI__` (стаб в test-setup, рантайм-инъекция Tauri). Не удалять.
- `updater.rs` / `core/config.rs` / `core/audio.rs` critical score — **FP**: раздут
  inline-тестами (updater) и cohesive-accessor/serde-default (config, lcom4=1). Реального
  prod-seam нет (уже в defer-list прошлого раунда).
- Полный dead-selector-скан фронта невозможен: стили инлайн в `index.html`/`widget.html`,
  `css_file_health` неприменим к не-`.css`. Потенциальный крупный пункт «экстракция inline-CSS
  в отдельный файл» — не срочно, требует продуктового решения.
- outdated deps (major: `thiserror` 1→2, `toml` 0.8→1.1, `typescript` 5→7, `vite` 6→8;
  minor критичный: `enigo` 0.3→0.6 — путь вставки текста) — вне scope рефакторинга, отдельный
  bump-спринт с live-verify по каждой критичной депе.
- ~~**Кандидат след. раунда (cross-file clone):** `registry.rs::download_model` else-блок~~
  **УСТРАНЕНО в этой же сессии** (по запросу пользователя, до финального squash). `download_model_files`
  (download.rs) стал `pub(crate)` общим helper'ом с расширенным контрактом (`url`/`model_id`/`extras`
  отдельными параметрами вместо `&SpeechModelInfo` — работает и для ручной загрузки по URL из UI, где
  модели может не быть в реестре). `registry.rs::download_model` else-блок (~30 строк) заменён вызовом
  helper'а. cc download_model **23 → 10** (registry score 75 → 85, warning → **ok**). breaking_changes:
  0 breaking (download_model_files pub(crate) internal; download_model tauri-сигнатура цела). Клона
  больше нет.
