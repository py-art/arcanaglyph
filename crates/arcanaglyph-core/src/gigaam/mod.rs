// crates/arcanaglyph-core/src/gigaam/mod.rs
//
// Модуль GigaAM v3 — высокоточная STT-модель для русского языка (SberDevices).
// Содержит препроцессинг (mel-спектрограмма) и транскрайбер (ONNX inference + CTC decode).

pub mod mel;

// `transcriber` (через ort) активен и для `gigaam` (ort с download-binaries),
// и для `gigaam-system-ort` (ort с load-dynamic). Реализация одна — отличается
// только способ доставки libonnxruntime.so (см. core/Cargo.toml).
#[cfg(any(feature = "gigaam", feature = "gigaam-system-ort"))]
pub mod transcriber;
