// crates/arcanaglyph-core/src/transcriber.rs

use crate::error::ArcanaError;
#[cfg(feature = "vosk")]
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

// ─── Vosk ──────────────────────────────────────────────────────────────────

/// Транскрайбер на основе Vosk (быстрый, потоковый, менее точный)
#[cfg(feature = "vosk")]
pub struct VoskTranscriber {
    recognizer: Mutex<vosk::Recognizer>,
}

#[cfg(feature = "vosk")]
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

#[cfg(feature = "vosk")]
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

// ─── Whisper ───────────────────────────────────────────────────────────────

/// Транскрайбер на основе Whisper (медленнее, значительно точнее)
#[cfg(feature = "whisper")]
pub struct WhisperTranscriber {
    ctx: whisper_rs::WhisperContext,
    /// Флаг отмены — устанавливается из `cancel()` (UI thread), читается из
    /// abort_callback в горячей петле whisper.cpp. AtomicBool/Relaxed достаточно:
    /// нам не нужны ordering-гарантии относительно других переменных, нужно лишь
    /// чтобы значение в конечном счёте дошло до читателя — а у whisper.cpp эта
    /// горячая петля проверяет сотни раз в секунду.
    cancel_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

#[cfg(feature = "whisper")]
impl WhisperTranscriber {
    /// Создаёт WhisperTranscriber: загружает модель ggml
    pub fn new(model_path: &std::path::Path) -> Result<Self, ArcanaError> {
        let model_path_str = model_path
            .to_str()
            .ok_or_else(|| ArcanaError::ModelLoad("Невалидный путь к модели (не UTF-8)".into()))?;

        // whisper-rs 0.16: ОБЯЗАТЕЛЬНО вызвать `install_logging_hooks()` чтобы
        // whisper.cpp+ggml внутренние сообщения роутились через `tracing` (target
        // `whisper_rs::*`). Без этого они печатаются напрямую в stderr минуя
        // фильтр и засоряют логи. Функция идемпотентная (Once-guard внутри).
        whisper_rs::install_logging_hooks();
        tracing::info!("Загрузка Whisper-модели из: {:?}", model_path);
        let ctx = whisper_rs::WhisperContext::new_with_params(
            model_path_str,
            whisper_rs::WhisperContextParameters::default(),
        )
        .map_err(|e| {
            ArcanaError::ModelLoad(format!(
                "Не удалось загрузить Whisper-модель из {}: {}",
                model_path_str, e
            ))
        })?;

        tracing::info!("Whisper-модель успешно загружена.");
        Ok(Self {
            ctx,
            cancel_flag: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        })
    }
}

#[cfg(feature = "whisper")]
impl Transcriber for WhisperTranscriber {
    /// Whisper не поддерживает корректное прерывание идущего инференса:
    /// `whisper_full_params.abort_callback` дал UB на Intel Atom Tremont CPU
    /// (см. подробный комментарий в `transcribe()`), а GGML-уровневого abort нет.
    /// Cancel реально работает только пост-фактум — устанавливаем флаг, после того
    /// как `state.full()` вернётся, проверяем его и возвращаем `Cancelled` чтобы UI
    /// не показывал «нежеланный» результат. Но CPU при этом не освобождается до
    /// конца natural-инференса. UI кнопку Стоп для Whisper не показывает.
    fn supports_cancel(&self) -> bool {
        false
    }

    fn cancel(&self) {
        self.cancel_flag.store(true, std::sync::atomic::Ordering::Relaxed);
        tracing::info!("Whisper транскрибация: cancel-флаг выставлен (post-encoder discard)");
    }

