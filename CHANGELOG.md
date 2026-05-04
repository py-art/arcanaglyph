# Changelog

<!-- markdownlint-disable-file MD024 -->

Все заметные изменения в проекте документируются в этом файле.
Формат основан на [Keep a Changelog](https://keepachangelog.com/ru/1.1.0/),
версионирование следует [Semantic Versioning](https://semver.org/lang/ru/).

## [Unreleased]

## [1.6.0] - 2026-05-04

### Добавлено

- Self-contained `.deb`-пакет: один артефакт работает на любом x86_64 Linux, AVX и без AVX,
  без ручной настройки после `dpkg -i`. Внутри `.deb` лежат:
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
- Новые скрипты: `scripts/prepare-bundled-libs.sh` (качает Microsoft ORT и alphacep vosk
  с пинами SHA256) и `scripts/build-deb.sh` (двойная сборка + post-process через
  `dpkg-deb -R/-b`). `make dist` теперь вызывает `build-deb.sh`.
- В git закоммичена `assets/libs/libonnxruntime-noavx.so` (24 МБ, наш self-build для
  CPU без AVX). Остальные нативные либы качаются build-скриптом и в git не лежат
  (исключены через `.gitignore`).

- STT-движки переведены на cargo features: `gigaam` (default), `vosk`, `whisper`, `qwen3asr`,
  `all-engines`. Сборка по умолчанию (`cargo build`) проходит на любой Linux-машине без `libvosk.so`
  и без CMake — через self-contained GigaAM v3 (ONNX качается автоматически).
- `make run` авто-выбирает features по CPU: при отсутствии AVX (старые Celeron/Atom) собирается
  с Whisper вместо GigaAM, поскольку pre-built ONNX Runtime требует AVX-инструкций.
- Runtime AVX-check в Tauri main: если в config выбран ONNX-движок, а CPU без AVX, приложение
  молча переключается на не-ONNX fallback (Whisper/Vosk при наличии в сборке) и показывает toast
  через существующий механизм `engine://fallback` — окно UI открывается всегда вместо SIGILL.
- Generic auto-download модели активного транскрайбера при первом запуске. Раньше качался только
  GigaAM, причём параллельно созданию engine — на Whisper-сборке без модели в логах появлялся
  ERROR. Теперь любой движок (Whisper, Vosk, GigaAM, Qwen3-ASR) при отсутствии модели
  автоматически скачивает её через `download_file()`, и engine создаётся **строго после**
  завершения скачивания.
- В главном окне UI отображается прогресс скачивания модели («Скачивается модель: 47%»)
  через существующий event `download://progress`.
- Новая Tauri-команда `get_compiled_engines`: возвращает список движков, включённых в сборку.
- Авто-fallback: если в SQLite-конфиге сохранён движок, не включённый в текущую сборку,
  приложение молча переключается на дефолтный движок и показывает toast в UI.
- В UI: пункты dropdown'а «Движок транскрибации», не включённые в сборку, помечаются disabled-стилем
  с подписью «не собрано» / "not built".
- `make test-all` — `cargo test --all-features` для машин со всеми STT-библиотеками.
- README: раздел «Сборка с другими движками (cargo features)» с таблицей системных требований.

### Изменено

- В runtime-зависимости `.deb` пакета добавлен `libayatana-appindicator3-1` (требуется для иконки в трее).
- `TranscriberType::default()` теперь `GigaAm` (ранее `Vosk`) — приведено в соответствие с CLAUDE.md,
  README и UI; новые установки получают рабочий движок «из коробки».
- `make run` на CPU без AVX автоматически добавляет в features `vosk` (если есть `/usr/local/lib/libvosk.so`)
  и `whisper` (если установлен `cmake`) — раньше собиралось только с `gigaam-system-ort`. Если зависимости
  отсутствуют — соответствующий движок пропускается с подсказкой как его подключить, сборка не падает.
- Cargo-фича `qwen3asr` больше не тянет `ort/download-binaries` принудительно — выбор ORT-backend'а
  (download-binaries vs load-dynamic) делегирован соседним `gigaam` / `gigaam-system-ort` фичам.
  Без этого `qwen3asr` нельзя было совмещать с `gigaam-system-ort` в одной сборке (конфликт
  ort-features), что блокировало self-contained `.deb` со всеми движками сразу.
- Runtime AVX-fallback в `arcanaglyph-app/src/main.rs` упрощён: условие AVX-требования для
  GigaAM и Qwen3-ASR теперь общее (`gigaam` без `gigaam-system-ort`), потому что qwen3asr
  теперь тоже умеет load-dynamic.
- Tab «Модели» в настройках показывает все известные движки, в т.ч. недоступные в текущей сборке
  (cargo feature off) — карточка рендерится в disabled-стиле со статусом «Недоступна в этой сборке»
  и заблокированными полями/кнопкой скачивания. Раньше такие модели просто отсутствовали в списке.
  Реализовано через новую функцию `transcription_models::all_with_availability()` и поле `available`
  в Tauri-команде `get_models`.

### Исправлено

- Совместимость с Rust 1.95 / Clippy 1.95: `clippy::explicit_counter_loop` в `qwen3asr/transcriber.rs`
  и `clippy::manual_checked_ops` в `arcanaglyph-app/main.rs`.
- Зависимость `arcanaglyph-core` в `arcanaglyph-app` теперь подключается с
  `default-features = false`. Без этого Cargo при `--features whisper` всё равно объединял с
  дефолтом core'а (gigaam) и тянул `ort` + onnxruntime в бинарь — на CPU без AVX это давало SIGILL
  даже в whisper-only сборке. Теперь бинарник `--features whisper` не содержит ни одного символа
  onnxruntime и поднимается на любом x86-64 CPU без AVX.
- Вставка распознанного текста на Linux X11 переведена с `enigo.text()` (посимвольный XKB-ремап)
  на clipboard (`arboard`) + `Shift+Insert` (`enigo`). На слабом CPU без AVX (Intel Celeron N5095)
  посимвольный ввод 75-символьной кириллицы давал задержку 20–40с между распознаванием и появлением
  текста, фризил GNOME-сессию и периодически портил часть символов из-за гонок раскладки. Теперь
  текст появляется мгновенно после транскрибации и точно соответствует логу. Wayland-путь
  (wl-copy + XDG RemoteDesktop) не изменился.
- В `main()` добавлен явный вызов glib `g_set_prgname("arcanaglyph")` (через FFI). Без этого GTK
  устанавливал `WM_CLASS = "arcanaglyph-noavx"` (или `-avx`), что не совпадало со
  `StartupWMClass=arcanaglyph` в `.desktop`-файле — GNOME Dash не группировал окно с ярлыком,
  показывая отдельный пункт «Arcanaglyph-noavx» рядом с launcher'ом. Теперь `WM_CLASS = "arcanaglyph"`.
- В `Makefile` `make install` и в README/CLAUDE.md команды установки переведены с
  `sudo dpkg -i ... && sudo apt-get install -f -y` на единый `sudo apt install ./<deb>` —
  apt сам разрешает зависимости (wl-clipboard и др.), не падает с ошибкой `dpkg`.
- Whisper транскрибация на Intel Atom Tremont (Celeron N5095, без AVX) падала с
  `whisper_full_with_state: failed to encode` (Error code: -6) после ~20с work-time.
  Корневая причина — `whisper_rs::FullParams::set_abort_callback_safe`: trampoline
  через `Box::into_raw` без free даёт UB, на slow-CPU без AVX whisper.cpp читает
  garbage в `abort_callback_user_data` и аборится мгновенно. Фикс: убрали вызов
  `set_abort_callback_safe` совсем; cancel-логика работает только пост-фактум
  (флаг проверяется после `state.full()`, при отмене возвращаем `ArcanaError::Cancelled`).
  `WhisperTranscriber::supports_cancel()` теперь возвращает `false` — UI и так не
  показывал кнопку Стоп для whisper, теперь это согласовано на уровне трейта.
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
- Models tab: per-model резолюция пути для Whisper-вариантов в `get_models` —
  Tiny и Large делят общий `config.whisper_model_path`, но статус каждой карточки
  теперь резолвится по своему `default_filename` в `models_base_dir`. Ранее одна
  карточка могла видеть статус другой (Large card проверяла `ggml-tiny.bin` и
  показывала «Не найдена», хотя файл Large лежит на диске).
- Models tab: путь в input'е карточки пустой если файл не установлен — раньше
  заполнялся default-локацией даже после удаления, что вводило в заблуждение.
- Models tab: прогресс-бар активной загрузки сохраняется через re-render — например
  при удалении одной модели во время скачивания другой. Реализовано через
  `activeDownloads` Map в JS + `applyProgressToCard` хелпер, вызываемый в конце
  `renderModelCards`.
- Models tab: после успешного скачивания backend (`download_model`) автоматически
  обновляет config-путь для соответствующего движка — пользователь не должен
  жать «Сохранить» отдельно. Симметрично `delete_model` чистит config-путь
  если он совпадал с удалённым файлом.
- Models tab: кнопка «Удалить» у установленных моделей с confirm-диалогом —
  физически удаляет файлы (`remove_file` для Whisper, `remove_dir_all` для
  Vosk/GigaAM/Qwen3-ASR) + чистит config-путь.
- Dropdown «Движок транскрибации» теперь disabled-стилем с лейблом «(нет модели)»
  для движков чьи модели не скачаны (в дополнение к существующему «(не собрано)»
  для движков, отсутствующих в cargo-сборке).
- В дроп-дауне «Движок транскрибации» Whisper расщеплён на «Whisper Tiny» и
  «Whisper Large V3 Turbo» — пользователь явно выбирает вариант. JS-логика
  мапит расщеплённые значения в backend `transcriber=whisper` + явный
  `whisper_model_path`; обратное при загрузке config'а определяется по имени
  файла модели.
- Toast-предупреждение при выборе тяжёлой модели на CPU без AVX2 (Whisper Large
  V3 Turbo / Qwen3-ASR) — текст сообщает что транскрибация будет в 10-30× медленнее
  обычного и прервать её можно только убийством приложения. Добавлены i18n-ключи
  `settings.slow_model_warning` для ru/en и `settings.model_not_installed`.
- Tauri-команды `get_cpu_features` (для toast-логики), `delete_model`,
  `cancel_transcription`, `active_supports_cancel`. Последние две зарезервированы
  под будущее process-isolated Whisper — UI не показывает кнопку Стоп для
  whisper, потому что abort реально работает только пост-фактум.
- `RUST_LOG`-override для tracing-фильтра — раньше он был захардкожен
  `info,whisper_rs=warn`, теперь читается из env (`EnvFilter::try_from_default_env`).
- GitLab CI job `build-deb-and-upload` (новая stage `deb` после `release`):
  собирает `.deb` через `scripts/build-deb.sh` на shared runner и заливает как
  ассет к свежему GitHub Release через GitHub API. Кешит `~/.cargo` (registry +
  `cargo-tauri-cli` бинарь) между прогонами.

### Изменено

- `g_set_prgname("arcanaglyph")` через FFI в начале `main()` — без этого GTK
  устанавливал `WM_CLASS = "arcanaglyph-noavx"` (имя физического бинаря), что
  не совпадало со `StartupWMClass=arcanaglyph` в `.desktop`-файле. GNOME Dash
  не группировал окно с ярлыком — показывал отдельный пункт «Arcanaglyph-noavx».
- `make install` и инструкции в README/CLAUDE.md переведены на единый
  `sudo apt install ./<deb>` вместо `sudo dpkg -i ... && sudo apt-get install -f` —
  apt сам резолвит зависимости (`wl-clipboard` и др.).

### Исправлено

- **Корневой баг**: Whisper транскрибация на Intel Atom Tremont (Celeron N5095,
  без AVX) падала с `whisper_full_with_state: failed to encode` (Error code: -6)
  через ~20с work-time. Причина — `whisper_rs::FullParams::set_abort_callback_safe`:
  trampoline через `Box::into_raw` без free даёт UB, на slow-CPU без AVX
  whisper.cpp читает garbage в `abort_callback_user_data` и аборится мгновенно.
  Фикс: убрали вызов `set_abort_callback_safe` совсем; cancel-логика работает
  только пост-фактум (флаг проверяется после `state.full()`).
  `WhisperTranscriber::supports_cancel()` теперь возвращает `false`. Подробности
  см. `memory/whisper-abort-callback-bug.md`.

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
- `make run` автоматически останавливает запущенный экземпляр

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

- GitHub Actions: автосборка .deb и .AppImage при создании релиза с SHA256-хэшами

## [1.3.3] - 2026-04-11

### Исправлено

- CI: релизы GitLab/GitHub теперь создаются в одном pipeline с тегом (раньше не срабатывали)

## [1.3.2] - 2026-04-11

### Добавлено

- `make install` — сборка (если нужно), остановка запущенного экземпляра, установка .deb и запуск приложения одной командой
- CI: автоматическое зеркалирование в GitHub при merge в main
- CI: автоматическое создание релизов в GitLab и GitHub из CHANGELOG

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
- Сборка .deb и .AppImage через Tauri v2
