<!-- arcanaglyph/Roadmap.md -->

# ArcanaGlyph: Roadmap

Кросс-платформенное десктопное приложение для голосового ввода текста (Linux/macOS/Windows).

---

## Выполненные этапы

### Phase 0: Foundation & Environment Setup

- [x] Инициализация Cargo workspace (arcanaglyph-core + arcanaglyph-app)
- [x] Настройка системных зависимостей (libasound2, libgtk-3, libwebkit2gtk-4.1)
- [x] Линтинг: cargo clippy, cargo fmt, taplo, markdownlint
- [x] Makefile с командами: run, all, fmt, lint, test, build, dist, clean

### Phase 1: Core Logic — захват аудио и транскрибация

- [x] Захват аудио с микрофона через `cpal`
- [x] Трейт `Transcriber` — единый интерфейс для STT-движков
- [x] VoskTranscriber — streaming распознавание (vosk-model-ru-0.42)
- [x] WhisperTranscriber — batch распознавание (whisper-rs + ggml-large-v3-turbo)
- [x] Подавление галлюцинаций Whisper, тишина-тримминг, ресемплинг

### Phase 2: Desktop Application Shell — Tauri v2

- [x] Tauri v2 приложение с vanilla JS фронтендом
- [x] IPC: Tauri commands (trigger, is_recording) + события engine → фронтенд
- [x] Базовый UI: кнопка записи, отображение результата

### Phase 3: System Integration

- [x] Глобальные горячие клавиши (tauri-plugin-global-shortcut для X11)
- [x] Автоматическая регистрация хоткеев через gsettings (Wayland/GNOME)
- [x] UDP-скрипты ag-trigger / ag-pause для Wayland
- [x] Вставка текста: wl-copy + XDG RemoteDesktop portal (Wayland), enigo (X11)
- [x] Иконка в системном трее: белая (idle), красная (запись), оранжевая (пауза)
- [x] Меню трея: Открыть / Запись / Настройки / Выход (с разделителями)

### Phase 4: Configuration & Polish

- [x] Настройки в SQLite (rusqlite, миграции БД)
- [x] UI настроек с табами: Основное / Модели / Клавиши
- [x] Кастомные dropdown-компоненты (без нативных `<select>`)
- [x] Реестр моделей (SpeechModelInfo) с метаданными и карточками в UI
- [x] Пул предзагрузки моделей — мгновенное переключение между движками
- [x] История транскрипций в SQLite с пагинацией
- [x] Сборка дистрибутивов: .deb, .AppImage

### Phase 5: GigaAM v3 — лучший движок для русского языка

- [x] Интеграция GigaAM v3 E2E CTC через ONNX Runtime (`ort` + `ndarray` + `rustfft`)
- [x] Mel-спектрограмма: STFT + HTK filterbank + log (16kHz, 64 mel bins)
- [x] CTC greedy decode + SentencePiece vocab (257 токенов)
- [x] Реестр моделей, UI, retranscribe, предзагрузка
- [x] Подавление verbose-логов ONNX Runtime

### Phase 8: UX-улучшения (частично)

- [x] Табы в настройках: Основное / Модели / Клавиши
- [x] Карточки моделей с описанием, размером, статусом installed/missing
- [x] Композер горячих клавиш с кнопками модификаторов + рекордер основной клавиши
- [x] Горячая клавиша паузы (hotkey_pause) + регистрация в GNOME через gsettings
- [x] Проверка конфликтов хоткеев перед сохранением (сканирование gsettings)
- [x] Настройка «Запуск в трей» (start_minimized)
- [x] Удаление слов-паразитов (э, э-э, ээ, эм, мм) — настраиваемый фильтр
- [x] Новый логотип: кольцо + точка (idle/recording/paused)

---

## Текущее состояние

**Работающий MVP** с тремя STT-движками:

| Движок | Модель | WER (рус.) | Размер | Streaming | Скорость |
| --- | --- | --- | --- | --- | --- |
| Vosk | vosk-model-ru-0.42 | ~11% | 42 MB | Да | Быстро |
| Whisper | ggml-large-v3-turbo | ~14% | 1.5 GB | Нет | 30-70 сек / 10 сек аудио |
| **GigaAM v3** | v3_e2e_ctc.int8.onnx | **~8.4%** | 225 MB | Нет | **~0.8 сек / 5 сек аудио** |

---

## Phase 5.4: GPU-ускорение (опционально)

- [ ] Проверить CUDA ExecutionProvider через `ort`
- [ ] Fallback на CPU если GPU недоступен
- [ ] Бенчмарки: CPU INT8 vs GPU FP32

---

## Phase 6: sherpa-onnx — универсальный runtime (альтернативный путь)

**Цель:** Оценить sherpa-onnx как единый runtime для всех моделей вместо отдельных интеграций.

- [ ] Исследовать sherpa-onnx Rust API (примеры в `rust-api-examples/`)
- [ ] Прототип: запуск GigaAM v3 через sherpa-onnx offline recognizer
- [ ] Сравнить с прямой `ort`-интеграцией по качеству, скорости, размеру зависимостей
- [ ] Решение: оставить `ort` или мигрировать на sherpa-onnx

