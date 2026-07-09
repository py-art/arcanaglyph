---
status: closed
created: 2026-07-09
audited: 2026-07-09
closed: 2026-07-09
server_version: 0.463.0
source: полный /aidit round-3 (7 параллельных категорий) + seed наблюдения round-2
note: seed дополнен полным прогоном /aidit. Приоритеты ниже — консолидация 7 категорий.
---

# Refactoring backlog — round 3 (audited)

Полный `/aidit` прогнан 2026-07-09 на server v0.463.0 (7 категорий параллельно). Ниже —
консолидированный приоритизированный бэклог. Триггер запуска рефактора — слово **«поехали»**
(см. память `project_refac_round3_seed`).

## Регламент (кратко)

- ✅ **Коммитить можно** без подтверждения (грант пользователя 2026-07-09, как в round-2).
- 🛑 **НЕ пушить** — ни в одном режиме, включая `auto`. Финальный squash + push — только пользователь.
- Правки **behaviour-preserving**, минимальные под пункт. Cover-before-refactor для непокрытого.
- Verify-before-refactor: перед правкой повторно проверить актуальность находки через MCP.
- Пометки: 🔒 footgun-gated (advisory-плагин), ⚠️ крупная/структурная/продуктовая правка.

## TL;DR — состояние здоровья

- **0 critical выше глобального порога**, 4 файла в зоне score < 60, 21 warning.
- Крупных god-object / data-clump кластеров **нет** (0 clone_clusters, 0 data_clumps в
  refactor_candidates). Основное — cc-hotspots + семантические клоны + magic-литералы.
- Архитектура чистая: **циклов нет**, нарушений слоёв нет, honesty-сигналы достоверны
  (`layering_zero_coverage: false`, слои реально покрыты — 13 файлов).
- Мёртвого прод-кода фактически **нет**. Зависимости здоровы: без copyleft, без
  критичных/эксплуатируемых CVE.
- Покрытие: frontend 100%, Rust 63% (весь долг — в app-крейте: setup 0%, models 20%).
- Техдолг-маркеры (TODO/FIXME/HACK): **отсутствуют**. Assertion-free тесты: **отсутствуют**.

## Группа A — Rust-декомпозиция (⚠️ структурная)

- [x] A1. ⚠️ **`qwen3asr/transcriber.rs` → `transcriber/` (mod.rs + load/encode/decoder)** —
  разбит на подмодули по концернам. Итог: score `mod.rs` 50→80, `load.rs` 90 (+ бонус: `new`
  cc 22→12 выносом `load_onnx_session`), `encode.rs` 85, `decoder.rs` 80 — god-object снят,
  max cc 22→16 (оркестратор `transcribe`). Публичный путь `Qwen3AsrTranscriber` сохранён
  (breaking_changes = FP от file-rename, компиляция+тесты подтверждают). Coverage 1.0
  (4 теста переехали), 95 тестов зелёные. Готово: `334eb1c`.
- [x] A2. ⚠️ **`updater.rs` → `updater/` (mod + version/asset/state)** — split по концернам.
  Verify подтвердил: НЕ FP (godobject_factor 15 от 39 fn/20 exports, churn 15). Итог: score
  `mod.rs` 40→90, `state.rs` 95, `version.rs`/`asset.rs` ok. Внешние пути `updater::*` сохранены
  через `pub use {version,asset,state}::*` (реэкспорт) — потребители не тронуты. breaking_changes
  = FP от file-move+реэкспорт (компиляция workspace + 73 app-теста подтверждают контракт). Все
  22 юнит-теста апдейтера переехали и зелёные. Coverage не изменилась (9 untested — те же
  сетевые/state-функции, backfill = H2). Готово: `4fcc8d7`.

## Группа B — cc-hotspots (точечное reduce_complexity)

- [x] B1. `setup/app_setup.rs::ensure_gnome_hotkeys` — cc **23→13** (nesting 3→2, loc 81→37).
  Вынес из функции два helper'а в `commands/hotkeys.rs`: `gsettings_get_keybinding(slot, key)`
  (устранил дублирование probe/cmd_outdated Command-блоков) + `kick_xkb_on_x11()` (XKB-пинок).
  app_setup.rs score 65→70, badge снят. breaking_changes чист (patch). 73+95 тестов зелёные.
  2 новых `pub(crate)` — side-effect process-обёртки (не юнит-тестируются по конвенции,
  как `gs_set`; чистая логика внутри покрыта). Готово: `a507886`.
