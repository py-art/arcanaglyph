---
status: closed
created: 2026-07-09
closed: 2026-07-09
source: /aidit (ArcanaCodex MCP full audit)
---

# Refactoring backlog — 2026-07-09

Источник — прогон `/aidit` (ArcanaCodex MCP). Исполнение — командой `/refac`
(по одному пункту, plan mode, полный dogfood-ритуал, коммит после каждого пункта).

## Регламент (кратко)

- 🛑 **НЕ пушить** — ни в одном режиме, включая `auto`. Коммит можно; финальный
  squash + push делает пользователь вручную.
- Правки **behaviour-preserving**, минимальные под пункт. Перед крупным сносом (⚠️) или
  footgun-gated (🔒) — подтверждение пользователя.
- Verify-before-refactor: перед правкой повторно проверить актуальность находки через MCP.
- Пометки: 🔒 footgun-gated (advisory-плагин), ⚠️ крупная/структурная правка.

## Контекст аудита (baseline)

- Архитектура чистая: 0 circular deps, 0 layering, 0 module-isolation.
- Debt-маркеры (TODO/FIXME/HACK/XXX): 0.
- Dead-code: 37 Rust-кандидатов = `#[tauri::command]` (framework-dispatched FP, НЕ трогать —
  [[feedback_dead_code_tauri_commands_fp]]); 7 TS-кандидатов требуют verify (см. группу G).
- Coverage: overall 0.70; провал `setup/` 0.0 (Tauri-framework), `models/` 0.25,
  persistence 0.25.
- Critical refactor-score (<60): audio.rs 42, updater.rs 45, main.rs 50, history.rs 50,
  config.rs 55, qwen3asr/transcriber.rs 55.

## Группа A — Безопасные точные дубликаты (Type-1 exact, extract_local_helper)

Механические, поведение-сохраняющие извлечения (`merge_safety: safe_identical`).

- [x] A1. `crates/arcanaglyph-app/src/tray.rs` — `set_tray_text` / `set_tray_visible`
  (identical, ~10 loc) → извлечён generic `with_tray_state<T,F>`, применён также в
  `set_tray_state`. health 85 (ok). `08c2f83`.
- [~] A2. `crates/arcanaglyph-core/src/history.rs` — `map_recording_row` /
  `map_transcription_row` — **verify-FP**: не клон, а две разные проекции таблиц в
  разные типы (4 vs 6 полей). Унификация = over-engineering. Defer-sync в
  `.arcanacodex.toml` (symbol-level).

## Группа B — Near-клоны внутри файла (Type-3, review divergence)

- [x] B1. `crates/arcanaglyph-app/src/updater.rs` — `dismiss` / `set_applying`
  (+ `clear_applying`, тот же RMW-паттерн) → извлечён `with_state_mut(db, |s| ...)`.
  Footgun clear (плагин только на CHANGELOG.md). Изменённые функции cc≤1. `4a73838`.
- [~] B2. `crates/arcanaglyph-app/src/net.rs` — `gsettings_get` / `system_proxy_url`
  — **verify-FP**: под разными `cfg(target_os)`, никогда не компилируются вместе;
  ~3 строки общего, постобработка различна. Дедуп фрагментировал бы платформо-код.
  Defer-sync в `.arcanacodex.toml`.
- [x] B3. `crates/arcanaglyph-core/src/engine/mod.rs` — `loaded_models` /
  `active_model_name` (+ `active_transcriber_type`, та же идиома) → извлечена
  ассоц. generic `Self::read_or_default(lock, f)`. clippy+88 тестов зелёные. `26aa122`.
- [~] B4. `crates/arcanaglyph-app/src/setup/bootstrap.rs` — `set_autostart` —
  **verify-FP**: не 2, а 4 cfg(target_os)-варианта (linux `.desktop` / windows реестр
  Run / macos plist / other no-op). Клон-детектор ловит пару linux↔macos (обе
  file-based), но: (1) под взаимоисключающими cfg → хелпер требовал бы
  `cfg(any(linux,macos))` + ещё слой в файле с 4 ветками; (2) macos-ветка НЕ
  компилируется локально И не в текущем CI (macos-порт лишь запланирован) — правка
  непроверяема нигде до порта; (3) windows (реестр) — иная логика, вне пары.
  Тот же класс, что B2/net.rs. Defer-sync в `.arcanacodex.toml` (symbol-level).
