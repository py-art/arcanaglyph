// crates/arcanaglyph-core/src/transcription_models/whisper_tiny_speech_model.rs
//
// OpenAI Whisper Tiny — самая маленькая Whisper модель.
//
// Используется как практичный fallback на CPU без AVX (например Intel Celeron N5095),
// где GigaAM невозможен (ort требует AVX, tract не translate'ит Range-оператор GigaAM).
// На таких CPU Whisper Large/Small слишком медленные (3+ минуты на 4 секунды речи),
// а Tiny даёт юзабельные ~5x slower than real-time.
//
// Точность для русского ниже Small/Large, но единственный практичный вариант.

use super::SpeechModelInfo;

pub static MODEL: SpeechModelInfo = SpeechModelInfo {
    id: "whisper-tiny",
    display_name: "Whisper Tiny",
    transcriber_type: "whisper",
    default_filename: "ggml-tiny.bin",
    description: "Самая быстрая модель Whisper. Подходит для слабых CPU без AVX, где GigaAM невозможен.",
    size: "~75 МБ",
    download_url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin",
    extra_files: None,
    expected_min_size_bytes: Some(70_000_000),
};
