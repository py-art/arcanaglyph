// crates/arcanaglyph-core/src/dsp/preprocess.rs
//
// Общий входной аудио-препроцессинг для «тяжёлых» движков (whisper + mel):
// обрезка тишины по RMS → i16→f32 нормализация → resample до 16 кГц.
// Vosk это НЕ использует (работает с i16 напрямую).

/// Обрезает тишину с обеих сторон аудио.
/// Whisper/GigaAM галлюцинируют на тихих участках, а короткое аудио = быстрее транскрибация.
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

/// Общий входной препроцессинг для mel/whisper-движков:
/// `trim_silence` → i16→f32 нормализация в [-1.0, 1.0] → resample до 16 кГц.
/// Возвращает f32-сэмплы при 16 кГц, готовые к mel/FFT.
/// Vosk это НЕ использует (работает с i16 напрямую). Порог минимальной длины
/// (`< 320`/`< 400`) проверяет caller — он зависит от n_fft конкретного движка.
pub(crate) fn preprocess_to_f32_16k(samples: &[i16], sample_rate: u32) -> Vec<f32> {
    let trimmed = trim_silence(samples, sample_rate);
    let audio_f32: Vec<f32> = trimmed.iter().map(|&s| s as f32 / 32768.0).collect();
    if sample_rate != 16000 {
        resample(&audio_f32, sample_rate, 16000)
    } else {
        audio_f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_preprocess_same_rate_normalizes() {
        // Громкий постоянный сигнал (rms выше порога тишины) при 16 кГц:
        // длина сохраняется, значения нормализованы i16 → f32.
        let samples = vec![1000i16; 16000];
        let out = preprocess_to_f32_16k(&samples, 16000);
        assert_eq!(out.len(), 16000);
        assert!((out[0] - 1000.0 / 32768.0).abs() < 1e-6);
    }

    #[test]
    fn test_preprocess_resamples_8k_to_16k() {
        // 8 кГц → 16 кГц: число сэмплов примерно удваивается.
        let samples = vec![1000i16; 8000];
        let out = preprocess_to_f32_16k(&samples, 8000);
        assert!(out.len() > 14000, "ожидали ~16000, получили {}", out.len());
    }

    // === resample (линейная интерполяция) ===

    #[test]
    fn test_resample_noop_same_rate_and_empty() {
        // Равные частоты и пустой вход — возврат без изменений (быстрый путь).
        assert_eq!(resample(&[1.0, 2.0, 3.0], 16000, 16000), vec![1.0, 2.0, 3.0]);
        assert_eq!(resample(&[], 8000, 16000), Vec::<f32>::new());
    }

    #[test]
    fn test_resample_up_and_down_length() {
        // Апсемплинг ×2 и даунсемплинг ÷2 дают предсказуемую длину выхода.
        assert_eq!(resample(&vec![1.0; 100], 8000, 16000).len(), 200);
        assert_eq!(resample(&vec![1.0; 100], 16000, 8000).len(), 50);
    }

    #[test]
    fn test_resample_interpolates_midpoint() {
        // 8→16 кГц на [0, 10]: новый сэмпл между точками — среднее (линейная интерп.).
        let out = resample(&[0.0, 10.0], 8000, 16000);
        assert_eq!(out, vec![0.0, 5.0, 10.0, 10.0]);
    }

    // === trim_silence (обрезка тишины по RMS) ===

    #[test]
    fn test_trim_silence_short_audio_untouched() {
        // Короче одного 100мс-блока — возвращается как есть (нечего анализировать).
        let samples = vec![0i16; 100];
        assert_eq!(trim_silence(&samples, 16000).len(), 100);
    }

    #[test]
    fn test_trim_silence_all_silence_returns_full() {
        // Сплошная тишина: речь не найдена → защитный возврат всего буфера.
        let samples = vec![0i16; 16000];
        assert_eq!(trim_silence(&samples, 16000).len(), 16000);
    }

    #[test]
    fn test_trim_silence_trims_edges_with_padding() {
        // 1с при 16кГц: тишина [0,4800) + речь [4800,11200) + тишина [11200,16000).
        // block=1600, padding=3200 → start 4800-3200=1600, end 11200+3200=14400.
        let mut samples = vec![0i16; 16000];
        for s in samples.iter_mut().take(11200).skip(4800) {
            *s = 1000;
        }
        let trimmed = trim_silence(&samples, 16000);
        assert_eq!(trimmed.len(), 12800);
        assert!(trimmed.len() < samples.len());
    }
}