    fn transcribe(&self, samples: &[i16], sample_rate: u32) -> Result<String, ArcanaError> {
        use std::sync::atomic::Ordering;
        // Сбрасываем cancel-флаг в начале каждой транскрибации — иначе если в прошлый
        // раз был отменён, новый прогон сразу прервётся.
        self.cancel_flag.store(false, Ordering::Relaxed);

        // Обрезаем тишину с обеих сторон — Whisper галлюцинирует на тихих участках,
        // а короткое аудио = быстрее транскрибация
        let trimmed = trim_silence(samples, sample_rate);

        // Конвертируем i16 → f32 (нормализация в [-1.0, 1.0])
        let mut audio_f32: Vec<f32> = trimmed.iter().map(|&s| s as f32 / 32768.0).collect();

        // Ресемплируем до 16 kHz если нужно (Whisper требует 16000 Hz)
        if sample_rate != 16000 {
            audio_f32 = resample(&audio_f32, sample_rate, 16000);
        }

        let mut state = self
            .ctx
            .create_state()
            .map_err(|e| ArcanaError::Recognizer(format!("Не удалось создать Whisper state: {}", e)))?;

        let mut params = whisper_rs::FullParams::new(whisper_rs::SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some("ru"));
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_print_special(false);
        // Подавляем галлюцинации Whisper (мусорный текст при тишине в конце записи).
        // В whisper-rs 0.16 переименованы: `set_suppress_non_speech_tokens` → `set_suppress_nst`.
        params.set_suppress_blank(true);
        params.set_suppress_nst(true);
        // Не добавлять контекст из предыдущих сегментов (уменьшает галлюцинации)
        params.set_no_context(true);
        // n_threads: по умолчанию = число ядер CPU, но можно перебить через env
        // `WHISPER_THREADS=N` (полезно для отладки ggml-проблем на конкретных CPU).
        let n_threads = std::env::var("WHISPER_THREADS")
            .ok()
            .and_then(|s| s.parse::<i32>().ok())
            .unwrap_or_else(|| {
                std::thread::available_parallelism()
                    .map(|n| n.get() as i32)
                    .unwrap_or(4)
            });
        params.set_n_threads(n_threads);

        // ВАЖНО: НЕ ставим `set_abort_callback_safe`. Эмпирически на Intel Atom Tremont
        // (Celeron N5095, без AVX) whisper-rs 0.13/0.16 trampoline даёт UB:
        // `Box::into_raw` теряет ownership без free, whisper.cpp читает garbage в
        // `abort_callback_user_data` и считает что abort==true → encoder аборится с
        // `whisper_full_with_state: failed to encode` (Error code: -6). На AVX-CPU
        // garbage случайно ведёт себя как false и проблема не проявляется. Убираем
        // abort_callback совсем; cancel-логика работает только пост-фактум через
        // `aborted_check` ниже (Cancelled error если флаг был установлен после full()).
        // Для UI это нормально: stop-button во время whisper-инференса всё равно скрыт
        // (см. dist/index.html — abort_callback в whisper.cpp вызывается только между
        // decode-сегментами, а на slow-CPU encoder длится десятки секунд непрерывно).

        let aborted_check = std::sync::Arc::clone(&self.cancel_flag);
        state.full(params, &audio_f32).map_err(|e| {
            if aborted_check.load(Ordering::Relaxed) {
                ArcanaError::Cancelled
            } else {
                ArcanaError::Recognizer(format!("Ошибка транскрибации Whisper: {}", e))
            }
        })?;
        // whisper.cpp's `abort_callback` опрашивается только между decode-сегментами;
        // во время encoder-прохода (на Large моделях это десятки секунд на безAVX2-CPU)
        // он не вызывается. Если пользователь нажал Стоп во время encoder'а, abort
        // не сработал, и `state.full()` завершилось нормально. Перепроверяем флаг
        // здесь — отбрасываем результат, UI получает Cancelled (без error toast).
        // Загрузку CPU это не останавливает (whisper уже доработал), но хотя бы
        // освобождает UI и не показывает «нежеланный» результат.
        if aborted_check.load(Ordering::Relaxed) {
            return Err(ArcanaError::Cancelled);
        }

        // whisper-rs 0.16: full_n_segments() возвращает c_int напрямую (без Result),
        // get_segment(i) возвращает Option<WhisperSegment> с методом .to_str().
        let num_segments = state.full_n_segments();

        let mut text = String::new();
        for i in 0..num_segments {
            if let Some(segment) = state.get_segment(i) {
                if let Ok(s) = segment.to_str_lossy() {
                    text.push_str(&s);
                }
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
/// Whisper/GigaAM галлюцинируют на тихих участках, а короткое аудио = быстрее транскрибация.
#[cfg(any(
    feature = "whisper",
    feature = "gigaam",
    feature = "gigaam-system-ort",
    feature = "gigaam-tract",
    feature = "qwen3asr"
))]
pub(crate) fn trim_silence(samples: &[i16], sample_rate: u32) -> &[i16] {
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

/// Простой ресемплинг через линейную интерполяцию
#[cfg(any(
    feature = "whisper",
    feature = "gigaam",
    feature = "gigaam-system-ort",
    feature = "gigaam-tract",
    feature = "qwen3asr"
))]
pub(crate) fn resample(input: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
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