- [x] B5. `crates/arcanaglyph-core/src/history.rs` — `set_setting` / `add_recording` /
  `add_transcription` (sim 0.69) → извлечён private `execute_write(sql, params, err_ctx)`
  (lock_conn + execute + map_err, возвращает rowid). Три метода переведены; текст ошибки —
  параметр, для `set_setting` rowid игнорируется. Прецедент в файле — `lock_conn`. Сигнатуры
  pub не тронуты, 88 тестов зелёные. `89e46d8`.
- [x] B6. ⚠️ `crates/arcanaglyph-app/src/commands/config_history.rs` — `retranscribe` /
  `get_audio_data` (sim 0.64) — **verify: функции разошлись**, общий только 8-строчный
  пролог (`query`→`find` по id→`audio_exists`-guard), тела ~85 loc уникальны. Извлечён
  private `find_recording_with_audio(db, id, limit, missing_msg)` — `into_iter().find()`
  (owned, без клона), `limit`/`msg` параметры (сохранён расхождение 1000 vs 100000). Обе
  `#[tauri::command]` целы (footgun tauri-commands-not-dead соблюдён). 88 тестов. `eb79af6`.
  _Остаток:_ `retranscribe` cc23 (цепочка early-return валидаций) — предсуществующий, не
  внесён этой правкой; кандидат для D-группы (снижение сложности), вне scope B6.

## Группа C — Cross-file клоны

- [x] C1. `argmax` (`gigaam/transcriber_rnnt.rs`) / `argmax_token`
  (`qwen3asr/transcriber.rs`) (sim 0.77, ~16 loc) → извлечён общий cfg-gated модуль
  `decode.rs` с `argmax_index(&[f32]) -> Option<usize>` (NaN-safe max_by-идиома once).
  Три потребителя стали тонкими обёртками со своим fallback: rnnt `argmax` (→0),
  qwen `argmax_token` (→`ENDOFTEXT_ID`), + inline-argmax в `ctc_greedy_decode` (→blank).
  structural/clone-детектор чист, breaking=0 (все fn приватные), 97 тестов (gigaam+qwen).
  `2f1611b`.
- [x] C2. ⚠️ `gigaam/transcriber.rs::transcribe` / `gigaam/transcriber_rnnt.rs::transcribe`
  (sim 0.80, ~100 loc, cross-file) — **review divergence: тела архитектурно различны**
  (CTC single-session + `ctc_greedy_decode` vs RNN-T encoder + авторегрессивный
  `greedy_decode` decoder+joint). Реально общий только пролог (препроцессинг + mel + два
  guard'а) → извлечён `mel::prepare_mel_input(&[i16], u32) -> Option<Array3<f32>>` (оба
  transcribe стали `let Some(mel_spec) = ... else { return Ok("") }`, unused-импорты убраны).
  Инференс/декод НЕ унифицирован (over-abstraction на STT-критичном пути) — остаток
  задокументирован как by-design в `.arcanacodex.toml` (symbol-level defer на обе `transcribe`).
  breaking=0 (`transcribe` — trait-impl, контракт `Transcriber` цел), +2 unit-теста
  на guard/happy-path, 92 теста. `9a2153b`.

## Группа D — Сложность / god-функции (⚠️ структурные)

