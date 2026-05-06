// crates/arcanaglyph-core/src/transcription_models/gigaam_v3_speech_model.rs
//
// GigaAM v3 E2E CTC — лучшая модель для русского языка (SberDevices)
//
// Характеристики:
// - Пакетная обработка (после записи целиком)
// - Высочайшая точность на русском (~8.4% WER — лучший результат)
// - Средняя скорость транскрибации (ONNX Runtime, INT8 квантизация)
// - Компактный размер (~225 МБ)
// - Только русский язык
// - Включает пунктуацию и капитализацию
// - Формат: ONNX (INT8)

use super::SpeechModelInfo;

pub static MODEL: SpeechModelInfo = SpeechModelInfo {
    id: "gigaam-v3-e2e-ctc",
    display_name: "GigaAM v3 E2E CTC",
    transcriber_type: "gigaam",
    default_filename: "gigaam-v3-e2e-ctc",
    description: "Лучшая модель для русского от SberDevices. WER ~8.4%, пунктуация, INT8 квантизация.",
    size: "~225 МБ",
    download_url: "https://huggingface.co/istupakov/gigaam-v3-onnx/resolve/main/v3_e2e_ctc.int8.onnx",
    extra_files: Some(&[(
        "https://huggingface.co/istupakov/gigaam-v3-onnx/resolve/main/v3_e2e_ctc_vocab.txt",
        "v3_e2e_ctc_vocab.txt",
    )]),
    // Главный файл `v3_e2e_ctc.int8.onnx` ~225 МБ; порог 200 МБ
    expected_min_size_bytes: Some(200_000_000),
};
