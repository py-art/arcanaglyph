// crates/arcanaglyph-core/src/transcriber/mod.rs

use crate::config::{CoreConfig, TranscriberType};
use crate::error::ArcanaError;

// Движки вынесены в отдельные файлы (симметрично gigaam/ и qwen3asr/).
// Каждый подмодуль гейтится своей feature; re-export сохраняет публичный путь
// `crate::transcriber::VoskTranscriber` / `WhisperTranscriber`.
#[cfg(feature = "vosk")]
mod vosk;
#[cfg(feature = "vosk")]
pub use vosk::VoskTranscriber;

#[cfg(feature = "whisper")]
mod whisper;
#[cfg(feature = "whisper")]
pub use whisper::WhisperTranscriber;

/// Единая cfg-gated фабрика транскрайберов: тип из конфига → `Box<dyn Transcriber>`.
/// Используется и движком (`ArcanaEngine::create_transcriber` оборачивает в `Arc`),
/// и `retranscribe`. Инициализация ORT-логирования для ONNX-движков остаётся на
/// стороне вызывающего — она зависит от способа линковки ORT (статика vs load-dynamic).
///
/// `allow(unused_variables)` — для сборок без единого движка все плечи `match`
/// стираются, и параметр `config` становится формально неиспользуемым.
#[allow(unused_variables)]
pub fn build_transcriber(config: &CoreConfig, t_type: &TranscriberType) -> Result<Box<dyn Transcriber>, ArcanaError> {
    match t_type {
        #[cfg(feature = "vosk")]
        TranscriberType::Vosk => Ok(Box::new(VoskTranscriber::new(
            &config.model_path,
            config.sample_rate as f32,
        )?)),
        #[cfg(feature = "whisper")]
        TranscriberType::Whisper => Ok(Box::new(WhisperTranscriber::new(&config.whisper_model_path)?)),
        #[cfg(any(feature = "gigaam", feature = "gigaam-system-ort"))]
        TranscriberType::GigaAm => Ok(Box::new(crate::gigaam::transcriber::GigaAmTranscriber::new(
            &config.gigaam_model_path,
        )?)),
        #[cfg(any(feature = "gigaam", feature = "gigaam-system-ort"))]
        TranscriberType::GigaAmRnnt => Ok(Box::new(crate::gigaam::transcriber_rnnt::GigaAmRnntTranscriber::new(
            &config.gigaam_rnnt_model_path,
        )?)),
        #[cfg(feature = "qwen3asr")]
        TranscriberType::Qwen3Asr => Ok(Box::new(crate::qwen3asr::transcriber::Qwen3AsrTranscriber::new(
            &config.qwen3asr_model_path,
        )?)),
        // Любая ветка без своего feature: сообщаем, что движок недоступен.
        #[allow(unreachable_patterns)]
        other => Err(ArcanaError::EngineNotAvailable(other.as_str().to_string())),
    }
}

/// Трейт для движков транскрибации (Vosk, Whisper и т.д.)
pub trait Transcriber: Send + Sync {
    /// Транскрибирует аудио (i16 сэмплы, mono)
    fn transcribe(&self, samples: &[i16], sample_rate: u32) -> Result<String, ArcanaError>;

    /// Поддерживает ли потоковую обработку (partial results в реальном времени)
    fn supports_streaming(&self) -> bool;

    /// Потоковая обработка сэмплов — вызывается из audio callback (только Vosk)
    fn accept_waveform(&self, _samples: &[i16]) -> Result<(), ArcanaError> {
        Ok(())
    }

    /// Получить промежуточный результат (только Vosk, debug mode)
    fn partial_result(&self) -> String {
        String::new()
    }

    /// Сброс состояния между записями
    fn reset(&self) {}

    /// Поддерживает ли движок прерывание идущей транскрибации.
    /// `true` — есть рабочий `cancel()`, UI покажет кнопку «Стоп» во время инференса.
    /// Сейчас только Whisper (через `whisper_full_params.abort_callback`); Vosk и
    /// ORT-based (GigaAM/Qwen3-ASR) — `cancel()` no-op.
    fn supports_cancel(&self) -> bool {
        false
    }

    /// Сигнализировать запущенному инференсу остановиться. Send+Sync, вызывается
    /// из другого потока (UI thread) пока transcribe() работает в worker'е.
    /// Дефолт — no-op (для движков без API прерывания).
    fn cancel(&self) {}
}

/// Слова-паразиты для удаления (сравнение в нижнем регистре, по целым словам)
const FILLER_WORDS: &[&str] = &["э", "э-э", "э-э-э", "э-ээ", "ээ", "эээ", "эм", "мм", "ммм"];

/// Удаляет слова-паразиты из транскрибации.
/// Сравнивает по целым словам в нижнем регистре, чтобы не повредить нормальные слова.
pub(crate) fn remove_filler_words(text: &str) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    let filtered: Vec<&str> = words
        .into_iter()
        .filter(|word| {
            // Убираем пунктуацию с краёв для сравнения (чтобы "э," тоже удалялось)
            let clean = word.trim_matches(|c: char| c.is_ascii_punctuation() || c == '—' || c == '–');
            let lower = clean.to_lowercase();
            !FILLER_WORDS.contains(&lower.as_str())
        })
        .collect();
    let result = filtered.join(" ");
    // Убираем двойные пробелы и лишние запятые/точки в начале
    result.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_remove_fillers_basic() {
        assert_eq!(remove_filler_words("э привет э-э как дела"), "привет как дела");
    }

    #[test]
    fn test_remove_fillers_case_insensitive() {
        assert_eq!(remove_filler_words("Э привет Ээ мир"), "привет мир");
    }

    #[test]
    fn test_remove_fillers_preserves_normal_words() {
        // "это", "эхо", "нужно" не должны быть затронуты
        assert_eq!(remove_filler_words("это эхо нужно"), "это эхо нужно");
    }

    #[test]
    fn test_remove_fillers_with_punctuation() {
        // "э," — филлер с запятой, должен быть удалён
        assert_eq!(remove_filler_words("э, привет мм, мир"), "привет мир");
    }

    #[test]
    fn test_remove_fillers_empty_result() {
        assert_eq!(remove_filler_words("э э-э мм"), "");
    }

    #[test]
    fn test_remove_fillers_extended() {
        // "э-э-э" и "э-ээ" — новые слова-паразиты
        assert_eq!(remove_filler_words("э-э-э привет э-ээ мир"), "привет мир");
        assert_eq!(remove_filler_words("Э-Э-Э тест Э-ЭЭ"), "тест");
    }
}
