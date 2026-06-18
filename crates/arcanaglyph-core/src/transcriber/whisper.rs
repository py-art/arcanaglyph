// crates/arcanaglyph-core/src/transcriber/whisper.rs
//
// Транскрайбер на основе Whisper (медленнее, значительно точнее).

use super::Transcriber;
use crate::dsp::preprocess_to_f32_16k;
use crate::error::ArcanaError;

/// Транскрайбер на основе Whisper (медленнее, значительно точнее)
pub struct WhisperTranscriber {
    ctx: whisper_rs::WhisperContext,
    /// Флаг отмены — устанавливается из `cancel()` (UI thread), читается из
    /// abort_callback в горячей петле whisper.cpp. AtomicBool/Relaxed достаточно:
    /// нам не нужны ordering-гарантии относительно других переменных, нужно лишь
    /// чтобы значение в конечном счёте дошло до читателя — а у whisper.cpp эта
    /// горячая петля проверяет сотни раз в секунду.
    cancel_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

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

        // Обрезаем тишину (Whisper галлюцинирует на тихих участках), нормализуем
        // i16 → f32 и ресемплируем до 16 кГц — общий препроцессинг для всех движков.
        let audio_f32 = preprocess_to_f32_16k(samples, sample_rate);

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
            if let Some(segment) = state.get_segment(i)
                && let Ok(s) = segment.to_str_lossy()
            {
                text.push_str(&s);
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
}
