// crates/arcanaglyph-core/src/transcription_models/gigaam_v3_rnnt_speech_model.rs
//
// GigaAM v3 E2E RNN-T — самая точная модель для русского языка (SberDevices)
//
// Характеристики:
// - Пакетная обработка (после записи целиком)
// - Точнее CTC-варианта (~8.4% WER против ~9.2% у E2E CTC)
// - Тяжелее по вычислениям: encoder + decoder + joint, авторегрессивный greedy-декод
//   (медленнее CTC, заметно на слабых CPU)
// - Размер сопоставим с CTC (~227 МБ: encoder ~225 + decoder + joint, INT8)
// - Только русский язык, с пунктуацией и капитализацией
// - Формат: ONNX (INT8), три файла + словарь

use super::SpeechModelInfo;

pub static MODEL: SpeechModelInfo = SpeechModelInfo {
    id: "gigaam-v3-e2e-rnnt",
    display_name: "GigaAM v3 E2E RNN-T",
    transcriber_type: "gigaam-rnnt",
    default_filename: "gigaam-v3-e2e-rnnt",
    description: "Самая точная для русского (SberDevices). WER ~8.4%, RNN-T-декод, пунктуация, INT8.",
    size: "~227 МБ",
    // Главный файл — энкодер (~225 МБ); decoder/joint/словарь идут как extra_files.
    download_url: "https://huggingface.co/istupakov/gigaam-v3-onnx/resolve/main/v3_e2e_rnnt_encoder.int8.onnx",
    extra_files: Some(&[
        (
            "https://huggingface.co/istupakov/gigaam-v3-onnx/resolve/main/v3_e2e_rnnt_decoder.int8.onnx",
            "v3_e2e_rnnt_decoder.int8.onnx",
        ),
        (
            "https://huggingface.co/istupakov/gigaam-v3-onnx/resolve/main/v3_e2e_rnnt_joint.int8.onnx",
            "v3_e2e_rnnt_joint.int8.onnx",
        ),
        (
            "https://huggingface.co/istupakov/gigaam-v3-onnx/resolve/main/v3_e2e_rnnt_vocab.txt",
            "v3_e2e_rnnt_vocab.txt",
        ),
    ]),
    // Главный файл `v3_e2e_rnnt_encoder.int8.onnx` ~225 МБ; порог 200 МБ.
    expected_min_size_bytes: Some(200_000_000),
};