- [x] B2. `commands/config_history.rs::retranscribe` — cc **23→18** (loc 70→51), badge понижен
  `reduce`→`review`, ниже критического порога. Вынес 2 приватных helper'а: `load_recording_samples`
  (find+read+decode) + `ensure_not_duplicate` (get_transcriptions+any). Feature-gated `build_transcriber`/
  spawn оставлены инлайн под `#[allow(unreachable_code)]` (нельзя проверить reduced-build локально).
  breaking чист, 73+95 тестов зелёные. Готово: `67e3f48`.
- [x] B3. `input.rs::type_text_x11` — cc **27→7** (loc 70→23), z=17.5 выброс устранён. Вынес
  две spawn_blocking-фазы в приватные cfg(linux)-helper'ы: `copy_to_x11_clipboards` (CLIPBOARD+
  PRIMARY через arboard) + `send_shift_insert` (enigo). Плагин-инвариант input-platform-insertion
  соблюдён (X11 = arboard+Shift+Insert, НЕ enigo.text; другие cfg-ветки не тронуты). input.rs
  score → 85 (ok). breaking чист, 73+95 тестов зелёные. Готово: `e8fb427`.
- [x] B4. `qwen3asr/mel.rs::mel_filterbank_slaney` — cc **26→4** (loc 69→33), file score 80→95.
  Вынес 2 чистые mel-конверсии `slaney_hz_to_mel`/`slaney_mel_to_hz` (+ константы `SLANEY_F_SP`/
  `SLANEY_MIN_LOG_HZ`, + roundtrip-тест → cover-плюс на DSP) и заполнение mel-строки
  `fill_triangular_mel_filter`. Математика идентична (behaviour-preserving), формы-тест + roundtrip
  зелёные. Оказалось НЕ «рискованно» — чистые под-формулы с тестом. Готово: `ab1e64e`.

## Группа C — Дубли и клоны (⚠️ к ручной проверке)

- [~] C1. **gnome-hotkeys унификация** (sim 0.90) — **verify-FP**. `register_gnome_hotkeys` —
  тонкая Tauri-команда-делегат, `register_gnome_hotkeys_linux` — её impl, `ensure_gnome_hotkeys` —
  совсем другая probe+XKB-логика. Это паттерн команда→impl→probe с разными ролями, sim 0.90 от
  общей gsettings-темы (пути custom-keybindings), а не от дублируемого кода — унифицировать нечего.
  Реальное дублирование (probe/cmd_outdated Command-блоки) уже устранено в B1. Defer-sync добавлен.
- [~] C2. **portal-логика в 3 местах** (sim 0.89) — **verify-FP**. Общая логика УЖЕ в core
  (`input::warmup_remote_desktop`); три функции — контекст-специфичные обёртки над ней с
  несовместимыми сигнатурами: `grant_portal_now` (Tauri IPC `→Result`, UI показывает ошибку),
  `run_grant_portal_and_exit` (standalone CLI `→!`, own runtime + exit-коды + stdout для install.sh),
  `spawn_portal_warmup` (фоновый spawn `→()`, log-only). Объединять нечего — адаптеры разных
  контекстов вызова. Defer-sync добавлен.
- [~] C3. **naming-реинвент** (confidence medium) — **verify-FP, обе пары**. (1) `create_transcriber`
  (engine-метод) ЯВНО делегирует `build_transcriber` (свободная фабрика) + добавляет ORT-logging и
  Arc-wrap — adapter/factory, переименование дало бы два `build_transcriber`. (2) `save_config`
  (Tauri-cmd: полный config от UI + side-effects) / `update_config` (private RMW-helper: load→mutate
  одно поле→save) / `engine.update_config` (engine-apply) — три разных контракта, «config» в имени
  совпал случайно. By-design, не реинвент. (naming вне defer-механики clone/magic — документировано здесь.)
- [~] C4. **data-clump** `(transcriber_type, filename, config)` — **verify: weak/FP**. Стабильная
  тройка есть только в `resolve_model_file_path` (одна функция). Остальные точки (`apply_model_config_path`,
  `clear_model_config_path_if_matches`, `persist_downloaded_model_path`) берут `transcriber_type` +
  РАЗНЫЕ path-параметры (main_dest/dest_path/path/deleted_path) — не общая тройка. Единственный
  сквозной параметр `transcriber_type: &str` — идиоматичный engine-id ключ диспетчеризации (уже
  by-design в defer round-2 для `models/*`). Struct на «ключ+config» = церемония без выгоды.
