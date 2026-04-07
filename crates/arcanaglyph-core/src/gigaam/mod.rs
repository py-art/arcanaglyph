// crates/arcanaglyph-core/src/gigaam/mod.rs
//
// Модуль GigaAM v3 — высокоточная STT-модель для русского языка (SberDevices).
// Содержит препроцессинг (mel-спектрограмма) и транскрайбер (ONNX inference + CTC decode).

pub mod mel;
pub mod transcriber;
