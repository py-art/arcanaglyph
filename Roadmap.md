<!-- arcanaglyph/Roadmap.md -->

# ArcanaGlyph: Roadmap

Пошаговый план развития ArcanaGlyph — десктопного приложения для голосового ввода текста на Linux.

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

- [x] Глобальные горячие клавиши (tauri-plugin-global-shortcut)
- [x] Вставка текста: wl-copy + XDG RemoteDesktop portal (Wayland), enigo (X11)
- [x] Иконка в системном трее (красная при записи) с меню

### Phase 4: Configuration & Polish

- [x] Настройки в SQLite (rusqlite, миграции БД)
- [x] UI настроек: выбор движка, модели, горячей клавиши
- [x] Кастомные dropdown-компоненты (без нативных `<select>`)
- [x] Реестр моделей (SpeechModelInfo) с метаданными
- [x] Пул предзагрузки моделей — мгновенное переключение между движками
- [x] История транскрипций в SQLite
- [x] Сборка дистрибутивов: .deb, .AppImage

---

## Текущее состояние

**Работающий MVP** с тремя STT-движками:

| Движок | Модель | WER (рус.) | Размер | Streaming | Скорость |
| --- | --- | --- | --- | --- | --- |
| Vosk | vosk-model-ru-0.42 | ~11% | 42 MB | Да | Быстро |
| Whisper | ggml-large-v3-turbo | ~14% | 1.5 GB | Нет | 30-70 сек / 10 сек аудио |
| **GigaAM v3** | v3_e2e_ctc.int8.onnx | **~8.4%** | 225 MB | Нет | **~0.8 сек / 5 сек аудио** |

---

## Phase 5: GigaAM v3 — лучший движок для русского языка

**Цель:** Интегрировать GigaAM v3 (Sber) — SOTA модель для русской речи (WER 8.4%), компактную (225 MB INT8)
и значительно точнее текущих движков.

**Технология:** ONNX Runtime через крейт `ort` (v2.0.0-rc.12).

**Модель:** `v3_e2e_ctc.int8.onnx` (225 MB) — CTC с пунктуацией, 257 SentencePiece-токенов.

### 5.1 Исследование и прототип