- [~] C5. `gigaam/mel.rs::hz_to_mel`/`mel_to_hz` — **verify: разные формулы, не общий helper**.
  gigaam — HTK (`2595*log10`), qwen (после B4) — Slaney (`f_sp`/`logstep`). Разные mel-шкалы,
  общий `dsp/`-helper объединил бы несовместимую математику. Оба движка теперь имеют СВОИ
  тестируемые конверсии (gigaam `test_hz_mel_roundtrip`, qwen `test_slaney_mel_hz_roundtrip`) —
  это правильная симметрия, а не дубль. Defer-sync добавлен.
- DEFER (не поднимать): `transcribe` gigaam↔rnnt↔qwen (sim 0.77, distinct STT-архитектуры) —
  подтверждён в defer-list, 11 override применено. `history.rs::add_recording`↔`add_transcription`
  (sim 0.94) — уже в defer.

## Группа D — Frontend god-функции (⚠️ каждая = масштаба round-2 D2/D3)

Cover-before-refactor характеризационным тестом (frontend 100% покрыт), потом декомпозиция
по секциям. `metric_anomalies` их НЕ ловит — peer-группы widgets/pages < 8 функций.

- [x] D1. ⚠️ `frontend/src/features/model-management/model-cards.ts::renderModelCards` — cc
  **52→6** (loc 149→25, была самая высокая в репо). Декомпозирован на `computeCardStatus`
  (чистая) + `buildModelCardHtml` + `attachPathPickHandlers`/`attachDownloadHandlers`/
  `attachDeleteHandlers`, `renderModelCards` — оркестратор. Убран unused import `invoke`.
  file score 72→85 (ok). type-check чист, 106 vitest-тестов зелёные (2 характеризационных
  на renderModelCards защитили декомпозицию). Готово: `b220eec`.
- [x] D2. ⚠️ `frontend/src/widgets/main-controls/main-controls.ts::mountMainControls` — cc
  **36→0** (loc 242→27), file score 62→85 (ok), badge снят. Декомпозирован на `queryRefs`
  (сбор DOM в `Refs`) + 3 фабрики helper'ов (`createStartLevelTimers`/`createResetToIdle`/
  `createOnModelReady`) + 4 wire-функции по концернам (`wireButtons`/`wireModelLifecycle`/
  `wireRecordingEvents`/`wireTranscriptionEvents`) + `startFinishWatchdog`; `mountMainControls` —
  оркестратор. State и DOM-refs остались приватными для модуля (наружу — только `onModelReady`,
  как и было). breaking_changes = 0 (additive — module-local `Refs::*`, экспорт-сигнатура та же).
  type-check чист, 106 vitest зелёные (характеризационный тест на `onModelReady` защитил).
  empty_catch×3 не тронуты (behaviour-preserving) — их разбор в E1. Готово: `0102f8e`.
- [x] D3. ⚠️ `frontend/src/widgets/update-banner/update-banner.ts::mountUpdateBanner` — cc
  **34→1** (loc 175→10), file score 70→90 (ok), badge снят. Общий mutable-state (`state` +
  `installPollTimer`) вынесен в `Ctx`-контейнер (нужен и на чтение, и на переприсваивание по
  ссылке из разных обработчиков). Декомпозирован на `queryRefs` (сбор+guard, возвращает
  `Refs|null`) + `render` (state-машина) + install-poll helpers (`start`/`stop`) + 3 wire-функции
  по концернам (`wireAvailableActions`/`wireApplyingActions`/`wireBannerSources`). `mountUpdateBanner` —
  10-loc оркестратор с ранним `return` при отсутствии DOM (no-op сохранён). breaking_changes = 0
  (additive — module-local `Refs::*`/`Ctx::*`, экспорт-сигнатура та же). type-check чист, 106
  vitest зелёные (4 характеризационных кейса защитили: manual show / apply_update / restart-disabled /
  no-op). unused_catch_binding×3 не тронуты (behaviour-preserving) — разбор в E3. Готово: `338cdee`.

## Группа E — Swallowed errors (policy-решение)

