// crates/arcanaglyph-core/src/lib.rs

pub mod audio;
pub mod config;
pub mod db;
pub mod engine;
pub mod error;
// Модуль gigaam подключается при ЛЮБОМ из двух backend-features:
// - `gigaam` (ort + Microsoft pre-built ONNX, требует AVX)
// - `gigaam-system-ort` (ort + локально собранная libonnxruntime.so без AVX)
#[cfg(any(feature = "gigaam", feature = "gigaam-system-ort"))]
pub mod gigaam;
pub mod history;
pub mod input;
#[cfg(feature = "qwen3asr")]
pub mod qwen3asr;
pub mod transcriber;
pub mod transcription_models;

pub use config::CoreConfig;
pub use engine::{ArcanaEngine, EngineEvent};
pub use error::ArcanaError;