- [x] D1. ⚠️ `crates/arcanaglyph-app/src/main.rs` — `main` (cc 32, hotspot #1 = 256):
  `extract_function` из `main` (setup-шаги в именованные функции). Извлечены 4 сидинг-
  хелпера (`handle_early_exit_args`, `init_diagnostics`, `load_startup_config`,
  `parse_shortcuts`) + 2 свободных хендлера вместо инлайн-замыканий
  (`handle_global_shortcut`, `handle_window_event`). `main` стал тонким оркестратором:
  cc 32→4, loc 175→97, score файла 50→70 (critical→warning, actionable-рекомендаций 0).
  Комментарии перенесены в хелперы. Все символы приватные (breaking=0), +1 unit-тест на
  инвариант `parse_shortcuts` (пустой pause → None). 93 теста зелёные. `35e7176`.
- [x] D2. ⚠️ `crates/arcanaglyph-core/src/audio.rs` (score 42, loc 856): снизить сложность
  `build_capture_stream` (cc 26); + data-clumps (`audio_level/debug/state`,
  `debug/state/stream`) → Parameter Object. Сделано: `build_capture_stream` cc **26→9**
  (loc 146→58) — вынесены `try_build_preferred_stream` + `build_native_stream`; пять
  разделяемых приёмников callback'а (`mic_gain/level/frames/samples/vosk_tx`) собраны в
  `CaptureSinks` (метод `process`) — снимает clump на build-пути; три control-ссылки цикла
  (`stream/audio_level/debug`) собраны в `Copy`-`RecordCtx`, переданный в
  `handle_command`/`resume`/`pause`/`pump_audio_progress`. **Оба data-clump'а растворились**
  (clump_count 2→0), actionable-рекомендаций файла 0, long-param-list 6→4 (бонус). breaking=0
  (тронутые символы приватные, pub `record_and_transcribe` не менялся). 67+92 тестов зелёные.
  _Остаток:_ score 50 (всё ещё critical-band) держат `loc_factor`(910 loc)+`godobject` — это
  territory split-файла (**D4/E**), вне scope D2 (cc+clumps). `build_native_stream` cc 15 —
  эссенциальный 4-way `sample_format`-match, неразбиваем без индирекции. `12b37c3`.
- [x] D3. ⚠️ `crates/arcanaglyph-core/src/qwen3asr/transcriber.rs` — `decoder_step_once`
  (cc 27) снизить сложность. Драйвер cc — 12 повторяющихся
  `.map_err(|e| ArcanaError::Recognizer(format!("…: {}", e)))?` замыканий. Введён свободный
  хелпер `recog(res, ctx)` (обёртка в `Recognizer` с идентичной строкой `"<ctx>: <e>"` —
  поведение и текст ошибок не меняются), заменивший замыкания. `decoder_step_once`
  cc **27→15**. Тот же паттерн применён к `decoder_prefill` (cc 21) и `encode_audio` (cc 23)
  — file cc_max **27→22**, cc_avg 7.55→5.68, cc_p90 22→15; три функции ушли из
  actionable-рекомендаций. Mutex-lock оставлен на `Internal` (единичный, иная ветка).
  breaking=0 (`recog` + тронутые методы приватные), 99 тестов зелёные. `b6eca69`.
  _Остаток:_ файл всё ещё score 50 (critical) — теперь держат `new` (cc 22, конструктор
  загрузки 4 ONNX-сессий) + loc(620)/lcom4/divergent-change (`Qwen3AsrTranscriber`,
  7 concern-групп). Это territory отдельного split-item (класс D4/D5, split по concern),
  вне scope D3 (cc `decoder_step_once`).
- [~] D4. ⚠️ `crates/arcanaglyph-core/src/history.rs` — `HistoryDB` divergent change
  (score 50) — **verify-FP: реального seam нет.** `extract_module_suggestion` (designated-тул
  пункта): `movable_symbols 32`, но `clusters_passing_threshold 1`, и тот единственный —
  **тестовый** (`test_settings_get_set_replace` + `get_setting/set_setting/get_all_settings`),
  а не прод-граница. `file_health.recommendations` **пуст**; `extractability =
  "no_top_level_named_items_above_threshold — refactor requires invention, not extraction"`.
  `HistoryDB` — когезивный DAO: все методы делят `conn: Arc<Mutex<Connection>>` +
  хелперы `lock_conn`/`execute_write` (`lcom4_internal_call_ratio 0.75`). Score 50 держат
  пять базовых факторов малого файла (loc/cc/nesting/churn/lcom4 = по 10), причём `churn_30d=7`
  раздут нашими же коммитами этой сессии (A2/B5). Split ради метрики фрагментировал бы один
  struct без выигрыша (lcom4 не падает — struct один). Defer-sync в `.arcanacodex.toml`. `3065579`.
- [~] D5. ⚠️ `crates/arcanaglyph-core/src/config.rs` (score 55, loc 709) — **verify: split не
  оправдан.** `file_health.recommendations` **пуст**; `lcom4_max=1` → `CoreConfig` идеально
  когезивен (не god-object — `godobject_factor=10` ложный, от числа методов: 46 функций, из них
  ~15 тривиальных path-аксессоров и ~10 serde-`default_*` по 3 loc). Score раздут 129 строками
  комментариев (serde-доки) + ~220 loc тестов (32% файла) + churn. `extract_module_suggestion`:
  все 5 кластеров тест-контаминированы (`external_coupling=0.0` — кластеризация по тест-рёбрам,
  не по прод-архитектуре) — реального seam нет. Единственный отделимый юнит `TranscriberType`
  (enum + 4 метода) уже логически развязан с `CoreConfig`; вынос лишь релоцировал бы код без
  снижения coupling/score (`godobject` на `CoreConfig` не падает, все 5 импортов — `config::TranscriberType`),
  а колокация value-enum рядом с config-struct идиоматична. Резать «не ради метрики». Defer-sync. `8a2a7d6`.

## Группа E — Long parameter lists (Introduce Parameter Object)

Резать только там, где параметры логически связаны; не ради метрики.

- [x] E1. `models/download.rs::download_file` (7 params) → Introduce Parameter Object.
  Три инварианта пакетной загрузки (`app`/`model_id`/`total_files`) — идентичны для главного и
  всех extra-файлов во всех 4 вызовах — собраны в `DownloadCtx<'a>` (pub(crate), field-only),
  строится один раз перед циклом. `download_file` **7→5 параметров** (ctx + url + dest + file_idx +
  min_size). Обновлены 4 call-site (2 в `download.rs`, 2 в `registry.rs`) + импорт. breaking=0
  (`download_file` — `pub(crate)`, смена сигнатуры internal; поведение/эмит-payload не тронуты).
  clippy+92 теста зелёные. `039b10e`.
  _Остаток:_ `ensure_active_model` cc34 (предсуществующая god-функция, всплыла как changed лишь
  из-за вставки строк `ctx` — сложность не внесена E1) — кандидат D-класса, вне scope E1.
- [x] E2. `audio.rs` long-param-lists — **частичный FP: реальный clump был только у одной.**
  `process_captured` (6, единственный caller = `CaptureSinks::process`) — 5 из 6 аргументов уже
  поля `CaptureSinks`, «объект» существовал → тело инлайнено в метод `process`, свободная функция
  удалена (function_count 35→34, loc −21, cc метода 0→3). Остальные 4 — verify-FP/by-design,
  defer-sync в `.arcanacodex.toml`: `finalize_transcription` (7) — разнородный оркестратор, 4/7 уже
  объекты, скаляры не clump, clippy `too_many_arguments` (>7) молчит; `should_stop_on_silence` (6) —
  чистая функция-решение, покрыта `test_should_stop_on_silence`, явные Duration by-design для
  тестируемости; `track_speech_and_check_stop` (5) — 1 caller, под clippy-порогом, params уже объект;
  `handle_command` — устарело, свёрнут в `RecordCtx` ещё в D2 (теперь 3 params). breaking=0
  (`process` приватный, тело перенесено дословно `param`→`self.field`), 92 теста зелёные. `ebd0693`
- [x] E3. `engine/record_session.rs` long-param-lists — **реальный clump извлечён.** Три RwLock
  пула моделей (`transcribers`/`active_model`/`last_used`), кочующие вместе в
  `register_reloaded_model` (6) и `resolve_reloaded_transcriber` (7), собраны в borrow-обёртку
  `ModelPool<'a>` (pub(super), field-only): `register` **6→4** (ушёл из long-param детектора),
  `resolve` **7→5** (снят `#[allow(too_many_arguments)]`). Call-site в `trigger` (`engine/mod.rs`)
  строит `ModelPool` inline. Остаток defer-sync: `resolve` (5 = ровно порог детектора, остаток
  разнороден/эссенциален — `need_reload`/`current`/`config`/`pool`/`event_tx`, не группируется),
  `finalize_recording` (7 — разнородный оркестратор доставки, уже `#[allow]` автором). file_health
  85 (ok), breaking=0 (символы `pub(super)`/private внутри крейта), 67+92 теста зелёные. `e426bae`
  _Остаток вне scope:_ `engine/mod.rs` — hotspot (`trigger` cc13 × churn8 = 104, hot_and_complex),
  предсуществующий god-путь, кандидат отдельного D-класса.

## Группа F — Frontend hotspots

- [x] F1. ⚠️ `frontend/src/features/settings/settings.ts` (cc_max 68 → 10, score 62 → 80).
  God-функция `mountSettings` (cc 68, 284 loc) разбита на тонкий оркестратор (cc 3, 52 loc) +
  13 module-private init-хелперов (по группе обработчиков: `initInputHandlers`,
  `initTranscriberChange`, `initLanguageSelect`, `initWaylandHints`, `initWidgetExtRow`,
  `bindWidgetExtToggle`, `initWidgetPositionPicker`, `initSettingsTabs`, `initToggleSwitches`,
  `bindCancelButton`, `bindSaveButton`) + `highlightChanges`/`highlightField` (change-detection).
  Разделяемые зависимости (`SaveUi` = save/cancel/notice, `checkChanges`) — через параметры.
  **cover-before-refactor соблюдён**: сперва характеризационный тест (`db32799`: guard, toggle-
  highlight, cancel-restore, save), потом extract под зелёными. semantic_diff: removed 0 /
  signature_drift 0 (контракт `mountSettings`/`applyEngineAvailability` цел). tsc + 90 тестов
  зелёные. `2df39f9`.
- [x] F2. ⚠️ `frontend/src/pages/history/render.ts` (cc_max 64 → 11, score 65 → 85, recs → 0).
  God-функция `renderHistoryEntries` (cc **64**, 181 loc) разбита на тонкий оркестратор (cc **2**,
  22 loc) + module-private хелперы: `buildModelOptions` (сбор опций dropdown'а), `fillContent`
  (единая заливка контента — **устранён дубль** первый-рендер ↔ change-handler), `doRetranscribe`
  (вынесен из вложенной), `buildMetaRow`, `buildModelSelect`, `renderEntryCard` (оркестратор карточки +
  привязка change/copy/delete/play), `renderPagination`. **cover-before-refactor соблюдён**: сперва
  6 характеризационных тестов (`5721859`: ★-оригинал, «нераспознано»-варианты, retranscribe-видимость,
  audio_deleted, переключение модели, delete→invoke+onReload), потом extract под зелёными. semantic_diff:
  removed 0 / signature_drift 0 (контракт `renderHistoryEntries`/`stopCurrentPlayer` цел). tsc + 96 тестов
  зелёные. Аудио-блок (`playAudio`/`stopCurrentPlayer`) — вне scope (cc уже низкий). `96a2e0a`.

## Группа G — Dead-code verify (TS)

Не delete-list — сперва `references` cross-check по каждому.

- [x] G1. Verify TS-кандидаты (references + grep_with_kind cross-check). Итог:
  `initApp` — **живой** (прод entry-point, `main.ts:4`), не мёртвый. `setValue`/`getValue`
  (custom-select) — **живые** (вызываются внутри `custom-select.ts`), FP-детектора. `bindDropdown`
  (`dropdown.ts`) — **реально мёртвый**: 0 импортеров (find_importers complete), без теста, комментарий
  про export-menu неактуален → **весь файл `dropdown.ts` удалён**. `getCurrentPage` / `errorMessage` /
  `tryInvoke` — вызываются ТОЛЬКО из своих `.test.ts` (прод-мёртвые), но by-design оставлены + defer-sync:
  getCurrentPage — test-observability приватного `currentPage` (2 навиг-теста), errorMessage/tryInvoke —
  valid shared/lib утилиты, симметричные живым соседям, покрыты юнитами. 86 тестов + tsc зелёные. `8152b9a`.
- [~] G2. Verify Rust: `history.rs::get_all_settings`, `history.rs::audio_exists`
  (no_callers, НЕ Tauri-команды) — **verify: не мёртвые.** who_calls 0 прод-вызовов, но
  references cross-check: оба вызываются из ВЫДЕЛЕННЫХ юнит-тестов
  (`test_get_all_settings_returns_all_pairs`, `test_audio_exists_checks_db_and_disk`). Критерий
  «удалить ИЛИ покрыть тестом» уже выполнен вторым путём. Оба — `pub`-методы когезивного DAO
  `HistoryDB`, симметричны живым (`get_setting`/`set_setting`; query-методам), rustc/clippy не
  флагают (`pub` = crate-surface). Dead-code detector FP на тестируемом публичном API. Оставлены +
  defer-sync в `.arcanacodex.toml`. `efbc2f2`.