- [x] E1. **6× empty_catch high → 0**: `main-controls.ts` (после D2-декомпозиции L196/267/308),
  `titlebar.ts` (L13/18/22 — вложенные `catch (_) {}`). Все шесть — легитимный failsafe/poll/
  best-effort (poll `is_model_loaded` каждые 200мс, watchdog `is_recording` каждые 1.5с, failsafe
  webview minimize/close, косметический max/unmax, best-effort `active_supports_cancel`). Логировать
  нельзя — спам консоли из hot-poll'ов; выбран вариант «оставить с комментом-обоснованием» (E1
  давал оба). В каждый catch добавлен комментарий-обоснование → детектор больше НЕ видит empty_catch
  (high 6→0). Осознанно НЕ трогал binding `catch (_)`: он понизил находки до `unused_catch_binding`
  medium — это ровно домен E3 (14× по всему фронту), где стиль `catch (_)` решается единым махом
  (не размываю согласованность стиля между 2 и 12 файлами). type-check чист, 106 vitest зелёные,
  behaviour-preserving. Готово: `7c9887e`.
- [x] E2. **2× unwrap medium** в `tray.rs` → graceful. L52 `default_window_icon().unwrap()` —
  функция `create_tray` уже `-> Result<_, Box<dyn Error>>`, заменил на
  `.ok_or("default window icon not configured")?` (при отсутствии иконки — ошибка вверх, не
  паника при построении трея). L123 `current.0.lock().unwrap()` — это лишь anti-flicker кэш
  состояния трея, при отравленном мьютексе паниковать в UI-потоке нельзя: заменил на
  `.unwrap_or_else(|e| e.into_inner())` (восстанавливаем guard, продолжаем). clippy чист,
  95 тестов зелёные, behaviour-preserving. Примечание: `file_health` держит pre-existing
  hotspot `create_tray` cc=20 (score 80, warning) — НЕ регрессия E2 (ok_or/`?`/into_inner
  cognitive-ветвей не добавляют; cc формирует `match on_menu_event`), вне consolidated-бэклога
  round-3, не трогал (scope=unwrap-fix). 6 untested pub tray.rs — side-effect Tauri-функции
  (реальный tray без рантайма не юнит-тестируются, H-FP-класс). Готово: `2e65984`.
- [x] E3. **unused_catch_binding medium → 0** (frontend). Единый стилевой проход: ES2019 optional
  catch binding — `catch (_)` / `catch (_e)` → `catch` там, где ошибка намеренно игнорируется
  (binding не используется). 20 мест в 14 файлах: `about.ts`, `history/render.ts`, `history-page.ts`,
  `tauri.ts`, `main-controls.ts` (×3), `update-banner.ts` (×3), `portal-banner.ts`, `titlebar.ts`
  (×5 — вложенные), `model-badge.ts`, `mic-device.ts`, `load-config.ts`, `engine-availability.ts`,
  `model-cards.ts` (`_e`), `page-navigation.ts`. `catch (e)` где `e` реально используется — не тронуты.
  Заодно закрыл 6 находок из E1-домена (main-controls/titlebar) — они понижались до этого medium.
  Комментарии-обоснования сохранены. grep `catch (_` = 0, error_handling update-banner/titlebar = 0
  findings, type-check чист, 106 vitest зелёные. Готово: `e55055b`.
- DEFER (by-design, не трогать): 83× `let _ = emit/send(...)` fire-and-forget (Tauri/broadcast);
  whisper abort_callback (UB на Tremont); `input.rs:157`/`bootstrap.rs:354` expect (безопасны).

## Группа F — Async correctness (⚠️ medium — самый весомый сигнал безопасности)

- [x] F1. ⚠️ **async inter-procedural blocking** — verify: главный страх ОПРОВЕРГНУТ, один
  реальный sink обёрнут. Ручная проверка стека (call_graph + чтение тел):
  - `audio::record_and_transcribe` (инференс + `thread::sleep`) — **уже** обёрнут в
    `spawn_blocking` в `run_record_session:241`. ✅
  - загрузка модели (`create_transcriber` → `qwen3asr::load_embeddings`, `config`) — **уже**
    обёрнута в `spawn_blocking` в `resolve_reloaded_transcriber:117`. ✅ (F1 боялся «модель
    грузится прямо в async» — НЕ грузится.)
  - **реальный остаток:** `finalize_recording` писал в SQLite (`add_recording`/`add_transcription`,
    синхронный rusqlite disk-I/O) прямо в async-стеке без `spawn_blocking`. Лёгкая (2 INSERT),
    но технически sync-I/O в async. Обёрнул запись в `tokio::task::spawn_blocking` (Arc-clone db +
    move строк), warn-on-error поведение сохранено. score 85 (ok), finalize cc 12 (< порога),
    clippy чист, 73+95 тестов зелёные. Behaviour-preserving. Готово: `7c9ea5d`.

