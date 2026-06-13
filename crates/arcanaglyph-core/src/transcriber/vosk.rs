// crates/arcanaglyph-core/src/transcriber/vosk.rs
//
// Транскрайбер на основе Vosk (быстрый, потоковый, менее точный).

use std::sync::Mutex;

use super::Transcriber;
use crate::error::ArcanaError;

/// Транскрайбер на основе Vosk (быстрый, потоковый, менее точный)
pub struct VoskTranscriber {
    recognizer: Mutex<vosk::Recognizer>,
}

impl VoskTranscriber {
    /// Создаёт VoskTranscriber: загружает модель и инициализирует распознаватель
    pub fn new(model_path: &std::path::Path, sample_rate: f32) -> Result<Self, ArcanaError> {
        vosk::set_log_level(vosk::LogLevel::Error);

        let model_path_str = model_path
            .to_str()
            .ok_or_else(|| ArcanaError::ModelLoad("Невалидный путь к модели (не UTF-8)".into()))?;

        tracing::info!("Загрузка Vosk-модели из: {:?}", model_path);
        let model = vosk::Model::new(model_path_str).ok_or_else(|| {
            ArcanaError::ModelLoad(format!("Не удалось загрузить Vosk-модель из: {}", model_path_str))
        })?;
        tracing::info!("Vosk-модель успешно загружена.");

        let recognizer = vosk::Recognizer::new(&model, sample_rate)
            .ok_or_else(|| ArcanaError::Recognizer("Не удалось создать Vosk-распознаватель".into()))?;

        Ok(Self {
            recognizer: Mutex::new(recognizer),
        })
    }
}

impl Transcriber for VoskTranscriber {
    fn transcribe(&self, samples: &[i16], _sample_rate: u32) -> Result<String, ArcanaError> {
        let mut rec = self
            .recognizer
            .lock()
            .map_err(|e| ArcanaError::Internal(format!("Mutex отравлен: {}", e)))?;

        // Прогоняем все сэмплы (нужно для retranscribe, когда данные не шли через accept_waveform)
        if !samples.is_empty() {
            rec.accept_waveform(samples)
                .map_err(|e| ArcanaError::Recognizer(format!("Ошибка обработки аудио: {:?}", e)))?;
        }

        let final_result = rec
            .final_result()
            .single()
            .ok_or_else(|| ArcanaError::Recognizer("Не удалось получить результат распознавания".into()))?;

        let text = final_result.text.to_string();
        Ok(text)
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn accept_waveform(&self, samples: &[i16]) -> Result<(), ArcanaError> {
        let mut rec = self
            .recognizer
            .lock()
            .map_err(|e| ArcanaError::Internal(format!("Mutex отравлен: {}", e)))?;
        rec.accept_waveform(samples)
            .map_err(|e| ArcanaError::Recognizer(format!("Ошибка при обработке аудиоданных: {:?}", e)))?;
        Ok(())
    }

    fn partial_result(&self) -> String {
        if let Ok(mut rec) = self.recognizer.lock() {
            rec.partial_result().partial.to_string()
        } else {
            String::new()
        }
    }

    fn reset(&self) {
        if let Ok(mut rec) = self.recognizer.lock() {
            rec.reset();
        }
    }
}
