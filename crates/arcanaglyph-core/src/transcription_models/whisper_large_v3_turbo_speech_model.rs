// crates/arcanaglyph-core/src/transcription_models/whisper_large_v3_turbo_speech_model.rs
//
// OpenAI Whisper Large V3 Turbo — высокоточная модель распознавания речи
//
// Характеристики:
// - Пакетная обработка (после записи целиком)
// - Высокая точность (~6-10% WER на русском)
// - Медленная транскрибация на CPU (~30-70с для 10с аудио)
// - Быстрая загрузка модели (~1с)
// - Большой размер (~1.5 ГБ)
// - Поддерживает 100 языков
// - Формат: ggml (whisper.cpp)

use super::SpeechModelInfo;

pub static MODEL: SpeechModelInfo = SpeechModelInfo {
    id: "whisper-large-v3-turbo",
    display_name: "Whisper Large V3 Turbo",
    transcriber_type: "whisper",
    default_filename: "ggml-large-v3-turbo.bin",
    description: "Высокоточная модель от OpenAI. Лучший баланс скорости и качества среди Whisper моделей.",
    size: "~1.5 ГБ",
    download_url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo.bin",
    extra_files: None,
    // Реальный размер ~1.62 ГБ; порог в 1.4 ГБ переживёт минорные апдейты в источнике
    expected_min_size_bytes: Some(1_400_000_000),
};
