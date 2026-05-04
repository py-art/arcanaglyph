// crates/arcanaglyph-core/src/gigaam/mod.rs
//
// Модуль GigaAM v3 — высокоточная STT-модель для русского языка (SberDevices).
// Содержит препроцессинг (mel-спектрограмма) и транскрайбер (ONNX inference + CTC decode).
//
// Два альтернативных backend'а с одинаковым публичным API (`GigaAmTranscriber`):
//   - `transcriber` (feature `gigaam`): через `ort` — Microsoft pre-built ONNX Runtime,
//     быстрый INT8 inference, требует AVX SIMD на CPU. Дефолт для современных x86_64.
//   - `transcriber_tract` (feature `gigaam-tract`): через `tract` — pure-Rust ONNX
//     inference, FP32, без AVX-зависимости. Для слабых/embedded CPU (например N5095).
// Features mutually-exclusive: одновременно собираться не должны.

pub mod mel;

// `transcriber` (через ort) активен и для `gigaam` (ort с download-binaries),
// и для `gigaam-system-ort` (ort с load-dynamic). Реализация одна — отличается
// только способ доставки libonnxruntime.so (см. core/Cargo.toml).
#[cfg(any(feature = "gigaam", feature = "gigaam-system-ort"))]
pub mod transcriber;

#[cfg(feature = "gigaam-tract")]
pub mod transcriber_tract;
