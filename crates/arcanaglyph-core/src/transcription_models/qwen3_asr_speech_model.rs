// crates/arcanaglyph-core/src/transcription_models/qwen3_asr_speech_model.rs
//
// Qwen3-ASR-0.6B — мультиязычная модель (Alibaba)
//
// Характеристики:
// - Пакетная обработка (авторегрессивный decoder)
// - Высокая точность на множестве языков (~5.76% WER средний)
// - 52 языка включая русский
// - Формат: ONNX (INT8 квантизация decoder)
// - Размер: ~2.5 ГБ (4 ONNX файла + embeddings + tokenizer)

use super::SpeechModelInfo;

pub static MODEL: SpeechModelInfo = SpeechModelInfo {
    id: "qwen3-asr-0.6b",
    display_name: "Qwen3-ASR 0.6B",
    transcriber_type: "qwen3asr",
    default_filename: "qwen3-asr-0.6b",
    description: "Мультиязычная модель от Alibaba. 52 языка, высокая точность, авторегрессивный decoder.",
    size: "~2.5 ГБ",
    download_url: "https://huggingface.co/Daumee/Qwen3-ASR-0.6B-ONNX-CPU/resolve/main/tokenizer.json",
    extra_files: Some(&[
        (
            "https://huggingface.co/Daumee/Qwen3-ASR-0.6B-ONNX-CPU/resolve/main/onnx_models/encoder_conv.onnx",
            "onnx_models/encoder_conv.onnx",
        ),
        (
            "https://huggingface.co/Daumee/Qwen3-ASR-0.6B-ONNX-CPU/resolve/main/onnx_models/encoder_conv.onnx.data",
            "onnx_models/encoder_conv.onnx.data",
        ),
        (
            "https://huggingface.co/Daumee/Qwen3-ASR-0.6B-ONNX-CPU/resolve/main/onnx_models/encoder_transformer.onnx",
            "onnx_models/encoder_transformer.onnx",
        ),
        (
            "https://huggingface.co/Daumee/Qwen3-ASR-0.6B-ONNX-CPU/resolve/main/onnx_models/encoder_transformer.onnx.data",
            "onnx_models/encoder_transformer.onnx.data",
        ),
        (
            "https://huggingface.co/Daumee/Qwen3-ASR-0.6B-ONNX-CPU/resolve/main/onnx_models/decoder_init.int8.onnx",
            "onnx_models/decoder_init.int8.onnx",
        ),
        (
            "https://huggingface.co/Daumee/Qwen3-ASR-0.6B-ONNX-CPU/resolve/main/onnx_models/decoder_step.int8.onnx",
            "onnx_models/decoder_step.int8.onnx",
        ),
        (
            "https://huggingface.co/Daumee/Qwen3-ASR-0.6B-ONNX-CPU/resolve/main/onnx_models/embed_tokens.bin",
            "onnx_models/embed_tokens.bin",
        ),
    ]),
    // Главный файл — `tokenizer.json` (~10 МБ), порог 2 МБ.
    // Большая часть объёма (~2.5 ГБ) лежит в extra_files; для них целостность
    // в текущей реализации проверяется только наличием файла.
    expected_min_size_bytes: Some(2_000_000),
};
