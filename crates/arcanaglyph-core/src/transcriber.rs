// crates/arcanaglyph-core/src/transcriber.rs

use crate::error::ArcanaError;
use std::sync::Mutex;

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
}

// ─── Vosk ──────────────────────────────────────────────────────────────────

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
        let model = vosk::Model::new(model_path_str)
            .ok_or_else(|| ArcanaError::ModelLoad(format!("Не удалось загрузить Vosk-модель из: {}", model_path_str)))?;
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
        let mut rec = self.recognizer.lock().map_err(|e| ArcanaError::Internal(format!("Mutex отравлен: {}", e)))?;

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
        let mut rec = self.recognizer.lock().map_err(|e| ArcanaError::Internal(format!("Mutex отравлен: {}", e)))?;
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

// ─── Whisper ───────────────────────────────────────────────────────────────

/// Транскрайбер на основе Whisper (медленнее, значительно точнее)
pub struct WhisperTranscriber {
    ctx: whisper_rs::WhisperContext,
}

impl WhisperTranscriber {
    /// Создаёт WhisperTranscriber: загружает модель ggml
    pub fn new(model_path: &std::path::Path) -> Result<Self, ArcanaError> {
        let model_path_str = model_path
            .to_str()
            .ok_or_else(|| ArcanaError::ModelLoad("Невалидный путь к модели (не UTF-8)".into()))?;

        // Подавляем встроенные логи whisper.cpp — перенаправляем через log crate
        whisper_rs::install_whisper_log_trampoline();

        tracing::info!("Загрузка Whisper-модели из: {:?}", model_path);
        let ctx = whisper_rs::WhisperContext::new_with_params(
            model_path_str,
            whisper_rs::WhisperContextParameters::default(),
        )
        .map_err(|e| ArcanaError::ModelLoad(format!("Не удалось загрузить Whisper-модель из {}: {}", model_path_str, e)))?;

        tracing::info!("Whisper-модель успешно загружена.");
        Ok(Self { ctx })
    }
}

impl Transcriber for WhisperTranscriber {
    fn transcribe(&self, samples: &[i16], sample_rate: u32) -> Result<String, ArcanaError> {
        // Обрезаем тишину с обеих сторон — Whisper галлюцинирует на тихих участках,
        // а короткое аудио = быстрее транскрибация
        let trimmed = trim_silence(samples, sample_rate);

        // Конвертируем i16 → f32 (нормализация в [-1.0, 1.0])
        let mut audio_f32: Vec<f32> = trimmed.iter().map(|&s| s as f32 / 32768.0).collect();

        // Ресемплируем до 16 kHz если нужно (Whisper требует 16000 Hz)
        if sample_rate != 16000 {
            audio_f32 = resample(&audio_f32, sample_rate, 16000);
        }

        let mut state = self.ctx.create_state()
            .map_err(|e| ArcanaError::Recognizer(format!("Не удалось создать Whisper state: {}", e)))?;

        let mut params = whisper_rs::FullParams::new(whisper_rs::SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some("ru"));
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_print_special(false);
        // Подавляем галлюцинации Whisper (мусорный текст при тишине в конце записи)
        params.set_suppress_blank(true);
        params.set_suppress_non_speech_tokens(true);
        // Не добавлять контекст из предыдущих сегментов (уменьшает галлюцинации)
        params.set_no_context(true);
        params.set_n_threads(std::thread::available_parallelism().map(|n| n.get() as i32).unwrap_or(4));

        state.full(params, &audio_f32)
            .map_err(|e| ArcanaError::Recognizer(format!("Ошибка транскрибации Whisper: {}", e)))?;

        let num_segments = state.full_n_segments()
            .map_err(|e| ArcanaError::Recognizer(format!("Ошибка получения сегментов: {}", e)))?;

        let mut text = String::new();
        for i in 0..num_segments {
            if let Ok(segment) = state.full_get_segment_text(i) {
                text.push_str(&segment);
            }
        }

        let mut result = text.trim().to_string();

        // Удаляем типичные галлюцинации Whisper в конце текста
        let hallucinations = [
            "Продолжение следует...",
            "Продолжение следует.",
            "Продолжение следует",
            "Спасибо за просмотр.",
            "Спасибо за просмотр!",
            "Субтитры сделал DimaTorzworworworworworwor",
        ];
        for h in &hallucinations {
            if result.ends_with(h) {
                result.truncate(result.len() - h.len());
                result = result.trim_end().to_string();
                tracing::info!("Удалена галлюцинация Whisper: '{}'", h);
                break;
            }
        }

        Ok(result)
    }

    fn supports_streaming(&self) -> bool {
        false
    }
}

/// Обрезает тишину с обеих сторон аудио.
/// Whisper галлюцинирует на тихих участках, а короткое аудио = быстрее транскрибация.
fn trim_silence(samples: &[i16], sample_rate: u32) -> &[i16] {
    let block_size = sample_rate as usize / 10; // 100 мс блоки
    let threshold: f64 = 50.0; // RMS порог (тишина < 50)
    let padding = sample_rate as usize / 5; // 200 мс отступ для естественного затухания

    if samples.len() < block_size {
        return samples;
    }

    // Ищем начало речи (с начала)
    let mut start = 0;
    let mut pos = 0;
    while pos + block_size <= samples.len() {
        let block = &samples[pos..pos + block_size];
        let sum_sq: f64 = block.iter().map(|&s| (s as f64) * (s as f64)).sum();
        let rms = (sum_sq / block.len() as f64).sqrt();
        if rms > threshold {
            start = pos;
            break;
        }
        pos += block_size;
    }

    // Ищем конец речи (с конца)
    let mut end = samples.len();
    pos = samples.len();
    while pos > block_size {
        pos -= block_size;
        let block = &samples[pos..pos + block_size];
        let sum_sq: f64 = block.iter().map(|&s| (s as f64) * (s as f64)).sum();
        let rms = (sum_sq / block.len() as f64).sqrt();
        if rms > threshold {
            end = pos + block_size;
            break;
        }
    }

    // Отступы: 200 мс до начала и после конца речи
    let start = start.saturating_sub(padding);
    let end = (end + padding).min(samples.len());

    if start >= end {
        return samples;
    }

    let trimmed_duration_ms = (end - start) * 1000 / sample_rate as usize;
    let original_duration_ms = samples.len() * 1000 / sample_rate as usize;
    if trimmed_duration_ms < original_duration_ms {
        tracing::info!(
            "Обрезка тишины: {}мс → {}мс (убрано {}мс)",
            original_duration_ms,
            trimmed_duration_ms,
            original_duration_ms - trimmed_duration_ms
        );
    }

    &samples[start..end]
}

/// Простой ресемплинг через линейную интерполяцию
fn resample(input: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate || input.is_empty() {
        return input.to_vec();
    }

    let ratio = from_rate as f64 / to_rate as f64;
    let output_len = (input.len() as f64 / ratio) as usize;
    let mut output = Vec::with_capacity(output_len);

    for i in 0..output_len {
        let src_idx = i as f64 * ratio;
        let idx = src_idx as usize;
        let frac = src_idx - idx as f64;

        let sample = if idx + 1 < input.len() {
            input[idx] as f64 * (1.0 - frac) + input[idx + 1] as f64 * frac
        } else {
            input[idx] as f64
        };

        output.push(sample as f32);
    }

    output
}
