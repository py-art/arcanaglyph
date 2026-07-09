// crates/arcanaglyph-core/src/qwen3asr/mel.rs
//
// Mel-спектрограмма для Qwen3-ASR (Whisper-совместимая).
// Параметры: 16kHz, 128 mel bins, n_fft=400, hop=160, Slaney norm, center=true.

use ndarray::{Array1, Array2};

use crate::dsp::{self, StftConfig};

const SAMPLE_RATE: u32 = 16000;
const N_FFT: usize = 400;
const HOP_LENGTH: usize = 160;
const N_MELS: usize = 128;

/// Вычисляет Whisper-совместимую log-mel спектрограмму.
/// Вход: f32 сэмплы при 16kHz. Выход: [128, T_frames].
pub fn compute_mel_spectrogram(samples: &[f32]) -> Array2<f32> {
    if samples.len() < N_FFT {
        return Array2::zeros((N_MELS, 0));
    }

    let filterbank = mel_filterbank_slaney();

    // STFT-ядро вынесено в dsp (общее с GigaAM). Qwen3-ASR — с center reflect-паддингом.
    let window = dsp::hann_window(N_FFT);
    let cfg = StftConfig {
        n_fft: N_FFT,
        hop_length: HOP_LENGTH,
        win_length: N_FFT,
        center: true,
    };
    // power_spec: канонический layout [n_bins, n_frames].
    let power_spec = dsp::stft_power(samples, &window, &cfg);

    // Mel filterbank → log scale (Whisper-style)
    let mut mel_spec = filterbank.dot(&power_spec);

    // log10(max(mel, 1e-10))
    mel_spec.mapv_inplace(|x| x.max(1e-10).log10());

    // Dynamic range: max(x, max - 8.0)
    let max_val = mel_spec.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let min_val = max_val - 8.0;
    mel_spec.mapv_inplace(|x| x.max(min_val));

    // Нормализация: (x + 4.0) / 4.0
    mel_spec.mapv_inplace(|x| (x + 4.0) / 4.0);

    mel_spec
}

// Slaney (non-HTK) mel scale: линейно до 1000 Гц, лог-шкала выше.
const SLANEY_F_SP: f32 = 200.0 / 3.0;
const SLANEY_MIN_LOG_HZ: f32 = 1000.0;

/// Slaney mel scale: Hz → mel. Чистая функция (тестируется roundtrip'ом).
fn slaney_hz_to_mel(hz: f32) -> f32 {
    let min_log_mel = SLANEY_MIN_LOG_HZ / SLANEY_F_SP;
    let logstep = (6.4f32).ln() / 27.0;
    if hz < SLANEY_MIN_LOG_HZ {
        hz / SLANEY_F_SP
    } else {
        min_log_mel + (hz / SLANEY_MIN_LOG_HZ).ln() / logstep
    }
}

/// Обратное `slaney_hz_to_mel`: mel → Hz. Чистая функция.
fn slaney_mel_to_hz(mel: f32) -> f32 {
    let min_log_mel = SLANEY_MIN_LOG_HZ / SLANEY_F_SP;
    let logstep = (6.4f32).ln() / 27.0;
    if mel < min_log_mel {
        mel * SLANEY_F_SP
    } else {
        SLANEY_MIN_LOG_HZ * ((mel - min_log_mel) * logstep).exp()
    }
}

/// Заполняет одну mel-строку filterbank треугольным окном [left, center, right]
/// с Slaney-нормализацией (деление на ширину полосы). Вынесено из цикла
/// `mel_filterbank_slaney` — убирает вложенность и снижает сложность.
fn fill_triangular_mel_filter(
    mut row: ndarray::ArrayViewMut1<f32>,
    fft_freqs: &Array1<f32>,
    left: f32,
    center: f32,
    right: f32,
) {
    for (bin, &freq) in fft_freqs.iter().enumerate() {
        if freq >= left && freq <= center && center > left {
            row[bin] = (freq - left) / (center - left);
        } else if freq > center && freq <= right && right > center {
            row[bin] = (right - freq) / (right - center);
        }
    }
    // Slaney normalization: делим на ширину полосы.
    let enorm = 2.0 / (right - left);
    for v in row.iter_mut() {
        *v *= enorm;
    }
}

/// Slaney mel filterbank: [128, 201]
/// fmin=0, fmax=8000, norm="slaney", htk=false
fn mel_filterbank_slaney() -> Array2<f32> {
    let n_bins = N_FFT / 2 + 1; // 201
    let fmin = 0.0f32;
    let fmax = (SAMPLE_RATE / 2) as f32; // 8000

    let mel_min = slaney_hz_to_mel(fmin);
    let mel_max = slaney_hz_to_mel(fmax);

    // n_mels + 2 точки
    let mel_points: Vec<f32> = (0..=N_MELS + 1)
        .map(|i| mel_min + (mel_max - mel_min) * i as f32 / (N_MELS + 1) as f32)
        .collect();
    let hz_points: Vec<f32> = mel_points.iter().map(|&m| slaney_mel_to_hz(m)).collect();

    // FFT bin frequencies
    let fft_freqs: Array1<f32> = Array1::from_vec(
        (0..n_bins)
            .map(|i| i as f32 * SAMPLE_RATE as f32 / N_FFT as f32)
            .collect(),
    );

    let mut filterbank = Array2::zeros((N_MELS, n_bins));
    for mel in 0..N_MELS {
        fill_triangular_mel_filter(
            filterbank.row_mut(mel),
            &fft_freqs,
            hz_points[mel],
            hz_points[mel + 1],
            hz_points[mel + 2],
        );
    }
    filterbank
}

/// Вычисляет длину выхода после 3x stride-2 свёрток
pub fn feat_extract_output_lengths(input_length: usize) -> usize {
    let mut l = input_length;
    for _ in 0..3 {
        l = (l - 1) / 2 + 1;
    }
    l
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mel_output_shape() {
        // 1 секунда при 16kHz
        let samples = vec![0.0f32; 16000];
        let mel = compute_mel_spectrogram(&samples);
        assert_eq!(mel.shape()[0], 128);
        // n_frames = (16000 + 400 - 400) / 160 + 1 = 101
        assert_eq!(mel.shape()[1], 101);
    }

    #[test]
    fn test_feat_output_lengths() {
        assert_eq!(feat_extract_output_lengths(100), 13);
        assert_eq!(feat_extract_output_lengths(50), 7);
    }

    #[test]
    fn test_slaney_filterbank_shape() {
        let fb = mel_filterbank_slaney();
        assert_eq!(fb.shape(), &[128, 201]);
        // Все значения >= 0
        assert!(fb.iter().all(|&v| v >= 0.0));
    }

    #[test]
    fn test_slaney_mel_hz_roundtrip() {
        // Hz → mel → Hz возвращает исходную частоту на обеих ветвях шкалы
        // (линейной < 1000 Гц и логарифмической выше).
        for &hz in &[0.0f32, 250.0, 999.0, 1000.0, 4000.0, 8000.0] {
            let back = slaney_mel_to_hz(slaney_hz_to_mel(hz));
            assert!((back - hz).abs() < 1.0, "roundtrip {hz} → {back}");
        }
    }
}
