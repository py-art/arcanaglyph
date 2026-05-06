# Changelog

<!-- markdownlint-disable-file MD024 -->

Все заметные изменения в проекте документируются в этом файле.
Формат основан на [Keep a Changelog](https://keepachangelog.com/ru/1.1.0/),
версионирование следует [Semantic Versioning](https://semver.org/lang/ru/).

## [Unreleased]

### Удалено

- GitLab CI job `build-deb-and-upload` (stage `deb` в `.gitlab-ci.yml`).
  Дублировал GitHub Actions workflow `release.yml`, который собирает `.deb`
  и заливает в GitHub Release автоматически на event `release: published`.
  GitLab job падал на base image `rust:1` из-за отсутствия libclang
  (нужен `whisper-rs-sys` через `bindgen`) — на `ubuntu-24.04` в GitHub
  Actions libclang ставится с dev-зависимостями. Вместо двух параллельных
  путей к одному ассету оставлен один.

## [1.6.0] - 2026-05-06

### Добавлено

- Self-contained `.deb`-пакет: один артефакт работает на любом x86_64 Linux, AVX и без AVX,
  без ручной настройки после установки. Внутри `.deb` лежат:
  - `arcanaglyph-avx` — бинарь с whisper.cpp, скомпилированным с AVX/AVX2/FMA/F16C;
  - `arcanaglyph-noavx` — бинарь с whisper.cpp без AVX (`-mno-avx -mno-avx2 -mno-avx512f`),
    универсально-безопасный;
  - `libonnxruntime-avx2.so` (Microsoft pre-built 1.20.1) и `libonnxruntime-noavx.so`
    (наша self-build 1.20.1) — load-dynamic backend для GigaAM/Qwen3-ASR;
  - `libvosk.so` (alphacep pre-built 0.3.45) — нужна для Vosk-движка.
  Wrapper `/usr/bin/arcanaglyph` (sh) проверяет `/proc/cpuinfo` и запускает соответствующий
  бинарь. В каждом бинаре `setup_ort_dylib_path()` выбирает `libonnxruntime.so` по
  AVX-detection, с приоритетом self-build override (`/usr/local/lib/libonnxruntime.so` если
  есть). RPATH `/usr/lib/arcanaglyph` обеспечивает поиск libvosk.so без `LD_LIBRARY_PATH`.
- В git закоммичена `assets/libs/libonnxruntime-noavx.so` (24 МБ, self-build для
  CPU без AVX). Остальные нативные либы (Microsoft ORT, alphacep vosk) качаются при
  сборке и в git не лежат (исключены через `.gitignore`).
- STT-движки переведены на cargo features: `gigaam` (default), `vosk`, `whisper`, `qwen3asr`,
  `all-engines`. Сборка по умолчанию (`cargo build`) проходит на любой Linux-машине без
  `libvosk.so` и без CMake — через self-contained GigaAM v3 (ONNX качается автоматически).
- При локальной сборке cargo-features автоматически подбираются под CPU и наличие
  system-deps:
  - на AVX-CPU: `gigaam` (default) + `vosk` (если найден `libvosk.so` в `/usr/local/lib`
    или `/usr/lib/arcanaglyph/`) + `whisper` (если есть `cmake`) + `qwen3asr` (всегда);
  - на CPU без AVX: `gigaam-system-ort` через локально собранный `libonnxruntime.so`
    плюс те же опциональные движки. Если `libvosk.so` нет ни в `/usr/local/lib`, ни в
    `/usr/lib/arcanaglyph/` — соответствующий движок пропускается с подсказкой.
- Runtime AVX-check в Tauri main: если в config выбран ONNX-движок, а CPU без AVX, приложение
  молча переключается на не-ONNX fallback (Whisper/Vosk при наличии в сборке) и показывает
  toast через существующий механизм `engine://fallback` — окно UI открывается всегда вместо
  SIGILL.
- Generic auto-download модели активного транскрайбера при первом запуске. Раньше качался
  только GigaAM, причём параллельно созданию engine — на Whisper-сборке без модели в логах
  появлялся ERROR. Теперь любой движок (Whisper, Vosk, GigaAM, Qwen3-ASR) при отсутствии
  модели автоматически скачивает её через `download_file()`, и engine создаётся **строго
  после** завершения скачивания.
- Авто-распаковка `.zip`-архивов моделей при скачивании через UI. Сейчас касается Vosk
  (vosk-model-ru-0.42 распространяется единым архивом ~1.8 ГБ); для будущих архивных
  моделей сработает автоматически по расширению `.zip`. Реализовано через крейт
  `zip = "8"` (default-features=false, только deflate — pure-Rust, без нативных deps).
  При ошибке распаковки `.zip` остаётся на диске — на retry перекачивать 1.8 ГБ не нужно,
  скрипт пробует распаковку повторно.
- Событие `download://extracting` для индикации фазы распаковки в карточке модели
  (frontend меняет текст кнопки на «Распаковка…»).
- Guard в `download_model`: если модель уже установлена и валидна (`is_model_installed`
  возвращает true), весь download+extract skip'аются, обновляется только config-путь
  и эмитится `download://complete`.
- Новое событие `engine://model-loading` (Tauri) и core-вариант
  `EngineEvent::ModelLoading(String)`. Frontend заменяет top-status на «Загрузка модели
  X…» и блокирует mic-btn до прихода `engine://model-loaded`. Эмитится из `preload_model`
  (eager) и из lazy-fallback в `trigger()`.
- Eager-preload модели в `save_config`: при смене движка в Settings новая модель грузится
  в фоне сразу, а не лениво при первом нажатии Ctrl+Ё. Когда пользователь возвращается на
  главный экран — модель уже в памяти, запись стартует мгновенно.
- В главном окне UI отображается прогресс скачивания модели («Скачивается модель: 47%»)
  через существующий event `download://progress`.
- Новая Tauri-команда `get_compiled_engines`: возвращает список движков, включённых в сборку.
- Авто-fallback: если в SQLite-конфиге сохранён движок, не включённый в текущую сборку,
  приложение молча переключается на дефолтный движок и показывает toast в UI.
- В UI: пункты dropdown'а «Движок транскрибации», не включённые в сборку, помечаются
  disabled-стилем с подписью «не собрано» / "not built".
- README: раздел «Сборка с другими движками (cargo features)» с таблицей системных требований.

### Изменено

- В runtime-зависимости `.deb` пакета добавлен `libayatana-appindicator3-1` (требуется для
  иконки в трее).
- `TranscriberType::default()` теперь `GigaAm` (ранее `Vosk`) — приведено в соответствие с
  README и UI; новые установки получают рабочий движок «из коробки».
- Cargo-фича `qwen3asr` больше не тянет `ort/download-binaries` принудительно — выбор
  ORT-backend'а (download-binaries vs load-dynamic) делегирован соседним `gigaam` /
  `gigaam-system-ort` фичам. Без этого `qwen3asr` нельзя было совмещать с
  `gigaam-system-ort` в одной сборке (конфликт ort-features), что блокировало
  self-contained `.deb` со всеми движками сразу.
- Runtime AVX-fallback в `arcanaglyph-app/src/main.rs` упрощён: условие AVX-требования для
  GigaAM и Qwen3-ASR теперь общее (`gigaam` без `gigaam-system-ort`), потому что qwen3asr
  теперь тоже умеет load-dynamic.
- Tab «Модели» в настройках показывает все известные движки, в т.ч. недоступные в текущей
  сборке (cargo feature off) — карточка рендерится в disabled-стиле со статусом
  «Недоступна в этой сборке» и заблокированными полями/кнопкой скачивания. Раньше такие
  модели просто отсутствовали в списке. Реализовано через новую функцию
  `transcription_models::all_with_availability()` и поле `available` в Tauri-команде
  `get_models`.
- Визуальный picker позиции виджета записи в Settings → Основное: миниатюра экрана
  с 9 точками-якорями (углы / центры сторон / центр), активная подсвечена. Выбор
  сохраняется в конфиг (`widget_position`) и применяется на лету при `save_config`
  через `WebviewWindow::set_position`. Default — `bottom-center` для всех платформ.
  На Wayland mutter может проигнорировать выбор (security-model `xdg_toplevel`) —
  под picker'ом показывается соответствующий хинт.
- GNOME Shell extension `arcanaglyph-widget@arfi.tech` для точного позиционирования
  виджета на Wayland, где приложение само не может задать координаты окна.
  Поставляется внутри `.deb` (`/usr/share/arcanaglyph/extension/`); при включении
  toggle'а «Точное позиционирование на Wayland (GNOME)» в Settings приложение
  копирует расширение в `~/.local/share/gnome-shell/extensions/`, перекомпилирует
  gschema через `glib-compile-schemas` и активирует через `gnome-extensions enable`.
  Дальше предлагает выйти из системы и войти заново через `gnome-session-quit`
  (mutter не может перезагружать shell на Wayland без relogin'а). Само расширение
  слушает gsettings-ключ `org.gnome.shell.extensions.arcanaglyph-widget position`
  (приложение пишет туда выбранное в picker'е значение) и через `Meta.Window.move_frame`
  переставляет окно с title `ArcanaGlyph Recording Widget`. Toggle виден только в
  GNOME-Wayland сессии; на X11 / KDE / sway / Cinnamon — скрыт.
  Поддерживаемые версии GNOME: 46/47/48/49 (Ubuntu 24.04 LTS — GNOME 46, Ubuntu
  25.10 — GNOME 49). На GNOME 50+ — потребуется тестирование и обновление
  `shell-version` в `metadata.json`.

### Исправлено

- Совместимость с Rust 1.95 / Clippy 1.95: `clippy::explicit_counter_loop` в
  `qwen3asr/transcriber.rs`, `clippy::manual_checked_ops` в `arcanaglyph-app/main.rs`,
  `clippy::collapsible_if` в `arcanaglyph-core/src/transcriber.rs:276` и `engine.rs:591`
  (заменены на `if let ... && ...` форму). `make all` снова проходит чисто.
- Зависимость `arcanaglyph-core` в `arcanaglyph-app` теперь подключается с
  `default-features = false`. Без этого Cargo при `--features whisper` всё равно объединял
  с дефолтом core'а (gigaam) и тянул `ort` + onnxruntime в бинарь — на CPU без AVX это
  давало SIGILL даже в whisper-only сборке. Теперь бинарник `--features whisper` не
  содержит ни одного символа onnxruntime и поднимается на любом x86-64 CPU без AVX.
- Вставка распознанного текста на Linux X11 переведена с `enigo.text()` (посимвольный
  XKB-ремап) на clipboard (`arboard`) + `Shift+Insert` (`enigo`). На слабом CPU без AVX
  (Intel Celeron N5095) посимвольный ввод 75-символьной кириллицы давал задержку 20–40с
  между распознаванием и появлением текста, фризил GNOME-сессию и периодически портил
  часть символов из-за гонок раскладки. Теперь текст появляется мгновенно после
  транскрибации и точно соответствует логу. Wayland-путь (wl-copy + XDG RemoteDesktop)
  не изменился.
- В `main()` добавлен явный вызов glib `g_set_prgname("arcanaglyph")` (через FFI). Без
  этого GTK устанавливал `WM_CLASS = "arcanaglyph-noavx"` (или `-avx`), что не совпадало
  со `StartupWMClass=arcanaglyph` в `.desktop`-файле — GNOME Dash не группировал окно с
  ярлыком, показывая отдельный пункт «Arcanaglyph-noavx» рядом с launcher'ом. Теперь
  `WM_CLASS = "arcanaglyph"`.
- Команда установки `.deb` в README заменена с `sudo dpkg -i ... && sudo apt-get install -f -y`
  на единый `sudo apt install ./<deb>` — apt сам разрешает зависимости (wl-clipboard и др.),
  не падает с ошибкой `dpkg`.
- Whisper транскрибация на Intel Atom Tremont (Celeron N5095, без AVX) падала с
  `whisper_full_with_state: failed to encode` (Error code: -6) после ~20с work-time.
  Корневая причина — `whisper_rs::FullParams::set_abort_callback_safe`: trampoline через
  `Box::into_raw` без free даёт UB, на slow-CPU без AVX whisper.cpp читает garbage в
  `abort_callback_user_data` и аборится мгновенно. Фикс: убрали вызов
  `set_abort_callback_safe` совсем; cancel-логика работает только пост-фактум (флаг
  проверяется после `state.full()`, при отмене возвращаем `ArcanaError::Cancelled`).
  `WhisperTranscriber::supports_cancel()` теперь возвращает `false` — UI и так не показывал
  кнопку Стоп для whisper, теперь это согласовано на уровне трейта.
- whisper-rs обновлён 0.13.2 → 0.16.0 (whisper.cpp 1.7.1 → 1.8.3). Миграция API:
  `install_whisper_log_trampoline` убран (теперь явный вызов `install_logging_hooks`,
  features `log_backend`/`tracing_backend`); `set_suppress_non_speech_tokens` →
  `set_suppress_nst`; `full_n_segments()` возвращает `c_int` напрямую без Result;
  `full_get_segment_text(i)` → `state.get_segment(i).to_str_lossy()`. Не пофиксило `-6`
  (это была наша ошибка с abort_callback), но обновили ради 1.8.3 фиксов в других местах.
- whisper.cpp/ggml внутренние логи теперь маршрутизируются через `tracing` (target
  `whisper_rs::*`) — раньше они печатались напрямую в stderr и засоряли вывод. Дефолтный
  фильтр `whisper_rs=warn` подавляет шумные INFO/DEBUG; через `RUST_LOG` можно перебить
  для отладки (`RUST_LOG=info,whisper_rs=trace` показывает per-token decode-логи).
- Models tab: per-model резолюция пути для Whisper-вариантов в `get_models` — Tiny и
  Large делят общий `config.whisper_model_path`, но статус каждой карточки теперь
  резолвится по своему `default_filename` в `models_base_dir`. Ранее одна карточка могла
  видеть статус другой (Large card проверяла `ggml-tiny.bin` и показывала «Не найдена»,
  хотя файл Large лежит на диске).
- Models tab: путь в input'е карточки пустой если файл не установлен — раньше заполнялся
  default-локацией даже после удаления, что вводило в заблуждение.
- Models tab: прогресс-бар активной загрузки сохраняется через re-render — например при
  удалении одной модели во время скачивания другой. Реализовано через `activeDownloads`
  Map в JS + `applyProgressToCard` хелпер, вызываемый в конце `renderModelCards`.
- Models tab: после успешного скачивания backend (`download_model`) автоматически
  обновляет config-путь для соответствующего движка — пользователь не должен жать
  «Сохранить» отдельно. Симметрично `delete_model` чистит config-путь если он совпадал
  с удалённым файлом.
- Models tab: кнопка «Удалить» у установленных моделей с confirm-диалогом — физически
  удаляет файлы (`remove_file` для Whisper, `remove_dir_all` для Vosk/GigaAM/Qwen3-ASR)
  плюс чистит config-путь.
- Dropdown «Движок транскрибации» теперь disabled-стилем с лейблом «(нет модели)» для
  движков чьи модели не скачаны (в дополнение к существующему «(не собрано)» для
  движков, отсутствующих в cargo-сборке).
- В дроп-дауне «Движок транскрибации» Whisper расщеплён на «Whisper Tiny» и «Whisper
  Large V3 Turbo» — пользователь явно выбирает вариант. JS-логика мапит расщеплённые
  значения в backend `transcriber=whisper` плюс явный `whisper_model_path`; обратное при
  загрузке config'а определяется по имени файла модели.
- Toast-предупреждение при выборе тяжёлой модели на CPU без AVX2 (Whisper Large V3 Turbo
  / Qwen3-ASR) — текст сообщает что транскрибация будет в 10-30× медленнее обычного и
  прервать её можно только убийством приложения. Добавлены i18n-ключи
  `settings.slow_model_warning` для ru/en и `settings.model_not_installed`.
- Tauri-команды `get_cpu_features` (для toast-логики), `delete_model`,
  `cancel_transcription`, `active_supports_cancel`. Последние две зарезервированы под
  будущее process-isolated Whisper — UI не показывает кнопку Стоп для whisper, потому
  что abort реально работает только пост-фактум.
- `RUST_LOG`-override для tracing-фильтра — раньше он был захардкожен
  `info,whisper_rs=warn`, теперь читается из env (`EnvFilter::try_from_default_env`).
- Скрипт сборки `.deb` теперь выбирает свежий артефакт по точной версии из
  `tauri.conf.json`. Раньше при наличии старых `.deb` в `target/release/bundle/deb/`
  post-process применялся к алфавитно первому файлу (например `1.5.0.deb` при
  пересборке `1.6.0`); свежий пакет оставался без bundled libs и wrapper'а, после
  установки получался broken пакет (`libvosk.so: cannot open shared object file`).
- Top-status «Готов к записи» больше не врёт во время фоновой загрузки модели: при
  `engine://model-loading` сбрасывается флаг `modelReady` и показывается «Загрузка
  модели X…». Заодно ушёл косяк с «непослушными» pause/stop-кнопками после переключения
  движка — корневая причина была в lazy-reload транскрайбера внутри `trigger()`, теперь
  устранена eager-preload'ом (см. Добавлено).
- Метаданные Vosk-модели в `vosk_russian_speech_model.rs`: `size: "~42 МБ"` →
  `"~1.8 ГБ"`, `expected_min_size_bytes` 40 МБ → 1.5 ГБ. Раньше описывали small-модель
  при URL большой модели — UI вводил в заблуждение.
- Версия на странице «О приложении» (UI): была захардкожена `v1.5.0` в
  `dist/index.html` при актуальной версии 1.6.0 — синхронизирована с фактической
  из `tauri.conf.json`.

### Удалено

- `gigaam-tract` backend и модель GigaAM v3 FP32 (~846 МБ) — tract не поддерживает
  ONNX-оператор `Range`, который использует GigaAM v3, и backend никогда не работал
  на этой модели (та же проблема в `wonnx`/`candle-onnx`). Удалены feature
  `gigaam-tract`, optional dep `tract-onnx`, файлы
  `crates/arcanaglyph-core/src/gigaam/transcriber_tract.rs` и
  `crates/arcanaglyph-core/src/transcription_models/gigaam_v3_fp32_speech_model.rs`,
  все cfg-блоки. На no-AVX-машинах GigaAM по-прежнему работает через
  `gigaam-system-ort` (load-dynamic ORT с локально собранной без-AVX libonnxruntime.so).

## [1.5.0] - 2026-04-21

### Добавлено

- i18n: интерфейс переведён на английский; язык выбирается в настройках (Основное → Язык интерфейса)
- По умолчанию язык определяется по системной локали
- Выбранный период фильтра на странице истории теперь сохраняется в конфиг и восстанавливается при запуске
- Toast-уведомление «Сохранено» при переключении периода истории и языка (автосохранение без кнопки «Сохранить»)

### Изменено

- Дата и длительность записей в истории форматируются под текущий язык интерфейса

## [1.4.1] - 2026-04-12

### Изменено

- Кастомные стрелки у числовых полей в стиле темы (вместо нативных серых)

## [1.4.0] - 2026-04-12

### Добавлено

- Пути моделей перенесены в карточки с кнопкой выбора директории (📁)
- Базовый путь к моделям — общая настройка над карточками
- Системный диалог выбора директории (tauri-plugin-dialog)

### Удалено

- Отдельные поля путей моделей (Vosk, Whisper, GigaAM, Qwen3-ASR) из верхней части вкладки «Модели»

## [1.3.7] - 2026-04-11

### Исправлено

- GitHub Actions: ubuntu-24.04 для совместимости с ONNX Runtime (glibc 2.38+)

## [1.3.6] - 2026-04-11

### Исправлено

- GitHub Actions: сборка только .deb (AppImage требует FUSE, недоступный в CI-контейнерах)

## [1.3.5] - 2026-04-11

### Исправлено

- CI: корректная обработка существующих релизов (пропуск вместо ошибки)
- CI: публичный README и чистый CHANGELOG для GitHub-зеркала

## [1.3.4] - 2026-04-11

### Добавлено

- GitHub Actions: автосборка `.deb` при создании релиза с SHA256-хэшами

## [1.3.3] - 2026-04-11

### Исправлено

- CI: релиз и тег создаются в одном проходе pipeline (раньше не срабатывали)

## [1.3.2] - 2026-04-11

### Добавлено

- CI: автоматическая публикация GitHub Release с release notes из CHANGELOG

## [1.3.1] - 2026-04-11

### Исправлено

- Запуск записи из системного трея — пункт меню «Начать запись» не работал из-за несовпадения типа engine в Tauri state

## [1.3.0] - 2026-04-09

### Добавлено

- Экспорт истории транскрипций в CSV (кнопка «Экспорт» на странице истории, сохранение в ~/Загрузки/)
- GPU-ускорение: опциональный CUDA ExecutionProvider для GigaAM и Qwen3-ASR (сборка с `--features cuda`)
- Toast-уведомления (всплывающие сообщения поверх контента)

## [1.2.0] - 2026-04-08

### Добавлено

- Настройка «Иконка в трее» — возможность скрыть иконку из системного трея
- Single-instance — повторный запуск приложения показывает окно существующего экземпляра вместо создания нового

## [1.1.0] - 2026-04-08

### Добавлено

- Плавающий виджет записи — компактное окно поверх всех окон при записи (таймер, уровень звука, пауза/стоп)
- Настройка «Виджет записи» для включения/отключения виджета (включён по умолчанию)

## [1.0.0] - 2026-04-07

### Добавлено

- Четыре STT-движка: Vosk, Whisper, GigaAM v3 (по умолчанию), Qwen3-ASR
- GigaAM v3 — лучшая модель для русского (WER 8.4%, 225 МБ, ONNX Runtime)
- Qwen3-ASR — мультиязычный движок (52 языка, авторегрессивный decoder)
- Автоскачивание GigaAM v3 при первом запуске
- Горячие клавиши: Ctrl+Ё (запись), Ctrl+Shift+Ё (пауза)
- Авторегистрация хоткеев в GNOME через gsettings (Wayland)
- Композер горячих клавиш с кнопками модификаторов
- VAD — авто-стоп записи при тишине после речи
- Удаление слов-паразитов (э, э-э, ээ, эм, мм)
- Настройки с тремя вкладками: Основное / Модели / Клавиши
- Карточки моделей с описанием, размером, статусом и кнопкой скачивания
- Встроенное скачивание моделей с прогресс-баром
- Автоочистка старых записей (настраиваемый период)
- Автозапуск при входе в систему
- Запуск в трей (start minimized)
- Меню трея: Открыть / Запись / Настройки / Выход (с разделителями)
- Иконка трея: белая (idle), красная (запись), оранжевая (пауза)
- История транскрипций в SQLite с пагинацией
- Ретранскрибация записей другой моделью
- Предзагрузка моделей — мгновенное переключение
- Пул моделей с ленивой загрузкой
- XDG-стандартные пути: модели, конфиг, кэш
- UDP-скрипты для Wayland (автоустановка)
- Проверка конфликтов хоткеев в GNOME
- Раздел «О приложении» в меню
- Сборка `.deb` через Tauri v2