## Группа G — Magic-литералы (точечный вынос в конст)

- [x] G1. `config.rs` — 3 repeated_number вынесены в именованные конст по конвенции файла
  (рядом с `DEFAULT_SAMPLE_RATE`/`DEFAULT_MAX_RECORD_SECS`): `DEFAULT_VAD_SILENCE_SECS = 15`,
  `DEFAULT_RETENTION_HOURS = 24`, `DEFAULT_HISTORY_FILTER_SECS = 86_400`. Заменены все
  вхождения: default-функции (`default_vad_silence_secs`/`default_retention_hours`/
  `default_history_filter_secs`), `impl Default` (L313/317/325) и тестовый roundtrip-конфиг —
  single source of truth. `default_gigaam_rnnt` не трогал (использует `..Self::default()`).
  `24.0`-пиксели в `widget_position_xy` (MARGIN) НЕ трогал — другая семантика/тип (уже локальная
  const). clippy чист, 73+95 тестов зелёные. Готово: `81fe706`.
- [x] G2. `bootstrap.rs`: env-ключ `"ORT_DYLIB_PATH"` → одна конст `ORT_DYLIB_PATH_ENV`
  (cfg-gated `any(linux, windows)` — иначе unused на macOS). Заменены 7 API-вызовов
  (`var_os`/`var`/`set_var`) в обеих cfg-функциях `setup_ort_dylib_path` (linux + windows).
  Человекочитаемые лог-строки `"ORT_DYLIB_PATH = {} (...)"` НЕ трогал — это текст сообщения,
  не ключ. Windows-ветка выверена по коду (локально cfg не собирается, [[gotcha_windows_branch_local_crosscheck]]).
  clippy чист, 73+95 тестов зелёные. Готово: `3e341cd`.
- [x] G3. `updater/` (после A2-split): (1) JSON-ключи `"name"`/`"state"` + значение `"uploaded"`
  в `asset.rs` → локальные конст `ASSET_KEY_NAME`/`ASSET_KEY_STATE`/`ASSET_STATE_UPLOADED`
  (2 prod-функции `release_has_installable_asset`/`installable_asset_url`; тестовые JSON-фикстуры
  не трогал — там `json!`-данные). (2) cache-headers оказались не просто magic-строкой, а
  **дублируемым блоком** GitHub-API заголовков (`Accept`+`X-GitHub-Api-Version`+`Cache-Control:no-cache`+
  `Pragma:no-cache`+cache-buster query) в `mod.rs::fetch_latest_release` и
  `asset.rs::fetch_installable_asset_url` — вынес helper `with_github_cache_bust(RequestBuilder)`
  в `mod.rs` (pub(super)), оба зовут его. `fetch_latest_release` cc снижен, оба файла score 85/90
  (ok). clippy чист, 73+95 тестов зелёные. Готово: `6119917`.
- [x] G4. `dsp/preprocess.rs`: целевой `16000` → `TARGET_SAMPLE_RATE` (2 prod-вхождения в
  `preprocess_to_f32_16k`: `!= 16000` + `resample(..., 16000)`), делитель `32768.0` → `I16_MAX`
  (нормализация i16→f32). Тестовые `16000`/`32768` не трогал (характеризационные данные).
  clippy чист, 95 тестов зелёные. Готово: `0a2dc92`.
- НЕ трогать: mel-коэффициенты (2595.0/700.0/f_sp — алгоритмические Slaney/HTK), frontend
  DOM-строки (298 repeated_string = селекторы/классы/события — шум для vanilla-TS).

## Группа H — Тесты (backfill, cover-before-refactor)

По убыванию ценности (Rust — весь долг покрытия там):

- [x] H1. **`core/src/audio.rs` критичный путь записи** — покрыты два непокрытых
  детерминированных элемента пути: `mic_silent_message` (user-facing сообщение о мёртвом
  микрофоне — проверка device_name в тексте + linux-специфика amixer) и `save_raw_audio`
  (LE-сериализация i16 + создание вложенной директории через round-trip в temp_dir, стиль
  проекта — без dev-dep `tempfile`). core-тесты 95→97, clippy чист, behaviour-preserving.
  Verify-нюанс: сами оркестраторы `record_and_transcribe`/`run_record_session`/
  `finalize_recording` — device/async (cpal-устройство + broadcast + SQLite); их юнит-тест
  требует инъекции зависимостей = рефакторинг-под-тест (фича, не behaviour-preserving
  backfill H1). Вся извлечённая из них чистая логика УЖЕ покрыта (downmix/gain/rms/
  should_stop_on_silence/classify_delivery/build_history_entry) + теперь эти два. Готово: `4489f43`.