---

## Phase 7: Qwen3-ASR — мультиязычный движок

**Цель:** Добавить Qwen3-ASR-0.6B как мультиязычный движок (52 языка, WER 5.76%).

- [ ] Мониторить появление Rust-биндингов к qwen3-asr.cpp
- [ ] Интегрировать через sherpa-onnx или `ort`
- [ ] Реализовать `Qwen3AsrTranscriber` по трейту `Transcriber`
- [ ] Добавить в реестр моделей и UI

---

## Phase 8: UX-улучшения (оставшееся)

- [ ] VAD (Voice Activity Detection) — автоматическая остановка записи при паузе
- [ ] Streaming-отображение частичных результатов для GigaAM RNNT
- [ ] Автоскачивание моделей из UI с прогресс-баром (встроенное)
- [ ] Выбор языка распознавания (для мультиязычных движков)
- [ ] Настройка параметров inference (количество потоков, GPU/CPU)

---

## Phase 9: Продвинутые возможности

- [ ] Пунктуация и капитализация для Vosk (постпроцессинг или отдельная модель)
- [ ] Замена слов / автокоррекция (пользовательский словарь)
- [ ] Экспорт истории транскрипций
- [ ] Статистика использования (время записей, распределение по движкам)

---

## Phase 10: Кросс-платформенность

**Цель:** Запуск на Linux (Wayland + X11), macOS и Windows.

### 10.1 Установка и скрипты

- [ ] Автоматическое создание UDP-скриптов (ag-trigger, ag-pause) при первом запуске на Wayland
- [ ] Установка скриптов в `~/.local/bin/` с проверкой PATH
- [ ] Для macOS/Windows: глобальные хоткеи через tauri-plugin-global-shortcut (работают нативно)

### 10.2 macOS

- [ ] Сборка и тестирование на macOS (Apple Silicon + Intel)
- [ ] Подпись приложения (codesign) и нотаризация
- [ ] Разрешение на микрофон (Privacy & Security)
- [ ] Вставка текста через CGEventPost или AppleScript
- [ ] DMG-пакет для распространения

### 10.3 Windows

- [ ] Сборка и тестирование на Windows 10/11
- [ ] Вставка текста через Windows API (SendInput)
- [ ] MSIX / MSI инсталлятор
- [ ] Автозапуск через реестр (опционально)

### 10.4 Linux — полировка

- [ ] Flatpak / Snap пакеты
- [ ] Поддержка PipeWire (если cpal не покрывает)
- [ ] KDE Plasma: хоткеи через kglobalaccel (не только GNOME gsettings)
- [ ] Автоустановка wl-clipboard если не найден

---

## Ближайшие приоритеты

1. **Автоустановка UDP-скриптов** — Phase 10.1 (быстро, важно для первого запуска)
2. **VAD** — Phase 8 (автоостановка записи при паузе в речи)
3. **Автоскачивание моделей** — Phase 8 (встроенное скачивание с прогресс-баром)
4. **macOS** — Phase 10.2 (Tauri v2 уже поддерживает macOS)

---

## Справочник: сравнение STT-моделей (апрель 2026)

| Модель | Разработчик | WER (рус.) | Размер | Лицензия | Rust-интеграция |
| --- | --- | --- | --- | --- | --- |
| **GigaAM v3 e2e_ctc** | Sber | **8.4%** | 225 MB (INT8) | MIT | ort (ONNX) / sherpa-onnx |
| Qwen3-ASR-0.6B | Alibaba | ~6% (multi) | 1.3 GB (Q8) | Apache 2.0 | qwen3-asr.cpp / sherpa-onnx |
| NVIDIA Canary 2.5B | NVIDIA | ~6% (multi) | 2.5B | CC BY 4.0 | NeMo (Python only) |
| Whisper Large V3 Turbo | OpenAI | ~14% | 1.5 GB | MIT | whisper-rs |
| Vosk 0.54 | alphacep | ~11% | 42-250 MB | Apache 2.0 | vosk-rs |

---

## Ссылки

- [GigaAM v3 ONNX](https://huggingface.co/istupakov/gigaam-v3-onnx) — готовые ONNX-модели
- [GigaAM v3 sherpa-onnx](https://huggingface.co/Smirnov75/GigaAM-v3-sherpa-onnx) — модели для sherpa-onnx
- [salute-developers/GigaAM](https://github.com/salute-developers/GigaAM) — исходный код и препроцессинг
- [ort crate](https://crates.io/crates/ort) — ONNX Runtime для Rust (v2.0.0-rc.12)
- [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx) — универсальный STT runtime с Rust API
- [Qwen3-ASR](https://github.com/QwenLM/Qwen3-ASR) — мультиязычная ASR от Alibaba
- [qwen3-asr.cpp](https://github.com/predict-woo/qwen3-asr.cpp) — C++ реализация на GGML
- [Open ASR Leaderboard](https://huggingface.co/spaces/hf-audio/open_asr_leaderboard) — бенчмарки
- [Русские ASR модели 2025](https://alphacephei.com/nsh/2025/04/18/russian-models.html) — обзор alphacephei
