// crates/arcanaglyph-core/src/transcription_models/vosk_russian_speech_model.rs
//
// Vosk — офлайн модель распознавания русской речи (большая, vosk-model-ru-0.42)
//
// Характеристики:
// - Потоковая обработка (real-time, partial results)
// - Точность ~5-10% WER на чистой русской речи
// - Быстрая транскрибация после загрузки
// - Долгая загрузка модели в память при старте (~10-20с)
// - Архив ~1.8 ГБ, распакованная модель ~2.6 ГБ на диске
// - Работает на CPU без GPU
// - URL качается архивом; авто-распаковка реализована в `extract_zip_to_parent`
//   (crates/arcanaglyph-app/src/main.rs).

use super::SpeechModelInfo;

pub static MODEL: SpeechModelInfo = SpeechModelInfo {
    id: "vosk-ru-0.42",
    display_name: "Vosk Russian 0.42",
    transcriber_type: "vosk",
    default_filename: "vosk-model-ru-0.42",
    description: "Большая офлайн-модель русской речи. Точная (~5-10% WER), потоковая. \
                  Требует ~2.6 ГБ места после распаковки.",
    size: "~1.8 ГБ",
    download_url: "https://alphacephei.com/vosk/models/vosk-model-ru-0.42.zip",
    extra_files: None,
    // Минимальный размер скачанного архива (1.5 ГБ). Реальный архив ~1.8 ГБ;
    // если HTTP вернул меньше — обрезанный ответ, файл удаляется как повреждённый.
    expected_min_size_bytes: Some(1_500_000_000),
};
