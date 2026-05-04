// crates/arcanaglyph-core/src/transcription_models/gigaam_v3_fp32_speech_model.rs
//
// GigaAM v3 E2E CTC — FP32 версия для tract-backend.
//
// Та же модель что и `gigaam_v3_speech_model::MODEL` (INT8), только в float32 формате.
// Используется на CPU без AVX (через feature `gigaam-tract`), где Microsoft pre-built
// ONNX Runtime крашит SIGILL до main(). FP32 универсально совместима с tract'ом
// (pure-Rust ONNX inference без AVX-зависимости).
//
// Размер: ~846 МБ (vs ~225 МБ INT8). Точность распознавания та же (~8.4% WER на русском).

use super::SpeechModelInfo;

pub static MODEL: SpeechModelInfo = SpeechModelInfo {
    id: "gigaam-v3-e2e-ctc-fp32",
    display_name: "GigaAM v3 E2E CTC (FP32, tract)",
    transcriber_type: "gigaam",
    default_filename: "gigaam-v3-e2e-ctc",
    description: "GigaAM v3 от SberDevices, FP32-версия для tract-backend (CPU без AVX). WER ~8.4%.",
    size: "~846 МБ",
    download_url: "https://huggingface.co/istupakov/gigaam-v3-onnx/resolve/main/v3_e2e_ctc.onnx",
    extra_files: Some(&[(
        "https://huggingface.co/istupakov/gigaam-v3-onnx/resolve/main/v3_e2e_ctc_vocab.txt",
        "v3_e2e_ctc_vocab.txt",
    )]),
    // Реальный размер ~846 МБ; порог 750 МБ переживёт минорные обновления модели в источнике
    expected_min_size_bytes: Some(750_000_000),
};
