// crates/arcanaglyph-core/src/dsp/mod.rs
//
// Общие DSP-примитивы аудио, не привязанные к конкретному движку. Два слоя:
// - `spectral`   — оконо Ханна + power-STFT для mel-бэкендов (GigaAM/Qwen3).
// - `preprocess` — обрезка тишины + i16→f32 + resample (общий вход whisper/mel).

// Спектральные примитивы нужны только mel-бэкендам (GigaAM/Qwen3); при
// whisper-only сборке модуль не компилируется (иначе dead_code под clippy -D).
#[cfg(any(feature = "gigaam", feature = "gigaam-system-ort", feature = "qwen3asr"))]
mod spectral;
#[cfg(any(feature = "gigaam", feature = "gigaam-system-ort", feature = "qwen3asr"))]
pub use spectral::{StftConfig, hann_window, stft_power};

// Входной препроцессинг нужен всем «тяжёлым» движкам (whisper + mel). Модуль
// компилируется при любом из них — он совпадает с cfg-гейтом самого `dsp`.
mod preprocess;
pub(crate) use preprocess::preprocess_to_f32_16k;