- [x] H2. **`updater/state.rs`** state-функции покрыты через реальную `temp_history_db`
  (in-memory `HistoryDB` на диске в temp): `set_applying`→`applying_version`→`clear_applying`
  RMW-цикл; `dismiss` подавляет `cached_pending_update` для dismiss'нутой версии (end-to-end
  через БД, а не конструкция struct); `unix_now > 0` (регресс на `unwrap_or(0)`). Раньше эти
  публичные RMW-функции напрямую тестами не вызывались — покрытие модуля 41%→выше. app-тесты
  73→76, clippy чист, state.rs score 95, behaviour-preserving. Готово: `d541be4`.
- [x] H3. **`engine/mod.rs`** публичное API покрыто через `make_engine`+`DummyTranscriber`
  (idle-engine без железа): `is_recording`/`is_paused` = false на свежем; `preload_model`
  для уже-загруженной модели → ранний `Ok(name)` без emit'ов (+ покрывает `subscribe`:
  receiver жив и пуст на этой ветке); `pause` без активной записи → no-op (спавн-таск видит
  `is_busy==false`, не трогает `is_paused`). Verify-нюанс: `trigger` — device-путь (стартует
  cpal-запись через `rt_handle.spawn`), юнит-тест невозможен без микрофона (как
  `record_and_transcribe` в H1); его дебаунс `trigger_debounced` — чистый, но завязан на
  process-global `static` счётчик. core-тесты 97→100, clippy чист, engine score 62 warning
  = pre-existing godobject/churn (не регрессия, вне scope backfill). Готово: `003ee20`.
- FP (не требуют юнит-теста): 6 Tauri-команд (`generate_handler!` макрос), большинство
  `setup/` и `input.rs` — платформенные `#[cfg]`/side-effect функции.

## Группа I — Мёртвый код и зависимости (низкий приоритет)

- [~] I1. `CustomSelect.setValue`/`getValue` (`shared/ui/custom-select.ts:19/21`) — вызываются
  ТОЛЬКО тестом (verify подтвердил: prod `bindCustomSelect('s-language')` отбрасывает handle,
  использует лишь `onChange`; для программной установки есть отдельная `setCustomSelectValue`).
  **Решение владельца 2026-07-09: ОСТАВИТЬ как задел** — симметричная get/set пара, осознанный
  публичный API компонента (автор вынес из main.ts inline с док-комментами). Не мёртвый код,
  а API-surface. Defer-sync добавлен (2× `[[audit.defer]]` на setValue/getValue).
- [x] I2. Transitive dep-bump через `cargo update -p`: `anyhow` 1.0.102→103, `crossbeam-epoch`
  0.9.18→20, `memmap2` 0.9.10→11 — обновлены, clippy+тесты (76+100) зелёные. `glib` НЕ поднят
  до 0.20: semver-граница gtk3-стека (Tauri тянет `glib ^0.18`) не пускает major-bump — это вне
  зоны безопасного backfill (потребовал бы согласованной замены gtk/tauri-версий). Готово: `4280797`.
- НЕ трогать: 35 Tauri-cmd (FP), `@tauri-apps/api`/`jsdom` unused (FP), TS 5.9 (TS7 несовместим),
  GTK3-адвайзори (системные для Tauri, вне зоны).

## Слепые пятна аудита (НЕ «чисто»)

- **Inline-CSS** в `frontend/index.html`/`widget.html` — `css_file_health` не индексирует
  не-`.css`. Мёртвые правила/классы там невидимы. Требует ручной проверки / продуктового
  решения по экстракции стилей (было E1 в seed — визуальные регрессии виджета, live-verify).
- **taint** — 50 findings, все confidence low (FP-паттерн доверенного реестра: env/argv/XDG-пути,
  gsettings-литералы). Отдельно вручную проверить asset/подпись в `updater.rs` (taint sink'ов
  там не нашёл — слепая зона, а не «безопасно»).
- **ownership** truck_factor=1 — solo-разработчик, сигнал условен (не дефект).
