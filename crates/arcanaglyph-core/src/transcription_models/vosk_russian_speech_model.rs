// crates/arcanaglyph-core/src/transcription_models/vosk_russian_speech_model.rs
//
// Vosk — офлайн модель распознавания русской речи
//
// Характеристики:
// - Потоковая обработка (real-time, partial results)
// - Низкая точность (~15-25% WER на русском)
// - Быстрая транскрибация (~0.5-3с)
// - Долгая загрузка модели (~20с)
// - Маленький размер (~42 МБ)
// - Работает на CPU без GPU

use super::SpeechModelInfo;

pub static MODEL: SpeechModelInfo = SpeechModelInfo {
    id: "vosk-ru-0.42",
    display_name: "Vosk Russian 0.42",
    transcriber_type: "vosk",
    default_filename: "vosk-model-ru-0.42",
    description: "Быстрая потоковая модель для русского языка. Работает в реальном времени, но менее точная.",
    size: "~42 МБ",
    download_url: "https://alphacephei.com/vosk/models/vosk-model-ru-0.42.zip",
    extra_files: None,
};