- [x] Добавить зависимости: `ort`, `ndarray`, `rustfft` в arcanaglyph-core
- [x] Скачать ONNX-модель с HuggingFace ([istupakov/gigaam-v3-onnx](https://huggingface.co/istupakov/gigaam-v3-onnx))
- [x] Реализовать mel-спектрограмму для GigaAM:
  - Sample rate: 16000 Hz
  - STFT: n_fft=320, win_length=320 (20ms), hop_length=160 (10ms), center=false
  - Mel filterbank: 64 бина, HTK scale, без нормализации
  - Log: `log(clamp(mel, 1e-9, 1e9))`
- [x] Загрузка ONNX-модели через `ort::Session`
- [x] Написать standalone тест: WAV-файл → mel → ONNX inference → текст

### 5.2 CTC-декодирование

- [x] Greedy CTC decode: argmax по кадрам, удаление дубликатов и blank-токенов
- [x] Загрузка словаря SentencePiece (v3_e2e_ctc_vocab.txt, 257 токенов)
- [x] Декодирование subword-токенов в текст (символ `▁` → пробел)
- [x] Валидация на тестовых аудиофайлах — сравнение с Python-референсом

### 5.3 Интеграция в архитектуру

- [x] Добавить `TranscriberType::GigaAM` в enum (`config.rs`)
- [x] Реализовать `GigaAmTranscriber` по трейту `Transcriber` (`transcriber.rs`)
  - `transcribe()`: i16 → f32 → resample 16kHz → mel → ONNX → CTC decode
  - `supports_streaming()`: false (CTC-модель — offline)
- [x] Добавить `gigaam_model_path` в `CoreConfig`
- [x] Обновить `engine.rs`: `create_transcriber()` для GigaAM
- [x] Создать `transcription_models/gigaam_v3_speech_model.rs` с метаданными
- [x] Обновить реестр моделей в `transcription_models/mod.rs`
- [x] DB-миграция не нужна — serde(default) обрабатывает старые конфиги

### 5.4 GPU-ускорение (опционально)

- [ ] Проверить CUDA ExecutionProvider через `ort`
- [ ] Fallback на CPU если GPU недоступен
- [ ] Бенчмарки: CPU INT8 vs GPU FP32

### 5.5 UI и UX

- [x] Добавить GigaAM в dropdown выбора движка в настройках
- [ ] Скачивание модели: прогресс-бар или инструкция
- [ ] Обновить описания моделей в UI (размер, WER, особенности)

### Ожидаемый результат

| Движок | WER (рус.) | Размер | RAM | Скорость (CPU) |
| --- | --- | --- | --- | --- |
| GigaAM v3 INT8 | ~8.4% | 225 MB | ~500 MB | 5-15x realtime |
| Whisper Turbo | ~14% | 1.5 GB | ~2 GB | 0.15-0.3x realtime |
| Vosk | ~11% | 42 MB | ~200 MB | realtime (streaming) |

---

## Phase 6: sherpa-onnx — универсальный runtime (альтернативный путь)

**Цель:** Оценить sherpa-onnx как единый runtime для всех моделей вместо отдельных интеграций.

**Зачем:** sherpa-onnx имеет официальный Rust API, встроенный VAD, поддержку GigaAM v3
([Smirnov75/GigaAM-v3-sherpa-onnx](https://huggingface.co/Smirnov75/GigaAM-v3-sherpa-onnx)),
Whisper, Qwen3-ASR и десятков других моделей через единый интерфейс.

- [ ] Исследовать sherpa-onnx Rust API (примеры в `rust-api-examples/`)
- [ ] Прототип: запуск GigaAM v3 через sherpa-onnx offline recognizer
- [ ] Сравнить с прямой `ort`-интеграцией (Phase 5) по:
  - Качество распознавания (идентично ли?)
  - Скорость inference
  - Размер зависимостей (FFI к C++ библиотеке)
  - Гибкость настройки
- [ ] Решение: оставить `ort` или мигрировать на sherpa-onnx
- [ ] Если sherpa-onnx — рефакторинг `Transcriber` трейта под единый runtime

---

## Phase 7: Qwen3-ASR — мультиязычный движок

**Цель:** Добавить Qwen3-ASR-0.6B как мультиязычный движок (52 языка, WER 5.76%).
Актуально, когда нужна поддержка не только русского.

**Предпосылки:** Появление ONNX/GGML-рантайма для Rust. На апрель 2026:

- ONNX-экспорт: [andrewleech/qwen3-asr-0.6b-onnx](https://huggingface.co/andrewleech/qwen3-asr-0.6b-onnx)
- C++ реализация на GGML: [predict-woo/qwen3-asr.cpp](https://github.com/predict-woo/qwen3-asr.cpp) (~1.3 GB Q8_0)
- sherpa-onnx: есть пример `run-qwen3-asr.sh`

- [ ] Мониторить появление Rust-биндингов к qwen3-asr.cpp
- [ ] Или интегрировать через sherpa-onnx (если Phase 6 подтвердит жизнеспособность)
- [ ] Или через `ort` с ONNX-моделями (3 файла: encoder + decoder_init + decoder_step)
- [ ] Реализовать `Qwen3AsrTranscriber` по трейту `Transcriber`
- [ ] Добавить в реестр моделей и UI

---

## Phase 8: UX-улучшения

- [x] Табы в настройках: «Основное» и «Модели»
- [x] Карточки моделей с описанием, размером, статусом (installed/missing)
- [x] Tauri-команда `get_models` — реестр моделей + проверка наличия на диске
- [x] Кнопка «Скачать» — открывает URL модели в браузере
- [ ] VAD (Voice Activity Detection) — автоматическая остановка записи при паузе
- [ ] Streaming-отображение частичных результатов для GigaAM RNNT
- [ ] Автоскачивание моделей из UI с прогресс-баром (встроенное)
- [ ] Выбор языка распознавания (для мультиязычных движков)
- [ ] Настройка параметров inference (количество потоков, GPU/CPU)
- [ ] Горячее переключение движка без перезапуска

---

## Phase 9: Продвинутые возможности

- [ ] Пунктуация и капитализация для Vosk (постпроцессинг или отдельная модель)
- [ ] Замена слов / автокоррекция (пользовательский словарь)
- [ ] Экспорт истории транскрипций
- [ ] Статистика использования (время записей, распределение по движкам)
- [ ] Flatpak / Snap пакеты
- [ ] Поддержка PipeWire (если cpal не покрывает)

---

## Справочник: сравнение STT-моделей (апрель 2026)

| Модель | Разработчик | WER (рус.) | Размер | Лицензия | Rust-интеграция |
| --- | --- | --- | --- | --- | --- |
| **GigaAM v3 e2e_ctc** | Sber | **8.4%** | 225 MB (INT8) | MIT | ort (ONNX) / sherpa-onnx |
| Qwen3-ASR-0.6B | Alibaba | ~6% (multi) | 1.3 GB (Q8) | Apache 2.0 | qwen3-asr.cpp / sherpa-onnx |
| NVIDIA Canary 2.5B | NVIDIA | ~6% (multi) | 2.5B | CC BY 4.0 | NeMo (Python only) |
| Whisper Large V3 Turbo | OpenAI | ~14% | 1.5 GB | MIT | whisper-rs |
| Vosk 0.54 | alphacep | ~11% | 42-250 MB | Apache 2.0 | vosk-rs |

**Приоритет для ArcanaGlyph:** GigaAM v3 (Phase 5) → sherpa-onnx evaluation (Phase 6) → Qwen3-ASR (Phase 7).

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
